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
| `DISCOVERY_BACKEND` | `mdns-sd`                   | mDNS backend (`mdns-sd` or `zeroconf`)        |
| `DISCOVERY_TIMEOUT` | `10`                        | mDNS discovery window in seconds at startup   |
| `RECONNECT_DELAY`   | `15`                        | Seconds between reconnection attempts         |
| `RUST_LOG`          | `info`                      | Log level (`trace`, `debug`, `info`, `warn`)  |

---

## mDNS Discovery Backends

`mqttcasters` supports two mDNS discovery backends:

- **`mdns-sd` (default)**: A pure Rust implementation. Works out of the box without system dependencies.
- **`zeroconf`**: Uses system mDNS daemons (Avahi on Linux, Bonjour on macOS/Windows). 
  - Required for reliable discovery when running in Docker with host mDNS integration.
  - Requires `libavahi-client-dev` (Linux) or Bonjour SDK (Windows) at build time.
  - To use, enable the `zeroconf` feature during build and set `DISCOVERY_BACKEND=zeroconf`.

---

## User Guide

For detailed instructions on running a local MQTT broker with Docker and using CLI tools to control your devices, see the [User Guide](USERGUIDE.md).

## Building & Running

```bash
# Build with default (mdns-sd)
cargo build --release

# Build with zeroconf support
cargo build --release --features zeroconf

# Run with zeroconf backend
DISCOVERY_BACKEND=zeroconf MQTT_URL=mqtt://192.168.1.10:1883 ./target/release/mqttcasters
```

### Docker with Avahi (example)

To use Avahi inside Docker, you must link against `libavahi-client` and mount the Avahi socket:

```dockerfile
# Build stage (Debian-based for easier avahi-client linking)
FROM rust:1.81-bookworm AS builder
RUN apt-get update && apt-get install -y libavahi-client-dev
WORKDIR /app
COPY . .
RUN cargo build --release --features zeroconf

# Run stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libavahi-client3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mqttcasters /usr/local/bin/
CMD ["mqttcasters"]
```

Run with:
```bash
docker run -d \
  -e DISCOVERY_BACKEND=zeroconf \
  -v /var/run/avahi-daemon/socket:/var/run/avahi-daemon/socket \
  mqttcasters
```

---

## License

MIT
