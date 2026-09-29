use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::OwnedMutexGuard;

/// Serializes operations that target the same key (e.g. a volume ID or a path).
#[derive(Debug, Default)]
pub(crate) struct KeyedLocks {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl KeyedLocks {
    pub async fn lock(&self, key: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
            // Drop entries nobody holds or waits for, so the map does not grow without bound.
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
            locks.entry(key.to_string()).or_default().clone()
        };
        lock.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn same_key_is_exclusive_and_other_keys_are_not() {
        let locks = KeyedLocks::default();
        let guard = locks.lock("a").await;
        let _other = locks.lock("b").await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), locks.lock("a"))
                .await
                .is_err()
        );
        drop(guard);
        let _again = locks.lock("a").await;
    }

    #[tokio::test]
    async fn released_entries_are_pruned() {
        let locks = KeyedLocks::default();
        drop(locks.lock("a").await);
        drop(locks.lock("b").await);
        assert_eq!(locks.locks.lock().unwrap().len(), 1);
    }
}
