use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// Upper bound for the pause between attempts to take a busy [`StorageLock`].
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

/// Limits blocked NFS syscalls, including late acquisitions and lock removal. A permit stays
/// with the blocking operation even after its caller times out, so retries cannot exhaust
/// Tokio's blocking pool. Holders also retain their permit until removal finishes.
static STORAGE_LOCK_SLOTS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(64)));

/// An atomic mkdir lock. Unlike an NFS advisory lock, ownership does not expire when a
/// client's lease expires. A crashed holder leaves the directory behind: it must only be
/// removed after that holder has been fenced (including any pending NFS operations).
#[derive(Debug)]
pub(crate) struct StorageLock {
    path: Option<PathBuf>,
    permit: Option<OwnedSemaphorePermit>,
}

/// An acquisition not yet handed to a volume operation is always safe to clean up on
/// cancellation, including when the result is buffered in the oneshot channel.
struct PendingLock(Option<StorageLock>);

impl Drop for PendingLock {
    fn drop(&mut self) {
        if let Some(lock) = self.0.take() {
            lock.release();
        }
    }
}

impl StorageLock {
    /// Returns None when acquisition cannot complete before the deadline. A late successful
    /// mkdir is cleaned up without running the caller's volume operation.
    pub async fn acquire(path: PathBuf, timeout: Duration) -> std::io::Result<Option<Self>> {
        Self::acquire_with(path, timeout, STORAGE_LOCK_SLOTS.clone(), |path| {
            std::fs::create_dir(path)
        })
        .await
    }

    async fn acquire_with<F>(
        path: PathBuf,
        timeout: Duration,
        slots: Arc<Semaphore>,
        mkdir: F,
    ) -> std::io::Result<Option<Self>>
    where
        F: Fn(&Path) -> std::io::Result<()> + Send + Sync + 'static,
    {
        let deadline = Instant::now() + timeout;
        let mkdir = Arc::new(mkdir);
        let mut delay = Duration::from_millis(20);
        loop {
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let permit =
                match tokio::time::timeout_at(deadline, slots.clone().acquire_owned()).await {
                    Ok(permit) => permit.map_err(std::io::Error::other)?,
                    Err(_) => return Ok(None),
                };
            let attempt = path.clone();
            let mkdir = mkdir.clone();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            tokio::task::spawn_blocking(move || {
                let result = match mkdir(&attempt) {
                    Ok(()) => Ok(Some(PendingLock(Some(Self {
                        path: Some(attempt),
                        permit: Some(permit),
                    })))),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
                    Err(e) => Err(e),
                };
                // The timed-out/cancelled caller cannot start a volume operation. A late
                // successful acquisition can therefore be released safely here.
                let _ = sender.send(result);
            });
            let lock = match tokio::time::timeout_at(deadline, receiver).await {
                Ok(result) => result
                    .map_err(std::io::Error::other)??
                    .map(|mut pending| pending.0.take().unwrap()),
                Err(_) => return Ok(None),
            };
            if Instant::now() >= deadline {
                if let Some(lock) = lock {
                    lock.release();
                }
                return Ok(None);
            }
            if lock.is_some() {
                return Ok(lock);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(delay.min(deadline - now)).await;
            delay = (delay * 2).min(MAX_RETRY_DELAY);
        }
    }

    /// Release only after the protected operation has returned. Dropping an operation during
    /// shutdown or a panic must leave its lock intact: its filesystem syscall may still run.
    pub fn release(mut self) {
        let Some(path) = self.path.take() else { return };
        let permit = self.permit.take();
        // Never block an async worker on NFS. If no runtime remains, leave the lock behind
        // rather than releasing it while a caller might still be changing storage.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || {
                let _permit = permit;
                if let Err(error) = std::fs::remove_dir(&path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(path = %path.display(), %error, "storage lock removal failed; fence its holder before manual removal");
                }
            });
        }
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
    async fn storage_lock_excludes_other_holders_until_released() {
        let path =
            std::env::temp_dir().join(format!("loop-csi-storage-lock-{}", std::process::id()));
        let held = StorageLock::acquire(path.clone(), Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        // Another process must also create the same directory exclusively.
        assert!(
            StorageLock::acquire(path.clone(), Duration::from_millis(50))
                .await
                .unwrap()
                .is_none()
        );
        held.release();
        StorageLock::acquire(path.clone(), Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap()
            .release();
        // Explicit release runs cleanup on the blocking pool.
        wait_for_removal(&path).await;
    }

    async fn wait_for_removal(path: &Path) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while path.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn abandoned_directory_never_expires() {
        let path = std::env::temp_dir().join(format!("loop-csi-abandoned-{}", std::process::id()));
        let held = StorageLock::acquire(path.clone(), Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        // Cancellation/panic drops the guard without proving that pending writes finished.
        drop(held);
        assert!(
            StorageLock::acquire(path.clone(), Duration::from_millis(50))
                .await
                .unwrap()
                .is_none()
        );
        assert!(path.is_dir());
        // Represents manual removal after fencing a crashed holder.
        std::fs::remove_dir(path).unwrap();
    }

    #[tokio::test]
    async fn timed_out_syscall_keeps_its_slot_and_cleans_up_late_success() {
        let path = std::env::temp_dir().join(format!("loop-csi-late-lock-{}", std::process::id()));
        let slots = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let started = Mutex::new(Some(started_tx));
        let resume = Mutex::new(resume_rx);
        let task = tokio::spawn(StorageLock::acquire_with(
            path.clone(),
            Duration::from_millis(100),
            slots.clone(),
            move |path| {
                started.lock().unwrap().take().unwrap().send(()).unwrap();
                resume.lock().unwrap().recv().unwrap();
                std::fs::create_dir(path)
            },
        ));
        started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert_eq!(slots.available_permits(), 0);
        // A retry cannot schedule another filesystem syscall until the old one finishes.
        assert!(
            StorageLock::acquire_with(
                path.clone(),
                Duration::from_millis(20),
                slots.clone(),
                |_| { panic!("a blocked acquisition must retain its slot") }
            )
            .await
            .unwrap()
            .is_none()
        );
        resume_tx.send(()).unwrap();
        // Cleanup must complete before the permit is returned.
        let _permit = tokio::time::timeout(Duration::from_secs(2), slots.acquire())
            .await
            .unwrap()
            .unwrap();
        assert!(!path.exists());
    }
}
