use mqttcasters::types::{DeviceCommand, DiscoveredDevice};
use mqttcasters::device::spawn_device_thread;
use tokio::sync::mpsc;
use std::time::Duration;

#[tokio::test]
async fn test_device_worker_lifecycle() -> anyhow::Result<()> {
    let (state_tx, mut state_rx) = mpsc::channel(100);
    let reconnect_delay = Duration::from_millis(100);

    let device = DiscoveredDevice {
        topic_name: "test_device".to_string(),
        friendly_name: "Test Device".to_string(),
        address: "127.0.0.1".to_string(),
        port: 9000, // Dummy port
    };

    let cmd_tx = spawn_device_thread(device, state_tx, reconnect_delay);

    // 1. Worker should immediately try to connect and fail (since nothing is listening at 127.0.0.1:9000)
    // 2. It should then send an "offline" state update.
    let update = tokio::time::timeout(Duration::from_secs(2), state_rx.recv()).await?
        .expect("Should receive an offline state update");

    assert_eq!(update.topic_name, "test_device");
    assert!(!update.state.online);

    // 3. Test UpdateAddress command handling
    cmd_tx.send(DeviceCommand::UpdateAddress {
        address: "127.0.0.2".to_string(),
        port: 9001,
    }).await?;

    // The worker should try to reconnect and eventually send another offline update.
    let update2 = tokio::time::timeout(Duration::from_secs(2), state_rx.recv()).await?
        .expect("Should receive another offline state update after address change");
    
    assert!(!update2.state.online);

    Ok(())
}
