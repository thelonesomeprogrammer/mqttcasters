use std::time::Duration;

use cast_sender::namespace::media::{
    GetStatusRequestData, Media as MediaPayload, MediaInformationBuilder,
    MusicTrackMediaMetadataBuilder, PlayerState, RequestData, StreamType,
};
use cast_sender::namespace::receiver::Receiver as ReceiverPayload;
use cast_sender::{App, AppId, MediaController, Payload, Receiver, Response};
use tokio::sync::mpsc::{Receiver as MpscReceiver, Sender, channel};
use tracing::{debug, error, info, warn};

use crate::types::{DeviceCommand, DeviceState, DiscoveredDevice, PlayerStateLocal, StateUpdate};

/// Shared command sender; the bridge pushes commands, the device task drains it.
pub type CommandSender = Sender<DeviceCommand>;

/// Spin up a background task that connects to the Chromecast device, polls for
/// state changes, and forwards them through `state_tx`. Commands are accepted via
/// the returned [`CommandSender`].
pub fn spawn_device_thread(
    device: DiscoveredDevice,
    state_tx: Sender<StateUpdate>,
    reconnect_delay: Duration,
) -> CommandSender {
    let (cmd_tx, cmd_rx) = channel(100);
    let device_clone = device.clone();

    tokio::spawn(async move {
        run_device_loop(device_clone, state_tx, cmd_rx, reconnect_delay).await;
    });

    cmd_tx
}

async fn run_device_loop(
    mut device: DiscoveredDevice,
    state_tx: Sender<StateUpdate>,
    mut cmd_rx: MpscReceiver<DeviceCommand>,
    reconnect_delay: Duration,
) {
    let mut local = LocalState::default();
    loop {
        info!(
            "[{}] Connecting to {}:{}…",
            device.friendly_name, device.address, device.port
        );

        let receiver = Receiver::new();
        match receiver.connect(&device.address).await {
            Ok(_) => {
                info!("[{}] Connected", device.friendly_name);
                match handle_connection(&device, &receiver, &state_tx, &mut cmd_rx, &mut local).await {
                    Ok(_) => {
                        // Orderly shutdown
                        return;
                    }
                    Err(crate::types::DeviceCommand::UpdateAddress { address, port }) => {
                        info!(
                            "[{}] IP address updated to {}:{}, reconnecting…",
                            device.friendly_name, address, port
                        );
                        device.address = address;
                        device.port = port;
                        continue;
                    }
                    Err(e) => {
                        error!("[{}] Connection error: {}", device.friendly_name, e);
                    }
                }
            }
            Err(e) => {
                warn!("[{}] Connection failed: {}", device.friendly_name, e);
            }
        }

        // Publish offline state before sleeping.
        let offline = StateUpdate {
            topic_name: device.topic_name.clone(),
            friendly_name: device.friendly_name.clone(),
            state: DeviceState::default(),
        };
        // LOSSY: Use try_send for state updates to avoid deadlocking if MQTT is blocked.
        if let Err(e) = state_tx.try_send(offline) {
            warn!(
                "[{}] Failed to send offline state (channel full): {}",
                device.friendly_name, e
            );
        }

        info!(
            "[{}] Reconnecting in {} s…",
            device.friendly_name,
            reconnect_delay.as_secs()
        );

        // During sleep, we still need to listen for UpdateAddress commands
        // so we don't wait 15s to reconnect to a newly discovered IP.
        tokio::select! {
            _ = tokio::time::sleep(reconnect_delay) => {}
            cmd = cmd_rx.recv() => {
                if let Some(DeviceCommand::UpdateAddress { address, port }) = cmd {
                    info!("[{}] IP address updated during sleep to {}:{}, reconnecting now", device.friendly_name, address, port);
                    device.address = address;
                    device.port = port;
                }
            }
        }
    }
}

/// A specialized error type or result to bubble up address updates.
type ConnectionResult = Result<(), DeviceCommand>;

async fn handle_connection(
    device: &DiscoveredDevice,
    receiver: &Receiver,
    state_tx: &Sender<StateUpdate>,
    cmd_rx: &mut MpscReceiver<DeviceCommand>,
    local: &mut LocalState,
) -> ConnectionResult {
    // Initial status fetch
    if let Ok(status) = receiver.status().await {
        let (changed, should_fetch_media) = apply_receiver_status(status, local, receiver);
        if should_fetch_media {
            if let Some(ref app) = local.app {
                let _ = receiver.send_request(app, MediaPayload::GetStatus(GetStatusRequestData::default())).await;
            }
        }
        if changed {
            let update = build_state_update(device, local, true);
            let _ = state_tx.try_send(update);
        }
    }

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(DeviceCommand::UpdateAddress { address, port }) => {
                        // Return this as an "error" to trigger a reconnect in the outer loop
                        return Err(DeviceCommand::UpdateAddress { address, port });
                    }
                    Some(cmd) => {
                        if let Err(e) = execute_command(receiver, &device.friendly_name, &cmd, local).await {
                            warn!("[{}] Command {:?} failed: {}", device.friendly_name, cmd, e);
                        }
                    }
                    None => {
                        // Command channel closed, shut down this device task
                        return Ok(());
                    }
                }
            }
            resp = receiver.receive() => {
                match resp {
                    Ok(Response { payload, .. }) => {
                        let changed = match payload {
                            Payload::Receiver(ReceiverPayload::ReceiverStatus(status)) => {
                                debug!("[{}] Receiver status received", device.friendly_name);
                                let (changed, should_fetch_media) = apply_receiver_status(status.status, local, receiver);
                                if should_fetch_media {
                                    if let Some(ref app) = local.app {
                                        let _ = receiver.send_request(app, MediaPayload::GetStatus(GetStatusRequestData::default())).await;
                                    }
                                }
                                changed
                            }
                            Payload::Media(MediaPayload::MediaStatus(status)) => {
                                debug!("[{}] Media status received", device.friendly_name);
                                if let Some(entry) = status.status.first() {
                                    apply_media_status(entry, local)
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };

                        if changed {
                            let update = build_state_update(device, &local, true);
                            if let Err(_) = state_tx.try_send(update) {
                                warn!("[{}] State update dropped (channel full)", device.friendly_name);
                            }
                        }
                    }
                    Err(_e) => {
                        // Return this as an "error" to trigger a reconnect in the outer loop
                        return Err(DeviceCommand::Stop);
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct LocalState {
    volume: f32,
    muted: bool,
    app: Option<App>,
    active_app_session_id: Option<String>,
    media_controller: Option<MediaController>,
    media_session_id: Option<i32>,
    player_state: Option<PlayerStateLocal>,
    current_time: Option<f32>,
    duration: Option<f32>,
}

fn apply_receiver_status(
    status: cast_sender::namespace::receiver::Status,
    local: &mut LocalState,
    receiver: &Receiver,
) -> (bool, bool) {
    let mut changed = false;
    let mut should_fetch_media = false;

    let new_vol = status.volume.level.unwrap_or(local.volume as f64) as f32;
    let new_muted = status.volume.muted.unwrap_or(local.muted);

    if (new_vol - local.volume).abs() > f32::EPSILON || new_muted != local.muted {
        local.volume = new_vol;
        local.muted = new_muted;
        changed = true;
    }

    let new_app = status.applications.and_then(|mut apps| apps.pop());

    if new_app.as_ref().map(|a| &a.session_id) != local.app.as_ref().map(|a| &a.session_id) {
        local.app = new_app;
        local.player_state = None;
        local.current_time = None;
        local.duration = None;

        if let Some(ref app) = local.app {
            if local.active_app_session_id.as_ref() != Some(&app.session_id) {
                local.media_controller = MediaController::new(app.clone(), receiver.clone()).ok();
                local.active_app_session_id = Some(app.session_id.clone());
                local.media_session_id = None;
                
                if app.namespaces.contains(&cast_sender::namespace::NamespaceUrn::Media) {
                    should_fetch_media = true;
                }
            }
        } else {
            local.media_controller = None;
            local.active_app_session_id = None;
            local.media_session_id = None;
        }
        changed = true;
    }

    (changed, should_fetch_media)
}

fn apply_media_status(
    status: &cast_sender::namespace::media::MediaStatus,
    local: &mut LocalState,
) -> bool {
    let new_session_id = Some(status.media_session_id);
    let new_state = match status.player_state {
        PlayerState::Playing => PlayerStateLocal::Playing,
        PlayerState::Paused => PlayerStateLocal::Paused,
        PlayerState::Buffering => PlayerStateLocal::Buffering,
        PlayerState::Idle => PlayerStateLocal::Idle,
    };
    let new_time = Some(status.current_time as f32);
    let new_dur = status
        .media
        .as_ref()
        .and_then(|m| m.duration.map(|d| d as f32));

    let changed = local.player_state != Some(new_state.clone())
        || local.current_time != new_time
        || local.duration != new_dur
        || local.media_session_id != new_session_id;

    local.player_state = Some(new_state);
    local.current_time = new_time;
    local.duration = new_dur;
    local.media_session_id = new_session_id;

    changed
}

fn build_state_update(device: &DiscoveredDevice, local: &LocalState, online: bool) -> StateUpdate {
    StateUpdate {
        topic_name: device.topic_name.clone(),
        friendly_name: device.friendly_name.clone(),
        state: DeviceState {
            online,
            volume: local.volume,
            muted: local.muted,
            app_id: local.app.as_ref().map(|a| a.app_id.to_string()),
            app_name: local.app.as_ref().map(|a| a.display_name.clone()),
            player_state: local.player_state.clone(),
            current_time: local.current_time,
            duration: local.duration,
        },
    }
}

async fn execute_command(
    receiver: &Receiver,
    friendly_name: &str,
    cmd: &DeviceCommand,
    local: &mut LocalState,
) -> anyhow::Result<()> {
    match cmd {
        DeviceCommand::Play => {
            let app = local
                .app
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no active app"))?;
            
            let session_id = if let Some(id) = local.media_session_id {
                id
            } else {
                debug!("[{}] No session ID, fetching media status…", friendly_name);
                let _ = receiver.send_request(app, MediaPayload::GetStatus(GetStatusRequestData::default())).await;
                // Wait a tiny bit for the response to be processed by the other loop
                tokio::time::sleep(Duration::from_millis(200)).await;
                local.media_session_id.ok_or_else(|| anyhow::anyhow!("no active media session after status fetch"))?
            };

            if let Err(e) = receiver
                .send_request(app, MediaPayload::Play(RequestData { media_session_id: Some(session_id) }))
                .await
            {
                let err_msg = e.to_string();
                if err_msg.contains("Did not receive request response") {
                    debug!("[{}] Play request sent, but response was handled by the listener loop.", friendly_name);
                } else {
                    warn!("[{}] Play failed: {}", friendly_name, e);
                    return Err(e.into());
                }
            }
            info!("[{}] Play", friendly_name);
        }
        DeviceCommand::Pause => {
            let app = local
                .app
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no active app"))?;

            let session_id = if let Some(id) = local.media_session_id {
                id
            } else {
                debug!("[{}] No session ID, fetching media status…", friendly_name);
                let _ = receiver.send_request(app, MediaPayload::GetStatus(GetStatusRequestData::default())).await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                local.media_session_id.ok_or_else(|| anyhow::anyhow!("no active media session after status fetch"))?
            };

            if let Err(e) = receiver
                .send_request(app, MediaPayload::Pause(RequestData { media_session_id: Some(session_id) }))
                .await
            {
                let err_msg = e.to_string();
                if err_msg.contains("Did not receive request response") {
                    debug!("[{}] Pause request sent, but response was handled by the listener loop.", friendly_name);
                } else {
                    warn!("[{}] Pause failed: {}", friendly_name, e);
                    return Err(e.into());
                }
            }
            info!("[{}] Pause", friendly_name);
        }
        DeviceCommand::Stop => {
            let app = local
                .app
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no active app"))?;

            let session_id = if let Some(id) = local.media_session_id {
                id
            } else {
                debug!("[{}] No session ID, fetching media status…", friendly_name);
                let _ = receiver.send_request(app, MediaPayload::GetStatus(GetStatusRequestData::default())).await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                local.media_session_id.ok_or_else(|| anyhow::anyhow!("no active media session after status fetch"))?
            };

            if let Err(e) = receiver
                .send_request(app, MediaPayload::Stop(RequestData { media_session_id: Some(session_id) }))
                .await
            {
                let err_msg = e.to_string();
                if err_msg.contains("Did not receive request response") {
                    debug!("[{}] Stop request sent, but response was handled by the listener loop.", friendly_name);
                } else {
                    warn!("[{}] Stop failed: {}", friendly_name, e);
                    return Err(e.into());
                }
            }
            info!("[{}] Stop", friendly_name);
        }
        DeviceCommand::SetVolume { value } => {
            let level = (*value as f64) / 100.0;
            if let Err(e) = receiver.set_volume(level, local.muted).await {
                let err_msg = e.to_string();
                if err_msg.contains("Did not receive request response") {
                    debug!("[{}] SetVolume request sent, but response was handled by the listener loop.", friendly_name);
                } else {
                    return Err(e.into());
                }
            }
            local.volume = level as f32;
            info!("[{}] Volume → {}", friendly_name, value);
        }
        DeviceCommand::SetMuted { muted } => {
            if let Err(e) = receiver.set_volume(local.volume as f64, *muted).await {
                let err_msg = e.to_string();
                if err_msg.contains("Did not receive request response") {
                    debug!("[{}] SetMuted request sent, but response was handled by the listener loop.", friendly_name);
                } else {
                    return Err(e.into());
                }
            }
            local.muted = *muted;
            info!("[{}] Muted → {}", friendly_name, muted);
        }
        DeviceCommand::Load {
            url,
            title,
            content_type,
        } => {
            // Always launch/ensure the Default Media Receiver is active for Load
            info!(
                "[{}] Ensuring Default Media Receiver is active for load…",
                friendly_name
            );
            let app = receiver.launch_app(AppId::DefaultMediaReceiver).await?;
            
            // Update local state immediately so subsequent play/pause can use this app
            local.app = Some(app.clone());
            local.active_app_session_id = Some(app.session_id.clone());

            let controller = MediaController::new(app, receiver.clone())?;
            // Store it so we keep the session ID sync if possible
            local.media_controller = Some(controller.clone());

            let mut metadata_builder = MusicTrackMediaMetadataBuilder::default();
            metadata_builder.title(title.as_deref().unwrap_or("mqttcasters stream"));
            // Add a placeholder artist if it's music metadata
            metadata_builder.artist("mqttcasters");
            let metadata = metadata_builder.build().unwrap();

            let media_info = MediaInformationBuilder::default()
                .content_id(url.clone())
                .content_type(content_type.as_deref().unwrap_or("audio/mpeg"))
                .stream_type(StreamType::Live)
                .metadata(metadata)
                .build()
                .unwrap();

            info!(
                "[{}] Loading media: {} (title: {:?})",
                friendly_name, url, title
            );

            if let Err(e) = controller.load(media_info).await {
                let err_msg = e.to_string();
                if err_msg.contains("Did not receive request response") {
                    debug!("[{}] Load request sent, but response was handled by the listener loop.", friendly_name);
                } else {
                    warn!("[{}] Load failed: {}", friendly_name, e);
                    return Err(e.into());
                }
            }
            }
        DeviceCommand::UpdateAddress { .. } => {}
    }
    Ok(())
}
