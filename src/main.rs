use mqttcasters::{bridge, config};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = config::Config::from_env()?;

    info!(
        "mqttcasters starting  |  broker={}  base_topic={}  discovery={}s",
        cfg.mqtt_url, cfg.base_topic, cfg.discovery_timeout_secs
    );

    bridge::run(cfg).await
}
