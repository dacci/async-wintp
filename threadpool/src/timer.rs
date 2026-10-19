use crate::{Error, Result};
use std::ffi::c_void;
use std::ptr::null;
use windows_sys::Win32::System::Threading::*;

#[doc(hidden)]
pub use std::time::Duration;

pub struct PoolTimer {
    handle: PTP_TIMER,
    _work: Box<dyn Fn() + Send>,
}

impl PoolTimer {
    pub(crate) fn new<F>(work: F, env: &TP_CALLBACK_ENVIRON_V3) -> Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        unsafe extern "system" fn callback<F: Fn()>(
            _: PTP_CALLBACK_INSTANCE,
            work: *mut c_void,
            _: PTP_TIMER,
        ) {
            unsafe {
                (work as *mut F).as_ref_unchecked()();
            }
        }

        let work = Box::new(work);

        match unsafe { CreateThreadpoolTimer(Some(callback::<F>), &*work as *const _ as _, env) } {
            0 => Err(Error::from_thread()),
            handle => Ok(Self {
                handle,
                _work: work,
            }),
        }
    }

    /// Sets the timer object, replacing the previous timer, if any.
    ///
    /// A worker thread calls the timer object's callback after the specified timeout expires.
    pub fn set(&self, due: Option<Duration>, period: Option<Duration>, window: Option<Duration>) {
        let due = due.map(|d| crate::util::AsFILETIME::as_filetime(&d));

        unsafe {
            SetThreadpoolTimer(
                self.handle,
                match &due {
                    Some(due) => due,
                    None => null(),
                },
                period.as_ref().map(Duration::as_millis).unwrap_or_default() as _,
                window.as_ref().map(Duration::as_millis).unwrap_or_default() as _,
            )
        };
    }

    /// Determines whether the timer object is currently set.
    pub fn is_set(&self) -> bool {
        unsafe { IsThreadpoolTimerSet(self.handle) != 0 }
    }

    /// Waits for outstanding timer callbacks to complete.
    pub fn join(&self) {
        unsafe { WaitForThreadpoolTimerCallbacks(self.handle, 0) };
    }
}

impl Drop for PoolTimer {
    fn drop(&mut self) {
        self.join();
        unsafe { CloseThreadpoolTimer(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_timer() {
        let pool = crate::Pool::new().unwrap();

        let n = Arc::new(AtomicUsize::new(0));
        assert_eq!(n.load(Ordering::Acquire), 0);

        let n2 = Arc::clone(&n);
        let timer = pool
            .create_timer(move || {
                n2.fetch_add(1, Ordering::AcqRel);
            })
            .unwrap();
        timer.set(
            Some(Duration::from_nanos(100)),
            Some(Duration::from_millis(100)),
            None,
        );

        std::thread::sleep(Duration::from_secs(1));
        timer.set(None, None, None);
        timer.join();

        assert!(n.load(Ordering::Acquire) >= 10);
    }

    #[test]
    fn test_is_set() {
        let pool = crate::Pool::new().unwrap();

        let timer = pool.create_timer(|| {}).unwrap();
        assert_eq!(timer.is_set(), false);

        timer.set(Some(Duration::from_hours(1)), None, None);
        assert_eq!(timer.is_set(), true);

        timer.set(None, None, None);
        assert_eq!(timer.is_set(), false);
    }
}
