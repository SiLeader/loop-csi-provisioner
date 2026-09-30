use crate::controller::mutex::ControllerMutex;
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use k8s_openapi::jiff::Timestamp;
use kube::api::PostParams;
use kube::{Api, Client};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tonic::async_trait;
use tracing::{debug, error, info, warn};

const DEFAULT_LEASE_NAME: &str = "loop-csi-controller";
const DEFAULT_DURATION: Duration = Duration::from_secs(10);
const DEFAULT_RETRY_PERIOD: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(crate) struct KubernetesControllerMutex {
    api: Api<Lease>,
    meta: LeaseMeta,
    /// Local deadline until which this instance may consider itself the leader.
    ///
    /// It is only extended by a successful renewal, so a leader that can no longer reach the API
    /// server stops serving once its lease could have been taken over.
    leader_until: Arc<Mutex<Option<Instant>>>,
    cancellation_token: CancellationToken,
}

#[derive(Clone)]
pub(crate) struct LeaseMeta {
    namespace: Option<String>,
    lease_name: String,
    identity: String,
    duration: Duration,
    retry_period: Duration,
}

#[async_trait]
impl ControllerMutex for KubernetesControllerMutex {
    async fn is_leader(&self) -> bool {
        self.leader_until
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|until| Instant::now() < until)
    }

    async fn release(&self) {
        self.cancellation_token.cancel();
        self.set_leader_until(None);
        if let Err(e) = self.release_lease().await {
            warn!("Failed to release lease: {}", e);
        }
    }
}

impl KubernetesControllerMutex {
    pub(crate) async fn try_default(meta: LeaseMeta) -> anyhow::Result<Self> {
        let client = Client::try_default().await?;
        let namespace = meta
            .namespace
            .clone()
            .unwrap_or_else(|| client.default_namespace().to_string());
        Ok(Self {
            api: Api::namespaced(client, &namespace),
            meta,
            leader_until: Arc::new(Mutex::new(None)),
            cancellation_token: CancellationToken::new(),
        })
    }

    pub(crate) fn start(&self) {
        let this = self.clone();
        tokio::spawn(this.renew_loop());
    }

    fn set_leader_until(&self, until: Option<Instant>) {
        *self.leader_until.lock().unwrap_or_else(|e| e.into_inner()) = until;
    }

    async fn renew_loop(self) {
        let mut was_leader = false;
        loop {
            // Measured before the request so that the deadline never outlives the lease.
            let attempt_started = Instant::now();
            let is_leader = match self.renew_lease().await {
                Ok(true) => {
                    self.set_leader_until(Some(attempt_started + self.meta.duration));
                    true
                }
                Ok(false) => {
                    self.set_leader_until(None);
                    false
                }
                Err(e) => {
                    // Keep the previous deadline: leadership lapses on its own if renewals
                    // keep failing.
                    error!("Failed to renew lease: {}", e);
                    self.is_leader().await
                }
            };
            if is_leader != was_leader {
                if is_leader {
                    info!(identity = %self.meta.identity, "Acquired controller leadership");
                } else {
                    info!(identity = %self.meta.identity, "Lost controller leadership");
                }
                was_leader = is_leader;
            } else {
                debug!(is_leader, "Lease checked");
            }

            tokio::select! {
                _ = self.cancellation_token.cancelled() => break,
                _ = tokio::time::sleep(self.meta.retry_period) => {}
            }
        }
    }

    /// Tries to acquire or renew the lease and returns whether this instance holds it afterwards.
    async fn renew_lease(&self) -> anyhow::Result<bool> {
        let name = &self.meta.lease_name;
        let now = MicroTime(Timestamp::now());
        let params = PostParams::default();

        let result = match self.api.get_opt(name).await? {
            None => self.api.create(&params, &self.meta.new_lease(now)).await,
            Some(current) => {
                let Some(next) = self.meta.next_lease(current, now) else {
                    return Ok(false);
                };
                // `next` carries the observed resourceVersion, so a concurrent writer makes
                // this a conflict instead of a silent overwrite.
                self.api.replace(name, &params, &next).await
            }
        };
        match result {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(e)) if e.code == 409 => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Clears the holder if it is this instance so that a standby can take over immediately.
    async fn release_lease(&self) -> anyhow::Result<()> {
        let Some(mut lease) = self.api.get_opt(&self.meta.lease_name).await? else {
            return Ok(());
        };
        let Some(spec) = lease.spec.as_mut() else {
            return Ok(());
        };
        if spec.holder_identity.as_deref() != Some(self.meta.identity.as_str()) {
            return Ok(());
        }
        spec.holder_identity = None;
        spec.renew_time = None;
        match self
            .api
            .replace(&self.meta.lease_name, &PostParams::default(), &lease)
            .await
        {
            Ok(_) => {
                info!("Released controller lease");
                Ok(())
            }
            Err(kube::Error::Api(e)) if e.code == 409 => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl LeaseMeta {
    pub fn new(identity: String, namespace: Option<String>) -> Self {
        Self {
            namespace,
            lease_name: DEFAULT_LEASE_NAME.to_string(),
            identity,
            duration: DEFAULT_DURATION,
            retry_period: DEFAULT_RETRY_PERIOD,
        }
    }

    fn lease_duration_seconds(&self) -> i32 {
        self.duration.as_secs() as i32
    }

    fn new_lease(&self, now: MicroTime) -> Lease {
        Lease {
            metadata: ObjectMeta {
                name: Some(self.lease_name.clone()),
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
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                acquire_time: Some(now.clone()),
                holder_identity: Some(self.identity.clone()),
                lease_duration_seconds: Some(self.lease_duration_seconds()),
                lease_transitions: Some(0),
                renew_time: Some(now),
                ..Default::default()
            }),
        }
    }

    /// Returns the lease to write back, or `None` if another holder's lease is still valid.
    fn next_lease(&self, mut lease: Lease, now: MicroTime) -> Option<Lease> {
        let spec = lease.spec.get_or_insert_with(Default::default);
        let holder = spec.holder_identity.as_deref().filter(|h| !h.is_empty());
        let is_current_holder = holder == Some(self.identity.as_str());

        if !is_current_holder && holder.is_some() {
            // Judge expiry by the duration the holder declared, not by our own setting.
            let holder_duration_ms = spec
                .lease_duration_seconds
                .map_or(self.duration.as_millis() as i64, |s| i64::from(s) * 1000);
            let alive = spec
                .renew_time
                .as_ref()
                .or(spec.acquire_time.as_ref())
                .is_some_and(|t| {
                    now.0.as_millisecond() - t.0.as_millisecond() < holder_duration_ms
                });
            if alive {
                return None;
            }
        }

        if !is_current_holder {
            spec.acquire_time = Some(now.clone());
            spec.lease_transitions = Some(spec.lease_transitions.unwrap_or(0) + 1);
        }
        spec.holder_identity = Some(self.identity.clone());
        spec.lease_duration_seconds = Some(self.lease_duration_seconds());
        spec.renew_time = Some(now);
        Some(lease)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> MicroTime {
        MicroTime(Timestamp::from_second(secs).unwrap())
    }

    fn lease(holder: Option<&str>, renewed: i64, transitions: i32) -> Lease {
        Lease {
            metadata: ObjectMeta {
                resource_version: Some("7".to_string()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                acquire_time: Some(at(renewed - 50)),
                holder_identity: holder.map(str::to_string),
                lease_duration_seconds: Some(10),
                lease_transitions: Some(transitions),
                renew_time: Some(at(renewed)),
                ..Default::default()
            }),
        }
    }

    fn meta() -> LeaseMeta {
        LeaseMeta::new("me".to_string(), None)
    }

    #[test]
    fn holder_renews_a_fresh_lease_without_a_transition() {
        let next = meta()
            .next_lease(lease(Some("me"), 100, 3), at(103))
            .unwrap();
        let spec = next.spec.unwrap();
        assert_eq!(spec.renew_time, Some(at(103)));
        assert_eq!(spec.acquire_time, Some(at(50)));
        assert_eq!(spec.lease_transitions, Some(3));
        assert_eq!(next.metadata.resource_version.as_deref(), Some("7"));
    }

    #[test]
    fn other_holders_valid_lease_is_respected() {
        assert!(
            meta()
                .next_lease(lease(Some("other"), 100, 3), at(105))
                .is_none()
        );
    }

    #[test]
    fn expired_lease_is_taken_over() {
        let spec = meta()
            .next_lease(lease(Some("other"), 100, 3), at(111))
            .unwrap()
            .spec
            .unwrap();
        assert_eq!(spec.holder_identity.as_deref(), Some("me"));
        assert_eq!(spec.acquire_time, Some(at(111)));
        assert_eq!(spec.lease_transitions, Some(4));
    }

    #[test]
    fn released_lease_is_taken_over_immediately() {
        let mut released = lease(None, 100, 3);
        released.spec.as_mut().unwrap().renew_time = None;
        let spec = meta().next_lease(released, at(101)).unwrap().spec.unwrap();
        assert_eq!(spec.holder_identity.as_deref(), Some("me"));
        assert_eq!(spec.lease_transitions, Some(4));
    }
}
