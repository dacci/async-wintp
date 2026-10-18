use std::ops::{Deref, DerefMut};

pub use std::io::{Error, ErrorKind, Result};
pub(crate) use threadpool::io::{OVERLAPPED, PoolIo};

#[repr(C)]
pub(crate) struct OverlappedCb<'a> {
    ov: OVERLAPPED,
    cb: Box<dyn Fn(Result<usize>) + Send + 'a>,
}

// SAFETY: OVERLAPPED is designed to be shared between threads.
unsafe impl Send for OverlappedCb<'_> {}

impl<'a> OverlappedCb<'a> {
    pub fn new<F: Fn(Result<usize>) + Send + 'a>(cb: F) -> Self {
        Self {
            ov: OVERLAPPED::default(),
            cb: Box::new(cb),
        }
    }

    pub fn set_offset(&mut self, offset: u64) {
        self.ov.Anonymous.Anonymous.Offset = offset as _;
        self.ov.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as _;
    }

    pub fn call(&self, res: Result<usize>) {
        (self.cb)(res)
    }
}

impl Deref for OverlappedCb<'_> {
    type Target = OVERLAPPED;

    fn deref(&self) -> &Self::Target {
        &self.ov
    }
}

impl DerefMut for OverlappedCb<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ov
    }
}
