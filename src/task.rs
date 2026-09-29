use std::future::Future;
use tracing::warn;

/// Runs `future` on its own task and waits for it, so it finishes even if the caller stops
/// waiting.
///
/// tonic drops a handler's future when the client cancels or its deadline passes. Operations
/// that change state (attaching loop devices, formatting, writing metadata) must not stop
/// half-way: the per-volume lock would be released while e.g. `mkfs` keeps running, and a
/// retry would observe a half-initialized device.
///
/// Failures are logged under `rpc`, since the CO often reports them only in its own events.
pub(crate) async fn run_to_completion<F, T, E>(rpc: &str, future: F) -> Result<T, tonic::Status>
where
    F: Future<Output = Result<T, E>> + Send + 'static,
    T: Send + 'static,
    E: Into<tonic::Status> + Send + 'static,
{
    let result = match tokio::spawn(future).await {
        Ok(result) => result.map_err(Into::into),
        Err(e) => Err(tonic::Status::internal(format!("Operation failed: {e}"))),
    };
    if let Err(status) = &result {
        warn!(rpc, code = ?status.code(), message = status.message(), "request failed");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn keeps_running_after_the_caller_is_dropped() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let call = run_to_completion("Test", async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            flag.store(true, Ordering::SeqCst);
            Ok::<_, tonic::Status>(())
        });
        // The caller gives up long before the operation is done.
        assert!(
            tokio::time::timeout(Duration::from_millis(1), call)
                .await
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(finished.load(Ordering::SeqCst));
    }
}
