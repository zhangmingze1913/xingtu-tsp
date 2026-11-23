use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use xingtu_tsp::security::logging::RedactingWriter;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("xingtu_tsp=info,tower_http=info")),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(|| RedactingWriter::new(std::io::stdout())),
        )
        .init();

    let config = xingtu_tsp::config::Config::from_env();
    if let Err(error) = xingtu_tsp::run(config).await {
        tracing::error!(%error, "fatal error");
        std::process::exit(1);
    }
}
