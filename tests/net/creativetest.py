"""Creative mode and /mode over the network: python3 creativetest.py PORT netclient.py
The modes, the creative inventory (tabs, search, taking, the trash), the
creative hand, unlimited placing, no damage, flying allowed."""
import sys, time, struct, os
src = open(sys.argv[2]).read()
exec(src[:src.index("port = int(sys.argv[1])")])
port = int(sys.argv[1])
players = os.path.join(os.environ["TEST_WORLD"], "players")
os.makedirs(players, exist_ok=True)
open(os.path.join(players, "maker.inv"), "w").write("kit 1\n")

def say(cl, text):
    u = text.encode("utf-16-be"); cl.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
def chat(m):
    o = 4; n = u16(m, o); o += 2 + 2 * n; n = u16(m, o); o += 2
    return m[o:o + 2 * n].decode("utf-16-be")
def interact(cl, action, under, above, item=0):
    pointed = struct.pack(">BB3h3h", 0, 1, *under, *above)
    p = [int(v * 100) for v in cl.pos_bs]
    cl.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed
            + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 3000, 0, 0, 72, 5))
def formspec(m): return m[6:6 + struct.unpack_from(">I", m, 2)[0]].decode()
def send_fields(cl, pairs):
    body = struct.pack(">H", 0) + struct.pack(">H", len(pairs))
    for k, v in pairs:
        body += struct.pack(">H", len(k)) + k.encode() + struct.pack(">I", len(v)) + v.encode()
    cl.send(0, struct.pack(">H", 0x3c) + body)

lines, privs, invs, detached, forms, huds_gone, nodes, hp = [], [], [], {}, [], [], {}, []
def watch(cl, secs, until=None):
    end = time.time() + secs
    while time.time() < end:
        cl.pump(0.05)
        for m in cl.cmds:
            cmd = u16(m)
            if cmd == 0x2F:
                lines.append(chat(m))
            elif cmd == 0x41:
                n = u16(m, 2); o = 4; got = []
                for _ in range(n):
                    ln = u16(m, o); got.append(m[o + 2:o + 2 + ln].decode()); o += 2 + ln
                privs.append(got)
            elif cmd == 0x27:
                invs.append(m[6:].decode(errors="replace"))
            elif cmd == 0x43:
                ln = u16(m, 2); name = m[4:4 + ln].decode()
                detached[name] = m[4 + ln + 3:].decode(errors="replace")
            elif cmd == 0x42:
                forms.append(formspec(m))
            elif cmd == 0x4a:
                huds_gone.append(struct.unpack_from(">I", m, 2)[0])
            elif cmd == 0x21:
                nodes[struct.unpack_from(">hhh", m, 2)] = u16(m, 8)
            elif cmd == 0x22:
                nodes[struct.unpack_from(">hhh", m, 2)] = 126
            elif cmd == 0x33:
                hp.append(u16(m, 2))
        cl.cmds = []
        if until and until(): return True
    return False
def ask(cl, text, want):
    del lines[:]; say(cl, text); watch(cl, 3, until=lambda: any(want in l for l in lines))
    found = [l for l in lines if want in l]; assert found, (text, lines); return found[0]
def items_of(text):
    """The item names in a detached inventory's main list, in order."""
    return [l.split()[1] for l in text.splitlines() if l.startswith("Item ")]
def main_slots(text):
    """The player's main list: item name or None per slot."""
    body = text.split("List main ")[1].split("EndInventoryList")[0].splitlines()[1:]
    return [l.split()[1] if l.startswith("Item ") else None for l in body if l.startswith(("Item", "Empty"))]

c = Client("Maker", port); c.login(); c.playerpos(); watch(c, 1.5)
# (The test build's players are admins, who have fly and noclip anyway.)
own = set(privs[-1]) if privs else set()
print(ask(c, "/mode", "game mode is"))
assert "classic" in lines[-1]

# Creative: fly and noclip allowed, the creative hand, the health bar gone,
# the window on the creative inventory's All tab.
del forms[:], invs[:], privs[:]
print(ask(c, "/mode creative", "now creative"))
watch(c, 1, until=lambda: forms and invs and privs)
assert {"fly", "noclip"} <= set(privs[-1]), privs[-1]
assert "List hand" in invs[-1] and "esp:creative_hand" in invs[-1], invs[-1][-200:]
assert 0 in huds_gone, huds_gone
fs = forms[-1]
assert "Crafting,Recipes,All,Nodes,Tools,Items;3;" in fs and "list[detached:creative_Maker;main;0.5,0.5;8,4;0]" in fs, fs[:300]
all_items = items_of(detached["creative_Maker"])
assert "esp:stone" in all_items and "esp:pick_steel" in all_items and "esp:creative_hand" not in all_items
print("creative inventory:", len(all_items), "items")

# Taken from the creative list: one stone in the first slot, and the list
# keeps it.
i = all_items.index("esp:stone")
del invs[:]
c.inventory_action(f"Move 1 detached:creative_Maker main {i} current_player main 0")
watch(c, 1, until=lambda: invs)
assert main_slots(invs[-1])[0] == "esp:stone", main_slots(invs[-1])[:3]
assert "esp:stone" in items_of(detached["creative_Maker"])

# Placed, the stone isn't used up.
c.pos_bs = [80.0, 30.0, 100.0]; c.playerpos(); watch(c, 0.5)
del invs[:]
interact(c, 3, (8, 2, 12), (8, 3, 12))
watch(c, 1, until=lambda: (8, 3, 12) in nodes)
STONE = nodes[(8, 3, 12)]
watch(c, 0.5)
assert not invs or main_slots(invs[-1])[0] == "esp:stone", "the stone was used up"
print("placed stone, still have it")

# The creative hand digs stone at once (dig completed straight away is
# accepted), and the stone isn't given twice.
interact(c, 0, (8, 3, 12), (8, 4, 12))
interact(c, 2, (8, 3, 12), (8, 4, 12), item=1)
watch(c, 1, until=lambda: nodes.get((8, 3, 12)) == 126)
assert nodes.get((8, 3, 12)) == 126, "the creative hand didn't dig it"
watch(c, 0.5)
assert sum(1 for s in main_slots(invs[-1]) if s == "esp:stone") == 1, main_slots(invs[-1])
print("dug with the creative hand")

# Into the trash: gone.
del invs[:]
c.inventory_action("Move 0 current_player main 0 detached:trash main 0")
watch(c, 1, until=lambda: invs)
assert main_slots(invs[-1])[0] is None
print("trashed")

# Tabs and search: Tools, then "steel".
del forms[:]
send_fields(c, [("sfinv_nav_tabs", "5")])
watch(c, 1, until=lambda: forms)
tools = items_of(detached["creative_Maker"])
assert ";5;" in forms[-1] and "esp:pick_steel" in tools and "esp:stone" not in tools
del forms[:]
send_fields(c, [("creative_filter", "steel"), ("key_enter_field", "creative_filter")])
watch(c, 1, until=lambda: forms)
found = items_of(detached["creative_Maker"])
assert found and all("steel" in n for n in found), found
print("Tools tab, searched for steel:", found)

# No damage: falling, mobs, anything (/kill still kills).
del hp[:]
c.send(0, struct.pack(">HH", 0x35, 10))  # TOSERVER_DAMAGE: a fall
watch(c, 1)
assert not hp or hp[-1] == 20, hp

# Survival: mobs on, creative off: no fly, no hand, the health bar back.
del forms[:], invs[:], privs[:]
print(ask(c, "/mode survival", "now survival"))
watch(c, 1, until=lambda: forms and invs and privs)
assert set(privs[-1]) == own and "List hand" not in invs[-1], (privs[-1], own)
assert "Crafting,Recipes;1;" in forms[-1]
print(ask(c, "/mobs", "Mobs are"))
assert "Mobs are on" in lines[-1]
watch(c, 2)
print(ask(c, "/mode classic", "now classic"))
print(ask(c, "/mobs", "Mobs are"))
assert "Mobs are off" in lines[-1]
c.raw(0, b"\x00\x03"); time.sleep(0.3); print("CREATIVE OK")
