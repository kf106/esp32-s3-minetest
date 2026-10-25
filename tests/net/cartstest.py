"""Carts over the network: python3 cartstest.py PORT netclient.py
Rails laid on the flat ground south of spawn, a cart put on them, pushed,
picking up a dropped item, ridden, switched at a junction, left, and
picked up again with a sneak-punch."""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "rider.inv"), "w").write("kit 1\n0 esp:rail 20\n1 esp:cart 1\n2 esp:stick 5\n")
SNEAK, LEFT, RIGHT = 64, 4, 8

def pos_packet(cl, keys=0, pitch=0.0, yaw=0.0):
    p = [int(v * 100) for v in cl.pos_bs]
    return struct.pack(">3i3iiiIBB", *p, 0, 0, 0, int(pitch * 100), int(yaw * 100), keys, 72, 5)
def interact(cl, action, pointed, keys=0, pitch=0.0, yaw=0.0, item=0):
    cl.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed + pos_packet(cl, keys, pitch, yaw))
def at_node(under, above):
    return struct.pack(">BB3h3h", 0, 1, *under, *above)
def at_object(oid):
    return struct.pack(">BBH", 0, 2, oid)
def go(cl, x, y, z, pitch=0.0, yaw=0.0, keys=0):
    cl.pos_bs = [x * 10.0, y * 10.0, z * 10.0]
    cl.send(0, struct.pack(">H", 0x23) + pos_packet(cl, keys, pitch, yaw))

nodes, added, removed, msgs, invs, eyes, turned = {}, {}, [], [], [], [], []
def watch(cl, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.cmds:
            cmd = u16(m)
            if cmd == 0x21:
                nodes[struct.unpack_from(">hhh", m, 2)] = u16(m, 8)
            elif cmd == 0x31:
                rem, add = remove_add(m)
                removed.extend(rem)
                o = 4 + 2 * len(rem) + 2
                for oid, name in add:
                    ln = struct.unpack_from(">I", m, o + 3)[0]; added[oid] = (name, m[o + 7:o + 7 + ln]); o += 7 + ln
            elif cmd == 0x32:
                o = 2
                while o < len(m):
                    i = u16(m, o); ln = u16(m, o + 2); msgs.append((i, m[o + 4:o + 4 + ln])); o += 4 + ln
            elif cmd == 0x27:
                invs.append(m)
            elif cmd == 0x52:
                eyes.append(struct.unpack_from(">3f", m, 2))
            elif cmd == 0x34:
                turned.append(struct.unpack_from(">5f", m, 2))
        cl.cmds = []
        if until and until(): return True
    return False
def attached(oid):
    return [struct.unpack_from(">h", m, 1)[0] for i, m in msgs if i == oid and m[0] == 8]
def moves(oid):
    return [struct.unpack_from(">3f3f", m, 1) for i, m in msgs if i == oid and m[0] == 1]
def where(oid):
    mv = moves(oid)
    return [round(v / 10, 2) for v in mv[-1][:3]] if mv else None

c = Client("Rider", port); c.login(); c.playerpos(); watch(c, 1.5)

# A line of rail along x at z 12 (x 6 to 13) on the flat ground at y 2, and
# a branch off it at x 10 going +z.
track = [(x, 12) for x in range(6, 14)] + [(10, 13), (10, 14)]
for x, z in track:
    go(c, x, 3, z - 2)
    interact(c, 3, at_node((x, 2, z), (x, 3, z)))
    watch(c, 0.15)
watch(c, 1, until=lambda: all((x, 3, z) in nodes for x, z in track))
RAIL = nodes[(6, 3, 12)]
assert all(nodes.get((x, 3, z)) == RAIL for x, z in track), nodes
print("rails laid:", len(track), "of", RAIL)

# The cart goes on the rail pointed at.
go(c, 6, 3, 10)
interact(c, 3, at_node((6, 3, 12), (6, 4, 12)), item=1)
watch(c, 3, until=lambda: any(n == "carts:cart" for n, _ in added.values()))
cart = next(oid for oid, (n, _) in added.items() if n == "carts:cart")
assert b"carts_cart.b3d" in added[cart][1] and b"carts_cart.png" in added[cart][1]
print("cart", cart)

# A stick dropped beside it, then a push the way the player looks (+x:
# yaw 270): it rolls off along the rails, taking the stick with it,
# straight on over the junction, and coasts to a stop.
go(c, 6, 2.5, 11, pitch=60.0, yaw=0.0)
watch(c, 0.3)
c.inventory_action("Drop 1 current_player main 2")
watch(c, 3, until=lambda: any(n == "__builtin:item" for n, _ in added.values()))
stick = next(oid for oid, (n, _) in added.items() if n == "__builtin:item")
watch(c, 2)
interact(c, 0, at_object(cart), pitch=0.0, yaw=270.0)
watch(c, 2, until=lambda: attached(stick))
assert attached(stick) == [cart], ("the stick wasn't picked up", attached(stick))
watch(c, 12, until=lambda: moves(cart) and moves(cart)[-1][3:] == (0.0, 0.0, 0.0))
stop = where(cart)
print("pushed along x, stopped at", stop, "after", len(moves(cart)), "updates")
assert stop[0] > 9.5 and stop[2] == 12.0 and stop[1] == 3.0

# Right-clicked, the player gets in, standing, eyes lowered.
go(c, stop[0], 3, 10.5)
interact(c, 3, at_object(cart))
watch(c, 2, until=lambda: attached(c.peer) and eyes)
assert attached(c.peer) == [cart] and eyes and eyes[-1] == (0.0, -4.0, 0.0), (attached(c.peer), eyes)
print("riding, eyes", eyes[-1])

# Pushed back (-x: yaw 90) with right held: at the junction it turns off
# onto the branch (+z). (The branch is short: off its end, it comes back.)
msgs.clear(); turned.clear()
interact(c, 0, at_object(cart), pitch=0.0, yaw=90.0, keys=RIGHT)
on_branch = lambda: any(m[0] == 100.0 and m[2] > 125.0 for m in moves(cart))
end = time.time() + 6
while time.time() < end and not on_branch():
    c.send(0, struct.pack(">H", 0x23) + pos_packet(c, RIGHT, yaw=90.0)); watch(c, 0.1)
assert on_branch(), [tuple(round(v / 10, 2) for v in m[:3]) for m in moves(cart)]
print("switched onto the branch")
# The rider turned with it: from facing -x (yaw 90) to +z (yaw 0), a
# quarter turn clockwise, in steps.
yaws = [round(m[4]) % 360 for m in turned]
assert 0 in yaws and any(0 < y < 90 for y in yaws), yaws
print("the rider's view turned:", yaws[:6])
watch(c, 8, until=lambda: moves(cart)[-1][3:] == (0.0, 0.0, 0.0))

# Right-clicked again, out; then a sneak-punch picks the cart up, and the
# stick falls out.
interact(c, 3, at_object(cart))
watch(c, 2, until=lambda: attached(c.peer)[-1:] == [0])
assert attached(c.peer)[-1] == 0 and eyes[-1] == (0.0, 0.0, 0.0)
invs.clear(); removed.clear()
go(c, 10, 3, 12)
interact(c, 0, at_object(cart), keys=SNEAK)
watch(c, 2, until=lambda: cart in removed and invs)
assert cart in removed and invs and b"esp:cart" in invs[-1], invs[-1:]
assert attached(stick)[-1] == 0, "the stick stayed attached"
print("picked up; the stick fell out")
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("CARTS OK")
