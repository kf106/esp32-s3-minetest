import socket, sys
try:
    c = socket.create_connection((sys.argv[1], 30001), timeout=5)
    print("connected; got", c.recv(100))
except OSError as e:
    print("failed:", e)
