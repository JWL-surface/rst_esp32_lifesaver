import socket
import struct
import matplotlib.pyplot as plt
import paho.mqtt.client as mqtt

# IOT hub parameters
iot_hub_name = "ih-iothub01.azure-devices.net"
device_id = "device01-piemulator"

# DO NOT PUSH
sas_token = ""  # full SAS token

# MQTT connection parameters
username = f"{iot_hub_name}/{device_id}/?api-version=2021-04-12"
topic = f"devices/{device_id}/messages/events/"


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

    client = mqtt.Client(client_id=device_id, protocol=mqtt.MQTTv311)
    client.username_pw_set(username=username, password=sas_token)
    client.connect(iot_hub_name, port=8883)
    client.tls_set()

    while True:
        print("Waiting for client connection...")
        client_socket, client_address = server_socket.accept()
        print(f"Connected to {client_address}")

        while True:
            try:
                data = recv_all(client_socket, buffer_size, timeout=1)

                print(data)
                shorts = struct.unpack('<' + 'H' * (len(data) // 2), data)
                print(f"Received {len(shorts)} values")

                payload = ','.join(map(str, shorts))

                client.publish(topic, payload)
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
    start_tcp_server()