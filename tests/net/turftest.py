"""Grass spreading onto dirt and dying under cover, over the network (with
LUANTI_GRASS_FAST: the ABMs every second, every time):
python3 turftest.py PORT netclient.py"""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "gardener.inv"), "w").write("kit 1\n0 esp:shovel_steel 1\n1 esp:stone 9\n2 esp:dirt 9\n3 esp:dry_grass_1 1\n")
def interact(cl, action, item, under, above):
    pointed = struct.pack(">BB3h3h", 0, 1, *under, *above)
    p = [int(v * 100) for v in cl.pos_bs]
    cl.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed
            + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 3000, 0, 0, 72, 5))
def becomes(cl, pos, secs=6):
    m = cl.wait_for(0x21, secs, cond=lambda m: struct.unpack_from(">hhh", m, 2) == pos)
    return u16(m, 8)
c = Client("Gardener", port); c.login(); c.playerpos(); c.pump(1.0)
GRASS, DIRT = None, None
# The flat grass south of spawn is at y 2 (x 6 to 14, z 8 to 14). Dig a
# grass block: the dirt under it, in daylight with grass round it, turns
# to grass.
c.cmds.clear(); interact(c, 2, 0, (10, 2, 11), (10, 3, 11))
c.wait_for(0x22, cond=lambda m: struct.unpack_from(">hhh", m, 2) == (10, 2, 11))
GRASS = becomes(c, (10, 1, 11)); print("the dirt under it became", GRASS)
# Stone on the grass beside: it dies back to dirt.
c.cmds.clear(); interact(c, 3, 1, (11, 2, 11), (11, 3, 11))
DIRT = becomes(c, (11, 2, 11)); print("under stone, grass became", DIRT)
assert DIRT != GRASS
# Dirt put down next to grass: grass in turn. (The covered dirt under the
# stone stays dirt: no light.)
c.cmds.clear(); interact(c, 3, 2, (12, 2, 12), (12, 3, 12))
assert becomes(c, (12, 3, 12)) == DIRT
assert becomes(c, (12, 3, 12)) == GRASS; print("placed dirt grew grass")
c.pump(3); assert not any(struct.unpack_from(">hhh", m, 2) == (11, 2, 11) for m in c.take(0x21))
# Savanna grass on dirt (up on stone, away from any grass): dirt with
# savanna grass, which spreads to dirt beside it.
for z in (13, 14):
    c.cmds.clear(); interact(c, 3, 1, (8, 2, z), (8, 3, z)); becomes(c, (8, 3, z))
c.cmds.clear(); interact(c, 3, 2, (8, 3, 13), (8, 4, 13)); assert becomes(c, (8, 4, 13)) == DIRT
c.cmds.clear(); interact(c, 3, 3, (8, 4, 13), (8, 5, 13)); becomes(c, (8, 5, 13))
SAVANNA = becomes(c, (8, 4, 13)); print("under savanna grass, dirt became", SAVANNA)
assert SAVANNA not in (DIRT, GRASS)
c.cmds.clear(); interact(c, 3, 2, (8, 3, 14), (8, 4, 14))
assert becomes(c, (8, 4, 14)) == DIRT
assert becomes(c, (8, 4, 14)) == SAVANNA; print("and spread to the dirt beside it")
# Stone placed pointing at the savanna grass goes in its place (as
# builtin's item_place_node), not above it; covered, the dirt dies back.
c.cmds.clear(); interact(c, 3, 1, (8, 5, 13), (8, 6, 13))
STONE = becomes(c, (8, 5, 13)); print("the grass became", STONE)
assert becomes(c, (8, 4, 13)) == DIRT; print("and under it, dirt again")
c.pump(1); assert not any(struct.unpack_from(">hhh", m, 2) == (8, 6, 13) for m in c.take(0x21)), "placed above the grass"
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("TURF OK")
