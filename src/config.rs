use clap::Parser;

/// Configuration loaded from environment variables.
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// MQTT broker URL (e.g., `mqtt://user:pass@192.168.1.100:1883`)
    #[arg(long, env = "MQTT_URL", default_value = "mqtt://localhost:1883")]
    pub mqtt_url: String,

    /// Base topic prefix (default: `mqttcasters`)
    #[arg(long, env = "MQTT_BASE_TOPIC", default_value = "mqttcasters")]
    pub base_topic: String,

    /// Seconds to wait for mDNS discovery on startup (default: `10`)
    #[arg(long, env = "DISCOVERY_TIMEOUT", default_value = "10")]
    pub discovery_timeout_secs: u64,

    /// Seconds between Chromecast reconnection attempts (default: `15`)
    #[arg(long, env = "RECONNECT_DELAY", default_value = "15")]
    pub reconnect_delay_secs: u64,
}

impl Config {
    /// Build configuration from environment variables and command line arguments.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self::parse())
    }
}
