use crate::{Error, Result};
use std::ffi::c_void;
use std::ptr::null_mut;
use windows_sys::Win32::System::Threading::*;

pub enum CallbackPriority {
    High,
    Normal,
    Low,
}

impl From<CallbackPriority> for i32 {
    fn from(p: CallbackPriority) -> Self {
        match p {
            CallbackPriority::High => TP_CALLBACK_PRIORITY_HIGH,
            CallbackPriority::Normal => TP_CALLBACK_PRIORITY_NORMAL,
            CallbackPriority::Low => TP_CALLBACK_PRIORITY_LOW,
        }
    }
}

pub struct Pool(Box<TP_CALLBACK_ENVIRON_V3>);

impl Pool {
    /// Creates a new thread pool with `Normal` callback priority.
    pub fn new() -> Result<Self> {
        Self::with_callback_priority(CallbackPriority::Normal)
    }

    /// Creates a new thread pool with specified callback priority.
    pub fn with_callback_priority(priority: CallbackPriority) -> Result<Self> {
        let pool = unsafe { CreateThreadpool(null_mut()) };
        if pool == 0 {
            return Err(Error::from_thread());
        }

        let cleanup_group = unsafe { CreateThreadpoolCleanupGroup() };
        if cleanup_group == 0 {
            let e = Error::from_thread();
            unsafe { CloseThreadpool(pool) };
            return Err(e);
        }

        let env = TP_CALLBACK_ENVIRON_V3 {
            Version: 3,
            Pool: pool,
            CleanupGroup: cleanup_group,
            CallbackPriority: priority.into(),
            Size: size_of::<TP_CALLBACK_ENVIRON_V3>() as _,
            ..Default::default()
        };

        Ok(Self(Box::new(env)))
    }

    /// Sets the maximum number of threads that the thread pool can allocate to process callbacks.
    pub fn set_thread_maximum(&self, max: u32) {
        unsafe { SetThreadpoolThreadMaximum(self.0.Pool, max) };
    }

    /// Sets the minimum number of threads that the thread pool must make available to process callbacks.
    pub fn set_thread_minimum(&self, min: u32) -> Result<()> {
        match unsafe { SetThreadpoolThreadMinimum(self.0.Pool, min) } {
            0 => Err(Error::from_thread()),
            _ => Ok(()),
        }
    }

    /// Sets the stack reserve and commit sizes for new threads in the thread pool.
    ///
    /// Stack reserve and commit sizes for existing threads are not changed.
    pub fn set_stack_info(&self, reserve: usize, commit: usize) -> Result<()> {
        let info = TP_POOL_STACK_INFORMATION {
            StackReserve: reserve,
            StackCommit: commit,
        };

        match unsafe { SetThreadpoolStackInformation(self.0.Pool, &info) } {
            0 => Err(Error::from_thread()),
            _ => Ok(()),
        }
    }

    /// Waits for all callback functions to complete.
    pub fn join(&self) {
        unsafe { CloseThreadpoolCleanupGroupMembers(self.0.CleanupGroup, 0, null_mut()) };
    }

    /// Requests that a thread pool worker thread call the specified callback function.
    pub fn submit_work<F>(&self, work: F) -> Result<()>
    where
        F: FnOnce() + Send,
    {
        unsafe extern "system" fn callback<F: FnOnce()>(
            _: PTP_CALLBACK_INSTANCE,
            work: *mut c_void,
        ) {
            unsafe {
                Box::from_raw(work as *mut F)();
            }
        }

        let work = Box::into_raw(Box::new(work));

        if unsafe { TrySubmitThreadpoolCallback(Some(callback::<F>), work as _, &*self.0) } == 0 {
            drop(unsafe { Box::from_raw(work) });
            return Err(Error::from_thread());
        }

        Ok(())
    }

    /// Creates a new work object.
    pub fn create_work<F>(&self, work: F) -> Result<crate::work::PoolWork>
    where
        F: Fn() + Send + 'static,
    {
        crate::work::PoolWork::new(work, &self.0)
    }

    /// Creates a new timer object.
    #[cfg(feature = "timer")]
    pub fn create_timer<F>(&self, work: F) -> Result<crate::timer::PoolTimer>
    where
        F: Fn() + Send + 'static,
    {
        crate::timer::PoolTimer::new(work, &self.0)
    }

    /// Creates a new I/O completion object.
    #[cfg(feature = "io")]
    pub fn create_io<F>(
        &self,
        handle: std::os::windows::io::RawHandle,
        work: F,
    ) -> Result<crate::io::PoolIo>
    where
        F: Fn(*mut crate::io::OVERLAPPED, i32, usize) + Send + 'static,
    {
        crate::io::PoolIo::new(handle, work, &self.0)
    }

    /// Creates a new wait object.
    #[cfg(feature = "sync")]
    pub fn create_wait<F>(&self, work: F) -> Result<crate::sync::PoolWait>
    where
        F: Fn(Result<()>) + Send + 'static,
    {
        crate::sync::PoolWait::new(work, &self.0)
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.join();

        unsafe {
            CloseThreadpoolCleanupGroup(self.0.CleanupGroup);
            CloseThreadpool(self.0.Pool);
        }
    }
}

// SAFETY: TP_CALLBACK_ENVIRON_V3::RaceDll is not a pointer
unsafe impl Send for Pool {}
unsafe impl Sync for Pool {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_submit_work() {
        let pool = Pool::new().unwrap();

        let n = AtomicUsize::new(0);
        assert_eq!(n.load(Ordering::Acquire), 0);

        pool.submit_work(|| {
            n.store(42, Ordering::Release);
        })
        .unwrap();

        pool.join();
        assert_eq!(n.load(Ordering::Acquire), 42);
    }
}
