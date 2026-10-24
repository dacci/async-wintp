use crate::io;
use crate::util::FutureState;
use std::fs::{File as StdFile, OpenOptions as StdOpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
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

    pub async fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
        let handle = self.0.open(path)?;

        let io = crate::runtime::current()
            .expect("not running in a runtime")
            .create_io(handle.as_raw_handle(), move |ov, res| {
                let ov = unsafe { ov.cast::<io::OverlappedCb>().as_ref_unchecked() };
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
    io: io::PoolIo,
    pos: u64,
}

impl File {
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::options().read(true).open(path).await
    }

    pub async fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
    }

    pub async fn create_new(path: impl AsRef<Path>) -> io::Result<Self> {
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

    pub async fn sync_all(&self) -> io::Result<()> {
        match unsafe { FlushFileBuffers(self.handle.as_raw_handle()) } {
            0 => Err(io::Error::last_os_error()),
            _ => Ok(()),
        }
    }

    pub async fn sync_data(&self) -> io::Result<()> {
        self.sync_all().await
    }

    pub async fn set_len(&self, size: u64) -> io::Result<()> {
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
            0 => Err(io::Error::last_os_error()),
            _ => Ok(()),
        }
    }
}

impl io::AsyncRead for File {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<io::ReadBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        let mut buf = mx.lock().unwrap();

        match buf.state() {
            FutureState::Init => {
                let cloned = Arc::clone(mx);
                let work = move |res| {
                    let mut inner = cloned.lock().unwrap();

                    let FutureState::Pending(w) = inner.ready(res) else {
                        panic!("illegal future state");
                    };
                    w.wake();
                };

                buf.set_offset(self.pos);
                buf.set_callback(work);

                let mut len = 0;

                self.io.start();
                let succeeded = unsafe {
                    ReadFile(
                        self.handle.as_raw_handle(),
                        buf.as_mut_ptr(),
                        buf.len() as _,
                        &mut len,
                        buf.as_overlapped(),
                    ) != 0
                };
                let e = io::Error::last_os_error();

                if !succeeded
                    && let Some(code) = e.raw_os_error()
                    && code == ERROR_IO_PENDING as _
                {
                    buf.pending(cx.waker());
                    Poll::Pending
                } else {
                    self.io.cancel();
                    buf.ready(if succeeded { Ok(len as _) } else { Err(e) });
                    Poll::Ready(Ok(()))
                }
            }
            FutureState::Pending(_) => {
                buf.pending(cx.waker());
                Poll::Pending
            }
            FutureState::Ready(res) => {
                if let Ok(len) = res {
                    self.pos += *len as u64;
                }
                Poll::Ready(Ok(()))
            }
            _ => unreachable!(),
        }
    }
}

impl io::AsyncWrite for File {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<io::WriteBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        let mut buf = mx.lock().unwrap();

        match buf.state() {
            FutureState::Init => {
                let cloned = Arc::clone(mx);
                let work = move |res| {
                    let mut inner = cloned.lock().unwrap();

                    let FutureState::Pending(w) = inner.ready(res) else {
                        panic!("illegal future state");
                    };
                    w.wake();
                };

                buf.set_callback(work);
                buf.set_offset(self.pos);

                let mut len = 0;

                self.io.start();
                let succeeded = unsafe {
                    WriteFile(
                        self.handle.as_raw_handle(),
                        buf.as_ptr(),
                        buf.len() as _,
                        &mut len,
                        buf.as_overlapped(),
                    ) != 0
                };
                let err = io::Error::last_os_error();

                if !succeeded
                    && let Some(code) = err.raw_os_error()
                    && code == ERROR_IO_PENDING as _
                {
                    buf.pending(cx.waker());
                    Poll::Pending
                } else {
                    self.io.cancel();
                    buf.ready(if succeeded { Ok(len as _) } else { Err(err) });
                    Poll::Ready(Ok(()))
                }
            }
            FutureState::Pending(_) => {
                buf.pending(cx.waker());
                Poll::Pending
            }
            FutureState::Ready(res) => {
                if let Ok(len) = res {
                    self.pos += *len as u64;
                }
                Poll::Ready(Ok(()))
            }
            _ => panic!("illegal future state"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{AsyncReadExt, AsyncWriteExt};

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
