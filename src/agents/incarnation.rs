//! Lifetime of one installed participant runtime (including its CLI retries).
//!
//! Killing the process is asynchronous. This fence separately revokes its
//! host-side writers and waits for writes already admitted to finish before a
//! replacement can be published under the same participant id/slug.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{RwLock, RwLockReadGuard};

#[derive(Debug)]
pub struct Incarnation {
    id: String,
    active: AtomicBool,
    writers: Arc<RwLock<()>>,
}

impl Incarnation {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            active: AtomicBool::new(true),
            writers: Arc::new(RwLock::new(())),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Keep the lease through every await that can mutate host state. A check
    /// followed by an unguarded async write would still race retirement.
    pub async fn enter(&self) -> Option<RwLockReadGuard<'_, ()>> {
        if !self.is_active() {
            return None;
        }
        let lease = self.writers.read().await;
        self.is_active().then_some(lease)
    }

    /// A queued control keeps its lease through the eventual delivery, even
    /// when selection and execution live in different functions.
    pub async fn enter_owned(&self) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        if !self.is_active() {
            return None;
        }
        let lease = Arc::clone(&self.writers).read_owned().await;
        self.is_active().then_some(lease)
    }

    /// Revoke new writers immediately, then drain admitted ones. Cancellation
    /// cannot reactivate the old runtime; a retry can finish draining it.
    pub async fn retire(&self) {
        self.active.store(false, Ordering::Release);
        let _drained = self.writers.write().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retirement_waits_for_an_admitted_async_write_and_refuses_new_ones() {
        let incarnation = Incarnation::new();
        let lease = incarnation.enter().await.unwrap();
        let retired = incarnation.clone();
        let mut task = tokio::spawn(async move { retired.retire().await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut task)
                .await
                .is_err()
        );
        assert!(!incarnation.is_active());
        assert!(incarnation.enter().await.is_none());
        drop(lease);
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(incarnation.enter().await.is_none());
        let replacement = Incarnation::new();
        assert_ne!(replacement.id(), incarnation.id());
        assert!(replacement.enter().await.is_some());
    }

    #[tokio::test]
    async fn a_cancelled_retirement_never_reactivates_the_old_runtime() {
        let incarnation = Incarnation::new();
        let lease = incarnation.enter().await.unwrap();
        let retired = incarnation.clone();
        let mut task = tokio::spawn(async move { retired.retire().await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut task)
                .await
                .is_err()
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(lease);
        assert!(incarnation.enter().await.is_none());
        incarnation.retire().await;
    }
}
