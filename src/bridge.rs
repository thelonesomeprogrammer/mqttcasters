use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::device::{CommandSender, spawn_device_thread};
use crate::types::{DeviceCommand, StateUpdate};

/// Run the bridge: discover devices, connect to each, and relay state ↔ MQTT.
pub async fn run(cfg: Config) -> anyhow::Result<()> {
    println!("BRIDGE RUN STARTING");
    // ------------------------------------------------------------------
    // 1. Setup channels.
    // ------------------------------------------------------------------
    let (state_tx, state_rx) = mpsc::channel::<StateUpdate>(100);
    let (discovery_tx, mut discovery_rx) = mpsc::channel::<crate::types::DiscoveryEvent>(32);

    // ------------------------------------------------------------------
    // 2. Start continuous discovery.
    // ------------------------------------------------------------------
    crate::discovery::start_discovery(discovery_tx, cfg.discovery_backend)?;

    // ------------------------------------------------------------------
    // 3. Create the MQTT client.
    // ------------------------------------------------------------------
    let client_id = format!("mqttcasters-{}", std::process::id());
    let mut url = cfg.mqtt_url.clone();
    if !url.contains("client_id=") {
        let separator = if url.contains('?') { '&' } else { '?' };
        url.push(separator);
        url.push_str("client_id=unused");
    }

    let mut mqttopts = MqttOptions::parse_url(url)?;
    mqttopts.set_client_id(client_id);
    mqttopts.set_keep_alive(Duration::from_secs(30));
    mqttopts.set_clean_session(true);

    let (client, eventloop) = AsyncClient::new(mqttopts, 64);

    // ------------------------------------------------------------------
    // 4. Main event loop.
    // ------------------------------------------------------------------
    let cmd_senders = Arc::new(RwLock::new(HashMap::<String, CommandSender>::new()));
    let reconnect = Duration::from_secs(cfg.reconnect_delay_secs);
    let mut known_addresses = HashMap::<String, (String, u16)>::new();

    tokio::select! {
        result = drive_mqtt(eventloop, &client, &cfg.base_topic, cmd_senders.clone()) => {
            if let Err(e) = result {
                error!("MQTT event loop error: {:#}", e);
            }
        }
        result = publish_states(state_rx, &client, &cfg.base_topic) => {
            if let Err(e) = result {
                error!("State publisher error: {:#}", e);
            }
        }
        _ = async {
            while let Some(event) = discovery_rx.recv().await {
                if let crate::types::DiscoveryEvent::Found(ref device) = event {
                    let current = known_addresses.get(&device.topic_name);
                    if current == Some(&(device.address.clone(), device.port)) {
                        // Skip redundant update
                        continue;
                    }
                    known_addresses.insert(device.topic_name.clone(), (device.address.clone(), device.port));
                }
                handle_discovery_event(event, cmd_senders.clone(), &state_tx, reconnect).await;
            }
        } => {
            warn!("Discovery event stream ended unexpectedly");
        }
    }

    Ok(())
}

async fn handle_discovery_event(
    event: crate::types::DiscoveryEvent,
    cmd_senders: Arc<RwLock<HashMap<String, CommandSender>>>,
    state_tx: &mpsc::Sender<StateUpdate>,
    reconnect: Duration,
) {
    use crate::types::DiscoveryEvent::*;

    match event {
        Found(device) => {
            let mut senders = cmd_senders.write().await;
            if let Some(sender) = senders.get(&device.topic_name) {
                // Device already exists, send an address update in case it changed.
                let _ = sender.try_send(crate::types::DeviceCommand::UpdateAddress {
                    address: device.address,
                    port: device.port,
                });
            } else {
                info!(
                    "New device discovered: '{}' ({topic}) at {}:{}",
                    device.friendly_name,
                    device.address,
                    device.port,
                    topic = device.topic_name
                );
                let sender = spawn_device_thread(device.clone(), state_tx.clone(), reconnect);
                senders.insert(device.topic_name.clone(), sender);
            }
        }
        Removed(fullname) => {
            debug!("mDNS removal for '{}' - device task will handle timeout", fullname);
        }
    }
}

/// Drive the MQTT event loop, dispatching incoming messages as device commands.
async fn drive_mqtt(
    mut eventloop: EventLoop,
    client: &AsyncClient,
    base_topic: &str,
    cmd_senders: Arc<RwLock<HashMap<String, CommandSender>>>,
) -> anyhow::Result<()> {
    use rumqttc::Event::Incoming;
    use rumqttc::Packet::{ConnAck, Publish};

    let set_topic = format!("{}/+/set", base_topic);

    loop {
        match eventloop.poll().await {
            Ok(Incoming(ConnAck(_))) => {
                info!("MQTT connected. Subscribing to '{}'...", set_topic);
                if let Err(e) = client.subscribe(&set_topic, QoS::AtLeastOnce).await {
                    error!("Failed to subscribe to '{}': {}", set_topic, e);
                }
            }
            Ok(Incoming(Publish(publish))) => {
                let topic = publish.topic.as_str();
                handle_incoming_publish(topic, &publish.payload, base_topic, cmd_senders.clone()).await;
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
async fn handle_incoming_publish(
    topic: &str,
    payload: &[u8],
    base_topic: &str,
    cmd_senders: Arc<RwLock<HashMap<String, CommandSender>>>,
) {
    // Expected topic: `{base_topic}/{device_name}/set`
    let prefix = format!("{}/", base_topic);
    let Some(rest) = topic.strip_prefix(&prefix) else {
        return;
    };
    let Some(device_name) = rest.strip_suffix("/set") else {
        return;
    };

    let senders = cmd_senders.read().await;
    let sender = match senders.get(device_name) {
        Some(q) => q,
        None => {
            warn!("Received command for unknown device '{}'", device_name);
            return;
        }
    };

    match serde_json::from_slice::<DeviceCommand>(payload) {
        Ok(cmd) => {
            if let Err(e) = sender.try_send(cmd) {
                warn!("Failed to enqueue command for '{}': {}", device_name, e);
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
    mut rx: mpsc::Receiver<StateUpdate>,
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
                    warn!(
                        "Failed to publish '{}' state to '{}': {}",
                        update.friendly_name, topic, e
                    );
                }
            }
            Err(e) => {
                error!(
                    "Failed to serialise state for '{}': {}",
                    update.friendly_name, e
                );
            }
        }
    }
    Ok(())
}
