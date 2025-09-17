import socket
import struct
import matplotlib.pyplot as plt

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

    while True:
        try:
            data = recv_all(client_socket, buffer_size)
            shorts = struct.unpack('<' + 'H' * (len(data) // 2), data)

            print(f"Received {len(shorts)} values")
            plot_data(shorts)
        except Exception as e:
            print(f"Error: {e}")
            client_socket.sendall(b"Error receiving data")

if __name__ == "__main__":
    start_tcp_server()