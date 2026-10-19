use std::mem;
use std::task::Waker;

#[allow(unused)]
#[derive(Default)]
pub(crate) enum FutureState<R> {
    #[default]
    Init,

    Pending(Waker),
    Ready(R),
    Done,
}

impl<R> FutureState<R> {
    pub fn pending(&mut self, w: &Waker) -> Self {
        mem::replace(self, Self::Pending(w.clone()))
    }

    pub fn ready(&mut self, r: R) -> Self {
        mem::replace(self, Self::Ready(r))
    }

    pub fn done(&mut self) -> Self {
        mem::replace(self, Self::Done)
    }
}
