import socket
import time

# Standard payload mapping for the ISO 8583 Mock
# Length: 12 Bytes (0x00, 0x0C)
# MTI: "0100" -> [0x30, 0x31, 0x30, 0x30]
# Bitmap: [0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]

HOST = '127.0.0.1'
PORT = 8080 # Update this dynamically based on Edge Ingress binding

def run_pos_client():
    print(f"[*] Preparing mock POS terminal connection to {HOST}:{PORT}")
    
    length_header = bytes([0x00, 0x0C])
    mti = b"0100"
    bitmap = bytes([0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
    
    packet = length_header + mti + bitmap
    
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.connect((HOST, PORT))
            print(f"[+] Connected to Payment Engine Ingress.")
            
            # Send single integration packet
            s.sendall(packet)
            print(f"[+] Sent RAW 0100 Packet: {packet.hex()}")
            
            # Note: For throughput benchmarking, wrap the sendall in a loop 
            # while maintaining the socket connection.
            
    except ConnectionRefusedError:
        print("[-] Connection Refused: Ensure the edge-ingress Engine is actively listening!")

if __name__ == "__main__":
    run_pos_client()
