use clap::Parser;
use std::net::SocketAddr;
use std::path::Path;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::controller::LoopCsiController;
use crate::controller::operator::ControllerOperator;
use crate::node::LoopCsiNode;
use crate::node::operator::NodeOperator;
use crate::proto::csi::v1::controller_server::ControllerServer;
use crate::proto::csi::v1::identity_server::IdentityServer;
use crate::proto::csi::v1::node_server::NodeServer;

mod controller;
mod filesystem;
mod identity;
mod mount;
mod node;
mod proto;
mod syscall;
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

    #[arg(long, help = "Use plaintext logging instead of structured logging")]
    plaintext_log: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    {
        let subscriber = tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env());
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

    let node = if node_api {
        Some(NodeServer::new(LoopCsiNode::new(
            NodeOperator::new(args.base_directory.clone()).await?,
        )))
    } else {
        None
    };
    let controller = if controller_api {
        Some(ControllerServer::new(LoopCsiController::new(
            ControllerOperator::new(args.default_size, args.base_directory).await?,
        )))
    } else {
        None
    };
    let identity = identity_api.then(|| IdentityServer::new(identity::NfsLoopCsiIdentity {}));

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
        let listener = UnixListener::bind(path)?;
        info!(listen = %args.listen, "CSI gRPC server listening");
        let result = router
            .serve_with_incoming_shutdown(UnixListenerStream::new(listener), shutdown_signal())
            .await;
        tokio::fs::remove_file(path).await?;
        result?;
    } else if let Some(address) = args.listen.strip_prefix("tcp://") {
        let address: SocketAddr = address.parse()?;
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
