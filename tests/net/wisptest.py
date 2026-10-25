"""Wisps over the network: python3 wisptest.py PORT netclient.py"""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "burner.inv"), "w").write("kit 1\n0 esp:sword_steel 1\n")

def say(cl, text):
    u = text.encode("utf-16-be"); cl.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
def chat(m):
    o = 4; n = u16(m, o); o += 2 + 2 * n; n = u16(m, o); o += 2
    return m[o:o + 2 * n].decode("utf-16-be")
lines, sounds, stopped, wisps, gone, hp, blocks, items, sprites, parts, deleted = [], [], [], {}, [], [], [], [], [], [], []
where = {}
def watch(cl, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.cmds:
            cmd = u16(m)
            if cmd == 0x2F:
                lines.append(chat(m))
            elif cmd == 0x3f:
                sid = struct.unpack_from(">i", m, 2)[0]; n = u16(m, 6); name = m[8:8 + n].decode()
                o = 8 + n + 4; kind = m[o]; obj = u16(m, o + 13); loop = m[o + 15]
                sounds.append((name, sid, kind, obj, loop))
            elif cmd == 0x40:
                stopped.append(struct.unpack_from(">i", m, 2)[0])
            elif cmd == 0x47:
                parts.append(spawner(m))
            elif cmd == 0x53:
                deleted.append(struct.unpack_from(">I", m, 2)[0])
            elif cmd == 0x31:
                removed, added = remove_add(m)
                for oid in removed:
                    if oid in wisps:
                        wisps.pop(oid); gone.append(oid)
                o = 4 + 2 * len(removed) + 2
                for oid, name in added:
                    ln = struct.unpack_from(">I", m, o + 3)[0]; init = m[o + 7:o + 7 + ln]; o += 7 + ln
                    if name == "mobs:wisp":
                        assert b"mobs_wisp.png" in init and b"sprite" in init, init
                        wisps[oid] = True
                    elif name == "__builtin:item":
                        items.append(init)
            elif cmd == 0x32:
                o = 2
                while o < len(m):
                    i = u16(m, o); ln = u16(m, o + 2); msg = m[o + 4:o + 4 + ln]; o += 4 + ln
                    if i in wisps and msg[0] == 3:
                        sprites.append(struct.unpack_from(">hhHf", msg, 1))
                    if i in wisps and msg[0] == 1:
                        mv = [v / 10 for v in struct.unpack_from(">6f", msg, 1)]
                        where[i] = (mv[:3], mv[3:], time.time())
            elif cmd == 0x33:
                hp.append(u16(m, 2))
            elif cmd == 0x20:
                blocks.append(struct.unpack_from(">hhh", m, 2))
        cl.cmds = []
        if until and until(): return True
    return False
def ask(cl, text, want):
    del lines[:]; say(cl, text); watch(cl, 3, until=lambda: any(want in l for l in lines))
    found = [l for l in lines if want in l]; assert found, (text, lines); return found[0]
def names(): return [s[0] for s in sounds]
def revive(cl):
    """A burst hurts as a creeper's does, and can kill: if it did, respawn."""
    if hp and hp[-1] == 0:
        cl.send(0, struct.pack(">H", 0x38)); mv = cl.wait_for(0x34)
        cl.pos_bs = struct.unpack_from(">fff", mv, 2); cl.playerpos(); watch(cl, 1)
        # Their bones have the sword: another, in the first slot.
        ask(cl, "/giveme esp:sword_steel", "added")
        print("respawned")

c = Client("Burner", port); c.login(); c.playerpos(); watch(c, 3)
ask(c, "/mobs on", "now on")
# A wisp: a glowing sprite, humming (a loop on the object).
print(ask(c, "/mobs spawn wisp", "A wisp comes"))
watch(c, 0.5)
assert len(wisps) == 1, wisps
wisp = next(iter(wisps))
hum = [s for s in sounds if s[0] == "mobs_wisp_hum"]
assert hum and hum[0][2] == 2 and hum[0][3] == wisp and hum[0][4] == 1, hum
# It hangs back, rushes, lights its fuse (a whine, fast frames) and bursts.
del hp[:], blocks[:]
watch(c, 20, until=lambda: wisp in gone)
print("sounds", names(), "HP", hp, "sprites", sprites)
assert "mobs_wisp_whine" in names() and "tnt_explode" in names(), names()
assert (0, 4, 4, 0.05000000074505806) in [(a, b, n, f) for a, b, n, f in sprites] or any(s[1] == 4 for s in sprites)
assert wisp in gone and hum[0][1] in stopped, (gone, stopped)
# A creeper's damage: about 17 HP at 3 nodes, more nearer.
assert hp and hp[-1] <= 20 - 10, hp
assert blocks, "the burst changed no nodes"
# Sparks from it while it whined, and the burst's flash and smoke.
assert any(p["texture"] == "mobs_wisp_spark.png" and p["attached"] == wisp for p in parts), parts
assert {"tnt_boom.png", "tnt_smoke.png"} <= {p["texture"] for p in parts}
print("it burst: HP", hp[-1], "and", len(blocks), "blocks sent again")
# With TNT off, the burst hurts but breaks nothing.
revive(c)
watch(c, 6)
ask(c, "/tnt off", "now off")
before = hp[-1]
ask(c, "/mobs spawn wisp", "A wisp comes")
wisp = max(wisps)
del blocks[:]
watch(c, 20, until=lambda: wisp in gone)
assert wisp in gone and not blocks, blocks
print("with TNT off: no nodes broken; HP", hp[-1] if hp else before)
revive(c)
# Hit with a steel sword (6 HP; it has 20, as a creeper: four blows), a
# wisp goes out and drops a coal lump.
watch(c, 6)
ask(c, "/tnt on", "now on")
ask(c, "/mobs spawn wisp", "A wisp comes")
wisp = max(wisps)
del items[:]
# Each blow knocks it back, and it keeps its distance: the player follows
# it, striking from 3.5 nodes (outside its reach, so it doesn't light).
for _ in range(12):
    if wisp in gone:
        break
    if wisp in where:
        # Where it is now, from where it was and how it was moving.
        (p0, v0, t0) = where[wisp]
        w = [p0[i] + v0[i] * (time.time() - t0) for i in range(3)]
        c.pos_bs = [(w[0] - 3.5) * 10, (w[1] - 1.0) * 10, w[2] * 10]; c.playerpos(); watch(c, 0.05)
    c.punch(wisp); watch(c, 0.85)
assert wisp in gone and any(b"esp:coal_lump" in i for i in items), (gone, len(items))
assert "mobs_wisp_fizzle" in names()
print("punched out, leaving coal")
# Mobs turned off: a wisp about is gone, and its hum stops.
watch(c, 6)
ask(c, "/mobs spawn wisp", "A wisp comes")
watch(c, 0.5)
wisp = max(wisps)
hums = [sid for name, sid, kind, obj, loop in sounds if name == "mobs_wisp_hum" and obj == wisp]
assert hums, sounds[-3:]
ask(c, "/mobs off", "now off"); watch(c, 0.5)
assert wisp in gone and hums[0] in stopped, (gone, hums, stopped[-3:])
print("mobs off: the wisp gone, its hum stopped")
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("WISP OK")
