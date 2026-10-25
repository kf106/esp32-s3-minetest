"""Hexers over the network: python3 hexertest.py PORT netclient.py"""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "seeker.inv"), "w").write("kit 1\n0 esp:sword_steel 1\n")

def say(cl, text):
    u = text.encode("utf-16-be"); cl.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
def chat(m):
    o = 4; n = u16(m, o); o += 2 + 2 * n; n = u16(m, o); o += 2
    return m[o:o + 2 * n].decode("utf-16-be")
lines, sounds, objs, gone, hp, nodes, items, punched, parts = [], [], {}, [], [], [], [], [], []
def watch(cl, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.cmds:
            cmd = u16(m)
            if cmd == 0x2F:
                lines.append(chat(m))
            elif cmd == 0x3f:
                n = u16(m, 6); sounds.append(m[8:8 + n].decode())
            elif cmd == 0x21:
                nodes.append((struct.unpack_from(">hhh", m, 2), u16(m, 8)))
            elif cmd == 0x31:
                removed, added = remove_add(m)
                for oid in removed:
                    if oid in objs:
                        objs.pop(oid); gone.append(oid)
                o = 4 + 2 * len(removed) + 2
                for oid, name in added:
                    ln = struct.unpack_from(">I", m, o + 3)[0]; init = m[o + 7:o + 7 + ln]; o += 7 + ln
                    if name.startswith("mobs:"):
                        nl = u16(init, 1)
                        objs[oid] = [name, struct.unpack_from(">3f", init, 3 + nl + 3)]
                        if name == "mobs:hexer":
                            assert b"mobs_hexer.png" in init and b"character.b3d" in init
                        if name == "mobs:hexer_orb":
                            assert b"mobs_hexer_orb.png" in init and b"sprite" in init
                    elif name == "__builtin:item":
                        items.append(init)
            elif cmd == 0x32:
                o = 2
                while o < len(m):
                    i = u16(m, o); ln = u16(m, o + 2); msg = m[o + 4:o + 4 + ln]; o += 4 + ln
                    if i in objs and msg[0] == 1:
                        objs[i][1] = struct.unpack_from(">3f", msg, 1)
                    if i in objs and msg[0] == 4:
                        punched.append((i, u16(msg, 1)))
            elif cmd == 0x33:
                hp.append(u16(m, 2))
            elif cmd == 0x47:
                parts.append(spawner(m))
        cl.cmds = []
        if until and until(): return True
    return False
def ask(cl, text, want):
    del lines[:]; say(cl, text); watch(cl, 3, until=lambda: any(want in l for l in lines))
    found = [l for l in lines if want in l]; assert found, (text, lines); return found[0]
def hexers(): return [i for i, (n, _) in objs.items() if n == "mobs:hexer"]

c = Client("Seeker", port); c.login(); c.playerpos(); watch(c, 3)
del hp[:]
ask(c, "/mobs on", "now on")
print(ask(c, "/mobs spawn hexer", "A hexer comes"))
watch(c, 0.5)
hexer = hexers()[0]
# It sees the player, hisses, keeps away, and casts: the ground at its
# feet dies, and an orb flies, humming, and hits.
watch(c, 15, until=lambda: hp and hp[-1] < 20)
orbs = [i for i in gone + list(objs) if objs.get(i, ["mobs:hexer_orb"])[0] == "mobs:hexer_orb" and i != hexer]
print("sounds", sorted(set(sounds)), "HP", hp, "drained", nodes[:4])
for s in ("mobs_hexer_hiss", "mobs_hexer_cast", "mobs_hexer_orb", "mobs_hexer_impact"):
    assert s in sounds, s
assert hp and hp[-1] < 20
# Its orbs trail glowing bits.
assert any(p["texture"] == "mobs_hexer_orb.png" and p["attached"] in orbs for p in parts), parts
assert nodes and all(n in (1, 288 + 0) or n > 0 for _, n in nodes)
# Five blows of the steel sword kill it (30 HP), leaving iron.
body = lambda: [v / 10 for v in c.pos_bs]
end = time.time() + 40
while hexer in objs and time.time() < end:
    # Stepping up to 5 nodes from it (any closer and it blinks away).
    p = objs[hexer][1]
    c.pos_bs = [p[0] + 50.0, p[1], p[2]]; c.playerpos()
    watch(c, 0.1)
    c.punch(hexer)
    watch(c, 0.8)
print("punched", punched)
assert hexer in gone and any(b"esp:iron_lump" in i for i in items), "not killed"
assert "mobs_hexer_hurt" in sounds and "mobs_hexer_death" in sounds
print("killed: iron lump")
# At dawn it vanishes in smoke.
watch(c, 6)
ask(c, "/time 6:00", "")
ask(c, "/mobs spawn hexer", "A hexer comes"); watch(c, 0.5)
hexer = max(hexers())
del punched[:]
watch(c, 15, until=lambda: hexer in gone)
assert hexer in gone and (hexer, 0) in punched and "mobs_hexer_vanish" in sounds, (gone, punched)
print("vanished at dawn")
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("HEXER OK")
