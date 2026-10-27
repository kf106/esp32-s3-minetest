"""The settings page (config_page.rs) as the PC build serves it: on
127.0.0.1, without the Wi-Fi and hotspot sections. The page and its
textures load; a bad form is refused without saving (announcing without a
description, or with a private address); a good one writes motd.txt,
world/settings.txt (with passwords required) and server.txt, tells the connected player the server
is restarting (with "reconnect"), and the server restarts in place: the
same process answers again, with the new settings.

    python3 configtest.py UDP_PORT HTTP_PORT DATA_DIR SERVER_PID netclient.py

(The board's Wi-Fi and hotspot sections are covered by config_page.rs's
unit tests.)
"""
import os, sys, time, struct, http.client, urllib.parse
src = open(sys.argv[5]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port, http_port, data, pid = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], int(sys.argv[4])

def get(path):
    h = http.client.HTTPConnection("127.0.0.1", http_port, timeout=10)
    h.request("GET", path); r = h.getresponse(); return r.status, r.getheader("Content-Type"), r.read()

def post(fields):
    h = http.client.HTTPConnection("127.0.0.1", http_port, timeout=10)
    body = urllib.parse.urlencode(fields)
    h.request("POST", "/save", body, {"Content-Type": "application/x-www-form-urlencoded"})
    r = h.getresponse(); return r.status, r.read().decode()

c = Client("Tester", port); c.login(); c.playerpos(); c.pump(0.5)

status, kind, body = get("/")
page = body.decode()
assert status == 200 and kind.startswith("text/html"), (status, kind)
assert "Server settings" in page and "this computer" in page
assert "Wi-Fi networks" not in page and "The board's hotspot" not in page, "the PC page shows Wi-Fi"
assert 'value="pc">Save and restart the server' in page
assert 'name="fire" checked' in page and 'name="creative">' in page
assert 'name="ann_on" disabled>' in page, "the announce switch should start greyed out"
print("page loads, without Wi-Fi")
status, kind, png = get("/t/esp_dirt.png")
assert status == 200 and kind == "image/png" and png[:4] == b"\x89PNG"
assert get("/t/../motd.txt")[0] == 404 and get("/nothing")[0] == 404
print("textures served, nothing else")

status, page = post([("ann_on", "on"), ("ann_name", "Stick"), ("ann_desc", ""), ("restart", "pc")])
assert "Not saved" in page and "description" in page, page[:300]
status, page = post([("ann_on", "on"), ("ann_name", "Stick"), ("ann_desc", "Small"), ("ann_addr", "192.168.1.5"), ("restart", "pc")])
assert "Not saved" in page and "private" in page, page[:300]
assert "server_name" not in open(os.path.join(data, "server.txt")).read()
print("announcing refused without a description, or with a private address")

status, page = post([("tnt", "on"), ("creative", "on"), ("motd", "Hello\r\nthere"),
                     ("ann_on", "on"), ("ann_name", "Minetest on a Stick"), ("ann_desc", "A small world.\nBe nice"),
                     ("ann_url", ""), ("ann_addr", "mc.zalmanac.com"), ("ann_port", "30001"), ("nopw", "on"), ("restart", "pc")])
assert status == 200 and "The server is restarting" in page, page[:300]
print("saved")

# The player is told, and offered a reconnect: TOCLIENT_ACCESS_DENIED,
# reason 11 (shutdown), a message, reconnect 1.
denied = []; end = time.time() + 5
while time.time() < end and not denied:
    c.pump(0.05); denied += c.take(0x0A)
assert denied, "no restart notice"
m = denied[0]; n = struct.unpack(">H", m[3:5])[0]
assert m[2] == 11 and m[5 + n] == 1, m
print("player told:", m[5:5 + n].decode())

settings = open(os.path.join(data, "world", "settings.txt")).read()
assert "enable_fire = false" in settings and "enable_tnt = true" in settings and "enable_mobs = false" in settings and "creative_mode = true" in settings and "disallow_empty_password = true" in settings, settings
assert open(os.path.join(data, "motd.txt")).read() == "Hello\nthere\n"
server = open(os.path.join(data, "server.txt")).read()
for want in ["server_announce = true", "server_name = Minetest on a Stick", "server_description = A small world. Be nice",
             "server_address = mc.zalmanac.com", "server_announce_port = 30001", "server_announce_send_players = false"]:
    assert want + "\n" in server, (want, server)
assert "serverlist_url = http://127.0.0.1:9\n" in server, "the page must keep serverlist_url"
assert not os.path.exists(os.path.join(data, "wifi.txt")) and not os.path.exists(os.path.join(data, "boot-hotspot"))
print("files written, and no Wi-Fi files")

# Restarted in place: the same process, the page back with the new settings.
page = None; end = time.time() + 20
while time.time() < end and page is None:
    time.sleep(0.5)
    try:
        page = get("/")[2].decode()
    except OSError:
        pass
assert page is not None, "the page didn't come back"
os.kill(pid, 0)  # still the same process
assert 'name="creative" checked' in page and 'name="ann_on" checked>' in page and 'name="nopw" checked' in page, "not the new settings"
c2 = Client("Tester2", port); c2.login(); c2.playerpos()
# Passwords required now: a newcomer with an empty one is refused, with
# reason 7 (empty password), as Luanti refuses them.
c3 = Client("Nopass", port)
name = c3.name.encode()
c3.send(0, struct.pack(">HBHHH", 0x02, 29, 0, 37, 53) + struct.pack(">H", len(name)) + name)
c3.wait_for(0x02)
c3.send(0, struct.pack(">H", 0x50) + struct.pack(">H", 16) + bytes(16) + struct.pack(">H", 1) + b"\x01" + b"\x01")
m = c3.wait_for(0x0A)
assert m[2] == 7, m
print("an empty password is refused")
print("restarted in place: the page and the game are back, with the new settings")
print("CONFIG PAGE OK")
