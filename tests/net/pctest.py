"""The PC build (pc/) as a server: a player logs in, is welcomed, is sent
the map, and /status names the build: python3 pctest.py PORT netclient.py"""
import sys, time, struct
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])

def chat(m):
    o = 4; n = u16(m, o); o += 2 + 2 * n; n = u16(m, o); o += 2
    return m[o:o + 2 * n].decode("utf-16-be")
def say(c, text):
    u = text.encode("utf-16-be"); c.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
def heard(c, want, secs=5):
    lines = []; end = time.time() + secs
    while time.time() < end and not any(want in l for l in lines):
        c.pump(0.05); lines += [chat(m) for m in c.take(0x2F)]
    return lines

c = Client("Tester", port); c.login(); c.playerpos()
l = heard(c, "Nobody else")
print("\n".join(l))
assert l[0].startswith("Welcome!") and "PC build" in l[0] and "Up to 3 players" in l[1]
# Luanti's join and leave lines, to everyone.
assert any(x == "*** Tester joined the game." for x in l + heard(c, "joined the game")), "no join line"
d = Client("Visitor", port); d.login(); d.playerpos()
assert any(x == "*** Visitor joined the game." for x in heard(c, "Visitor joined")), "the others weren't told of a join"
d.raw(0, b"\x00\x03")  # DISCO
assert any(x == "*** Visitor left the game." for x in heard(c, "Visitor left")), "the others weren't told of a leave"
print("join and leave lines heard")
# Map blocks arrive.
blocks = 0; end = time.time() + 20
while time.time() < end and blocks < 50:
    c.pump(0.1); blocks += len(c.take(0x20))
print("blocks", blocks); assert blocks >= 50
# Every file, as a new player's client asks for them: sent a bunch at a
# time, every file whole. (In requests of 40 names: this client doesn't
# split what it sends, as Luanti's does.)
import os, zstandard
media = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "media")
names = sorted(f for f in os.listdir(media) if f.endswith((".png", ".ogg", ".b3d", ".obj")))
c.pump(0.3); c.take(0x38)  # the empty answer to the login's request
for i in range(0, len(names), 40):
    part = names[i:i + 40]
    c.send(1, struct.pack(">HH", 0x40, len(part)) + b"".join(struct.pack(">H", len(n)) + n.encode() for n in part))
got = {}; bunches = 0; end = time.time() + 120
while time.time() < end and len(got) < len(names):
    c.pump(0.05)
    for m in c.take(0x38):
        bunches += 1
        o = 10
        for _ in range(struct.unpack_from(">I", m, 6)[0]):
            n = u16(m, o); name = m[o + 2:o + 2 + n].decode(); o += 2 + n
            ln = struct.unpack_from(">I", m, o)[0]; data = m[o + 4:o + 4 + ln]; o += 4 + ln
            got[name] = zstandard.ZstdDecompressor().decompressobj().decompress(data)
missing = [n for n in names if n not in got]
assert not missing, (len(missing), missing[:5])
bad = [n for n in names if got[n] != open(os.path.join(media, n), "rb").read()]
assert not bad, bad[:5]
print(f"media: {len(names)} files in {bunches} bunches, all whole")
say(c, "/status")
l = heard(c, "# Server")
print(l[-1]); assert any("PC build" in x and "players (1/3)" in x for x in l)
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("PC SERVER OK")
