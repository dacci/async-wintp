use crate::util::FutureState;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

pub use std::time::Duration;
pub(crate) use threadpool::PoolTimer;

struct SleepInner {
    state: FutureState<()>,
    duration: Duration,
    timer: Option<PoolTimer>,
}

pub struct Sleep(Arc<Mutex<SleepInner>>);

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.0.lock().unwrap();

        match inner.state {
            FutureState::Init => {
                inner.state.pending(cx.waker());
                inner.timer.as_ref().expect("this must not occur").set(
                    Some(inner.duration),
                    None,
                    None,
                );
                Poll::Pending
            }
            FutureState::Pending(_) => {
                inner.state.pending(cx.waker());
                Poll::Pending
            }
            FutureState::Ready(_) => {
                let FutureState::Ready(r) = inner.state.done() else {
                    panic!("illegal future state");
                };
                Poll::Ready(r)
            }
            _ => panic!("illegal future state"),
        }
    }
}

pub fn sleep(dur: Duration) -> Sleep {
    let inner = Arc::new(Mutex::new(SleepInner {
        state: FutureState::Init,
        duration: dur,
        timer: None,
    }));

    let cloned = Arc::clone(&inner);
    let work = move || {
        let mut inner = cloned.lock().unwrap();

        match inner.state {
            FutureState::Pending(_) => {
                let FutureState::Pending(w) = inner.state.ready(()) else {
                    panic!("illegal sleep state");
                };
                w.wake();
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
