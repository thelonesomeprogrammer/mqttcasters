use std::env;

/// Configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// MQTT broker hostname (default: `localhost`)
    pub mqtt_host: String,
    /// MQTT broker port (default: `1883`)
    pub mqtt_port: u16,
    /// Optional MQTT username
    pub mqtt_username: Option<String>,
    /// Optional MQTT password
    pub mqtt_password: Option<String>,
    /// Base topic prefix (default: `chromecast2mqtt`)
    pub base_topic: String,
    /// Seconds to wait for mDNS discovery on startup (default: `10`)
    pub discovery_timeout_secs: u64,
    /// Seconds between Chromecast reconnection attempts (default: `15`)
    pub reconnect_delay_secs: u64,
}

impl Config {
    /// Build configuration from environment variables.
    ///
    /// | Variable              | Default           | Description                             |
    /// |-----------------------|-------------------|-----------------------------------------|
    /// | `MQTT_HOST`           | `localhost`       | MQTT broker hostname                    |
    /// | `MQTT_PORT`           | `1883`            | MQTT broker port                        |
    /// | `MQTT_USERNAME`       | –                 | Optional MQTT username                  |
    /// | `MQTT_PASSWORD`       | –                 | Optional MQTT password                  |
    /// | `BASE_TOPIC`          | `chromecast2mqtt` | MQTT topic prefix                       |
    /// | `DISCOVERY_TIMEOUT`   | `10`              | mDNS discovery window in seconds        |
    /// | `RECONNECT_DELAY`     | `15`              | Seconds between reconnect attempts      |
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            mqtt_host: env::var("MQTT_HOST").unwrap_or_else(|_| "localhost".to_string()),
            mqtt_port: env::var("MQTT_PORT")
                .unwrap_or_else(|_| "1883".to_string())
                .parse()
                .map_err(|_| anyhow::anyhow!("MQTT_PORT must be a valid port number"))?,
            mqtt_username: env::var("MQTT_USERNAME").ok(),
            mqtt_password: env::var("MQTT_PASSWORD").ok(),
            base_topic: env::var("BASE_TOPIC")
                .unwrap_or_else(|_| "chromecast2mqtt".to_string()),
            discovery_timeout_secs: env::var("DISCOVERY_TIMEOUT")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .map_err(|_| anyhow::anyhow!("DISCOVERY_TIMEOUT must be a positive integer"))?,
            reconnect_delay_secs: env::var("RECONNECT_DELAY")
                .unwrap_or_else(|_| "15".to_string())
                .parse()
                .map_err(|_| anyhow::anyhow!("RECONNECT_DELAY must be a positive integer"))?,
        })
    }
}
