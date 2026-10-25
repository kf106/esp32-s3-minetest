"""Boats over the network: python3 boatstest.py PORT netclient.py X,Y,Z (open water).
A boat placed on the water, boarded, steered, cruised, seen by a second
player, left and picked up again."""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
water = tuple(int(v) for v in sys.argv[3].split(","))
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "sailor.inv"), "w").write("kit 1\n0 esp:boat 2\n")

def chat(m):
    o = 4; n = u16(m, o); o += 2 + 2 * n; n = u16(m, o); o += 2
    return m[o:o + 2 * n].decode("utf-16-be")
def pos_packet(cl, keys=0, yaw=0.0):
    p = [int(v * 100) for v in cl.pos_bs]
    return struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 0, int(yaw * 100), keys, 72, 5)
def interact(cl, action, pointed, keys=0):
    cl.send(0, struct.pack(">HBH", 0x39, action, 0) + struct.pack(">I", len(pointed)) + pointed + pos_packet(cl, keys))
def at_node(under, above):
    return struct.pack(">BB3h3h", 0, 1, *under, *above)
def at_object(oid):
    return struct.pack(">BBH", 0, 2, oid)
def drive(cl, keys, secs):
    end = time.time() + secs
    while time.time() < end:
        cl.send(0, struct.pack(">H", 0x23) + pos_packet(cl, keys)); watch(cl, 0.1)

lines, added, removed, msgs, invs, moves = [], {}, [], [], [], []
def watch(cl, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.cmds:
            cmd = u16(m)
            if cmd == 0x2F:
                lines.append(chat(m))
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
            elif cmd == 0x34:
                moves.append(struct.unpack_from(">5f", m, 2))
        cl.cmds = []
        if until and until(): return True
    return False
def attached(oid):
    """The parents oid was attached to, in order (0: let go)."""
    return [struct.unpack_from(">h", m, 1)[0] for i, m in msgs if i == oid and m[0] == 8]
def boat_moves(oid):
    return [struct.unpack_from(">3f3f", m, 1) for i, m in msgs if i == oid and m[0] == 1]

c = Client("Sailor", port); c.login(); c.playerpos(); watch(c, 0.5)
x, y, z = water
c.pos_bs = [x * 10.0, (y + 1) * 10.0, (z - 3) * 10.0]
want = tuple(v // 16 for v in water)
end = time.time() + 20
while time.time() < end and not any(struct.unpack_from(">hhh", m, 2) == want for m in c.take(0x20)):
    c.playerpos(); c.pump(0.2)
watch(c, 0.5)

# Pointed at dry ground, nothing; at the water, a boat floats on it.
interact(c, 3, at_node((x - 3, y - 1, z - 3), (x - 3, y, z - 3))); watch(c, 1)
assert not any(n == "boats:boat" for n, _ in added.values()), "a boat on land"
invs.clear()
interact(c, 3, at_node(water, (x, y + 1, z)))
watch(c, 3, until=lambda: any(n == "boats:boat" for n, _ in added.values()))
boat = next(oid for oid, (n, _) in added.items() if n == "boats:boat")
init = added[boat][1]
assert b"boats_boat.obj" in init and b"esp_apple_wood.png" in init
start = struct.unpack_from(">3f", init, 3 + u16(init, 1) + 3)
print("boat", boat, "at", [round(v / 10, 2) for v in start])
assert abs(start[1] / 10 - (y + 0.5)) < 0.01
watch(c, 0.5, until=lambda: invs)
assert invs and b"esp:boat" in invs[-1] and b"esp:boat 2" not in invs[-1], "the boat wasn't taken from the hand"

# Right-clicked, the player sits in it.
interact(c, 3, at_object(boat))
watch(c, 2, until=lambda: attached(c.peer))
assert attached(c.peer) == [boat], attached(c.peer)
sit = [struct.unpack_from(">2f", m, 1) for i, m in msgs if i == c.peer and m[0] == 6]
assert sit and sit[-1] == (81.0, 160.0), sit
print("sitting in boat", boat)

# Forward: it speeds off the way it faces (+z, as the player looked).
drive(c, 1, 3.0)
mv = boat_moves(boat)
assert mv, "no boat updates"
pz, vz = mv[-1][2] / 10, mv[-1][5] / 10
print(f"after 3 s forward: z {pz:.1f}, going {vz:.1f} n/s, {len(mv)} updates")
assert pz > z + 2 and vz > 2.5
# Forward and back together: cruising; back alone stops it.
drive(c, 3, 0.3)
drive(c, 0, 1.0)
assert any("cruise mode on" in l for l in lines), lines
drive(c, 2, 0.3)
assert any("cruise mode off" in l for l in lines), lines

# Someone joining sees the boat, and the sailor in it.
d = Client("Watcher", port); d.login(); d.playerpos()
seen = {}
end = time.time() + 5
while time.time() < end and c.peer not in seen:
    d.pump(0.1)
    for m in d.take(0x31):
        rem, add = remove_add(m)
        o = 4 + 2 * len(rem) + 2
        for oid, name in add:
            ln = struct.unpack_from(">I", m, o + 3)[0]; seen[oid] = (name, m[o + 7:o + 7 + ln]); o += 7 + ln
    watch(c, 0.05)
assert seen.get(boat, ("",))[0] == "boats:boat", seen.keys()
assert struct.pack(">BhH", 8, boat, 0) in seen[c.peer][1], "not shown in the boat"
order = list(seen)
assert order.index(boat) < order.index(c.peer)
print("Watcher sees the boat, and the sailor in it")
d.raw(0, b"\x00\x03")

# Right-clicked again, they get out, a little above it.
moves.clear()
interact(c, 3, at_object(boat))
watch(c, 2, until=lambda: attached(c.peer)[-1:] == [0] and moves)
assert attached(c.peer)[-1] == 0 and moves, (attached(c.peer), moves)
print("out at", [round(v / 10, 2) for v in moves[-1][:3]])
c.pos_bs = list(moves[-1][:3])

# Punched, it goes back into the inventory.
invs.clear(); removed.clear()
interact(c, 0, at_object(boat))
watch(c, 2, until=lambda: boat in removed and invs)
assert boat in removed and b"esp:boat 2" in invs[-1], invs[-1:]
print("picked up again")
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("BOATS OK")
