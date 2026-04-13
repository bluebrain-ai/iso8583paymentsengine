import socket
import struct
import time

def send_mock_tx():
    # Valid ISO8583 Message: "0100" MTI, and a 8-byte bitmap
    mti = b"0100"
    bitmap = b"\xFF\x00\x00\x00\x00\x00\x00\x00"
    payload = mti + bitmap
    
    # 2-byte Big Endian header
    header = struct.pack("!H", len(payload))
    packet = header + payload
    
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.connect(("127.0.0.1", 8000))
    s.sendall(packet)
    time.sleep(0.1) # Small delay to allow span completion before drop
    s.close()
    print("Sent transaction!")

for i in range(3):
    send_mock_tx()
    time.sleep(0.5)
