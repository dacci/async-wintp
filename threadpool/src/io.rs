use crate::{Error, Result};
use std::ffi::c_void;
use std::os::windows::io::RawHandle;
use windows_sys::Win32::System::Threading::*;

#[doc(hidden)]
pub use windows_sys::Win32::System::IO::{OVERLAPPED, OVERLAPPED_0, OVERLAPPED_0_0};

pub struct PoolIo {
    handle: PTP_IO,
    _work: Box<dyn Fn(*mut OVERLAPPED, i32, usize) + Send>,
}

impl PoolIo {
    pub(crate) fn new<F>(handle: RawHandle, work: F, env: &TP_CALLBACK_ENVIRON_V3) -> Result<Self>
    where
        F: Fn(*mut OVERLAPPED, i32, usize) + Send + 'static,
    {
        unsafe extern "system" fn callback<F: Fn(*mut OVERLAPPED, i32, usize)>(
            _: PTP_CALLBACK_INSTANCE,
            work: *mut c_void,
            ov: *mut c_void,
            error: u32,
            len: usize,
            _: PTP_IO,
        ) {
            unsafe {
                (work as *mut F).as_ref_unchecked()(ov as _, error as _, len);
            }
        }

        let work = Box::new(work);

        match unsafe {
            CreateThreadpoolIo(handle, Some(callback::<F>), &*work as *const _ as _, env)
        } {
            0 => Err(Error::from_thread()),
            handle => Ok(Self {
                handle,
                _work: work,
            }),
        }
    }

    /// Notifies the thread pool that I/O operations may possibly begin for the I/O completion object.
    ///
    /// A worker thread calls the I/O completion object's callback function after the operation completes on the file handle bound to this
    /// object.
    pub fn start(&self) {
        unsafe { StartThreadpoolIo(self.handle) };
    }

    /// Cancels the notification from the [start()](Self::start()) function.
    pub fn cancel(&self) {
        unsafe { CancelThreadpoolIo(self.handle) };
    }

    /// Waits for outstanding I/O completion callbacks to complete.
    pub fn join(&self) {
        unsafe { WaitForThreadpoolIoCallbacks(self.handle, 0) };
    }
}

impl Drop for PoolIo {
    fn drop(&mut self) {
        self.join();
        unsafe { CloseThreadpoolIo(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::ptr::null_mut;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
    use windows_result::WIN32_ERROR;
    use windows_sys::Win32::Foundation::ERROR_IO_PENDING;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OVERLAPPED, ReadFile};

    #[test]
    fn test_read_file() {
        let pool = crate::Pool::new().unwrap();

        let err = Arc::new(AtomicI32::new(0));
        let len = Arc::new(AtomicUsize::new(0));

        let f = File::options()
            .read(true)
            .custom_flags(FILE_FLAG_OVERLAPPED)
            .open("Cargo.toml")
            .unwrap();
        let io = {
            let err = Arc::clone(&err);
            let len = Arc::clone(&len);
            pool.create_io(f.as_raw_handle(), move |_, _err, _len| {
                err.store(_err, Ordering::Release);
                len.store(_len, Ordering::Release);
            })
            .unwrap()
        };

        let mut buf = [0; 16];
        let mut ov = OVERLAPPED::default();

        io.start();
        let res = unsafe {
            ReadFile(
                f.as_raw_handle(),
                buf.as_mut_ptr(),
                buf.len() as _,
                null_mut(),
                &mut ov,
            )
        };
        assert_eq!(res, 0);
        assert_eq!(WIN32_ERROR::from_thread().0, ERROR_IO_PENDING);

        io.join();
        assert_eq!(len.load(Ordering::Acquire), 16);
    }
}
