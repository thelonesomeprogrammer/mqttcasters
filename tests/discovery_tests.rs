use mqttcasters::discovery::{sanitise_topic_name, start_discovery};
use tokio::sync::mpsc;
use std::time::Duration;

#[test]
fn test_sanitisation() {
    assert_eq!(sanitise_topic_name("Living Room TV"), "living_room_tv");
    assert_eq!(sanitise_topic_name("Küche"), "k_che");
    assert_eq!(sanitise_topic_name("Device #1!"), "device__1_");
}

#[tokio::test]
async fn test_discovery_stream_setup() -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel(10);
    
    start_discovery(tx)?;
    
    // We expect discovery to be active and potentially find devices if they exist on the network.
    // If it's a CI environment with no devices, it will timeout, which is also fine for this test
    // as it verifies the task doesn't crash.
    let _ = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    
    Ok(())
}
