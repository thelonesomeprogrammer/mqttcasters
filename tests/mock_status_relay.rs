use mqttcasters::types::{DeviceState, StateUpdate};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_status_relay() -> anyhow::Result<()> {
    // 1. Setup MQTT client for verification
    let mut mqtt_options = MqttOptions::new("verifier", "127.0.0.1", 1883);
    mqtt_options.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(mqtt_options, 10);

    // 2. Subscribe to the expected state topic
    client
        .subscribe("mqttcasters/mock_device/state", QoS::AtLeastOnce)
        .await?;

    // 3. Setup the channel used by the bridge to receive updates from devices
    let (state_tx, state_rx) = mpsc::unbounded_channel::<StateUpdate>();

    // 4. Start a mock publisher loop (similar to bridge::publish_states)
    let client_clone = client.clone();
    tokio::spawn(async move {
        let mut rx = state_rx;
        while let Some(update) = rx.recv().await {
            let topic = format!("mqttcasters/{}/state", update.topic_name);
            let payload = serde_json::to_vec(&update.state).unwrap();
            client_clone
                .publish(topic, QoS::AtLeastOnce, true, payload)
                .await
                .unwrap();
        }
    });

    // 5. Send a mock status update
    let mock_update = StateUpdate {
        topic_name: "mock_device".to_string(),
        friendly_name: "Mock Device".to_string(),
        state: DeviceState {
            online: true,
            volume: 0.5,
            muted: false,
            app_id: Some("CC1AD845".to_string()),
            app_name: Some("Default Media Receiver".to_string()),
            player_state: Some(mqttcasters::types::PlayerStateLocal::Playing),
            current_time: Some(10.0),
            duration: Some(300.0),
        },
    };

    // Give some time for subscription to settle
    tokio::time::sleep(Duration::from_millis(500)).await;
    state_tx.send(mock_update.clone())?;

    // 6. Verify the message arrives in the MQTT event loop
    let mut found = false;
    let timeout = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            result = eventloop.poll() => {
                match result? {
                    Event::Incoming(Packet::Publish(p)) => {
                        if p.topic == "mqttcasters/mock_device/state" {
                            let received_state: DeviceState = serde_json::from_slice(&p.payload)?;
                            assert_eq!(received_state, mock_update.state);
                            found = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            _ = &mut timeout => {
                break;
            }
        }
    }

    assert!(found, "Did not receive status update on MQTT topic");
    Ok(())
}
