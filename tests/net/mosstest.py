"""Moss growing on cobblestone beside water, over the network, on a fresh
world: python3 mosstest.py PORT netclient.py X,Y,Z (open water).
The server runs with LUANTI_MOSS_FAST, so moss grows within seconds."""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
water = tuple(int(v) for v in sys.argv[3].split(","))
COBBLE, MOSSY, STAIR_COBBLE, STAIR_MOSSY = 17, 372, 45, 374
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "mason.inv"), "w").write("kit 1\n0 esp:cobble 3\n1 esp:stair_cobble 2\n2 esp:stone 2\n")
def interact(cl, action, item, under, above):
    pointed = struct.pack(">BB3h3h", 0, 1, *under, *above)
    p = [int(v * 100) for v in cl.pos_bs]
    cl.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed
            + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 3000, -9000, 0, 72, 5))
c = Client("Mason", port); c.login(); c.playerpos(); c.pump(0.3)
c.pos_bs = [water[0] * 10, (water[1] + 2) * 10, water[2] * 10]; c.playerpos()
want = tuple(v // 16 for v in water)
end = time.time() + 20
while time.time() < end and not any(struct.unpack_from(">hhh", m, 2) == want for m in c.take(0x20)):
    c.playerpos(); c.pump(0.2)
x, y, z = water
# Cobblestone and a cobblestone stair placed into the water; stone too.
spots = {(x, y, z): 0, (x + 2, y, z): 1, (x + 4, y, z): 2}
# Every node change, in order: moss can grow on the first one placed while
# the others are still being placed, so none may be dropped.
changes = []
def collect(secs, until):
    end = time.time() + secs
    while time.time() < end and not until():
        c.pump(0.05)
        changes.extend((struct.unpack_from(">hhh", m, 2), (u16(m, 8), m[11])) for m in c.take(0x21))
placed = {}
for spot, slot in spots.items():
    interact(c, 3, slot, (spot[0], y - 1, spot[2]), spot)
    collect(5, lambda: any(p == spot for p, _ in changes))
    placed[spot] = next(n for p, n in changes if p == spot); print("placed", *placed[spot], "at", spot)
assert placed[(x, y, z)][0] == COBBLE and placed[(x + 2, y, z)][0] == STAIR_COBBLE
# Moss grows on the cobblestone and the stair (which keeps its turn), not
# on the stone: what each became after it was placed.
def changed():
    return {p: n for p, n in changes if p in placed and n != placed[p]}
collect(15, lambda: len(changed()) >= 2)
grown = changed()
print("grown:", grown, flush=True)
assert grown.get((x, y, z), (0,))[0] == MOSSY
assert grown.get((x + 2, y, z)) == (STAIR_MOSSY, placed[(x + 2, y, z)][1])
assert (x + 4, y, z) not in grown
# On dry land it doesn't: a cobblestone up on the stone.
dry = (x + 4, y + 1, z)
c.cmds.clear(); interact(c, 3, 0, (x + 4, y, z), dry)
c.wait_for(0x21, 5, cond=lambda m: struct.unpack_from(">hhh", m, 2) == dry)
c.cmds.clear(); c.pump(4)
assert not any(struct.unpack_from(">hhh", m, 2) == dry for m in c.take(0x21)), "moss grew away from water"
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("MOSS OK")
