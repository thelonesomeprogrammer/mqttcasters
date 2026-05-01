use serde::{Deserialize, Serialize};

/// Current state of a Chromecast device, serialised as JSON and published to MQTT.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceState {
    /// Whether the device is reachable.
    pub online: bool,
    /// Volume level in the range `[0.0, 1.0]`.
    pub volume: f32,
    /// Whether audio is muted.
    pub muted: bool,
    /// Cast application identifier (e.g. `"CC1AD845"` for Default Media Receiver).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Human-readable application name (e.g. `"YouTube"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    /// Media player state: `"PLAYING"`, `"PAUSED"`, `"BUFFERING"`, or `"IDLE"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_state: Option<String>,
    /// Media title (song / video title).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Media artist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    /// Media album.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    /// Current playback position in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_time: Option<f32>,
    /// Total media duration in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,
}

impl Default for DeviceState {
    fn default() -> Self {
        DeviceState {
            online: false,
            volume: 0.0,
            muted: false,
            app_id: None,
            app_name: None,
            player_state: None,
            title: None,
            artist: None,
            album: None,
            current_time: None,
            duration: None,
        }
    }
}

/// A state update emitted by a device thread and consumed by the bridge.
#[derive(Debug, Clone)]
pub struct StateUpdate {
    /// Sanitised device name used as the MQTT sub-topic (spaces → underscores, lowercase).
    pub topic_name: String,
    /// Human-readable friendly name.
    pub friendly_name: String,
    /// New device state.
    pub state: DeviceState,
}

/// Commands that can be sent to a device thread via MQTT.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum DeviceCommand {
    Play,
    Pause,
    Stop,
    /// Set volume; expects a `"value"` field in `[0, 100]` (integer percentage).
    SetVolume { value: u8 },
    /// Mute or unmute; expects a `"muted"` boolean field.
    SetMuted { muted: bool },
}

/// A discovered Chromecast device.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// Sanitised topic-safe name.
    pub topic_name: String,
    /// Friendly name from the TXT record (`fn` key).
    pub friendly_name: String,
    /// IP address (IPv4 preferred).
    pub address: String,
    /// Cast port (almost always `8009`).
    pub port: u16,
}
