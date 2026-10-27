# probe.py: python3 probe.py <your-home-public-ip>
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(5)
s.sendto(b"\x4f\x45\x74\x03\x00\x00\x00\x01", (sys.argv[1], 30001))
try:
    data, src = s.recvfrom(1024)
    print("reply from", src, data.hex())
except socket.timeout:
    print("no reply within 5 s")
