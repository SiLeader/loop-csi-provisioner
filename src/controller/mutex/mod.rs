#[cfg(feature = "kubernetes")]
pub mod kubernetes;
#[cfg(not(feature = "kubernetes"))]
mod single;

use crate::controller::error::ControllerError;
use std::sync::Arc;
use tonic::async_trait;

pub use kubernetes::DEFAULT_NAMESPACE;

#[async_trait]
pub(crate) trait ControllerMutex: Send + Sync {
    async fn is_leader(&self) -> Result<bool, ControllerError>;
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

pub async fn default_controller_mutex(
    namespace: String,
    identity: String,
) -> ControllerLeaseHolder {
    #[cfg(feature = "kubernetes")]
    {
        let meta = kubernetes::LeaseMeta::new(identity).with_namespace(namespace);
        let cm = kubernetes::KubernetesControllerMutex::try_default(meta)
            .await
            .expect("Failed to create KubernetesControllerMutex");
        cm.start();
        ControllerLeaseHolder::new(cm)
    }
    #[cfg(not(feature = "kubernetes"))]
    {
        let _ = namespace;
        let _ = identity;
        ControllerLeaseHolder::new(SingleNodeControllerMutex)
    }
}
