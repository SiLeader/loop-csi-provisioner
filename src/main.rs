use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod controller;
mod identity;
mod mount;
mod node;
mod proto;
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

    #[arg(long, help = "Use plaintext logging instead of structured logging")]
    plaintext_log: bool,
}

#[tokio::main]
async fn main() {
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
}
