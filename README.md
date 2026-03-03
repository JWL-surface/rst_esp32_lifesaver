# RST ESP32 Lifesaver

A real-time ECG (electrocardiogram) data acquisition and visualization system using an ESP32 microcontroller. The system measures ECG signals at 125Hz, sends them over MQTT to a local broker, and displays live waveforms on a host computer.

## Overview

This project consists of two main components:

1. **ESP32 Firmware** (Rust): Samples ECG data from an analog sensor and publishes it via MQTT
2. **Host Server** (Python): Receives ECG data via MQTT and displays real-time visualization with a scrolling waveform

## System Architecture

### ESP32 (Firmware)

The ESP32 continuously:
- **Samples ECG data** from GPIO2 at 125Hz via a 12-bit ADC with 11dB attenuation
- **Buffers samples** in a double-buffer system (1250 samples per buffer = 10 seconds of data)
- **Connects to WiFi** using your iPhone hotspot (see Network Setup below)
- **Publishes via MQTT** to the Mosquitto broker every 10 seconds with a complete buffer of ECG samples
- **Maintains persistent connection** with periodic PING packets to keep the MQTT session alive
- **Handles partial writes** by chunking large packets to overcome socket buffer limitations

#### Key Specifications

- **Sampling Rate**: 125 Hz
- **Buffer Size**: 1250 samples (10 seconds of data)
- **Publish Interval**: 10 seconds
- **Resolution**: 12-bit ADC (0-4095 counts)
- **Voltage Range**: 0-3.1V (11dB attenuation)
- **MQTT QoS**: 1 (at-least-once delivery)
- **Socket TX/RX Buffers**: 8000 bytes each

#### Supported Pins

- **GPIO2**: ECG signal input (ADC1)
- **GPIO3**: LO- detection input (lead-off detection)
- **GPIO4**: LO+ detection input (lead-off detection)
- **GPIO21**: Status LED

### Host Server (Python)

The Python server:
- **Connects to MQTT broker** and subscribes to ECG data topics
- **Applies 45Hz low-pass Butterworth filter** to reduce noise
- **Displays real-time scrolling waveforms** with matplotlib
- **Updates at 100Hz** for smooth animation
- **Runs MQTT client in background thread** to keep the UI responsive
- **Maintains rolling buffer** of up to 2500 samples for smooth scrolling visualization

#### Visualization Features

- 1250-sample rolling window (10 seconds at 125Hz)
- Smooth scrolling animation at the correct sampling rate
- Interactive matplotlib window (pan, zoom, maximize)
- Real-time grid display
- Low-pass filtered waveform display

## Network Setup

### iPhone Hotspot Configuration

The system uses an **iPhone hotspot as the local network**:

1. Enable Personal Hotspot on your iPhone
2. Connect your computer to the hotspot (note the network name/SSID)
3. The ESP32 also connects to this same hotspot
4. All devices are on the same local network (typically 172.20.x.x range)

### Fixed IP Address

The ESP32 uses a **hardcoded fixed IP address** to connect to the MQTT broker:

- **MQTT Broker IP**: `172.20.10.4`
- **MQTT Broker Port**: `1883`

**Important**: Your MQTT broker machine must have this IP address on the iPhone hotspot network. You may need to:
- Configure your device's network settings to get a static IP of `172.20.10.4`
- Or modify the hardcoded IP in `src/bin/main.rs` to match your broker's actual IP on the hotspot network

To find your broker's IP:
```bash
# On Linux/Mac
ifconfig | grep 172.20

# On Windows
ipconfig | findstr 172.20
```

## Getting Started

### Prerequisites

**ESP32 Development**:
- Rust toolchain with `xtensa-esp32s3` target
- `espflash` tool for flashing

**Host Server**:
- Python 3.7+
- matplotlib
- scipy
- paho-mqtt

**Hardware**:
- ESP32-S3 microcontroller
- ECG analog front-end connected to GPIO2
- Mosquitto MQTT broker running on a machine connected to the iPhone hotspot

### Installation

**Clone the repository**:
```bash
git clone https://github.com/yourusername/rst_esp32_lifesaver.git
cd rst_esp32_lifesaver
```

**Build and flash ESP32**:
```bash
cargo build --release
espflash flash target/xtensa-esp32s3-none-elf/release/rst_esp32_lifesaver
```

**Install Python dependencies**:
```bash
pip install -r host/requirements.txt
```

### Running the System

1. **Start Mosquitto MQTT broker** on your computer (the one with IP 172.20.10.4):
   ```bash
   mosquitto -p 1883
   ```

2. **Connect ESP32 to iPhone hotspot** (configure WiFi credentials in firmware)

3. **Run the host server** to visualize data:
   ```bash
   python host/server.py
   ```

The ECG waveform will appear on your screen, updating with fresh data every 10 seconds in a smooth scrolling animation.

## MQTT Communication

### Topic Structure

The ESP32 publishes to:
```
devices/{DEVICE_ID}/messages/events/
```

### Payload Format

ECG data is published as JSON:
```json
{
  "ecg": [sample1, sample2, sample3, ..., sample1250]
}
```

Where each sample is a 12-bit value (0-4095) representing the ADC reading in millivolts.

### Host Subscription

The host server subscribes to:
```
devices/+/messages/events/#
```

This wildcard subscription allows it to receive data from any device publishing to the events topic.

## Configuration

### ESP32 Configuration

Edit `src/bin/main.rs`:

- **MQTT Broker**: Line 541 - Change `172.20.10.4` to your broker's IP
- **WiFi SSID/Password**: Configure in WiFi initialization section
- **Sampling Rate**: Line 351 - Currently set to 125Hz
- **Publish Period**: Line 353 - Currently 10 seconds (1250 samples)
- **Buffer Size**: Line 354 - Automatically calculated from rate and period
- **Socket Buffers**: Line 535-538 - TX/RX buffer sizes (8000 bytes each)

### Host Server Configuration

Edit `host/server.py`:

- **MQTT Broker**: Line 9 - Change `MQTT_HOST` to match your broker's IP
- **Filter Cutoff**: Line 36 - Butterworth filter cutoff frequency (default 45Hz)
- **Display Samples**: Line 27 - Number of samples shown at once (default 1250)

## SAS Token (Legacy)

For Azure IoT Hub integration, the machine running `server.py` must set the environment variable:
```bash
export IOTHUB_DEVICE_KEY="your-device-key"
```

The SAS token is automatically generated with a 1-hour expiration for authentication.

## Troubleshooting

**ESP32 won't connect to MQTT broker**:
- Verify the MQTT broker IP is correct and reachable from the ESP32
- Check that the broker is running (`mosquitto` service)
- Ensure ESP32 is connected to the iPhone hotspot WiFi
- Check connection logs in ESP32 serial output

**Plot window is unresponsive**:
- The MQTT client now runs in a background thread, so the UI should always be responsive
- If it freezes, check for exceptions in the console output

**Data looks garbled/corrupted**:
- Verify the ECG analog front-end is properly connected to GPIO2
- Check that the voltage is in the 0-3.1V range for 11dB attenuation
- Verify the MQTT buffer sizes are sufficient for your packet size (currently 8000 bytes)

**Missing or incomplete data**:
- Check that PING packets are being sent every 100ms to keep the connection alive
- Verify the socket write/flush operations complete successfully
- Monitor MQTT broker logs for connection drops

## Performance Notes

- ECG samples are processed at 125Hz (8ms per sample)
- Each buffer contains 1250 samples = 10 seconds of data
- Buffers are sent every 10 seconds via MQTT
- The plot updates at 100Hz for smooth animations
- Rolling buffer keeps only 2500 samples in memory to limit resource usage

## License

See LICENSE file for details.

## Support

For issues or questions, please open an issue on GitHub or contact the maintainers.
