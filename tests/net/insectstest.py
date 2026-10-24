"""Butterflies and fireflies over the network: python3 insectstest.py PORT netclient.py"""
import sys, time, struct, os, zstandard
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "catcher.inv"), "w").write(
    "kit 1\n0 esp:butterfly_white 2\n1 esp:bug_net 1\n2 esp:firefly_bottle 1\n3 esp:firefly 1\n4 esp:glass_bottle 1\n")

def interact(cl, action, item, under, above):
    pointed = struct.pack(">BB3h3h", 0, 1, *under, *above)
    p = [int(v * 100) for v in cl.pos_bs]
    cl.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed
            + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 3000, 0, 0, 72, 5))
def say(cl, text):
    u = text.encode("utf-16-be"); cl.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
nodes, invs = {}, []
def watch(cl, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.cmds:
            cmd = u16(m)
            if cmd == 0x21:
                nodes[struct.unpack_from(">hhh", m, 2)] = u16(m, 8)
            elif cmd == 0x22:
                nodes[struct.unpack_from(">hhh", m, 2)] = 126
            elif cmd == 0x20:
                bx, by, bz = struct.unpack_from(">hhh", m, 2)
                raw = zstandard.ZstdDecompressor().decompressobj().decompress(m[8:])
                for i in range(4096):
                    z, y, x = i // 256, (i // 16) % 16, i % 16
                    nodes[(bx * 16 + x, by * 16 + y, bz * 16 + z)] = struct.unpack_from(">H", raw, 5 + 2 * i)[0]
            elif cmd == 0x27:
                invs.append(m)
        cl.cmds = []
        if until and until(): return True
    return False

c = Client("Catcher", port); c.login(); c.playerpos(); watch(c, 2)
c.pos_bs = [60.0, 30.0, 90.0]; c.playerpos(); watch(c, 0.5)
say(c, "/time 12:00"); watch(c, 1)
# A butterfly put on the flat ground south of spawn (y 2), out in the sun.
here = (7, 3, 12)
interact(c, 3, 0, (7, 2, 12), here); watch(c, 0.5)
WHITE = nodes[here]; print("butterfly", WHITE)
assert WHITE not in (126, None)
watch(c, 2.5)
assert nodes[here] == WHITE, "hid by day"
# Night: it hides (an invisible node), and comes out again by day.
say(c, "/time 23:00")
assert watch(c, 4, until=lambda: nodes[here] == WHITE + 3), nodes[here]
print("hidden at night:", nodes[here])
say(c, "/time 12:00")
assert watch(c, 4, until=lambda: nodes[here] == WHITE), nodes[here]
print("out again by day")
# Caught with the net: into the inventory (two again), the net worn.
del invs[:]
interact(c, 0, 1, here, (7, 4, 12)); interact(c, 2, 1, here, (7, 4, 12))
watch(c, 1, until=lambda: invs and b"esp:butterfly_white 2" in invs[-1])
assert nodes[here] == 126 and invs and b"esp:butterfly_white 2" in invs[-1], invs[-1:] if invs else None
assert re.search(rb"esp:bug_net 1 \d+", invs[-1]), invs[-1]
print("caught in the net")
# A firefly in a bottle, set down and right-clicked: the firefly goes two
# up, the bottle is empty; in the sun the firefly soon hides.
bottle = (9, 3, 12)
interact(c, 3, 2, (9, 2, 12), bottle); watch(c, 0.5)
BOTTLE = nodes[bottle]
interact(c, 3, 0, bottle, (9, 4, 12)); watch(c, 0.5)
up = (9, 5, 12)
FIREFLY = WHITE + 6
print("bottle", BOTTLE, "now", nodes[bottle], "firefly", nodes.get(up))
assert nodes.get(up) in (FIREFLY, FIREFLY + 1) and nodes[bottle] != BOTTLE
assert watch(c, 4, until=lambda: nodes[up] == FIREFLY + 1), nodes[up]
print("let out; hidden in the sun")
# A firefly over a glass bottle makes a firefly in a bottle.
c.inventory_action("Move 1 current_player main 3 current_player craft 1")
c.inventory_action("Move 1 current_player main 4 current_player craft 4")
c.wait_for(0x27, cond=lambda m: b"esp:firefly_bottle" in m.split(b"List craftpreview")[1][:80] if b"List craftpreview" in m else False)
print("firefly + bottle = firefly in a bottle")
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("INSECTS OK")
