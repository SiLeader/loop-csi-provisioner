use rustix::fs::FlockOperation;
use rustix::io::Errno;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::OwnedMutexGuard;
use tokio::time::Instant;

/// Upper bound for the pause between attempts to take a busy [`FileLock`].
const MAX_RETRY_DELAY: Duration = Duration::from_secs(1);

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

/// An exclusive `flock(2)` on a file, released when dropped.
///
/// On NFS the kernel implements `flock` with byte-range locks held by the server, so the lock
/// also excludes processes on other hosts that mount the same export.
#[derive(Debug)]
pub(crate) struct FileLock {
    _file: File,
}

impl FileLock {
    /// Locks `path`, creating the file if needed. Returns `None` if someone else still holds
    /// the lock after `timeout`.
    pub async fn acquire(path: PathBuf, timeout: Duration) -> std::io::Result<Option<Self>> {
        let deadline = Instant::now() + timeout;
        let mut delay = Duration::from_millis(20);
        loop {
            let attempt = path.clone();
            // Locking goes to the NFS server, so keep it off the async worker threads.
            let file = tokio::task::spawn_blocking(move || try_lock(&attempt))
                .await
                .map_err(std::io::Error::other)??;
            if let Some(file) = file {
                return Ok(Some(Self { _file: file }));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(delay.min(deadline - now)).await;
            delay = (delay * 2).min(MAX_RETRY_DELAY);
        }
    }
}

fn try_lock(path: &Path) -> std::io::Result<Option<File>> {
    // NFS emulates exclusive `flock` with write locks, which need a writable descriptor.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(file)),
        Err(Errno::WOULDBLOCK) => Ok(None),
        Err(e) => Err(e.into()),
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

    #[tokio::test]
    async fn file_lock_excludes_other_holders_until_dropped() {
        let path = std::env::temp_dir().join(format!("loop-csi-file-lock-{}", std::process::id()));
        let held = FileLock::acquire(path.clone(), Duration::ZERO)
            .await
            .unwrap()
            .unwrap();
        // Each acquisition opens its own descriptor, as another process would.
        assert!(
            FileLock::acquire(path.clone(), Duration::from_millis(50))
                .await
                .unwrap()
                .is_none()
        );
        drop(held);
        assert!(
            FileLock::acquire(path.clone(), Duration::ZERO)
                .await
                .unwrap()
                .is_some()
        );
        std::fs::remove_file(path).unwrap();
    }
}
