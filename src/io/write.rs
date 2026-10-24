use crate::io;
use crate::util::FutureState;
use pin_project_lite::pin_project;
use std::marker::PhantomPinned;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker, ready};

pub struct WriteBuf<'a> {
    buf: &'a [u8],
    ov: io::OverlappedCb<'a>,
    state: FutureState<io::Result<usize>>,
}

impl<'a> WriteBuf<'a> {
    pub fn as_ptr(&self) -> *const u8 {
        self.buf.as_ptr()
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
    pub struct Write<'a, W: ?Sized> {
        writer: &'a mut W,
        mx: Arc<Mutex<WriteBuf<'a>>>,

        #[pin]
        _pin: PhantomPinned,
    }
}

impl<W: AsyncWrite + Unpin + ?Sized> Future for Write<'_, W> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        ready!(Pin::new(this.writer).poll_write(cx, this.mx))?;

        let mut guard = this.mx.lock().unwrap();

        let FutureState::Ready(res) = guard.done() else {
            panic!("illegal future state");
        };
        Poll::Ready(res)
    }
}

pub trait AsyncWrite {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<WriteBuf<'_>>>,
    ) -> Poll<io::Result<()>>;
}

impl<W: AsyncWrite + Unpin + ?Sized> AsyncWrite for &mut W {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<WriteBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut **self).poll_write(cx, mx)
    }
}

impl<W: AsyncWrite + Unpin + ?Sized> AsyncWrite for Box<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<WriteBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut **self).poll_write(cx, mx)
    }
}

impl<D> AsyncWrite for Pin<D>
where
    D: DerefMut,
    D::Target: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mx: &Arc<Mutex<WriteBuf<'_>>>,
    ) -> Poll<io::Result<()>> {
        self.as_deref_mut().poll_write(cx, mx)
    }
}

pub trait AsyncWriteExt: AsyncWrite {
    fn write<'a>(&'a mut self, buf: &'a [u8]) -> Write<'a, Self>
    where
        Self: Unpin,
    {
        Write {
            writer: self,
            mx: Arc::new(Mutex::new(WriteBuf {
                buf,
                state: FutureState::Init,
                ov: io::OverlappedCb::default(),
            })),
            _pin: PhantomPinned,
        }
    }
}

impl<W: AsyncWrite + ?Sized> AsyncWriteExt for W {}
