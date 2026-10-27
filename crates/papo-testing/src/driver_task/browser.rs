pub(crate) struct DriverTask(Option<tokio::sync::oneshot::Sender<()>>);

impl DriverTask {
    pub(crate) fn spawn(driver: papo::Driver) -> Self {
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        wasm_bindgen_futures::spawn_local(async move {
            tokio::select! {
                _ = driver => {}
                _ = cancelled => {}
            }
        });
        Self(Some(cancel))
    }

    pub(crate) fn abort(&mut self) {
        if let Some(cancel) = self.0.take() {
            _ = cancel.send(());
        }
    }
}
