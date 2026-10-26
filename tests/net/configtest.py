"""The settings page (config_page.rs), on the PC build started with
--config-page: the page and its textures load, a bad form is refused
without saving (a short password; announcing without a description or
with a private address), and a good one writes wifi.txt, motd.txt,
world/settings.txt, server.txt and the hotspot marker, tells the connected player the
server is restarting (with "reconnect"), and stops the server.

    python3 configtest.py UDP_PORT HTTP_PORT DATA_DIR netclient.py
"""
import os, sys, time, struct, http.client, urllib.parse
src = open(sys.argv[4]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port, http_port, data = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]

def get(path):
    h = http.client.HTTPConnection("127.0.0.1", http_port, timeout=10)
    h.request("GET", path); r = h.getresponse(); return r.status, r.getheader("Content-Type"), r.read()

def post(fields):
    h = http.client.HTTPConnection("127.0.0.1", http_port, timeout=10)
    body = urllib.parse.urlencode(fields)
    h.request("POST", "/save", body, {"Content-Type": "application/x-www-form-urlencoded"})
    r = h.getresponse(); return r.status, r.read().decode()

def write(name, text):
    os.makedirs(os.path.dirname(os.path.join(data, name)), exist_ok=True)
    open(os.path.join(data, name), "w").write(text)

# What the board's files might hold.
write("wifi.txt", "network Home\npassword secret123\n\nnetwork Cafe\n\nhotspot MTG-0BC4\npassword minetest\n")

c = Client("Tester", port); c.login(); c.playerpos(); c.pump(0.5)

status, kind, body = get("/")
page = body.decode()
assert status == 200 and kind.startswith("text/html"), (status, kind)
assert "Server settings" in page and 'value="Home"' in page and 'value="Cafe"' in page
assert "secret123" not in page, "a network password was sent to the page"
assert 'name="fire" checked' in page and 'name="mobs">' in page
assert 'name="ann_on" disabled>' in page, "the announce switch should start greyed out"
print("page loads; network passwords not shown")
status, kind, png = get("/t/esp_dirt.png")
assert status == 200 and kind == "image/png" and png[:4] == b"\x89PNG"
assert get("/t/../wifi.txt")[0] == 404 and get("/nothing")[0] == 404
print("textures served, nothing else")

base = [("ssid0", "Cafe"), ("pass0", ""), ("orig0", "Cafe"),
        ("ssid1", "Home"), ("pass1", ""), ("orig1", "Home"),
        ("hs_ssid", "Den"), ("hs_pass", "drawbridge")]
status, page = post(base + [("ssid2", "Office"), ("pass2", "short"), ("restart", "hotspot")])
assert status == 200 and "Not saved" in page and "8 to 63" in page, page[:300]
assert "secret123" in open(os.path.join(data, "wifi.txt")).read()
assert not os.path.exists(os.path.join(data, "boot-hotspot"))
print("bad password refused, nothing saved")
status, page = post(base + [("ann_on", "on"), ("ann_name", "Stick"), ("ann_desc", ""), ("restart", "hotspot")])
assert "Not saved" in page and "description" in page, page[:300]
status, page = post(base + [("ann_on", "on"), ("ann_name", "Stick"), ("ann_desc", "Small"), ("ann_addr", "192.168.1.5"), ("restart", "hotspot")])
assert "Not saved" in page and "private" in page, page[:300]
assert not os.path.exists(os.path.join(data, "server.txt"))
print("announcing refused without a description, or with a private address")

status, page = post(base + [("ssid2", "Office"), ("pass2", "hunter22"), ("orig2", ""),
                            ("tnt", "on"), ("creative", "on"), ("motd", "Hello\r\nthere"),
                            ("ann_on", "on"), ("ann_name", "Minetest on a Stick"), ("ann_desc", "A small world.\nBe nice"),
                            ("ann_url", ""), ("ann_addr", "mc.zalmanac.com"), ("ann_port", "30001"), ("restart", "hotspot")])
assert status == 200 and "restarting on its hotspot" in page and "Den" in page, page[:300]
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

wifi = open(os.path.join(data, "wifi.txt")).read()
assert "network Cafe\n\nnetwork Home\npassword secret123\n\nnetwork Office\npassword hunter22\n" in wifi, wifi
assert "hotspot Den\npassword drawbridge\n" in wifi, wifi
settings = open(os.path.join(data, "world", "settings.txt")).read()
assert "enable_fire = false" in settings and "enable_tnt = true" in settings and "enable_mobs = false" in settings and "creative_mode = true" in settings, settings
assert open(os.path.join(data, "motd.txt")).read() == "Hello\nthere\n"
assert os.path.exists(os.path.join(data, "boot-hotspot"))
server = open(os.path.join(data, "server.txt")).read()
for want in ["server_announce = true", "server_name = Minetest on a Stick", "server_description = A small world. Be nice",
             "server_address = mc.zalmanac.com", "server_announce_port = 30001", "server_announce_send_players = false"]:
    assert want + "\n" in server, (want, server)
print("files written")
print("CONFIG PAGE OK")
