use std::{future::Future, pin::Pin, time::SystemTime};

use tokio::io::{AsyncRead, AsyncWrite};
use web_time::Instant;

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin {}

impl<T: AsyncRead + AsyncWrite + Unpin> Io for T {}

pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pub(crate) fn spawn<F>(future: F)
where
    F: Future + 'static,
    F::Output: 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        drop(future.await);
    });
}

pub(crate) async fn sleep_until(deadline: Instant) {
    Timer::new(deadline, || {}).await;
}

pub(crate) fn now_wall() -> SystemTime {
    wall_time_from_millis(js_sys::Date::now())
}

fn wall_time_from_millis(milliseconds: f64) -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64(milliseconds / 1_000.0)
}

pub(crate) fn now_mono() -> Instant {
    Instant::now()
}

struct Timer {
    receiver: tokio::sync::oneshot::Receiver<()>,
    global: wasm_bindgen::JsValue,
    clear_timeout: js_sys::Function,
    token: Option<wasm_bindgen::JsValue>,
    callback: Option<wasm_bindgen::closure::Closure<dyn FnMut()>>,
}

impl Timer {
    fn new(deadline: Instant, on_fire: impl FnOnce() + 'static) -> Self {
        use wasm_bindgen::JsCast;

        let global = js_sys::global();
        let function = |name: &str| {
            js_sys::Reflect::get(&global, &name.into())
                .expect("worker global has timer functions")
                .unchecked_into::<js_sys::Function>()
        };
        let set_timeout = function("setTimeout");
        let clear_timeout = function("clearTimeout");
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let callback = wasm_bindgen::closure::Closure::once(move || {
            on_fire();
            _ = sender.send(());
        });
        let milliseconds = deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f64()
            .mul_add(1_000.0, 0.0)
            .ceil()
            .min(f64::from(i32::MAX));
        let token = set_timeout
            .call2(&global, callback.as_ref(), &milliseconds.into())
            .expect("worker timer can be scheduled");
        Self {
            receiver,
            global: global.into(),
            clear_timeout,
            token: Some(token),
            callback: Some(callback),
        }
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        Pin::new(&mut self.receiver).poll(context).map(|_| {
            self.token.take();
            self.callback.take();
        })
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            _ = self.clear_timeout.call1(&self.global, &token);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        rc::Rc,
        time::{Duration as D, SystemTime},
    };

    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{Timer, now_mono, now_wall, sleep_until, spawn};

    #[wasm_bindgen_test]
    fn converts_javascript_milliseconds_to_system_time() {
        assert_eq!(
            super::wall_time_from_millis(1_500.0),
            SystemTime::UNIX_EPOCH + D::from_millis(1_500)
        );
    }

    #[wasm_bindgen_test]
    async fn orders_worker_timers_and_spawns_locally() {
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        spawn(async move {
            sleep_until(now_mono() + D::from_millis(20)).await;
            _ = sender.send(2);
        });
        sleep_until(now_mono() + D::from_millis(1)).await;
        assert_eq!(
            receiver.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        );
        assert_eq!(receiver.await.unwrap(), 2);
    }

    #[wasm_bindgen_test]
    async fn dropping_a_timer_cancels_its_callback() {
        let fired = Rc::new(Cell::new(false));
        let probe = Rc::clone(&fired);
        let timer = Timer::new(now_mono() + D::from_millis(1), move || probe.set(true));
        drop(timer);
        sleep_until(now_mono() + D::from_millis(10)).await;
        assert!(!fired.get());
    }

    #[wasm_bindgen_test]
    async fn worker_clocks_track_elapsed_and_wall_time() {
        let javascript_wall =
            SystemTime::UNIX_EPOCH + D::from_secs_f64(js_sys::Date::now() / 1_000.0);
        assert!(now_wall().duration_since(javascript_wall).unwrap() < D::from_secs(1));

        let earlier = now_mono();
        sleep_until(earlier + D::from_millis(2)).await;
        let elapsed = now_mono().duration_since(earlier);
        assert!(elapsed >= D::from_millis(1));
        assert!(elapsed < D::from_secs(1));
    }
}
