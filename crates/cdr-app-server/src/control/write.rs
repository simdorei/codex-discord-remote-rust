use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::AppServerError;
use crate::client::{AppServerClient, Inner};
use crate::transport::mark_closed;

struct WriteAttemptGuard {
    inner: Arc<Inner>,
    complete: bool,
}

impl WriteAttemptGuard {
    fn new(inner: &Arc<Inner>) -> Self {
        Self {
            inner: Arc::clone(inner),
            complete: false,
        }
    }

    fn complete(&mut self) {
        self.complete = true;
    }
}

impl Drop for WriteAttemptGuard {
    fn drop(&mut self) {
        if !self.complete {
            mark_closed(&self.inner, "app-server write outcome indeterminate");
        }
    }
}

impl AppServerClient {
    pub(crate) async fn write(&self, value: Value) -> Result<(), AppServerError> {
        self.write_with_hook(value, || {}).await
    }

    pub(crate) async fn write_with_hook(
        &self,
        value: Value,
        write_started: impl FnOnce(),
    ) -> Result<(), AppServerError> {
        self.write_with_preflight(value, || Ok(()), write_started)
            .await
    }

    pub(super) async fn write_with_preflight<T>(
        &self,
        value: Value,
        preflight: impl FnOnce() -> Result<T, AppServerError>,
        write_started: impl FnOnce(),
    ) -> Result<T, AppServerError> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(AppServerError::Closed);
        }
        let mut encoded = serde_json::to_vec(&value)?;
        encoded.push(b'\n');
        #[cfg(test)]
        let write_pause = self
            .inner
            .write_pause
            .lock()
            .expect("write pause lock")
            .clone();
        #[cfg(test)]
        if let Some(write_pause) = write_pause.as_ref() {
            write_pause.before_lock();
        }
        let mut stdin_guard = self.inner.stdin.lock().await;
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(AppServerError::Closed);
        }
        // Admission is evaluated after waiting for the writer, immediately before dispatch.
        let admitted = preflight()?;
        let stdin = stdin_guard.as_mut().ok_or(AppServerError::Closed)?;
        let mut attempt = WriteAttemptGuard::new(&self.inner);
        write_started();
        stdin.write_all(&encoded).await?;
        #[cfg(test)]
        if let Some(write_pause) = write_pause {
            write_pause.after_write().await?;
        }
        stdin.flush().await?;
        attempt.complete();
        Ok(admitted)
    }
}
