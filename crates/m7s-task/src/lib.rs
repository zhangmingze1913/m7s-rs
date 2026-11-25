//! Hierarchical task group (foundation crate).

use tokio::task::{JoinError, JoinSet};

#[derive(Default)]
pub struct TaskGroup {
    set: JoinSet<()>,
}

impl TaskGroup {
    pub fn new() -> Self {
        Self { set: JoinSet::new() }
    }

    pub fn spawn<F>(&mut self, fut: F) -> tokio::task::AbortHandle
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.set.spawn(fut)
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Abort all tasks and drain them.
    pub async fn cancel_all(&mut self) {
        self.set.abort_all();
        while self.set.join_next().await.is_some() {}
    }

    pub async fn join_next(&mut self) -> Option<Result<(), JoinError>> {
        self.set.join_next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancels_all() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut g = TaskGroup::new();
                for _ in 0..4 {
                    g.spawn(async {
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    });
                }
                assert_eq!(g.len(), 4);
                g.cancel_all().await;
                assert!(g.is_empty());
            });
    }
}
