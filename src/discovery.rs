use mdns_sd::{ServiceDaemon, ServiceEvent};
use tracing::{debug, info, warn};

use crate::types::DiscoveredDevice;

const CHROMECAST_SERVICE_TYPE: &str = "_googlecast._tcp.local.";

/// Start a background mDNS discovery task.
pub fn start_discovery(tx: tokio::sync::mpsc::Sender<crate::types::DiscoveryEvent>) -> anyhow::Result<()> {
    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse(CHROMECAST_SERVICE_TYPE)?;

    tokio::task::spawn_blocking(move || {
        info!("Continuous mDNS discovery started for '{}'", CHROMECAST_SERVICE_TYPE);

        loop {
            match receiver.recv() {
                Ok(event) => {
                    debug!("mDNS Event: {:?}", event);
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let friendly_name = info
                                .get_property_val_str("fn")
                                .unwrap_or_else(|| info.get_hostname())
                                .to_string();

                            let topic_name = sanitise_topic_name(&friendly_name);

                            let address = {
                                let v4: Vec<_> = info.get_addresses_v4().into_iter().collect();
                                if let Some(ip) = v4.first() {
                                    ip.to_string()
                                } else {
                                    match info.get_addresses().iter().next() {
                                        Some(scoped) => {
                                            let s = scoped.to_string();
                                            // Strip IPv6 scope ID (e.g., %eth0) if present
                                            s.split('%').next().unwrap_or(&s).to_string()
                                        }
                                        None => {
                                            warn!("No address for Chromecast '{}', skipping", friendly_name);
                                            continue;
                                        }
                                    }
                                }
                            };

                            let port = info.get_port();
                            let fullname = info.get_fullname().to_string();

                            debug!(
                                "mDNS Resolved: '{}' ({}) at {}:{}",
                                friendly_name, fullname, address, port
                            );

                            let event = crate::types::DiscoveryEvent::Found(DiscoveredDevice {
                                topic_name,
                                friendly_name,
                                address,
                                port,
                            });

                            if let Err(_) = tx.blocking_send(event) {
                                break; // Channel closed, exit
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            debug!("mDNS removal: {}", fullname);
                            let _ = tx.blocking_send(crate::types::DiscoveryEvent::Removed(fullname));
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    warn!("mDNS discovery error: {}", e);
                    break;
                }
            }
        }
        let _ = mdns.shutdown();
    });

    Ok(())
}

/// Convert a friendly name to a safe MQTT sub-topic component.
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
