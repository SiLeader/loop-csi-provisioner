mod fs;
mod loopdev;
mod mount;

use std::io::ErrorKind;

pub(crate) use mount::*;

#[derive(Debug, Default)]
pub(crate) struct Syscall {}

impl Syscall {
    async fn spawn<F, R>(f: F) -> Result<R, std::io::Error>
    where
        F: FnOnce() -> Result<R, std::io::Error> + Send + 'static,
        R: Send + 'static,
    {
        tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| {
            if e.is_cancelled() {
                Err(std::io::Error::from(ErrorKind::Interrupted))
            } else {
                Err(std::io::Error::other(e))
            }
        })
    }
}
