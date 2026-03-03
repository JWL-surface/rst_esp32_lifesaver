import json
import matplotlib.pyplot as plt
from scipy.signal import butter, filtfilt
import paho.mqtt.client as mqtt
import time
import threading

# MQTT configuration
MQTT_HOST = '172.20.10.4'
MQTT_PORT = 1883

# Global plot setup
plt.ion()
fig, ax = plt.subplots(figsize=(14, 6))
line, = ax.plot([], [], lw=2)
ax.set_title("Real-Time ECG Data")
ax.set_xlabel("Time (samples)")
ax.set_ylabel("Value (mV)")
ax.grid(True)
manager = plt.get_current_fig_manager()
manager.window.wm_geometry("+0+0")

# Rolling window for plot (limit to last N samples for performance)
# Keep 2500 samples but display 1250 at a time (shows smooth scrolling effect)
# New data slides in from the right as the buffer fills up
BUFFER_SIZE = 2500
MAX_DISPLAY_SAMPLES = 1250
SAMPLING_RATE = 125  # Hz
plot_buffer = []
plot_lock = threading.Lock()  # Protects plot_buffer from race conditions
buffer_start_sample = 0  # Which sample index to start displaying from (for scrolling effect)
last_buffer_length = 0  # Track buffer length at last update
buffer_arrival_time = time.time()  # When the current buffer started arriving


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
    """Add new data to buffer and reset scrolling animation."""
    global plot_buffer, buffer_arrival_time, last_buffer_length

    with plot_lock:
        # Add new data to buffer
        plot_buffer.extend(data)

        # Keep only the last BUFFER_SIZE samples
        if len(plot_buffer) > BUFFER_SIZE:
            plot_buffer = plot_buffer[-BUFFER_SIZE:]

        # Reset animation when new buffer arrives
        last_buffer_length = len(plot_buffer)
        buffer_arrival_time = time.time()


def update_plot_smooth():
    """Update plot smoothly with scrolling animation."""
    global buffer_start_sample, buffer_arrival_time, last_buffer_length

    with plot_lock:
        if len(plot_buffer) == 0:
            return

        # If we have less than or equal to one full display, just show what we have without scrolling
        if len(plot_buffer) <= MAX_DISPLAY_SAMPLES:
            plot_data_copy = plot_buffer
        else:
            # Calculate elapsed time since buffer arrival
            elapsed = time.time() - buffer_arrival_time

            # Calculate which sample to start displaying (scroll left as time progresses)
            # At 125Hz, we show MAX_DISPLAY_SAMPLES in (MAX_DISPLAY_SAMPLES/125) seconds
            scroll_amount = int(elapsed * SAMPLING_RATE)

            # The display window slides across the buffer
            # We show samples starting from scroll_amount
            display_start = scroll_amount
            display_end = display_start + MAX_DISPLAY_SAMPLES

            # Clamp to buffer bounds
            display_start = max(0, min(display_start, len(plot_buffer) - 1))
            display_end = min(display_end, len(plot_buffer))

            if display_end > display_start:
                plot_data_copy = plot_buffer[display_start:display_end]
            else:
                return

        if plot_data_copy:
            # Update plot with the visible window
            line.set_xdata(range(len(plot_data_copy)))
            line.set_ydata(plot_data_copy)
            ax.relim()
            ax.autoscale_view()
            fig.canvas.draw_idle()


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

        # Start MQTT client in background thread
        client.loop_start()

        # Keep the plot window responsive to user interactions
        print("Plot window is ready. Close the window to exit.")
        while True:
            # Update plot smoothly for real-time scrolling effect
            update_plot_smooth()
            plt.pause(0.01)  # 100Hz update rate for smooth animation
            time.sleep(0.005)

    except KeyboardInterrupt:
        print("\nShutting down...")
    except Exception as e:
        print(f"Error: {e}")
    finally:
        client.loop_stop()
        client.disconnect()
        plt.close('all')


if __name__ == "__main__":
    main()
