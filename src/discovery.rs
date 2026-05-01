use std::collections::HashMap;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent};
use tracing::{debug, info, warn};

use crate::types::DiscoveredDevice;

const CHROMECAST_SERVICE_TYPE: &str = "_googlecast._tcp.local.";

/// Discover Chromecast devices on the local network via mDNS-SD.
///
/// Browses `_googlecast._tcp.local.` for `timeout_secs` seconds and returns all
/// devices found.  Devices that appear and are immediately removed before the
/// window closes are excluded.
pub fn discover_devices(timeout_secs: u64) -> anyhow::Result<Vec<DiscoveredDevice>> {
    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse(CHROMECAST_SERVICE_TYPE)?;

    info!(
        "Browsing mDNS for Chromecast devices ({} s)…",
        timeout_secs
    );

    let mut devices: HashMap<String, DiscoveredDevice> = HashMap::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match receiver.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let friendly_name = info
                    .get_property_val_str("fn")
                    .unwrap_or_else(|| info.get_hostname())
                    .to_string();

                let topic_name = sanitise_topic_name(&friendly_name);

                // Prefer IPv4 addresses; fall back to IPv6.
                let address = {
                    let v4: Vec<_> = info.get_addresses_v4().into_iter().collect();
                    if let Some(ip) = v4.first() {
                        ip.to_string()
                    } else {
                        // Fall back to any address (IPv6 or otherwise); ScopedIp
                        // implements Display as its IP address string.
                        match info.get_addresses().iter().next() {
                            Some(scoped) => scoped.to_string(),
                            None => {
                                warn!(
                                    "No address for Chromecast '{}', skipping",
                                    friendly_name
                                );
                                continue;
                            }
                        }
                    }
                };

                let port = info.get_port();
                let fullname = info.get_fullname().to_string();

                info!(
                    "Discovered: '{}' ({}) at {}:{}",
                    friendly_name, fullname, address, port
                );

                devices.insert(
                    fullname,
                    DiscoveredDevice {
                        topic_name,
                        friendly_name,
                        address,
                        port,
                    },
                );
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                debug!("mDNS removal: {}", fullname);
                devices.remove(&fullname);
            }
            Ok(_) => {}
            Err(_) => {
                // recv_timeout returns Err on timeout – check deadline.
                if std::time::Instant::now() >= deadline {
                    break;
                }
            }
        }
    }

    let _ = mdns.shutdown();

    info!("Discovery finished: {} device(s) found", devices.len());
    Ok(devices.into_values().collect())
}

/// Convert a friendly name to a safe MQTT sub-topic component.
///
/// Lowercases the ASCII representation and replaces every character that is
/// not ASCII alphanumeric or `-` with an underscore, ensuring only printable
/// ASCII appears in MQTT topic segments.
pub fn sanitise_topic_name(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::sanitise_topic_name;

    #[test]
    fn sanitise_spaces_and_special_chars() {
        assert_eq!(sanitise_topic_name("Living Room TV"), "living_room_tv");
        assert_eq!(sanitise_topic_name("Küche"), "k_che");
        assert_eq!(sanitise_topic_name("my-device"), "my-device");
        assert_eq!(sanitise_topic_name("Device #1!"), "device__1_");
    }
}
