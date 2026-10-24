use crate::io;
use crate::util::FutureState;
use pin_project_lite::pin_project;
use std::marker::PhantomPinned;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker, ready};

pub struct ReadBuf<'a> {
    buf: &'a mut [u8],
    ov: io::OverlappedCb<'a>,
    state: FutureState<io::Result<usize>>,
}

impl<'a> ReadBuf<'a> {
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buf.as_mut_ptr()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_overlapped(&mut self) -> &mut io::OVERLAPPED {
        self.ov.as_mut()
    }

    pub fn set_callback<F: Fn(io::Result<usize>) + Send + 'a>(&mut self, cb: F) {
        self.ov.set_callback(cb);
    }

    pub fn set_offset(&mut self, offset: u64) {
        self.ov.set_offset(offset);
    }

    pub fn state(&self) -> &FutureState<io::Result<usize>> {
        &self.state
    }

    pub fn pending(&mut self, w: &Waker) -> FutureState<io::Result<usize>> {
        self.state.pending(w)
    }

    pub fn ready(&mut self, r: io::Result<usize>) -> FutureState<io::Result<usize>> {
        self.state.ready(r)
    }

    pub fn done(&mut self) -> FutureState<io::Result<usize>> {
        self.state.done()
    }
}

pin_project! {
    pub struct Read<'a, R: ?Sized> {
        reader: &'a mut R,
        mx: Arc<Mutex<ReadBuf<'a>>>,

        #[pin]
        _pin: PhantomPinned,
    }
}

impl<R: AsyncRead + Unpin + ?Sized> Future for Read<'_, R> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        ready!(Pin::new(this.reader).poll_read(cx, this.mx))?;

        let mut guard = this.mx.lock().unwrap();

        let FutureState::Ready(res) = guard.done() else {
            panic!("illegal future state");
        };
        Poll::Ready(res)
    }
}

pub trait AsyncRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<ReadBuf<'_>>>,
    ) -> Poll<io::Result<()>>;
}

impl<R: AsyncRead + Unpin + ?Sized> AsyncRead for &mut R {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<ReadBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut **self).poll_read(cx, mx)
    }
}

impl<R: AsyncRead + Unpin + ?Sized> AsyncRead for Box<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<ReadBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut **self).poll_read(cx, mx)
    }
}

impl<D> AsyncRead for Pin<D>
where
    D: DerefMut,
    D::Target: AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<ReadBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        self.as_deref_mut().poll_read(cx, mx)
    }
}

pub trait AsyncReadExt: AsyncRead {
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> Read<'a, Self>
    where
        Self: Unpin,
    {
        Read {
            reader: self,
            mx: Arc::new(Mutex::new(ReadBuf {
                buf,
                state: FutureState::Init,
                ov: io::OverlappedCb::default(),
            })),
            _pin: PhantomPinned,
        }
    }
}

impl<R: AsyncRead + ?Sized> AsyncReadExt for R {}
