"""Gunpowder and TNT over the network: python3 tnttest.py PORT netclient.py"""
import sys, time, struct, os, zstandard
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "blaster.inv"), "w").write(
    "kit 1\n0 esp:torch 5\n1 esp:gunpowder 20\n2 esp:tnt 5\n3 esp:flint_and_steel 1\n4 esp:chest 1\n9 esp:coal_lump 1\n10 esp:gravel 1\n")

def interact(cl, action, item, under, above):
    pointed = struct.pack(">BB3h3h", 0, 1, *under, *above)
    p = [int(v * 100) for v in cl.pos_bs]
    cl.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed
            + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 3000, 0, 0, 72, 5))
def say(cl, text):
    u = text.encode("utf-16-be"); cl.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
def chat(m):
    o = 4; n = u16(m, o); o += 2 + 2 * n; n = u16(m, o); o += 2
    return m[o:o + 2 * n].decode("utf-16-be")
def heard(cl, want, secs=3):
    lines = []; end = time.time() + secs
    while time.time() < end and not any(want in l for l in lines):
        cl.pump(0.05); lines += [chat(m) for m in cl.take(0x2F)]
    return lines
def go(cl, x, y, z):
    cl.pos_bs = [x * 10.0, y * 10.0, z * 10.0]; cl.playerpos(); cl.pump(0.2)
nodes, sounds, events, pkts = {}, [], [], []
def watch(cl, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.cmds:
            cmd = u16(m)
            if cmd == 0x21:
                p = struct.unpack_from(">hhh", m, 2); nodes[p] = u16(m, 8); events.append(("add", p, u16(m, 8)))
            elif cmd == 0x22:
                p = struct.unpack_from(">hhh", m, 2); nodes[p] = 126; events.append(("remove", p))
            elif cmd == 0x20:
                bx, by, bz = struct.unpack_from(">hhh", m, 2)
                raw = zstandard.ZstdDecompressor().decompressobj().decompress(m[8:])
                for i in range(4096):
                    z, y, x = i // 256, (i // 16) % 16, i % 16
                    nodes[(bx * 16 + x, by * 16 + y, bz * 16 + z)] = struct.unpack_from(">H", raw, 5 + 2 * i)[0]
            elif cmd == 0x3f:
                n = u16(m, 6); sounds.append(m[8:8 + n].decode())
            elif cmd in (0x33, 0x2b, 0x31, 0x47):
                pkts.append((cmd, m))
        cl.cmds = []
        if until and until(): return True
    return False
def place(cl, slot, under, above):
    interact(cl, 3, slot, under, above); watch(cl, 0.3)
    assert nodes.get(above) not in (None, 126), f"nothing placed at {above}"
    return nodes[above]

c = Client("Blaster", port); c.login(); c.playerpos(); watch(c, 1.5)
# Coal and gravel: 5 gunpowder.
c.inventory_action("Move 1 current_player main 9 current_player craft 0")
c.inventory_action("Move 1 current_player main 10 current_player craft 4")
c.wait_for(0x27, cond=lambda m: b"esp:gunpowder 5" in m); print("coal and gravel make 5 gunpowder")

# On the flat ground south of spawn (y 2): a trail of gunpowder from x 7
# to x 10 leading to TNT at x 11, all at z 12, and a chest beside it.
go(c, 6, 3, 9)
for x in range(7, 11):
    POWDER = place(c, 1, (x, 2, 12), (x, 3, 12))
TNT = place(c, 2, (11, 2, 12), (11, 3, 12))
CHEST = place(c, 4, (11, 2, 10), (11, 3, 10))
print("gunpowder", POWDER, "tnt", TNT)
# A torch's punch lights the trail's end; it burns along to the TNT.
interact(c, 0, 0, (7, 3, 12), (7, 4, 12)); watch(c, 0.5)
assert nodes[(7, 3, 12)] == POWDER + 1, nodes[(7, 3, 12)]
watch(c, 6, until=lambda: nodes.get((11, 3, 12)) == TNT + 1)
assert nodes[(11, 3, 12)] == TNT + 1, "the trail didn't reach the TNT"
lit = [e[1][0] for e in events if e[0] == "add" and e[2] == POWDER + 1]
print("trail burnt along x", lit, "and lit the TNT"); assert lit == [7, 8, 9, 10]
assert "tnt_gunpowder_burning" in sounds and "tnt_ignite" in sounds
# 4 s later: the blast. The player 5 nodes off is hurt and thrown back.
pkts.clear()
watch(c, 6, until=lambda: "tnt_explode" in sounds)
assert "tnt_explode" in sounds, "no explosion"
watch(c, 1.0)
hp = [struct.unpack_from(">H", m, 2)[0] for k, m in pkts if k == 0x33]
push = [struct.unpack_from(">3f", m, 2) for k, m in pkts if k == 0x2b]
print("HP now", hp, "thrown", push); assert hp and hp[-1] < 20 and push and push[0][0] < 0
# A crater: the TNT, the ground under it and the chest are gone.
for p in [(11, 3, 12), (11, 2, 12), (11, 1, 12), (11, 3, 10), (12, 2, 12)]:
    assert nodes.get(p) in (126, None) or nodes[p] >= 395 and nodes[p] <= 400, (p, nodes.get(p))
# The blast's flash, smoke and flying bits of stone (the tnt mod's
# add_effects): particle spawners, read as the client reads them.
effects = [spawner(m) for k, m in pkts if k == 0x47]
print("particles:", [(e["texture"], e["amount"], e["node"]) for e in effects])
assert {"tnt_boom.png", "tnt_smoke.png", "tnt_blast.png"} <= {e["texture"] for e in effects}
assert any(e["node"] != 127 for e in effects), "no bits of the blown-up node"
added = [a for k, m in pkts if k == 0x31 for a in remove_add(m)[1]]
print("thrown out:", len(added), "items")
assert len(added) >= 3
# Out of the blast's reach, the ground stays.
assert nodes.get((11, 2, 18), 126) != 126

# /tnt off: TNT can't be lit; on again, it can, and turned off before it
# goes off, it goes out.
say(c, "/tnt off"); assert any("now off" in l for l in heard(c, "now off"))
go(c, 9, 3, 15)
place(c, 2, (13, 2, 15), (13, 3, 15))
interact(c, 4, 3, (13, 3, 15), (13, 4, 15)); watch(c, 1)
assert nodes[(13, 3, 15)] == TNT, "lit with TNT off"
say(c, "/tnt on"); heard(c, "now on")
interact(c, 4, 3, (13, 3, 15), (13, 4, 15)); watch(c, 0.5)
assert nodes[(13, 3, 15)] == TNT + 1, "not lit with TNT on"
say(c, "/tnt off"); heard(c, "now off")
watch(c, 5)
assert nodes[(13, 3, 15)] == TNT and sounds.count("tnt_explode") == 1, "went off with TNT off"
print("TNT off: not lit, and lit TNT goes out")
say(c, "/tnt"); assert any("TNT is off" in l for l in heard(c, "TNT is"))
say(c, "/tnt on"); heard(c, "now on")
# Another player without the privileges can't turn it off.
d = Client("Griefer", port); d.login(); d.pump(0.5)
say(d, "/tnt off"); l = heard(d, "permission"); print(l); assert any("missing privilege" in x for x in l)
d.raw(0, b"\x00\x03")

# Two TNT next to each other go off together, in one blast.
go(c, 7, 3, 9)
for x in (13, 14):
    place(c, 2, (x, 2, 9), (x, 3, 9))
sounds.clear()
interact(c, 4, 3, (13, 3, 9), (13, 4, 9)); watch(c, 0.5)
go(c, 4, 3, 2)
watch(c, 6, until=lambda: "tnt_explode" in sounds); watch(c, 1)
print("sounds:", sounds)
assert sounds.count("tnt_explode") == 1 and nodes.get((14, 3, 9)) in (126, None)
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("TNT OK")
