import socket
import struct
import matplotlib.pyplot as plt
import paho.mqtt.client as mqtt
import os
from base64 import b64encode, b64decode
from hashlib import sha256
from time import time
from urllib import parse
from hmac import HMAC
import json
from scipy.signal import butter, filtfilt

# IOT hub parameters
iot_hub_name = "ih-iothub01.azure-devices.net"
device_id = "device01-piemulator"
device_key = str(os.getenv("IOTHUB_DEVICE_KEY"))

# MQTT connection parameters
username = f"{iot_hub_name}/{device_id}/?api-version=2021-04-12"
topic = f"devices/{device_id}/messages/events/"

plt.ion()
fig, ax = plt.subplots(figsize=(14, 6))
line, = ax.plot([], [], lw=2)

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

#https://learn.microsoft.com/en-us/azure/iot-hub/authenticate-authorize-sas?tabs=python
def generate_sas_token(uri, key, policy_name, expiry=3600):
    ttl = time() + expiry
    sign_key = "%s\n%d" % ((parse.quote_plus(uri)), int(ttl))
    print(sign_key)
    signature = b64encode(HMAC(b64decode(key), sign_key.encode('utf-8'), sha256).digest())

    rawtoken = {
        'sr' :  uri,
        'sig': signature,
        'se' : str(int(ttl))
    }

    if policy_name is not None:
        rawtoken['skn'] = policy_name

    return 'SharedAccessSignature ' + parse.urlencode(rawtoken)

def recv_all(sock, length, timeout=5):
    """Receive exactly 'length' bytes from the socket with timeout."""
    sock.settimeout(timeout)
    data = b''
    try:
        while len(data) < length:
            more = sock.recv(length - len(data))
            if not more:
                raise EOFError("Socket closed before receiving all data")
            more = more.replace(b'PING', b'')
            data += more
    except socket.timeout:
        raise TimeoutError("Socket timed out waiting for data")
    finally:
        sock.settimeout(None)  # Reset to blocking mode
    return data

def plot_data(data):
    """Plot the unpacked short integers using matplotlib."""
    """Update the plot with new data in real time."""
    line.set_xdata(range(len(data)))
    line.set_ydata(data)
    ax.relim()
    ax.autoscale_view()
    fig.canvas.draw()
    fig.canvas.flush_events()

def start_tcp_server(host='172.20.10.4', port=8080, buffer_size=2500):
    server_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server_socket.bind((host, port))
    server_socket.listen(5)
    
    print("Generating SAS token...")
    uri = f"{iot_hub_name}/devices/{device_id}"
    sas_token = generate_sas_token(uri, device_key, None)

    print("Connecting to mqtt client...")
    client = mqtt.Client(client_id=device_id, protocol=mqtt.MQTTv311)
    client.username_pw_set(username=username, password=sas_token)
    client.tls_set()
    client.connect(iot_hub_name, port=8883)

    while True:
        print("Waiting for client connection...")
        client_socket, client_address = server_socket.accept()
        print(f"Connected to {client_address}")

        while True:
            try:
                data = recv_all(client_socket, buffer_size, timeout=3)

                shorts = struct.unpack('<' + 'H' * (len(data) // 2), data)
                print(f"Received {len(shorts)} values")
                ecg_dict = {"ecg": list(shorts)}
                payload = json.dumps(ecg_dict)
                client.publish(topic, payload)
                filtered_shorts = lowpass_filter(shorts)
                plot_data(filtered_shorts)

            except (EOFError, ConnectionResetError, TimeoutError) as e:
                print(f"Client disconnected: {e}")
                client_socket.close()
                break
            except Exception as e:
                print(f"Error: {e}")
                try:
                    client_socket.sendall(b"Error receiving data")
                except:
                    pass

if __name__ == "__main__":
    ax.set_title("Real-Time ECG Data")
    ax.set_xlabel("Index")
    ax.set_ylabel("Value")
    ax.grid(True)

    manager = plt.get_current_fig_manager()
    manager.window.wm_geometry("+0+0")

    start_tcp_server()
