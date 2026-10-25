"""Mobs over the network: python3 mobstest.py PORT netclient.py"""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "hunter.inv"), "w").write("kit 1\n0 esp:sword_steel 1\n")

def say(cl, text):
    u = text.encode("utf-16-be"); cl.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
def chat(m):
    o = 4; n = u16(m, o); o += 2 + 2 * n; n = u16(m, o); o += 2
    return m[o:o + 2 * n].decode("utf-16-be")
lines, sounds, mobs, gone, hp, pushes, items, punched = [], [], {}, [], [], [], [], []
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
            elif cmd == 0x31:
                removed, added = remove_add(m)
                for oid in removed:
                    mobs.pop(oid, None); gone.append(oid)
                o = 4 + 2 * len(removed) + 2
                for oid, name in added:
                    ln = struct.unpack_from(">I", m, o + 3)[0]; init = m[o + 7:o + 7 + ln]; o += 7 + ln
                    if name == "mobs:mossling":
                        assert 0x6000 <= oid < 0x8000, oid
                        nl = u16(init, 1)
                        mobs[oid] = struct.unpack_from(">3f", init, 3 + nl + 3)
                        assert b"mobs_mossling.png" in init and b"character.b3d" in init
                    elif name == "__builtin:item":
                        items.append(init)
            elif cmd == 0x32:
                o = 2
                while o < len(m):
                    i = u16(m, o); ln = u16(m, o + 2); msg = m[o + 4:o + 4 + ln]; o += 4 + ln
                    if i in mobs and msg[0] == 1:
                        mobs[i] = struct.unpack_from(">3f", msg, 1)
                    if i in mobs and msg[0] == 4:
                        punched.append((i, u16(msg, 1)))
            elif cmd == 0x33:
                hp.append(u16(m, 2))
            elif cmd == 0x2b:
                pushes.append(struct.unpack_from(">3f", m, 2))
        cl.cmds = []
        if until and until(): return True
    return False
def ask(cl, text, want):
    del lines[:]; say(cl, text); watch(cl, 3, until=lambda: any(want in l for l in lines))
    found = [l for l in lines if want in l]; assert found, (text, lines); return found[0]

c = Client("Hunter", port); c.login(); c.playerpos(); watch(c, 1.5)
print(ask(c, "/mobs", "Mobs are"))
assert "off" in ask(c, "/mobs spawn", "Mobs are off")
print(ask(c, "/mobs on", "now on"))
# A pack in front of the player (facing +z), at night.
def spawn():
    """A pack from /mobs spawn (others may spawn by themselves at night)."""
    before = set(mobs); print(ask(c, "/mobs spawn", "come")); watch(c, 0.5)
    near = lambda p: sum((p[k] - c.pos_bs[k]) ** 2 for k in range(3)) < 100 ** 2
    pack = [i for i in mobs if i not in before and near(mobs[i])]; assert len(pack) == 3, (pack, mobs)
    return pack
pack = spawn()
print("mosslings", [hex(i) for i in pack])
# They see the player, call, come and swipe.
watch(c, 12, until=lambda: hp and hp[-1] < 20)
print("sounds", sorted(set(sounds)), "HP", hp, "pushed", pushes[:1])
assert "mobs_mossling_call" in sounds and "mobs_mossling_swipe" in sounds
assert hp and hp[-1] < 20 and pushes
# The steel sword kills each in two blows (6 then 6 of 10 HP).
body = [v / 10 for v in c.pos_bs]
for oid in pack:
    for blow in range(4):
        if oid not in mobs:
            break
        c.punch(oid); watch(c, 0.9)
    assert oid not in mobs and oid in gone, f"{oid:#x} still there"
print("punched", punched)
assert any(h == 4 for _, h in punched) and any(h == 0 for _, h in punched)
assert "mobs_mossling_hurt" in sounds and "mobs_mossling_death" in sounds
assert sum(b"esp:stick" in m for m in items) == 3, "no sticks"
print("all three dead, each leaving sticks")
# Off, any about are gone at once.
spawn()
ask(c, "/mobs off", "now off"); watch(c, 0.5)
assert not mobs, mobs
# At dawn they sink into the ground and are gone. (A pause first: the
# chat flood limit.)
watch(c, 8)
ask(c, "/mobs on", "now on")
ask(c, "/time 6:00", "")
spawn()
del sounds[:]
watch(c, 15, until=lambda: not mobs)
assert not mobs and "default_dig_crumbly" in sounds, (mobs, sounds)
print("gone at dawn")
# Another player can't turn them off.
d = Client("Prey", port); d.login(); d.pump(0.5)
del lines[:]; say(d, "/mobs off"); end = time.time() + 3; got = []
while time.time() < end and not any("permission" in l for l in got):
    d.pump(0.05); got += [chat(m) for m in d.take(0x2F)]
assert any("missing privilege" in l for l in got), got
# Digging: the other player sees bits of the node fly (builtin's
# handle_node_drops); the digger's own client makes its own.
c.pos_bs = [70.0, 30.0, 100.0]; c.playerpos(); d.pos_bs = [80.0, 30.0, 100.0]; d.playerpos(); d.pump(0.3)
d.take(0x47); c.take(0x47)
pointed = struct.pack(">BB3h3h", 0, 1, 7, 2, 12, 7, 3, 12)
c.send(0, struct.pack(">HBH", 0x39, 2, 0) + struct.pack(">I", len(pointed)) + pointed
       + struct.pack(">3i3iiiIBB", 7000, 3000, 10000, 0, 0, 0, 3000, 0, 0, 72, 5))
end = time.time() + 2; seen = []
while time.time() < end and not seen:
    d.pump(0.05); seen = [spawner(m) for m in d.take(0x47)]
c.pump(0.3)
print("dig bits seen by the other player:", seen)
assert seen and seen[0]["node"] not in (0, 127) and not [m for m in c.take(0x47)]
d.raw(0, b"\x00\x03")
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("MOBS OK")
