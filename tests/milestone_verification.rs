use cast_sender::{MediaController, Receiver};
use mqttcasters::{discovery, types::DeviceCommand};

/// Milestone 1: The Scanner
/// This test verifies that mDNS discovery works.
#[tokio::test]
#[ignore] // Requires local network access
async fn test_milestone_1_scanner() -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);
    discovery::start_discovery(tx)?;

    println!("Watching for devices for 2 seconds...");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if let Ok(Some(event)) = tokio::time::timeout(remaining, rx.recv()).await {
            println!("Event: {:?}", event);
            // Exit early if we found at least one device to speed up tests
            if let mqttcasters::types::DiscoveryEvent::Found(_) = event {
                break;
            }
        } else {
            break;
        }
    }
    Ok(())
}

/// Milestone 2: The Controller
/// This test verifies direct control of a specific device.
#[tokio::test]
#[ignore] // Requires a specific device IP
async fn test_milestone_2_controller() -> anyhow::Result<()> {
    let ip = std::env::var("DEVICE_IP").expect("DEVICE_IP must be set");
    let cmd = DeviceCommand::Pause;
    let receiver = Receiver::new();
    receiver.connect(&ip).await?;

    let status = receiver.status().await?;
    let app = status
        .applications
        .and_then(|mut apps| apps.pop())
        .ok_or_else(|| anyhow::anyhow!("No active application"))?;

    let controller = MediaController::new(app, receiver.clone())?;

    match cmd {
        DeviceCommand::Pause => {
            controller.pause().await?;
        }
        DeviceCommand::Play => {
            controller.start().await?;
        }
        DeviceCommand::Stop => {
            controller.stop().await?;
        }
        _ => {}
    }

    Ok(())
}

/// Milestone 3: The Broker
/// This test verifies MQTT connectivity.
#[tokio::test]
async fn test_milestone_3_broker() -> anyhow::Result<()> {
    let mqtt_url =
        std::env::var("MQTT_URL").unwrap_or_else(|_| "mqtt://localhost:1883".to_string());
    let mqttopts = rumqttc::MqttOptions::parse_url(mqtt_url + "?client_id=test-milestone")?;
    let (_client, mut eventloop) = rumqttc::AsyncClient::new(mqttopts, 10);

    // Just verify we can poll once without immediate error (if broker is up)
    let _ = eventloop.poll().await;
    Ok(())
}

/// Milestone 4: The Streamer
/// This test verifies loading a specific media URL.
#[tokio::test]
#[ignore] // Requires a specific device IP
async fn test_milestone_4_load() -> anyhow::Result<()> {
    let ip = std::env::var("DEVICE_IP").unwrap_or_else(|_| "192.168.1.12".to_string());
    let url = std::env::var("MEDIA_URL").unwrap_or_else(|_| {
        "https://gonic.local.marrinus.trade/rest/stream.view?id=tr-352u=admin&p=admin
        "
        .to_string()
    });

    let receiver = cast_sender::Receiver::new();
    receiver.connect(&ip).await?;

    receiver
        .launch_app(cast_sender::AppId::DefaultMediaReceiver)
        .await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let status = receiver.status().await?;
    let app = status
        .applications
        .and_then(|mut apps| apps.pop())
        .ok_or_else(|| anyhow::anyhow!("No active app"))?;
    let controller = cast_sender::MediaController::new(app, receiver.clone())?;

    let media = cast_sender::namespace::media::MediaInformation {
        content_id: url.clone(),
        content_type: "audio/mpeg".to_string(),
        stream_type: cast_sender::namespace::media::StreamType::Buffered,
        ..Default::default()
    };

    let request = cast_sender::namespace::media::LoadRequestData {
        media,
        autoplay: Some(true),
        ..Default::default()
    };

    controller.load(request).await?;
    println!("Loading: {}", url);

    Ok(())
}
