#[cfg(feature = "kubernetes")]
pub mod kubernetes;

use crate::controller::error::ControllerError;
use std::sync::Arc;
use tonic::async_trait;

#[async_trait]
pub(crate) trait ControllerMutex: Send + Sync {
    async fn is_leader(&self) -> Result<bool, ControllerError>;
}

pub(crate) struct SingleNodeControllerMutex;

#[async_trait]
impl ControllerMutex for SingleNodeControllerMutex {
    async fn is_leader(&self) -> Result<bool, ControllerError> {
        Ok(true)
    }
}

pub(crate) struct ControllerLeaseHolder {
    inner: Arc<dyn ControllerMutex>,
}

impl ControllerLeaseHolder {
    pub fn new<C: ControllerMutex + 'static>(cm: C) -> Self {
        Self {
            inner: Arc::new(cm),
        }
    }

    pub async fn is_leader(&self) -> bool {
        self.inner.is_leader().await.is_ok_and(|l| l)
    }
}
