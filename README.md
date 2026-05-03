# MQTT Casters

A lightweight Chromecast → MQTT connector written in Rust.

Discovers Chromecast devices on the local network via mDNS-SD and continuously
publishes their state (volume, muted, now-playing metadata, player state) as
retained JSON messages to an MQTT broker.  Supports basic playback control via
MQTT commands.

---

## Features

- **Zero-config discovery** – finds all `_googlecast._tcp.local.` devices automatically
- **Retained state** – publishes JSON to `{base_topic}/{device}/state` (retained)
- **Playback control** – accepts JSON commands on `{base_topic}/{device}/set`
- **Auto-reconnect** – reconnects automatically if a device becomes unreachable
- **Configurable** via environment variables

---

## MQTT Topics

### State (published, retained)

```
{MQTT_BASE_TOPIC}/{device_name}/state
```

Example payload:

```json
{
  "online": true,
  "volume": 0.45,
  "muted": false,
  "app_id": "CC1AD845",
  "app_name": "Default Media Receiver",
  "player_state": "PLAYING",
  "title": "Never Gonna Give You Up",
  "artist": "Rick Astley",
  "album": "Whenever You Need Somebody",
  "current_time": 42.1,
  "duration": 213.0
}
```

`device_name` is the friendly name lowercased with non-ASCII/non-alphanumeric
characters replaced by underscores (e.g. `"Living Room TV"` → `living_room_tv`).

### Commands (subscribe)

```
{MQTT_BASE_TOPIC}/{device_name}/set
```

Publish a JSON object with a `"command"` field:

| Payload | Effect |
|---|---|
| `{"command":"play"}` | Resume playback |
| `{"command":"pause"}` | Pause playback |
| `{"command":"stop"}` | Stop playback |
| `{"command":"set_volume","value":70}` | Set volume to 70 % |
| `{"command":"set_muted","muted":true}` | Mute / unmute |

---

## Configuration

All configuration is through environment variables:

| Variable            | Default                     | Description                                   |
|---------------------|-----------------------------|-----------------------------------------------|
| `MQTT_URL`          | `mqtt://localhost:1883`     | MQTT broker URL                               |
| `MQTT_BASE_TOPIC`   | `mqttcasters`               | MQTT topic prefix                             |
| `DISCOVERY_TIMEOUT` | `10`                        | mDNS discovery window in seconds at startup   |
| `RECONNECT_DELAY`   | `15`                        | Seconds between reconnection attempts         |
| `RUST_LOG`          | `info`                      | Log level (`trace`, `debug`, `info`, `warn`)  |

---

## User Guide

For detailed instructions on running a local MQTT broker with Docker and using CLI tools to control your devices, see the [User Guide](USERGUIDE.md).

## Building & Running

```bash
# Build (release)
cargo build --release

# Run
MQTT_URL=mqtt://192.168.1.10:1883 ./target/release/mqttcasters
```

### Docker (example)

```dockerfile
FROM rust:1.94 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/chromecast2mqtt /usr/local/bin/
CMD ["chromecast2mqtt"]
```

---

## License

MIT
