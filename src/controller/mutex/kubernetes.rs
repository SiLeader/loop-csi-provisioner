use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use k8s_openapi::jiff::Timestamp;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

#[derive(Clone)]
pub(crate) struct KubernetesControllerMutex {
    api: Api<Lease>,
    meta: LeaseMeta,
    is_leader: Arc<AtomicBool>,
    cancellation_token: CancellationToken,
}

const DEFAULT_NAMESPACE: &str = "loop-csi";
const DEFAULT_LEASE_NAME: &str = "controller";
const DEFAULT_DURATION: Duration = Duration::seconds(10);

const FIELD_MANAGER: &str = "sileader.net/loop-csi/controller";

#[derive(Clone)]
pub(crate) struct LeaseMeta {
    namespace: String,
    lease_name: String,
    identity: String,
    duration: Duration,
}

impl KubernetesControllerMutex {
    fn new(client: Client, meta: LeaseMeta) -> Self {
        Self {
            api: Api::namespaced(client, &meta.namespace),
            meta,
            is_leader: Arc::new(AtomicBool::new(false)),
            cancellation_token: CancellationToken::new(),
        }
    }

    pub(crate) async fn try_default(meta: LeaseMeta) -> anyhow::Result<Self> {
        let client = Client::try_default().await?;
        Ok(Self::new(client, meta))
    }

    async fn start(&self) {
        let this = self.clone();
        tokio::spawn(this.renew_loop());
    }

    async fn renew_loop(self) {
        while !self.cancellation_token.is_cancelled() {
            match self.renew_lease().await {
                Ok(updated) => {
                    if updated {
                        info!("Lease holder is got");
                    } else {
                        debug!("Not lease holder");
                    }
                }
                Err(e) => {
                    error!("Failed to renew lease: {}", e);
                }
            }
        }
    }

    async fn renew_lease(&self) -> anyhow::Result<bool> {
        let lease = self.api.get_opt(&self.meta.lease_name).await?;

        let Some(next) = self.meta.to_lease(lease)? else {
            return Ok(false);
        };

        match self
            .api
            .patch(
                &self.meta.lease_name,
                &PatchParams::apply(FIELD_MANAGER),
                &Patch::Apply(next),
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(e)) if e.is_conflict() => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

impl LeaseMeta {
    pub fn new(identity: String) -> Self {
        Self {
            namespace: DEFAULT_NAMESPACE.to_string(),
            lease_name: DEFAULT_LEASE_NAME.to_string(),
            duration: DEFAULT_DURATION,
            identity,
        }
    }

    pub fn with_namespace(mut self, namespace: &str) -> Self {
        self.namespace = namespace.to_string();
        self
    }

    pub fn with_lease_name(mut self, lease_name: &str) -> Self {
        self.lease_name = lease_name.to_string();
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    fn to_lease(&self, current: Option<Lease>) -> anyhow::Result<Option<Lease>> {
        let now = Utc::now();
        let time = MicroTime(Timestamp::from_millisecond(now.timestamp_millis())?);

        let resource_version = current
            .as_ref()
            .and_then(|l| l.metadata.resource_version.clone());

        let spec = current.and_then(|l| l.spec);
        let is_current_holder = spec
            .as_ref()
            .and_then(|s| s.holder_identity.as_ref())
            .is_some_and(|holder| holder == self.identity.as_str());

        if let Some(renew_time) = spec.as_ref().and_then(|s| s.renew_time.clone()) {
            let renew_time = DateTime::from_timestamp_millis(renew_time.0.as_millisecond())
                .context("Failed to parse renew time")?;
            if now - renew_time < self.duration {
                return Ok(None);
            }
        }

        Ok(Some(Lease {
            metadata: ObjectMeta {
                resource_version,
                labels: Some(BTreeMap::from([
                    (
                        "app.kubernetes.io/name".to_string(),
                        "controller".to_string(),
                    ),
                    (
                        "app.kubernetes.io/part-of".to_string(),
                        "loop-csi".to_string(),
                    ),
                ])),
                name: Some(self.lease_name.clone()),
                namespace: Some(self.namespace.clone()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                acquire_time: Some(if is_current_holder {
                    spec.as_ref()
                        .and_then(|s| s.acquire_time.clone())
                        .unwrap_or_else(|| time.clone())
                } else {
                    time.clone()
                }),
                holder_identity: Some(self.identity.clone()),
                lease_duration_seconds: Some(self.duration.num_seconds() as i32),
                lease_transitions: Some(
                    spec.and_then(|s| s.lease_transitions)
                        .map(|l| l + 1)
                        .unwrap_or(0),
                ),
                renew_time: Some(time.clone()),
                ..Default::default()
            }),
        }))
    }
}
