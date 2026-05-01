use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::{MediaResponse, Metadata};
use rust_cast::channels::receiver::ReceiverResponse;
use rust_cast::{CastDevice, ChannelMessage};
use tracing::{debug, error, info, warn};

use crate::types::{DeviceCommand, DeviceState, DiscoveredDevice, StateUpdate};

/// Shared command queue; the bridge pushes commands, the device thread drains it.
pub type CommandQueue = Arc<Mutex<VecDeque<DeviceCommand>>>;

/// Spin up a background thread that connects to the Chromecast device, polls for
/// state changes, and forwards them through `state_tx`.  Commands are accepted via
/// the returned [`CommandQueue`].
pub fn spawn_device_thread(
    device: DiscoveredDevice,
    state_tx: tokio::sync::mpsc::UnboundedSender<StateUpdate>,
    reconnect_delay: Duration,
) -> CommandQueue {
    let cmd_queue: CommandQueue = Arc::new(Mutex::new(VecDeque::new()));
    let cmd_queue_clone = Arc::clone(&cmd_queue);

    std::thread::Builder::new()
        .name(format!("cast/{}", device.topic_name))
        .spawn(move || {
            loop {
                info!(
                    "[{}] Connecting to {}:{}…",
                    device.friendly_name, device.address, device.port
                );

                match run_device(&device, &state_tx, &cmd_queue_clone) {
                    Ok(()) => {
                        info!("[{}] Session ended cleanly", device.friendly_name);
                    }
                    Err(e) => {
                        warn!("[{}] Error: {:#}", device.friendly_name, e);
                    }
                }

                // Publish offline state before sleeping.
                let offline = StateUpdate {
                    topic_name: device.topic_name.clone(),
                    friendly_name: device.friendly_name.clone(),
                    state: DeviceState::default(),
                };
                let _ = state_tx.send(offline);

                info!(
                    "[{}] Reconnecting in {} s…",
                    device.friendly_name,
                    reconnect_delay.as_secs()
                );
                std::thread::sleep(reconnect_delay);
            }
        })
        .expect("failed to spawn device thread");

    cmd_queue
}

/// Inner device loop – returns when the connection is lost or an unrecoverable
/// error occurs.
fn run_device(
    device: &DiscoveredDevice,
    state_tx: &tokio::sync::mpsc::UnboundedSender<StateUpdate>,
    cmd_queue: &CommandQueue,
) -> anyhow::Result<()> {
    let cast = CastDevice::connect_without_host_verification(
        device.address.as_str(),
        device.port,
    )?;

    // Establish Cast connection to the main receiver endpoint.
    cast.connection.connect("receiver-0")?;
    cast.heartbeat.ping()?;

    // Track internal state used for commands and state diffing.
    let mut local = LocalState::default();

    // Bootstrap receiver status (best effort – ignore errors at startup).
    match cast.receiver.get_status() {
        Ok(status) => {
            apply_receiver_status(&status, &mut local);
            if let Some(ref tid) = local.transport_id.clone() {
                try_connect_to_app(&cast, &device.friendly_name, tid, &mut local);
            }
            let update = build_state_update(device, &local, true);
            let _ = state_tx.send(update);
        }
        Err(e) => {
            debug!("[{}] Initial get_status failed: {}", device.friendly_name, e);
        }
    }

    info!("[{}] Entering receive loop", device.friendly_name);

    loop {
        // Drain any pending commands before blocking on receive().
        drain_commands(&cast, &device.friendly_name, cmd_queue, &mut local);

        match cast.receive() {
            Ok(ChannelMessage::Heartbeat(HeartbeatResponse::Ping)) => {
                cast.heartbeat.pong()?;
                debug!("[{}] Heartbeat ping → pong", device.friendly_name);
            }
            Ok(ChannelMessage::Heartbeat(_)) => {}

            Ok(ChannelMessage::Receiver(ReceiverResponse::Status(status))) => {
                debug!("[{}] Receiver status received", device.friendly_name);
                let changed = apply_receiver_status(&status, &mut local);

                if let Some(ref tid) = local.transport_id.clone() {
                    if !local.media_connected {
                        try_connect_to_app(&cast, &device.friendly_name, tid, &mut local);
                    }
                }

                if changed {
                    let update = build_state_update(device, &local, true);
                    let _ = state_tx.send(update);
                }
            }
            Ok(ChannelMessage::Receiver(_)) => {}

            Ok(ChannelMessage::Media(MediaResponse::Status(status))) => {
                debug!(
                    "[{}] Media status received ({} entries)",
                    device.friendly_name,
                    status.entries.len()
                );
                let changed = apply_media_status(&status, &mut local);
                if changed {
                    let update = build_state_update(device, &local, true);
                    let _ = state_tx.send(update);
                }
            }
            Ok(ChannelMessage::Media(_)) => {}

            Ok(ChannelMessage::Connection(_)) => {}
            Ok(ChannelMessage::Raw(_)) => {}

            Err(e) => {
                error!("[{}] Receive error: {}", device.friendly_name, e);
                return Err(e.into());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------

/// Per-connection mutable state used only within the device thread.
#[derive(Default)]
struct LocalState {
    volume: f32,
    muted: bool,
    app_id: Option<String>,
    app_name: Option<String>,
    /// Cast session ID used when stopping the app.
    session_id: Option<String>,
    /// Transport ID used for media commands.
    transport_id: Option<String>,
    /// Whether we have sent a CONNECT to the current transport_id.
    media_connected: bool,
    media_session_id: Option<i32>,
    player_state: Option<String>,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    current_time: Option<f32>,
    duration: Option<f32>,
}

/// Update local state from a receiver status; return `true` if anything changed.
fn apply_receiver_status(
    status: &rust_cast::channels::receiver::Status,
    local: &mut LocalState,
) -> bool {
    let mut changed = false;

    let new_vol = status.volume.level.unwrap_or(local.volume);
    let new_muted = status.volume.muted.unwrap_or(local.muted);

    if (new_vol - local.volume).abs() > f32::EPSILON || new_muted != local.muted {
        local.volume = new_vol;
        local.muted = new_muted;
        changed = true;
    }

    let new_app = status.applications.first().map(|a| a.app_id.clone());
    let new_name = status.applications.first().map(|a| a.display_name.clone());
    let new_transport = status.applications.first().map(|a| a.transport_id.clone());
    let new_session = status.applications.first().map(|a| a.session_id.clone());

    if new_app != local.app_id {
        // App changed – reset media state.
        local.app_id = new_app;
        local.app_name = new_name;
        local.session_id = new_session;
        local.transport_id = new_transport;
        local.media_connected = false;
        local.media_session_id = None;
        local.player_state = None;
        local.title = None;
        local.artist = None;
        local.album = None;
        local.current_time = None;
        local.duration = None;
        changed = true;
    }

    changed
}

/// Update local state from a media status; return `true` if anything changed.
fn apply_media_status(
    status: &rust_cast::channels::media::Status,
    local: &mut LocalState,
) -> bool {
    let entry = match status.entries.first() {
        Some(e) => e,
        None => return false,
    };

    let new_state = entry.player_state.to_string();
    let new_msid = entry.media_session_id;
    let new_time = entry.current_time;
    let new_dur = entry.media.as_ref().and_then(|m| m.duration);

    let (new_title, new_artist, new_album) = extract_metadata(entry);

    let changed = local.player_state.as_deref() != Some(new_state.as_str())
        || local.media_session_id != Some(new_msid)
        || local.current_time != new_time
        || local.title != new_title
        || local.artist != new_artist
        || local.album != new_album;

    local.player_state = Some(new_state);
    local.media_session_id = Some(new_msid);
    local.current_time = new_time;
    local.duration = new_dur;
    local.title = new_title;
    local.artist = new_artist;
    local.album = new_album;

    changed
}

fn extract_metadata(
    entry: &rust_cast::channels::media::StatusEntry,
) -> (Option<String>, Option<String>, Option<String>) {
    match entry.media.as_ref().and_then(|m| m.metadata.as_ref()) {
        Some(Metadata::MusicTrack(m)) => (
            m.title.clone(),
            m.artist.clone(),
            m.album_name.clone(),
        ),
        Some(Metadata::Movie(m)) => (m.title.clone(), None, None),
        Some(Metadata::TvShow(m)) => (m.series_title.clone(), None, None),
        Some(Metadata::Generic(m)) => (m.title.clone(), None, None),
        _ => (None, None, None),
    }
}

fn build_state_update(device: &DiscoveredDevice, local: &LocalState, online: bool) -> StateUpdate {
    StateUpdate {
        topic_name: device.topic_name.clone(),
        friendly_name: device.friendly_name.clone(),
        state: DeviceState {
            online,
            volume: local.volume,
            muted: local.muted,
            app_id: local.app_id.clone(),
            app_name: local.app_name.clone(),
            player_state: local.player_state.clone(),
            title: local.title.clone(),
            artist: local.artist.clone(),
            album: local.album.clone(),
            current_time: local.current_time,
            duration: local.duration,
        },
    }
}

// ---------------------------------------------------------------------------
// App connection helper
// ---------------------------------------------------------------------------

fn try_connect_to_app(
    cast: &CastDevice,
    friendly_name: &str,
    transport_id: &str,
    local: &mut LocalState,
) {
    debug!("[{}] Connecting to transport {}", friendly_name, transport_id);
    if let Err(e) = cast.connection.connect(transport_id) {
        warn!(
            "[{}] Failed to connect to transport {}: {}",
            friendly_name, transport_id, e
        );
        return;
    }
    local.media_connected = true;

    // Request media status immediately; best-effort.
    match cast.media.get_status(transport_id, None) {
        Ok(status) => {
            apply_media_status(&status, local);
        }
        Err(e) => {
            debug!(
                "[{}] Initial media status failed: {}",
                friendly_name, e
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Command execution
// ---------------------------------------------------------------------------

fn drain_commands(
    cast: &CastDevice,
    friendly_name: &str,
    cmd_queue: &CommandQueue,
    local: &mut LocalState,
) {
    let cmds: Vec<DeviceCommand> = {
        if let Ok(mut q) = cmd_queue.try_lock() {
            q.drain(..).collect()
        } else {
            return;
        }
    };

    for cmd in cmds {
        if let Err(e) = execute_command(cast, friendly_name, &cmd, local) {
            warn!("[{}] Command {:?} failed: {}", friendly_name, cmd, e);
        }
    }
}

fn execute_command(
    cast: &CastDevice,
    friendly_name: &str,
    cmd: &DeviceCommand,
    local: &mut LocalState,
) -> anyhow::Result<()> {
    match cmd {
        DeviceCommand::Play => {
            let tid = local
                .transport_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no active app"))?;
            let msid = local
                .media_session_id
                .ok_or_else(|| anyhow::anyhow!("no active media session"))?;
            cast.media.play(tid, msid)?;
            info!("[{}] Play", friendly_name);
        }
        DeviceCommand::Pause => {
            let tid = local
                .transport_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no active app"))?;
            let msid = local
                .media_session_id
                .ok_or_else(|| anyhow::anyhow!("no active media session"))?;
            cast.media.pause(tid, msid)?;
            info!("[{}] Pause", friendly_name);
        }
        DeviceCommand::Stop => {
            let tid = local
                .transport_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no active app"))?;
            let msid = local
                .media_session_id
                .ok_or_else(|| anyhow::anyhow!("no active media session"))?;
            cast.media.stop(tid, msid)?;
            info!("[{}] Stop", friendly_name);
        }
        DeviceCommand::SetVolume { value } => {
            let level = (*value as f32) / 100.0;
            cast.receiver
                .set_volume(rust_cast::channels::receiver::Volume::from(level))?;
            local.volume = level;
            info!("[{}] Volume → {}", friendly_name, value);
        }
        DeviceCommand::SetMuted { muted } => {
            cast.receiver
                .set_volume(rust_cast::channels::receiver::Volume::from(*muted))?;
            local.muted = *muted;
            info!("[{}] Muted → {}", friendly_name, muted);
        }
    }
    Ok(())
}
