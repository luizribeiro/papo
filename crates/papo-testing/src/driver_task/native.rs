pub(crate) struct DriverTask(tokio::task::JoinHandle<Result<(), papo::Error>>);

impl DriverTask {
    pub(crate) fn spawn(driver: papo::Driver) -> Self {
        Self(tokio::spawn(driver))
    }

    pub(crate) fn abort(&self) {
        self.0.abort();
    }
}
