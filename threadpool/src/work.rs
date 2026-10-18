use crate::{Error, Result};
use std::ffi::c_void;
use windows_sys::Win32::System::Threading::*;

pub struct PoolWork {
    handle: PTP_WORK,
    _work: Box<dyn Fn() + Send>,
}

impl PoolWork {
    pub(crate) fn new<F>(work: F, env: &TP_CALLBACK_ENVIRON_V3) -> Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        unsafe extern "system" fn callback<F: Fn()>(
            _: PTP_CALLBACK_INSTANCE,
            work: *mut c_void,
            _: PTP_WORK,
        ) {
            unsafe {
                (work as *mut F).as_ref_unchecked()();
            }
        }

        let work = Box::new(work);

        match unsafe { CreateThreadpoolWork(Some(callback::<F>), &*work as *const _ as _, env) } {
            0 => Err(Error::from_thread()),
            handle => Ok(Self {
                handle,
                _work: work,
            }),
        }
    }

    /// Posts a work object to the thread pool.
    ///
    /// A worker thread calls the work object's callback function.
    pub fn submit(&self) {
        unsafe { SubmitThreadpoolWork(self.handle) };
    }

    /// Waits for outstanding work callbacks to complete.
    pub fn join(&self) {
        unsafe { WaitForThreadpoolWorkCallbacks(self.handle, 0) };
    }
}

impl Drop for PoolWork {
    fn drop(&mut self) {
        self.join();
        unsafe { CloseThreadpoolWork(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_submit() {
        let pool = crate::Pool::new().unwrap();

        let n = Arc::new(AtomicUsize::new(0));
        assert_eq!(n.load(Ordering::Acquire), 0);

        let n2 = Arc::clone(&n);
        let work = pool
            .create_work(move || n2.store(42, Ordering::Release))
            .unwrap();
        work.submit();
        work.join();
        assert_eq!(n.load(Ordering::Acquire), 42);
    }
}
