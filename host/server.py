import json
import matplotlib.pyplot as plt
from scipy.signal import butter, filtfilt
import paho.mqtt.client as mqtt

# MQTT configuration
MQTT_HOST = '172.20.10.4'
MQTT_PORT = 1883

# Global plot setup
plt.ion()
fig, ax = plt.subplots(figsize=(14, 6))
line, = ax.plot([], [], lw=2)
ax.set_title("Real-Time ECG Data")
ax.set_xlabel("Index")
ax.set_ylabel("Value (mV)")
ax.grid(True)
manager = plt.get_current_fig_manager()
manager.window.wm_geometry("+0+0")


def lowpass_filter(data, cutoff=45.0, fs=125, order=4):
    """
    Apply a low-pass Butterworth filter to the data.
    - cutoff: cutoff frequency in Hz
    - fs: sampling rate in Hz
    - order: filter order
    """
    nyquist = 0.5 * fs
    normal_cutoff = cutoff / nyquist
    b, a = butter(order, normal_cutoff, btype='low', analog=False)
    filtered = filtfilt(b, a, data)
    return filtered


def plot_data(data):
    """Update the plot with new data in real time."""
    line.set_xdata(range(len(data)))
    line.set_ydata(data)
    ax.relim()
    ax.autoscale_view()
    fig.canvas.draw()
    fig.canvas.flush_events()


def process_ecg_message(data_dict):
    """Process ECG data from MQTT message."""
    try:
        ecg_values = data_dict.get("ecg", [])

        if ecg_values:
            print(f"Received {len(ecg_values)} ECG values")

            # Convert to float for filtering and plotting
            ecg_data = [float(val) for val in ecg_values]

            # Apply low-pass filter
            filtered_data = lowpass_filter(ecg_data)

            # Plot the filtered data
            plot_data(filtered_data)
        else:
            print("No ECG data found in payload")

    except Exception as e:
        print(f"Error processing ECG message: {e}")


def on_connect(client, userdata, flags, rc):
    """Callback for when the client connects to the broker."""
    if rc == 0:
        print(f"Connected to MQTT broker at {MQTT_HOST}:{MQTT_PORT}")
        # Subscribe to ECG topics
        client.subscribe("devices/+/messages/events/#", qos=1)
        print("Subscribed to topic: devices/+/messages/events/#")
    else:
        print(f"Failed to connect with code {rc}")


def on_message(client, userdata, msg):
    """Callback for when a message is received from the broker.

    paho-mqtt handles all MQTT packet validation and decoding.
    The message object contains properly parsed data.
    """
    try:
        topic = msg.topic
        payload = msg.payload.decode('utf-8')

        print(f"Received message on topic '{topic}'")

        # Try to parse as JSON
        try:
            payload_data = json.loads(payload)
            process_ecg_message(payload_data)
        except json.JSONDecodeError as e:
            print(f"Failed to decode JSON: {e}")

    except Exception as e:
        print(f"Error handling message: {e}")


def main():
    """Main entry point for the MQTT ECG client."""
    print(f"Starting MQTT Client for ECG data reception...")
    print(f"Connecting to {MQTT_HOST}:{MQTT_PORT}")

    # Create MQTT client
    client = mqtt.Client(
        callback_api_version=mqtt.CallbackAPIVersion.VERSION1,
        client_id="ecg-server"
    )

    # Set callbacks
    client.on_connect = on_connect
    client.on_message = on_message

    # Enable debug logging
    client.enable_logger()

    # Connect to broker
    try:
        client.connect(MQTT_HOST, MQTT_PORT, keepalive=60)
        print("Waiting for ESP32 to send ECG data...")

        # Blocking call that processes network traffic and dispatches callbacks
        client.loop_forever()

    except Exception as e:
        print(f"Error: {e}")
    finally:
        client.disconnect()


if __name__ == "__main__":
    main()
