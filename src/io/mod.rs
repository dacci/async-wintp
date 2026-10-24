pub use std::io::{Error, ErrorKind, Result};
pub(crate) use threadpool::io::{OVERLAPPED, PoolIo};

#[repr(C)]
#[derive(Default)]
pub struct OverlappedCb<'a> {
    ov: OVERLAPPED,
    cb: Option<Box<dyn Fn(Result<usize>) + Send + 'a>>,
}

// SAFETY: OVERLAPPED is designed to be shared between threads.
unsafe impl Send for OverlappedCb<'_> {}

impl<'a> OverlappedCb<'a> {
    pub fn with_callback<F: Fn(Result<usize>) + Send + 'a>(cb: F) -> Self {
        let mut val = Self::default();
        val.set_callback(cb);
        val
    }

    pub fn set_offset(&mut self, offset: u64) {
        self.ov.Anonymous.Anonymous.Offset = offset as _;
        self.ov.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as _;
    }

    pub fn set_callback<F: Fn(Result<usize>) + Send + 'a>(&mut self, cb: F) {
        self.cb = Some(Box::new(cb));
    }

    pub fn call(&self, res: Result<usize>) {
        if let Some(cb) = &self.cb {
            cb(res)
        }
    }
}

impl AsRef<OVERLAPPED> for OverlappedCb<'_> {
    fn as_ref(&self) -> &OVERLAPPED {
        &self.ov
    }
}

impl AsMut<OVERLAPPED> for OverlappedCb<'_> {
    fn as_mut(&mut self) -> &mut OVERLAPPED {
        &mut self.ov
    }
}

mod read;
pub use read::*;

mod write;
pub use write::*;
