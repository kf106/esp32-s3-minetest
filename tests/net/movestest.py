"""Players' positions passed on: python3 movestest.py PORT netclient.py
Two players move; a third gets both positions, together in one packet
(sent every server step, as Luanti gathers its objects' messages), and
never its own."""
import sys, time, struct
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])

added = {}
def watch(cls, secs, log):
    end = time.time() + secs
    while time.time() < end:
        for cl in cls:
            cl.pump(0.02)
            for m in cl.cmds:
                if u16(m) == 0x31:
                    for oid, name in remove_add(m)[1]:
                        added[oid] = name
                elif u16(m) == 0x32:
                    ids = []
                    o = 2
                    while o < len(m):
                        i = u16(m, o); ln = u16(m, o + 2)
                        if m[o + 4] == 1:
                            ids.append(i)
                        o += 4 + ln
                    log.setdefault(cl.name, []).append(ids)
            cl.cmds = []

a, b, c = Client("Watcher", port), Client("Walker", port), Client("Runner", port)
for cl in (a, b, c):
    cl.login(); cl.playerpos()
log = {}
watch([a, b, c], 2, log)
log.clear()
# Walker and Runner move together a few times.
for step in range(5):
    for cl, dx in ((b, 5.0), (c, -5.0)):
        cl.pos_bs = [cl.pos_bs[0] + dx, cl.pos_bs[1], cl.pos_bs[2]]
        cl.playerpos()
    watch([a, b, c], 0.2, log)
watch([a, b, c], 0.5, log)
moves = [ids for ids in log.get("Watcher", []) if ids]
print("Watcher's position packets:", moves)
both = [ids for ids in moves if len(set(ids)) == 2]
assert both, "the two positions never came together"
others = {oid for oid, n in added.items()}
# Nobody hears their own position.
for name, cl in (("Walker", b), ("Runner", c)):
    assert all(cl.peer not in ids for ids in log.get(name, [])), (name, log.get(name))
print("MOVES OK")
