pub(crate) struct WriteTestPause {
    before_lock: tokio::sync::Semaphore,
    entered: tokio::sync::Semaphore,
    fail_after_write: bool,
    pause_before_resolve: bool,
    release: tokio::sync::Semaphore,
}

impl WriteTestPause {
    pub(crate) fn new() -> Self {
        Self {
            before_lock: tokio::sync::Semaphore::new(0),
            entered: tokio::sync::Semaphore::new(0),
            fail_after_write: false,
            pause_before_resolve: false,
            release: tokio::sync::Semaphore::new(0),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            before_lock: tokio::sync::Semaphore::new(0),
            entered: tokio::sync::Semaphore::new(0),
            fail_after_write: true,
            pause_before_resolve: false,
            release: tokio::sync::Semaphore::new(0),
        }
    }

    pub(crate) fn for_response_resolution() -> Self {
        Self {
            pause_before_resolve: true,
            ..Self::new()
        }
    }

    pub(crate) fn release(&self) {
        self.release.add_permits(1);
    }

    pub(crate) async fn before_response_resolve(&self) {
        if self.pause_before_resolve {
            self.entered.add_permits(1);
            self.release
                .acquire()
                .await
                .expect("response pause open")
                .forget();
        }
    }

    pub(crate) fn before_lock(&self) {
        self.before_lock.add_permits(1);
    }

    pub(crate) async fn wait_until_before_lock(&self) {
        self.before_lock
            .acquire()
            .await
            .expect("write test open")
            .forget();
    }

    pub(crate) async fn after_write(&self) -> std::io::Result<()> {
        if self.pause_before_resolve {
            return Ok(());
        }
        self.entered.add_permits(1);
        if self.fail_after_write {
            return Err(std::io::Error::other("injected post-write failure"));
        }
        self.release
            .acquire()
            .await
            .expect("write pause open")
            .forget();
        Ok(())
    }

    pub(crate) async fn wait_until_entered(&self) {
        self.entered
            .acquire()
            .await
            .expect("write pause open")
            .forget();
    }
}
