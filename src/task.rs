use crate::runtime::RuntimeInner;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Wake, Waker, ready};

enum State<R> {
    Pending(Pin<Box<dyn Future<Output = R> + Send>>),
    Ready(Option<R>),
}

struct TaskInner<R> {
    state: Mutex<State<R>>,
    ready: Condvar,
}

impl<R: Send + 'static> TaskInner<R> {
    fn poll(self: &Arc<Self>) {
        let mut state = self.state.lock().unwrap();

        if let State::Pending(fut) = state.deref_mut() {
            let waker = Waker::from(Arc::clone(self));
            let mut cx = Context::from_waker(&waker);

            if let Poll::Ready(r) = fut.as_mut().poll(&mut cx) {
                *state = State::Ready(Some(r));
                self.ready.notify_all();
            }
        }
    }
}

impl<R: Send + 'static> Wake for TaskInner<R> {
    fn wake(self: Arc<Self>) {
        crate::runtime::current()
            .expect("no running in a runtime")
            .submit_work(move || self.poll())
            .expect("failed to schedule work");
    }
}

#[repr(transparent)]
pub struct Task<R>(Arc<TaskInner<R>>);

impl<R: Send + 'static> Task<R> {
    pub(crate) fn new<F>(fut: F) -> Self
    where
        F: Future<Output = R> + Send + 'static,
    {
        Self(Arc::new(TaskInner {
            state: Mutex::new(State::Pending(Box::pin(fut))),
            ready: Condvar::new(),
        }))
    }

    pub(crate) fn poll_on(&self, rt: &Arc<RuntimeInner>) {
        let state = Arc::clone(&self.0);
        rt.submit_work(move || state.poll())
            .expect("faield to schedule work");
    }

    pub fn join(self) -> R {
        let mut state = self.0.state.lock().unwrap();

        loop {
            if let State::Ready(opt) = state.deref_mut() {
                return opt.take().expect("task already awaited");
            }

            state = self.0.ready.wait(state).unwrap();
        }
    }
}

impl<R> Future for Task<R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.state.lock().unwrap();

        match state.deref_mut() {
            State::Pending(fut) => {
                let r = ready!(fut.as_mut().poll(cx));
                *state = State::Ready(None);
                Poll::Ready(r)
            }
            State::Ready(opt) => Poll::Ready(opt.take().expect("task already awaited")),
        }
    }
}

pub fn spawn<F>(fut: F) -> Task<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    crate::runtime::current()
        .expect("not running in a runtime")
        .spawn(fut)
}
