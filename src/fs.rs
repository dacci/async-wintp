use crate::io::{Error, OverlappedCb, PoolIo, Result};
use crate::util::FutureState;
use std::fs::{File as StdFile, OpenOptions as StdOpenOptions};
use std::ops::DerefMut;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use windows_sys::Win32::Foundation::ERROR_IO_PENDING;
use windows_sys::Win32::Storage::FileSystem::*;

#[repr(transparent)]
pub struct OpenOptions(StdOpenOptions);

impl OpenOptions {
    pub fn new() -> Self {
        let mut opts = Self(StdOpenOptions::new());

        opts.custom_flags(FILE_FLAG_OVERLAPPED);

        opts
    }

    pub fn read(&mut self, read: bool) -> &mut Self {
        self.0.read(read);
        self
    }

    pub fn write(&mut self, write: bool) -> &mut Self {
        self.0.write(write);
        self
    }

    pub fn append(&mut self, append: bool) -> &mut Self {
        self.0.append(append);
        self
    }

    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.0.truncate(truncate);
        self
    }

    pub fn create(&mut self, create: bool) -> &mut Self {
        self.0.create(create);
        self
    }

    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.0.create_new(create_new);
        self
    }

    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File> {
        let handle = self.0.open(path)?;

        let io = crate::runtime::current()
            .expect("not running in a runtime")
            .create_io(handle.as_raw_handle(), move |ov, res| {
                let ov = unsafe { ov.cast::<OverlappedCb>().as_ref_unchecked() };
                ov.call(res);
            })?;

        Ok(File { handle, io, pos: 0 })
    }
}

impl OpenOptionsExt for OpenOptions {
    fn access_mode(&mut self, access: u32) -> &mut Self {
        self.0.access_mode(access);
        self
    }

    fn share_mode(&mut self, val: u32) -> &mut Self {
        self.0.share_mode(val);
        self
    }

    fn custom_flags(&mut self, flags: u32) -> &mut Self {
        self.0.custom_flags(flags | FILE_FLAG_OVERLAPPED);
        self
    }

    fn attributes(&mut self, val: u32) -> &mut Self {
        self.0.attributes(val);
        self
    }

    fn security_qos_flags(&mut self, flags: u32) -> &mut Self {
        self.0.security_qos_flags(flags);
        self
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub struct File {
    handle: StdFile,
    io: PoolIo,
    pos: u64,
}

impl File {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::options().read(true).open(path).await
    }

    pub async fn create(path: impl AsRef<Path>) -> Result<Self> {
        Self::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
    }

    pub async fn create_new(path: impl AsRef<Path>) -> Result<Self> {
        Self::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .await
    }

    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    pub async fn sync_all(&self) -> Result<()> {
        match unsafe { FlushFileBuffers(self.handle.as_raw_handle()) } {
            0 => Err(Error::last_os_error()),
            _ => Ok(()),
        }
    }

    pub async fn sync_data(&self) -> Result<()> {
        self.sync_all().await
    }

    pub async fn set_len(&self, size: u64) -> Result<()> {
        let info = FILE_END_OF_FILE_INFO {
            EndOfFile: size as _,
        };

        match unsafe {
            SetFileInformationByHandle(
                self.handle.as_raw_handle(),
                FileEndOfFileInfo,
                &info as *const _ as _,
                size_of::<FILE_END_OF_FILE_INFO>() as _,
            )
        } {
            0 => Err(Error::last_os_error()),
            _ => Ok(()),
        }
    }

    pub fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> Read<'a> {
        let inner = Arc::new(Mutex::new(ReadInner {
            f: self,
            buf,
            state: FutureState::Init,
            ov: None,
            waker: None,
            res: None,
        }));

        let cloned = Arc::clone(&inner);
        let work = move |res| {
            let mut inner = cloned.lock().unwrap();

            if let Ok(len) = res {
                inner.f.pos += len as u64;
            }

            inner.state = FutureState::Ready;
            inner.res = Some(res);
            inner.waker.take().expect("this mut not occur").wake();
        };

        let mut guard = inner.lock().expect("this must not occur");
        guard.ov = Some(OverlappedCb::new(work));
        drop(guard);

        Read(inner)
    }

    pub fn write<'a>(&'a mut self, buf: &'a [u8]) -> Write<'a> {
        let inner = Arc::new(Mutex::new(WriteInner {
            f: self,
            buf,
            state: FutureState::Init,
            ov: None,
            waker: None,
            res: None,
        }));

        let cloned = Arc::clone(&inner);
        let work = move |res| {
            let mut inner = cloned.lock().unwrap();

            if let Ok(len) = res {
                inner.f.pos += len as u64;
            }

            inner.state = FutureState::Ready;
            inner.res = Some(res);
            inner.waker.take().expect("this mut not occur").wake();
        };

        let mut guard = inner.lock().expect("this must not occur");
        guard.ov = Some(OverlappedCb::new(work));
        drop(guard);

        Write(inner)
    }
}

struct ReadInner<'a> {
    f: &'a mut File,
    buf: &'a mut [u8],
    state: FutureState,
    ov: Option<OverlappedCb<'a>>,
    waker: Option<Waker>,
    res: Option<Result<usize>>,
}

pub struct Read<'a>(Arc<Mutex<ReadInner<'a>>>);

impl Future for Read<'_> {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.0.lock().unwrap();

        match inner.state {
            FutureState::Init => {
                let mut len = 0;
                let pos = inner.f.pos;
                inner.ov.as_mut().unwrap().set_offset(pos);

                inner.f.io.start();
                let succeeded = unsafe {
                    ReadFile(
                        inner.f.handle.as_raw_handle(),
                        inner.buf.as_mut_ptr(),
                        inner.buf.len() as _,
                        &mut len,
                        inner.ov.as_mut().unwrap().deref_mut(),
                    )
                } != 0;
                let e = Error::last_os_error();

                if !succeeded
                    && let Some(code) = e.raw_os_error()
                    && code == ERROR_IO_PENDING as _
                {
                    inner.state = FutureState::Pending;
                    inner.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }

                inner.f.io.cancel();
                inner.state = FutureState::Ready;
                Poll::Ready(if succeeded { Ok(len as _) } else { Err(e) })
            }
            FutureState::Pending => {
                inner.waker = Some(cx.waker().clone());
                Poll::Pending
            }
            FutureState::Ready => Poll::Ready(inner.res.take().expect("this must not occur")),
        }
    }
}

struct WriteInner<'a> {
    f: &'a mut File,
    buf: &'a [u8],
    state: FutureState,
    ov: Option<OverlappedCb<'a>>,
    waker: Option<Waker>,
    res: Option<Result<usize>>,
}

pub struct Write<'a>(Arc<Mutex<WriteInner<'a>>>);

impl Future for Write<'_> {
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.0.lock().unwrap();

        match inner.state {
            FutureState::Init => {
                let mut len = 0;
                let pos = inner.f.pos;
                inner.ov.as_mut().unwrap().set_offset(pos);

                inner.f.io.start();
                let succeeded = unsafe {
                    WriteFile(
                        inner.f.handle.as_raw_handle(),
                        inner.buf.as_ptr(),
                        inner.buf.len() as _,
                        &mut len,
                        inner.ov.as_mut().unwrap().deref_mut(),
                    )
                } != 0;
                let err = Error::last_os_error();

                if !succeeded
                    && let Some(code) = err.raw_os_error()
                    && code == ERROR_IO_PENDING as _
                {
                    inner.state = FutureState::Pending;
                    inner.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }

                inner.f.io.cancel();
                inner.state = FutureState::Ready;
                Poll::Ready(if succeeded { Ok(len as _) } else { Err(err) })
            }
            FutureState::Pending => {
                inner.waker = Some(cx.waker().clone());
                Poll::Pending
            }
            FutureState::Ready => Poll::Ready(inner.res.take().expect("this must not occur")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read() {
        crate::runtime::Runtime::default().block_on(async move {
            let mut f = File::open("Cargo.toml").await.unwrap();
            let mut buf = vec![0; 16];
            let len = f.read(&mut buf).await.unwrap();
            assert_eq!(len, buf.len());
        });
    }

    #[test]
    fn test_read_nul() {
        crate::runtime::Runtime::default().block_on(async move {
            let mut f = File::open("NUL").await.unwrap();
            let mut buf = vec![0; 16];
            let res = f.read(&mut buf).await;
            assert!(res.is_err());
        });
    }

    #[test]
    fn test_write() {
        crate::runtime::Runtime::default().block_on(async move {
            let mut f = File::create("CON").await.unwrap();
            let buf = vec![0; 16];
            let len = f.write(&buf).await.unwrap();
            assert_eq!(len, 16);
        });
    }

    #[test]
    fn test_write_nul() {
        crate::runtime::Runtime::default().block_on(async move {
            let mut f = File::create("NUL").await.unwrap();
            let buf = vec![0; 16];
            let res = f.write(&buf).await;
            assert!(res.is_ok());
        });
    }
}
