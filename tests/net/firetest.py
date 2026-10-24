"""Fire over the network, with LUANTI_FIRE_FAST (its ABMs every second,
every time, and flames' timers 1 to 2 s): python3 firetest.py PORT netclient.py"""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "firestarter.inv"), "w").write(
    "kit 1\n0 esp:flint_and_steel 1\n1 esp:apple_wood 40\n2 esp:coalblock 1\n3 esp:pick_stone 1\n9 esp:flint 1\n10 esp:steel_ingot 1\n")

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
nodes = {}
added = []
def watch(cl, secs):
    """Node changes, as last seen, and every node added."""
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.take(0x21):
            p = struct.unpack_from(">hhh", m, 2); nodes[p] = (u16(m, 8), m[11]); added.append((p, u16(m, 8), m[11]))
        for m in cl.take(0x22): nodes[struct.unpack_from(">hhh", m, 2)] = (126, 0)
def place(cl, slot, under, above):
    cl.cmds.clear(); interact(cl, 3, slot, under, above); watch(cl, 0.3)
    assert above in nodes, f"nothing placed at {above}"

c = Client("Firestarter", port); c.login(); c.playerpos(); c.pump(0.5)
WOOD = None
# Flint beside a steel ingot: flint and steel.
c.cmds.clear(); c.inventory_action("Move 1 current_player main 9 current_player craft 0")
c.inventory_action("Move 1 current_player main 10 current_player craft 1")
inv = c.wait_for(0x27, cond=lambda m: b"craftpreview" in m and b"esp:flint_and_steel" in m.split(b"craftpreview")[1][:80])
print("flint and steel crafts")

# A tower of planks, 20 tall, on the flat grass south of spawn (ground y 2).
X, Z = 10, 12
go(c, X + 2, 3, Z)
place(c, 1, (X, 2, Z), (X, 3, Z)); WOOD = nodes[(X, 3, Z)][0]
for y in range(4, 23):
    if y % 6 == 0: go(c, X + 2, y - 1, Z)
    place(c, 1, (X, y - 1, Z), (X, y, Z))
print("tower of", sum(1 for y in range(3, 23) if nodes.get((X, y, Z), (0,))[0] == WOOD), "planks")
go(c, X + 3, 3, Z)
# Strike at its foot, on its east face: a flame beside it, step 0.
c.cmds.clear(); interact(c, 4, 0, (X, 3, Z), (X + 1, 3, Z)); watch(c, 0.5)
FLAME, step = nodes[(X + 1, 3, Z)]
print("flame", FLAME, "step", step); assert step == 0 and FLAME not in (WOOD, 126)
# It burns the tower and spreads through the grass round it, but no flame
# is lit more than 8 nodes from the first.
go(c, X + 4, 8, Z)
# The crackle, looped, to this player.
m = c.wait_for(0x3f, 5, cond=lambda m: b"fire_fire" in m)
n = u16(m, 6); looped = m[8 + n + 4 + 1 + 12 + 2]
print("crackle, looped:", looped); assert looped == 1
watch(c, 30)
flames = {p: s for p, i, s in added if i == FLAME}
print("flames lit at", len(flames), "places; furthest step", max(flames.values()))
far = max(max(abs(p[0] - X - 1), abs(p[1] - 3), abs(p[2] - Z)) for p in flames)
print("furthest", far, "nodes from the first")
assert far <= 8 and all(s <= 8 for s in flames.values()) and max(flames.values()) >= 4, flames
burnt = [y for y in range(3, 23) if nodes.get((X, y, Z)) == (126, 0)]
print("burnt planks at heights", burnt)
assert 3 in burnt and not any(y >= 13 for y in burnt), burnt
# Once nothing is left to burn beside them, the flames go out.
watch(c, 6)
left = [p for p, (i, s) in nodes.items() if i == FLAME]
print("flames left:", left); assert not left
# With them out, the crackle stops.
c.wait_for(0x40, 5); print("crackle stopped")
c.cmds.clear(); say(c, "/fire"); l = heard(c, "Fire spreading"); print(l)
assert any("Fire spreading is on" in x for x in l)

# /fire off: a flame still lights, but nothing burns.
say(c, "/fire off"); assert any("now off" in x for x in heard(c, "now off"))
go(c, X - 2, 3, Z - 3)
place(c, 1, (X - 4, 2, Z - 3), (X - 4, 3, Z - 3))
c.cmds.clear(); interact(c, 4, 0, (X - 4, 3, Z - 3), (X - 4, 4, Z - 3)); watch(c, 4)
lit_off = [p for p, i, s in added if p == (X - 4, 4, Z - 3) and i == FLAME]
assert lit_off and nodes[(X - 4, 3, Z - 3)][0] == WOOD, "not lit, or burnt, with fire off"
# ... and the flame goes out at its timer, though the plank is beside it.
assert nodes[(X - 4, 4, Z - 3)] == (126, 0), nodes[(X - 4, 4, Z - 3)]
print("fire off: the flame lights and goes out, the plank stays")
say(c, "/fire on"); heard(c, "now on")
# A coal block's permanent flame, which goes when the block is dug.
go(c, X + 1, 3, Z - 2)
place(c, 2, (X + 3, 2, Z - 4), (X + 3, 3, Z - 4))
c.cmds.clear(); interact(c, 4, 0, (X + 3, 3, Z - 4), (X + 3, 4, Z - 4)); watch(c, 0.5)
PERMANENT = nodes[(X + 3, 4, Z - 4)][0]; print("permanent flame", PERMANENT); assert PERMANENT == FLAME + 1
watch(c, 3); assert nodes[(X + 3, 4, Z - 4)][0] == PERMANENT, "the permanent flame went out"
c.cmds.clear(); interact(c, 2, 3, (X + 3, 3, Z - 4), (X + 3, 4, Z - 4)); watch(c, 0.5)
assert nodes[(X + 3, 4, Z - 4)] == (126, 0), "the permanent flame outlived its coal"
print("permanent flame gone with its coal")
# Flint and steel wears by 1/66 with each strike (three so far).
c.raw(0, b"\x00\x03"); time.sleep(0.5)
saved = open(os.path.join(players, "firestarter.inv")).read()
fs = [l for l in saved.splitlines() if "flint_and_steel" in l]; print("saved:", fs)
assert fs and 3 * (65536 // 66) <= int(fs[0].split()[3]) <= 3 * (65536 // 66) + 3, fs
print("FIRE OK")
