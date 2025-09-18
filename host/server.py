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

# IOT hub parameters
iot_hub_name = "ih-iothub01.azure-devices.net"
device_id = "device01-piemulator"
device_key = os.getenv("IOTHUB_DEVICE_KEY")

# MQTT connection parameters
username = f"{iot_hub_name}/{device_id}/?api-version=2021-04-12"
topic = f"devices/{device_id}/messages/events/"

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

def recv_all(sock, length):
    """Receive exactly 'length' bytes from the socket."""
    data = b''
    while len(data) < length:
        more = sock.recv(length - len(data))
        if not more:
            raise EOFError("Socket closed before receiving all data")
        data += more
    return data

def plot_data(data):
    """Plot the unpacked short integers using matplotlib."""
    plt.figure(figsize=(10, 4))
    plt.plot(data)
    plt.title("Received TCP Data")
    plt.xlabel("Index")
    plt.ylabel("Value")
    plt.grid(True)
    plt.tight_layout()
    plt.show()

def start_tcp_server(host='172.20.10.4', port=8080, buffer_size=4000):
    server_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server_socket.bind((host, port))
    server_socket.listen(5)

    print(f"Server listening on {host}:{port}")
    client_socket, client_address = server_socket.accept()
    print(f"Connection from {client_address}")

    
    print("Generating SAS token...")
    uri = f"{iot_hub_name}/devices/{device_id}"
    sas_token = generate_sas_token(uri, device_key, None)

    print("Connecting to mqtt client...")
    client = mqtt.Client(client_id=device_id, protocol=mqtt.MQTTv311)
    client.username_pw_set(username=username, password=sas_token)
    client.tls_set()
    client.connect(iot_hub_name, port=8883)

    while True:
        try:
            data = recv_all(client_socket, buffer_size)
            shorts = struct.unpack('<' + 'H' * (len(data) // 2), data)
            print(f"Received {len(shorts)} values")
            
            ecg_dict = {"ecg": shorts}
            payload = json.dumps(ecg_dict) 

            client.publish(topic, payload)
        except Exception as e:
            print(f"Error: {e}")
            client_socket.sendall(b"Error receiving data")

if __name__ == "__main__":
    start_tcp_server()
