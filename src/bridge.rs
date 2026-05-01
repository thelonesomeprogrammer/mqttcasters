use std::collections::HashMap;
use std::time::Duration;

use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::device::{spawn_device_thread, CommandQueue};
use crate::discovery::discover_devices;
use crate::types::{DeviceCommand, StateUpdate};

/// Run the bridge: discover devices, connect to each, and relay state ↔ MQTT.
pub async fn run(cfg: Config) -> anyhow::Result<()> {
    // ------------------------------------------------------------------
    // 1. Discover Chromecast devices via mDNS.
    // ------------------------------------------------------------------
    let devices = tokio::task::spawn_blocking({
        let timeout = cfg.discovery_timeout_secs;
        move || discover_devices(timeout)
    })
    .await??;

    if devices.is_empty() {
        warn!("No Chromecast devices found during discovery window. Continuing anyway – devices found later (after restart) will be picked up.");
    }

    // ------------------------------------------------------------------
    // 2. Create the MQTT client.
    // ------------------------------------------------------------------
    let client_id = format!("chromecast2mqtt-{}", std::process::id());
    let mut mqttopts = MqttOptions::new(&client_id, &cfg.mqtt_host, cfg.mqtt_port);
    mqttopts.set_keep_alive(Duration::from_secs(30));
    mqttopts.set_clean_session(true);

    if let (Some(user), Some(pass)) = (&cfg.mqtt_username, &cfg.mqtt_password) {
        mqttopts.set_credentials(user, pass);
    }

    let (client, eventloop) = AsyncClient::new(mqttopts, 64);

    // ------------------------------------------------------------------
    // 3. Spawn device threads and collect their state channels.
    // ------------------------------------------------------------------
    let (state_tx, state_rx) = mpsc::unbounded_channel::<StateUpdate>();
    let reconnect = Duration::from_secs(cfg.reconnect_delay_secs);

    // Map topic_name → CommandQueue so incoming MQTT commands can be routed.
    let mut cmd_queues: HashMap<String, CommandQueue> = HashMap::new();

    for device in devices {
        info!(
            "Starting monitor thread for '{}' ({topic})",
            device.friendly_name,
            topic = device.topic_name
        );
        let queue = spawn_device_thread(device.clone(), state_tx.clone(), reconnect);
        cmd_queues.insert(device.topic_name.clone(), queue);
    }

    // ------------------------------------------------------------------
    // 4. Subscribe to command topics.
    // ------------------------------------------------------------------
    let set_topic = format!("{}/+/set", cfg.base_topic);
    client.subscribe(&set_topic, QoS::AtLeastOnce).await?;
    info!("Subscribed to '{}'", set_topic);

    // ------------------------------------------------------------------
    // 5. Main event loop – drive MQTT and state updates concurrently.
    // ------------------------------------------------------------------
    tokio::select! {
        result = drive_mqtt(eventloop, &client, &cfg.base_topic, &cmd_queues) => {
            if let Err(e) = result {
                error!("MQTT event loop error: {:#}", e);
            }
        }
        result = publish_states(state_rx, &client, &cfg.base_topic) => {
            if let Err(e) = result {
                error!("State publisher error: {:#}", e);
            }
        }
    }

    Ok(())
}

/// Drive the MQTT event loop, dispatching incoming messages as device commands.
async fn drive_mqtt(
    mut eventloop: EventLoop,
    _client: &AsyncClient,
    base_topic: &str,
    cmd_queues: &HashMap<String, CommandQueue>,
) -> anyhow::Result<()> {
    use rumqttc::Event::Incoming;
    use rumqttc::Packet::Publish;

    loop {
        match eventloop.poll().await {
            Ok(Incoming(Publish(publish))) => {
                let topic = publish.topic.as_str();
                handle_incoming_publish(topic, &publish.payload, base_topic, cmd_queues);
            }
            Ok(_) => {}
            Err(e) => {
                warn!("MQTT connection error: {}; reconnecting…", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Parse an incoming MQTT publish and enqueue the command for the target device.
fn handle_incoming_publish(
    topic: &str,
    payload: &[u8],
    base_topic: &str,
    cmd_queues: &HashMap<String, CommandQueue>,
) {
    // Expected topic: `{base_topic}/{device_name}/set`
    let prefix = format!("{}/", base_topic);
    let Some(rest) = topic.strip_prefix(&prefix) else {
        return;
    };
    let Some(device_name) = rest.strip_suffix("/set") else {
        return;
    };

    let queue = match cmd_queues.get(device_name) {
        Some(q) => q,
        None => {
            warn!("Received command for unknown device '{}'", device_name);
            return;
        }
    };

    match serde_json::from_slice::<DeviceCommand>(payload) {
        Ok(cmd) => {
            if let Ok(mut q) = queue.lock() {
                q.push_back(cmd);
            }
        }
        Err(e) => {
            warn!(
                "Invalid command payload for '{}': {} – payload: {}",
                device_name,
                e,
                String::from_utf8_lossy(payload)
            );
        }
    }
}

/// Consume state updates and publish them to MQTT.
async fn publish_states(
    mut rx: mpsc::UnboundedReceiver<StateUpdate>,
    client: &AsyncClient,
    base_topic: &str,
) -> anyhow::Result<()> {
    while let Some(update) = rx.recv().await {
        let topic = format!("{}/{}/state", base_topic, update.topic_name);
        match serde_json::to_vec(&update.state) {
            Ok(payload) => {
                if let Err(e) = client
                    .publish(&topic, QoS::AtLeastOnce, true, payload)
                    .await
                {
                    warn!("Failed to publish '{}' state to '{}': {}", update.friendly_name, topic, e);
                }
            }
            Err(e) => {
                error!("Failed to serialise state for '{}': {}", update.friendly_name, e);
            }
        }
    }
    Ok(())
}
