use crate::util::AsFILETIME;
use crate::{Error, Result};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_result::WIN32_ERROR;
use windows_sys::Win32::System::Threading::*;

#[doc(hidden)]
pub use std::os::windows::raw::HANDLE;
#[doc(hidden)]
pub use std::time::Duration;

pub struct PoolWait {
    handle: PTP_WAIT,
    _work: Box<dyn Fn(Result<()>)>,
}

impl PoolWait {
    pub(crate) fn new<F>(work: F, env: &TP_CALLBACK_ENVIRON_V3) -> Result<Self>
    where
        F: Fn(Result<()>) + Send + 'static,
    {
        unsafe extern "system" fn callback<F: Fn(Result<()>)>(
            _: PTP_CALLBACK_INSTANCE,
            work: *mut c_void,
            _: PTP_WAIT,
            result: u32,
        ) {
            let res = match result {
                0 => Ok(()),
                code => Err(WIN32_ERROR(code).into()),
            };

            unsafe {
                (work as *mut F).as_ref_unchecked()(res);
            }
        }

        let work = Box::new(work);

        match unsafe { CreateThreadpoolWait(Some(callback::<F>), &*work as *const _ as _, env) } {
            0 => Err(Error::from_thread()),
            handle => Ok(Self {
                handle,
                _work: work,
            }),
        }
    }

    /// Sets the wait object, replacing the previous wait object, if any.
    ///
    /// A worker thread calls the wait object's callback function after the handle becomes signaled or after the specified timeout expires.
    pub fn set(&self, handle: Option<HANDLE>, timeout: Option<Duration>) {
        unsafe {
            SetThreadpoolWait(
                self.handle,
                handle.unwrap_or_else(null_mut),
                match timeout {
                    Some(timeout) => &timeout.as_filetime(),
                    None => null(),
                },
            )
        };
    }

    /// Waits for outstanding wait callbacks to complete.
    pub fn join(&self) {
        unsafe { WaitForThreadpoolWaitCallbacks(self.handle, 0) };
    }
}

impl Drop for PoolWait {
    fn drop(&mut self) {
        self.join();
        unsafe { CloseThreadpoolWait(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_wait_event() {
        let pool = crate::Pool::new().unwrap();

        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        assert!(!event.is_null());

        let res = Arc::new(Mutex::new(None));

        let res2 = Arc::clone(&res);
        let wait = pool
            .create_wait(move |res| {
                *res2.lock().unwrap() = Some(res);
            })
            .unwrap();

        wait.set(Some(event), Some(Duration::from_millis(100)));
        std::thread::sleep(Duration::from_millis(200));
        wait.join();

        let guard = res.lock().unwrap();
        assert!(guard.is_some());
        assert!(guard.as_ref().unwrap().is_err());
        drop(guard);

        assert_ne!(unsafe { SetEvent(event) }, 0);

        wait.set(Some(event), Some(Duration::from_nanos(100)));
        std::thread::sleep(Duration::from_millis(200));
        wait.join();

        let guard = res.lock().unwrap();
        assert!(guard.is_some());
        assert!(guard.as_ref().unwrap().is_ok());
        drop(guard);
    }
}
