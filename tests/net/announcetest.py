"""Announcing on the server list (announce.rs), from the PC build, to a
stand-in for the list server: the same checks on the announcement as
luanti-org/serverlist's server.py (its schema and field rules), and the
same probe of the game port (its serverUp packets). The server announces
"start" a few seconds after starting; the stand-in finds the game port
answering; switching announcing off on the settings page takes the server
off the list ("delete") before it restarts.

    python3 announcetest.py PROGRAM UDP_PORT HTTP_PORT LIST_PORT DATA_DIR
"""
import json, os, socket, subprocess, sys, threading, time, urllib.parse, http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

program, port, http_port, list_port, data = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]

# server.py's fields: name -> (required, type, item type)
FIELDS = {
    "address": (False, str), "port": (False, int),
    "clients": (True, int), "clients_max": (True, int), "uptime": (True, int), "game_time": (True, int),
    "lag": (False, float), "clients_list": (False, list, str), "mods": (False, list, str),
    "version": (True, str), "proto_min": (True, int), "proto_max": (True, int),
    "gameid": (True, str), "mapgen": (False, str), "url": (False, str), "privs": (False, str),
    "name": (True, str), "description": (True, str),
    "creative": (False, bool), "dedicated": (False, bool), "damage": (False, bool), "pvp": (False, bool),
    "password": (False, bool), "rollback": (False, bool), "can_see_far_names": (False, bool),
}

def schema_ok(req):
    for name, spec in FIELDS.items():
        if name not in req:
            if spec[0]:
                return f"missing {name}"
            continue
        v = req[name]
        if type(v) is not spec[1]:
            return f"{name} is {type(v).__name__}, not {spec[1].__name__}"
        if len(spec) > 2 and any(type(x) is not spec[2] for x in v):
            return f"{name} has a non-{spec[2].__name__}"
    if req["proto_min"] > req["proto_max"]:
        return "proto_min > proto_max"
    return None

def server_up(port):
    """server.py's serverUp: an empty ORIGINAL packet; a SET_PEER_ID back;
    a DISCO to finish. The ping, or None."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(3)
    try:
        s.connect(("127.0.0.1", port))
        s.send(b"\x4f\x45\x74\x03\x00\x00\x00\x01")
        t = time.monotonic()
        data = s.recv(1024)
        ping = time.monotonic() - t
        if data[0:4] != b"\x4f\x45\x74\x03" or data[4:6] != b"\x00\x01" or data[11] != 0x01:
            return None
        s.send(b"\x4f\x45\x74\x03" + data[12:14] + b"\x00\x00\x03")
        return ping
    except OSError:
        return None
    finally:
        s.close()

announcements, listed, lock = [], {}, threading.Lock()

class List(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, code, text):
        body = text.encode()
        self.send_response(code)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        if self.path != "/announce":
            return self.reply(404, "Not found")
        kind = self.headers.get("Content-Type", "")
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        assert kind.startswith("multipart/form-data; boundary="), kind
        boundary = kind.split("boundary=", 1)[1].encode()
        parts = [p for p in body.split(b"--" + boundary) if p.strip() not in (b"", b"--")]
        assert len(parts) == 1, parts
        head, value = parts[0].split(b"\r\n\r\n", 1)
        assert b'name="json"' in head, head
        req = json.loads(value[:-2] if value.endswith(b"\r\n") else value)
        with lock:
            announcements.append(dict(req))
        action = req.pop("action", "")
        key = req.get("port", 30000)
        if action == "delete":
            with lock:
                gone = listed.pop(key, None)
            return self.reply(200, "Removed from server list." if gone else "Server to remove not found.")
        why = schema_ok(req)
        if why:
            return self.reply(400, "JSON data does not conform to schema. " + why)
        if action == "update" and key not in listed:
            return self.reply(200, "Server to update not found.")
        ping = server_up(req["port"])
        if ping is None:
            return self.reply(409, f"Request has been filed, but the previous one encountered an error:\nServer port {key} did not respond to ping")
        with lock:
            listed[key] = dict(req, ping=ping)
        return self.reply(202, "Request has been filed.")

ThreadingHTTPServer.allow_reuse_address = True
list_server = ThreadingHTTPServer(("127.0.0.1", list_port), List)
threading.Thread(target=list_server.serve_forever, daemon=True).start()

os.makedirs(data, exist_ok=True)
open(os.path.join(data, "server.txt"), "w").write(
    "server_announce = true\nserver_name = Stick test\nserver_description = A test of announcing\n"
    f"server_announce_send_players = true\nserverlist_url = http://127.0.0.1:{list_port}\n")
log = open(os.path.join(data, "server.log"), "w")
server = subprocess.Popen([program, "--port", str(port), "--bind", "127.0.0.1", "--data", data, "--config-page", str(http_port)],
                          stdout=subprocess.DEVNULL, stderr=log)

def wait(what, cond, secs):
    end = time.time() + secs
    while time.time() < end:
        if cond():
            return
        time.sleep(0.2)
    raise AssertionError(f"timed out waiting for {what}; announcements: {announcements}")

try:
    wait("the start", lambda: port in listed, 20)
    start = announcements[0]
    print("announced:", {k: start[k] for k in ("action", "name", "version", "proto_min", "proto_max", "port")})
    assert start["action"] == "start" and start["name"] == "Stick test" and start["description"] == "A test of announcing"
    assert start["proto_min"] == 53 and start["proto_max"] == 53 and start["gameid"] == "minetest"
    assert start["version"].startswith("esp32-s3-minetest-pc-") and " " not in start["version"]
    assert start["damage"] is True and start["creative"] is False and start["pvp"] is False
    assert start["clients"] == 0 and start["clients_list"] == [] and start["clients_max"] == 15
    assert "address" not in start and "url" not in start
    assert start["mapgen"] == "esp-gen" and "esp:default" in start["mods"] and "esp:mobs" in start["mods"]
    print(f"listed: the game port answered the probe ({listed[port]['ping'] * 1000:.2f} ms)")

    # Announcing off on the settings page: off the list before the restart.
    h = http.client.HTTPConnection("127.0.0.1", http_port, timeout=10)
    form = urllib.parse.urlencode([("ann_name", "Stick test"), ("ann_desc", "A test of announcing"), ("fire", "on"), ("restart", "pc")])
    h.request("POST", "/save", form, {"Content-Type": "application/x-www-form-urlencoded"})
    assert h.getresponse().status == 200
    wait("the delete", lambda: any(a.get("action") == "delete" for a in announcements), 15)
    assert port not in listed
    delete = [a for a in announcements if a.get("action") == "delete"][0]
    assert set(delete) <= {"action", "port", "address"}, delete
    print("switched off: taken off the list before the restart")
    # Restarted without announcing: nothing more.
    time.sleep(8)
    assert [a["action"] if "action" in a else "?" for a in announcements] == ["start", "delete"], announcements
    print("ANNOUNCE OK")
finally:
    server.kill()
    server.wait()
    list_server.shutdown()
