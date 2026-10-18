use crate::util::FutureState;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

pub use std::time::Duration;
pub(crate) use threadpool::PoolTimer;

struct SleepInner {
    state: FutureState,
    duration: Duration,
    waker: Option<Waker>,
    timer: Option<PoolTimer>,
}

pub struct Sleep(Arc<Mutex<SleepInner>>);

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.0.lock().unwrap();

        match inner.state {
            FutureState::Init => {
                inner.state = FutureState::Pending;
                inner.waker = Some(cx.waker().clone());
                inner.timer.as_ref().expect("this must not occur").set(
                    Some(inner.duration),
                    None,
                    None,
                );
                Poll::Pending
            }
            FutureState::Pending => {
                inner.waker = Some(cx.waker().clone());
                Poll::Pending
            }
            FutureState::Ready => Poll::Ready(()),
        }
    }
}

pub fn sleep(dur: Duration) -> Sleep {
    let inner = Arc::new(Mutex::new(SleepInner {
        state: FutureState::Init,
        duration: dur,
        waker: None,
        timer: None,
    }));

    let cloned = Arc::clone(&inner);
    let work = move || {
        let mut inner = cloned.lock().unwrap();

        match inner.state {
            FutureState::Pending => {
                inner.state = FutureState::Ready;
                inner.waker.take().expect("this must not occur").wake();
            }
            _ => panic!("illegal sleep state"),
        }
    };

    let timer = crate::runtime::current()
        .expect("not running in a runtime")
        .create_timer(work)
        .expect("failed to create timer");
    inner.lock().expect("this must not panic").timer = Some(timer);

    Sleep(inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_sleep() {
        let elapsed = crate::runtime::Runtime::default().block_on(async {
            let start = Instant::now();
            sleep(Duration::from_secs(1)).await;
            start.elapsed()
        });
        assert!(elapsed.as_millis() >= 990);
    }
}
