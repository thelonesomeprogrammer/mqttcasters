# User Guide

This guide explains how to set up a local MQTT broker using Mosquitto and Docker, and how to use the Mosquitto CLI tools to interact with `mqttcasters`.

## Running Mosquitto in Docker

To run a Mosquitto broker locally for testing, you can use the provided configuration file and Docker.

### 1. Create necessary directories
Make sure the data and log directories exist and are writable:
```bash
mkdir -p mosquitto/config/data mosquitto/config/log
```

### 2. Start Mosquitto
Run the following command from the project root:
```bash
docker run -d \
  --name mosquitto \
  -p 1883:1883 \
  -v $(pwd)/mosquitto/config/mosquitto.conf:/mosquitto/config/mosquitto.conf \
  -v $(pwd)/mosquitto/config/data:/mosquitto/data \
  -v $(pwd)/mosquitto/config/log:/mosquitto/log \
  eclipse-mosquitto
```

## Running mqttcasters

Once your broker is running, you can start `mqttcasters`. By default, it connects to `mqtt://localhost:1883`.

```bash
# Build and run with default settings
cargo run

# Or with a custom broker and topic
MQTT_URL=mqtt://192.168.1.10:1883 MQTT_BASE_TOPIC=myhome/chromecast cargo run
```

## Using Mosquitto CLI Tools

The `mosquitto-clients` package (which includes `mosquitto_sub` and `mosquitto_pub`) is very useful for inspecting and controlling devices.

### Subscribing to State Updates
To see the state of all discovered Chromecasts in real-time:
```bash
mosquitto_sub -h localhost -t "mqttcasters/+/state" -v
```
*(Replace `mqttcasters` with your `MQTT_BASE_TOPIC` if you changed it.)*

### Controlling a Device
To send a command to a specific device, publish a JSON message to its `set` topic. Replace `living_room_tv` with the sanitized name of your device (e.g., `bedroom_speaker`).

**Pause playback:**
```bash
mosquitto_pub -h localhost -t "mqttcasters/living_room_tv/set" -m '{"command":"pause"}'
```

**Resume playback:**
```bash
mosquitto_pub -h localhost -t "mqttcasters/living_room_tv/set" -m '{"command":"play"}'
```

**Stop playback:**
```bash
mosquitto_pub -h localhost -t "mqttcasters/living_room_tv/set" -m '{"command":"stop"}'
```

**Set volume (0-100):**
```bash
mosquitto_pub -h localhost -t "mqttcasters/living_room_tv/set" -m '{"command":"set_volume","value":50}'
```

**Mute/Unmute:**
```bash
mosquitto_pub -h localhost -t "mqttcasters/living_room_tv/set" -m '{"command":"set_muted","muted":true}'
```

**Load Media:**
```bash
mosquitto_pub -h localhost -t "mqttcasters/living_room_tv/set" -m '{"command":"load","url":"http://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4","title":"Big Buck Bunny"}'
```
