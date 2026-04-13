import socket

HOST = '127.0.0.1'
PORT = 9090 # Downstream Target Host Port (Visa/Mastercard)

def run_bank_server():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind((HOST, PORT))
        s.listen()
        print(f"[*] Mock Bank Server awaiting TCP connection on {HOST}:{PORT}...")
        
        conn, addr = s.accept()
        with conn:
            print(f"[+] Egress Client connected from {addr}")
            while True:
                length_bytes = conn.recv(2)
                if not length_bytes:
                    break
                
                payload_len = int.from_bytes(length_bytes, byteorder='big')
                print(f"[*] Incoming payload expected explicitly of length: {payload_len} bytes")
                
                payload = conn.recv(payload_len)
                
                # SEDA Edge Egress generates specifically the MTI prefix natively!
                mti = payload[:4].decode('utf-8', errors='ignore')
                print(f"[+] Successfully isolated mapped transaction! MTI: {mti} | Hex Dump: {payload.hex()}")
                
if __name__ == "__main__":
    run_bank_server()
