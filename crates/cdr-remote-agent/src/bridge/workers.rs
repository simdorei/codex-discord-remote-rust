use std::future::Future;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::Semaphore;
use tokio::task::{JoinError, JoinSet};

#[derive(Debug, Error)]
#[error("the local bridge worker queue is full")]
pub struct WorkerCapacityError;

pub struct BoundedWorkers<T> {
    tasks: JoinSet<T>,
    permits: Arc<Semaphore>,
    max_pending: usize,
}

impl<T: Send + 'static> BoundedWorkers<T> {
    #[must_use]
    pub fn new(max_workers: usize, max_pending: usize) -> Self {
        assert!(max_workers > 0, "worker count must be positive");
        assert!(max_pending >= max_workers, "pending limit must fit workers");
        Self {
            tasks: JoinSet::new(),
            permits: Arc::new(Semaphore::new(max_workers)),
            max_pending,
        }
    }

    pub fn submit<F>(&mut self, future: F) -> Result<(), WorkerCapacityError>
    where
        F: Future<Output = T> + Send + 'static,
    {
        if self.tasks.len() >= self.max_pending {
            return Err(WorkerCapacityError);
        }
        let permits = Arc::clone(&self.permits);
        self.tasks.spawn(async move {
            let _permit = permits
                .acquire_owned()
                .await
                .expect("the worker semaphore remains open");
            future.await
        });
        Ok(())
    }

    pub async fn join_next(&mut self) -> Option<Result<T, JoinError>> {
        self.tasks.join_next().await
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}
