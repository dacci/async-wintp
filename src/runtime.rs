use crate::Result;
use crate::task::Task;
use std::cell::RefCell;
use std::mem::transmute;
use std::sync::Arc;
use threadpool::pool::Pool;

pub use threadpool::pool::CallbackPriority;

pub struct Builder {
    callback_priority: CallbackPriority,
    thread_maximum: Option<u32>,
    thread_minimum: Option<u32>,
    stack_info: Option<(usize, usize)>,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            callback_priority: CallbackPriority::Normal,
            thread_maximum: None,
            thread_minimum: None,
            stack_info: None,
        }
    }

    pub fn callback_priority(&mut self, priority: CallbackPriority) -> &mut Self {
        self.callback_priority = priority;
        self
    }

    pub fn thread_maximum(&mut self, max: u32) -> &mut Self {
        self.thread_maximum = Some(max);
        self
    }

    pub fn thread_minimum(&mut self, min: u32) -> &mut Self {
        self.thread_minimum = Some(min);
        self
    }

    pub fn stack_info(&mut self, reserve: usize, commit: usize) -> &mut Self {
        self.stack_info = Some((reserve, commit));
        self
    }

    pub fn build(self) -> Result<Runtime> {
        let pool = Pool::with_callback_priority(self.callback_priority)?;

        if let Some(max) = self.thread_maximum {
            pool.set_thread_maximum(max);
        }

        if let Some(min) = self.thread_minimum {
            pool.set_thread_minimum(min)?;
        }

        if let Some((reserve, commit)) = self.stack_info {
            pool.set_stack_info(reserve, commit)?;
        }

        Ok(Runtime(Arc::new(RuntimeInner { pool })))
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Runtime(pub(crate) Arc<RuntimeInner>);

impl Default for Runtime {
    fn default() -> Self {
        Builder::new().build().expect("failed to build Runtime")
    }
}

impl Runtime {
    pub fn block_on<F>(&self, fut: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.0.spawn(fut).join()
    }
}

pub(crate) struct RuntimeInner {
    pool: Pool,
}

impl RuntimeInner {
    pub(crate) fn spawn<F>(self: &Arc<Self>, fut: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let task = Task::new(fut);
        task.poll_on(self);
        task
    }

    pub(crate) fn submit_work<F>(self: &Arc<Self>, work: F) -> Result<()>
    where
        F: FnOnce() + Send,
    {
        let rt = Arc::clone(self);
        self.pool
            .submit_work(move || {
                let prev = CURRENT.replace(Some(unsafe { transmute::<&_, &'static _>(&rt) }));

                work();

                CURRENT.set(prev);
            })
            .map_err(Into::into)
    }

    #[cfg(feature = "time")]
    pub(crate) fn create_timer<F>(self: &Arc<Self>, work: F) -> Result<crate::time::PoolTimer>
    where
        F: Fn() + Send + 'static,
    {
        let rt = Arc::clone(self);
        let timer = self.pool.create_timer(move || {
            let prev = CURRENT.replace(Some(unsafe { transmute::<&_, &'static _>(&rt) }));

            work();

            CURRENT.set(prev);
        })?;

        Ok(timer)
    }

    #[cfg(feature = "io")]
    pub(crate) fn create_io<F>(
        self: &Arc<Self>,
        handle: std::os::windows::io::RawHandle,
        work: F,
    ) -> crate::io::Result<crate::io::PoolIo>
    where
        F: Fn(*mut crate::io::OVERLAPPED, crate::io::Result<usize>) + Send + 'static,
    {
        let rt = Arc::clone(self);
        let io = self.pool.create_io(handle, move |ov, code, len| {
            let res = if code == 0 {
                Ok(len)
            } else {
                Err(crate::io::Error::from_raw_os_error(code))
            };

            let prev = CURRENT.replace(Some(unsafe { transmute::<&_, &'static _>(&rt) }));

            work(ov, res);

            CURRENT.set(prev);
        })?;

        Ok(io)
    }
}

thread_local! {
    static CURRENT: RefCell<Option<&'static Arc<RuntimeInner>>> = const { RefCell::new(None) };
}

pub(crate) fn current<'a>() -> Option<&'a Arc<RuntimeInner>> {
    CURRENT.with_borrow(|c| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_work() {
        Runtime::default().block_on(async move {
            assert!(current().is_some());
        });
    }
}
