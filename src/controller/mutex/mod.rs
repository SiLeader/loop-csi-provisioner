#[cfg(feature = "kubernetes")]
pub mod kubernetes;
#[cfg(not(feature = "kubernetes"))]
mod single;

use std::sync::Arc;
use tonic::async_trait;

#[async_trait]
pub(crate) trait ControllerMutex: Send + Sync {
    async fn is_leader(&self) -> bool;

    /// Gives up leadership so that another instance can take over without waiting for expiry.
    async fn release(&self) {}
}

#[derive(Clone)]
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
        self.inner.is_leader().await
    }

    pub async fn release(&self) {
        self.inner.release().await
    }
}

/// `namespace` defaults to the namespace the pod runs in; `identity` must be unique per instance.
pub async fn default_controller_mutex(
    namespace: Option<String>,
    identity: String,
) -> anyhow::Result<ControllerLeaseHolder> {
    #[cfg(feature = "kubernetes")]
    {
        anyhow::ensure!(
            !identity.is_empty(),
            "--pod-name (or POD_NAME) is required to identify this controller instance"
        );
        let meta = kubernetes::LeaseMeta::new(identity, namespace);
        let cm = kubernetes::KubernetesControllerMutex::try_default(meta).await?;
        cm.start();
        Ok(ControllerLeaseHolder::new(cm))
    }
    #[cfg(not(feature = "kubernetes"))]
    {
        let _ = (namespace, identity);
        Ok(ControllerLeaseHolder::new(
            single::SingleNodeControllerMutex,
        ))
    }
}
