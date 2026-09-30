use clap::Parser;
use std::net::SocketAddr;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::controller::LoopCsiController;
use crate::controller::mutex::{DEFAULT_NAMESPACE, default_controller_mutex};
use crate::controller::operator::ControllerOperator;
use crate::mount::MountManager;
use crate::node::LoopCsiNode;
use crate::node::operator::NodeOperator;
use crate::proto::csi::v1::controller_server::ControllerServer;
use crate::proto::csi::v1::identity_server::IdentityServer;
use crate::proto::csi::v1::node_server::NodeServer;

mod capability;
mod controller;
mod filesystem;
mod identity;
mod lock;
mod mount;
mod node;
mod proto;
mod syscall;
mod task;
mod volume_id;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, help = "Enable Node gRPC API")]
    node_api: bool,

    #[arg(long, help = "Enable Controller gRPC API")]
    controller_api: bool,

    #[arg(long, help = "Enable Identity gRPC API")]
    identity_api: bool,

    #[arg(
        long,
        help = "gRPC listening address with scheme, e.g. unix:///csi/csi.sock or tcp://127.0.0.1:1234",
        default_value = "unix:///csi/csi.sock"
    )]
    listen: String,

    #[arg(
        long,
        default_value = "/var/lib/loop-csi-provisioner",
        help = "Base directory for mounted backing storage"
    )]
    base_directory: String,

    #[arg(
        long,
        default_value_t = 1_073_741_824,
        help = "Default volume size in bytes"
    )]
    default_size: i64,

    #[arg(
        long = "allowed-url-prefix",
        help = "Only allow storage URLs equal to or below this prefix, e.g. nfs://nfs.example.com/export. \
                Repeatable. Without it, any URL found in a volume ID or StorageClass is mounted"
    )]
    allowed_url_prefixes: Vec<String>,

    #[arg(
        long,
        env = "NODE_ID",
        default_value = "",
        help = "Node identifier reported by the Node API; must match the name the CO uses for \
                ControllerPublishVolume (in Kubernetes, the node name). Required with the Node API"
    )]
    node_id: String,

    #[arg(long, env = "NAMESPACE", default_value = DEFAULT_NAMESPACE, help = "Kubernetes namespace for the Lease resource")]
    namespace: String,

    #[arg(
        long,
        env = "POD_NAME",
        default_value = "",
        help = "Kubernetes pod name for the Lease resource"
    )]
    pod_name: String,

    #[arg(long, help = "Use plaintext logging instead of structured logging")]
    plaintext_log: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    {
        // `from_default_env` alone would log errors only when RUST_LOG is unset.
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let subscriber = tracing_subscriber::fmt().with_env_filter(filter);
        if args.plaintext_log {
            subscriber.init();
        } else {
            subscriber.json().init();
        }
    }

    info!("Starting loop-csi-provisioner version {}", VERSION);

    anyhow::ensure!(args.default_size > 0, "--default-size must be positive");
    let any_api = args.node_api || args.controller_api || args.identity_api;
    let node_api = args.node_api || !any_api;
    let controller_api = args.controller_api || !any_api;
    let identity_api = args.identity_api || !any_api;

    if args.allowed_url_prefixes.is_empty() {
        warn!("--allowed-url-prefix is not set: any storage URL in a volume ID will be mounted");
    }
    // Shared so that mounting is serialized even when several APIs run in this process.
    let mounter =
        Arc::new(MountManager::default().with_allowed_prefixes(args.allowed_url_prefixes));

    let node = if node_api {
        anyhow::ensure!(
            !args.node_id.is_empty(),
            "--node-id (or NODE_ID) is required when serving the Node API"
        );
        Some(NodeServer::new(LoopCsiNode::new(
            NodeOperator::new(args.base_directory.clone(), mounter.clone()).await?,
            args.node_id.clone(),
        )))
    } else {
        None
    };
    let controller = if controller_api {
        let controller_mutex = default_controller_mutex(args.namespace, args.pod_name).await;

        Some(ControllerServer::new(LoopCsiController::new(
            ControllerOperator::new(args.default_size, args.base_directory, mounter).await?,
            controller_mutex,
        )))
    } else {
        None
    };
    let identity = identity_api.then(|| {
        IdentityServer::new(identity::LoopCsiIdentity {
            controller_service: controller_api,
        })
    });

    let router = Server::builder()
        .add_optional_service(node)
        .add_optional_service(controller)
        .add_optional_service(identity);

    if let Some(path) = args.listen.strip_prefix("unix://") {
        let path = Path::new(path);
        anyhow::ensure!(path.is_absolute(), "Unix socket path must be absolute");
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // A socket left behind by a crashed run would make bind fail with EADDRINUSE.
        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.file_type().is_socket() => {
                tokio::fs::remove_file(path).await?
            }
            Ok(_) => anyhow::bail!("{} exists and is not a socket", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let listener = UnixListener::bind(path)?;
        info!(listen = %args.listen, "CSI gRPC server listening");
        let result = router
            .serve_with_incoming_shutdown(UnixListenerStream::new(listener), shutdown_signal())
            .await;
        // Report the server's own result even if the socket file is already gone.
        let _ = tokio::fs::remove_file(path).await;
        result?;
    } else if let Some(address) = args.listen.strip_prefix("tcp://") {
        let address: SocketAddr = address.parse()?;
        if !address.ip().is_loopback() {
            warn!(
                "the gRPC API has no authentication or TLS; do not expose it beyond trusted hosts"
            );
        }
        info!(listen = %args.listen, "CSI gRPC server listening");
        router
            .serve_with_shutdown(address, shutdown_signal())
            .await?;
    } else {
        anyhow::bail!("--listen must start with unix:// or tcp://");
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(unix)]
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    #[cfg(not(unix))]
    ctrl_c.await;
}
