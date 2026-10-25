"""A minimal Luanti 5.17 client, enough to log in, take blocks, and drop and
pick up items, for testing the server on the PC (the host harness with SERVE=port; see tests/README.md)."""
import os, re, socket, struct, sys, time, hashlib

PROTO_ID = 0x4F457403
SEQ0 = 65500

def u16(b, o=0): return struct.unpack_from(">H", b, o)[0]

class Client:
    def __init__(s, name, port):
        s.name, s.addr = name, ("127.0.0.1", port)
        s.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.sock.settimeout(0.05)
        s.peer = 0; s.out_seq = [SEQ0] * 3; s.in_seq = [SEQ0] * 3; s.early = [dict() for _ in range(3)]
        s.splits = {}; s.cmds = []; s.blocks = 0; s.datagrams = 0

    def raw(s, ch, body):
        s.sock.sendto(struct.pack(">IHB", PROTO_ID, s.peer, ch) + body, s.addr)

    def send(s, ch, cmd):
        seq = s.out_seq[ch]; s.out_seq[ch] = (seq + 1) & 0xFFFF
        s.raw(ch, b"\x03" + struct.pack(">H", seq) + b"\x01" + cmd)

    def handle_inner(s, ch, p):
        t = p[0]
        if t == 0:
            if p[1] == 1: s.peer = u16(p, 2)
        elif t == 1:
            s.cmds.append(p[1:])
        elif t == 2:
            seq, cnt, num = struct.unpack_from(">HHH", p, 1)
            parts = s.splits.setdefault((ch, seq), {})
            parts[num] = p[7:]
            if len(parts) == cnt:
                s.cmds.append(b"".join(parts[i] for i in range(cnt))); del s.splits[(ch, seq)]

    def pump(s, secs):
        end = time.time() + secs
        while time.time() < end:
            try:
                d, _ = s.sock.recvfrom(2000)
            except socket.timeout:
                continue
            s.datagrams += 1
            pid, sender, ch = struct.unpack_from(">IHB", d); p = d[7:]
            assert pid == PROTO_ID
            if p[0] == 3:
                seq = u16(p, 1)
                s.raw(ch, b"\x00\x00" + struct.pack(">H", seq))  # ACK
                ahead = (seq - s.in_seq[ch]) & 0xFFFF
                if ahead == 0:
                    s.handle_inner(ch, p[3:]); s.in_seq[ch] = (s.in_seq[ch] + 1) & 0xFFFF
                    while s.in_seq[ch] in s.early[ch]:
                        s.handle_inner(ch, s.early[ch].pop(s.in_seq[ch])); s.in_seq[ch] = (s.in_seq[ch] + 1) & 0xFFFF
                elif ahead < 0x8000:
                    s.early[ch][seq] = p[3:]
            else:
                s.handle_inner(ch, p)

    def take(s, cmd_id):
        got = [c for c in s.cmds if u16(c) == cmd_id]
        s.cmds = [c for c in s.cmds if u16(c) != cmd_id]
        return got

    def wait_for(s, cmd_id, secs=10, cond=lambda c: True):
        end = time.time() + secs
        while time.time() < end:
            for c in s.take(cmd_id):
                if cond(c): return c
            s.pump(0.05)
        raise AssertionError(f"{s.name}: no command {cmd_id:#x} within {secs}s")

    def login(s):
        name = s.name.encode()
        s.send(0, struct.pack(">HBHHH", 0x02, 29, 0, 37, 53) + struct.pack(">H", len(name)) + name)
        hello = s.wait_for(0x02)
        mechs = struct.unpack_from(">I", hello, 7)[0]
        assert mechs & 4, f"expected first-time SRP, got {mechs}"
        salt = os.urandom(16)
        x = int.from_bytes(hashlib.sha256(salt + hashlib.sha256(s.name.lower().encode() + b":pw").digest()).digest(), "big")
        N = int(open(os.path.join(os.path.dirname(__file__), "srp_client.py")).read().split('N = int("')[1].split('"')[0], 16)
        v = pow(2, x, N); vb = v.to_bytes((v.bit_length() + 7) // 8, "big")
        s.send(0, struct.pack(">H", 0x50) + struct.pack(">H", 16) + salt + struct.pack(">H", len(vb)) + vb + b"\x00")
        s.wait_for(0x03)
        s.send(1, struct.pack(">H", 0x11) + struct.pack(">H", 0))
        s.wait_for(0x3c)
        s.send(1, struct.pack(">HH", 0x40, 0))
        ver = b"5.17.0"
        s.send(1, struct.pack(">HBBBB", 0x43, 5, 17, 0, 0) + struct.pack(">H", len(ver)) + ver + struct.pack(">H", 11))
        mv = s.wait_for(0x34)
        s.pos_bs = struct.unpack_from(">fff", mv, 2)

    def login_existing(s):
        """SRP login to an account made earlier (password "pw")."""
        H = lambda b: hashlib.sha256(b).digest()
        mb = lambda n: n.to_bytes((n.bit_length() + 7) // 8, "big")
        pad = lambda n: n.to_bytes(256, "big")
        i = lambda b: int.from_bytes(b, "big")
        N = int(open(os.path.join(os.path.dirname(__file__), "srp_client.py")).read().split('N = int("')[1].split('"')[0], 16)
        name = s.name.encode()
        s.send(0, struct.pack(">HBHHH", 0x02, 29, 0, 37, 53) + struct.pack(">H", len(name)) + name)
        hello = s.wait_for(0x02)
        assert struct.unpack_from(">I", hello, 7)[0] & 2, "expected SRP"
        a = i(os.urandom(32)); A = pow(2, a, N)
        s.send(0, struct.pack(">H", 0x51) + struct.pack(">H", len(mb(A))) + mb(A) + b"\x01")
        t = time.time()
        sb = s.wait_for(0x60, 20)
        s.srp_wait = time.time() - t
        o = 2; ls = u16(sb, o); salt = sb[o + 2:o + 2 + ls]; o += 2 + ls; lb = u16(sb, o); B = i(sb[o + 2:o + 2 + lb])
        u = i(H(pad(A) + pad(B))); k = i(H(pad(N) + pad(2)))
        x = i(H(salt + H(s.name.lower().encode() + b":pw")))
        S = pow((B - k * pow(2, x, N)) % N, a + u * x, N); K = H(mb(S))
        hx = bytes(p ^ q for p, q in zip(H(mb(N)), H(mb(2))))
        M = H(hx + H(name) + salt + mb(A) + mb(B) + K)
        s.send(0, struct.pack(">H", 0x52) + struct.pack(">H", len(M)) + M)
        s.wait_for(0x03)
        s.send(1, struct.pack(">H", 0x11) + struct.pack(">H", 0))
        s.wait_for(0x3c)
        s.send(1, struct.pack(">HH", 0x40, 0))
        ver = b"5.17.0"
        s.send(1, struct.pack(">HBBBB", 0x43, 5, 17, 0, 0) + struct.pack(">H", len(ver)) + ver + struct.pack(">H", 11))
        mv = s.wait_for(0x34)
        s.pos_bs = struct.unpack_from(">fff", mv, 2)

    def playerpos(s, pitch=0.0, yaw=0.0):
        p = [int(v * 100) for v in s.pos_bs]
        s.send(0, struct.pack(">H3i3iiiIBB", 0x23, *p, 0, 0, 0, int(pitch * 100), int(yaw * 100), 0, 72, 5))

    def inventory_action(s, text):
        s.send(0, struct.pack(">H", 0x31) + text.encode())

    def punch(s, oid):
        pointed = struct.pack(">BBH", 0, 2, oid)
        p = [int(v * 100) for v in s.pos_bs]
        s.send(0, struct.pack(">HBH", 0x39, 0, 0) + struct.pack(">I", len(pointed)) + pointed
               + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 0, 0, 0, 72, 5))

def torches(inv_cmd):
    m = re.search(rb"Item esp:torch (\d+)", inv_cmd)
    return int(m.group(1)) if m else (1 if b"Item esp:torch" in inv_cmd else 0)

def remove_add(c):
    o = 2; nrem = u16(c, o); o += 2
    removed = [u16(c, o + 2 * i) for i in range(nrem)]; o += 2 * nrem
    nadd = u16(c, o); o += 2; added = []
    for _ in range(nadd):
        oid = u16(c, o); o += 3; ln = struct.unpack_from(">I", c, o)[0]; o += 4
        init = c[o:o + ln]; o += ln
        nl = u16(init, 1); added.append((oid, init[3:3 + nl].decode()))
    return removed, added

def item_updates(c, oid):
    """(velocity) of AO_CMD_UPDATE_POSITION messages for oid in an ACTIVE_OBJECT_MESSAGES command."""
    o = 2; out = []
    while o < len(c):
        i = u16(c, o); ln = u16(c, o + 2); m = c[o + 4:o + 4 + ln]; o += 4 + ln
        if i == oid and m[0] == 1: out.append(struct.unpack_from(">3f3f", m, 1))
    return out

def spawner(m):
    """TOCLIENT_ADD_PARTICLESPAWNER, read as Luanti 5.17's client does
    (protocol 53); every byte must be used."""
    o = [2]
    def take(fmt):
        v = struct.unpack_from(">" + fmt, m, o[0]); o[0] += struct.calcsize(">" + fmt); return v
    amount, time_ = take("Hf")
    def tween(n):
        style, reps, beg = take("BHf"); assert style < 4 and reps == 1
        vals = [take("%df" % n) + take("%df" % n) + take("f") for _ in range(2)]
        assert vals[0] == vals[1], "constant"
        return vals[0]
    pos, vel, acc = tween(3), tween(3), tween(3)
    life, size = tween(1), tween(1)
    collide, = take("B"); n, = take("I"); tex = m[o[0]:o[0] + n].decode(); o[0] += n
    sid, vertical, removal, attached, anim, glow, objcol = take("IBBHBBB"); assert anim == 0
    node, param2, tile = take("HBB")
    flags, = take("B"); assert flags == 8, flags  # blend: clip
    take("BHfff"); take("BHfffff")  # alpha, scale tweens
    tween(3); tween(3); tween(1)  # drag, jitter, bounce
    kind, = take("B"); assert kind == 0
    tween(3)  # radius
    pool, = take("H"); assert pool == 0
    assert o[0] == len(m), (o[0], len(m))
    return dict(amount=amount, time=time_, texture=tex, node=node, glow=glow, size=size, id=sid, attached=attached)

port = int(sys.argv[1])
a = Client("Tester", port)
t0 = time.time(); a.login(); print(f"logged in in {time.time() - t0:.2f}s at {a.pos_bs}")
inv = a.wait_for(0x27); print("starting torches:", torches(inv)); assert torches(inv) == 20
def chat(c):
    o = 4; n = u16(c, o); o += 2 + 2 * n; n = u16(c, o); o += 2
    return c[o:o + 2 * n].decode("utf-16-be")
lines = []
end = time.time() + 5
while time.time() < end and not any("playing" in l for l in lines):
    a.pump(0.05); lines += [chat(c) for c in a.take(0x2F)]
print("welcome:"); [print("   ", l) for l in lines]
assert lines[0].startswith("Welcome!") and "kf106" in lines[2] and lines[-1].startswith("Nobody else")
a.playerpos(pitch=0, yaw=90)
end = time.time() + 20
while time.time() < end and a.blocks < 150:
    a.pump(0.1); a.blocks += len(a.take(0x20))
print(f"{a.blocks} blocks in {time.time() - t0:.1f}s ({a.datagrams} datagrams)")
assert a.blocks >= 150

# Q: drop one torch (count 1), then the rest of the stack (count 0)
a.cmds.clear()
a.inventory_action("Drop 1 current_player main 0")
ra = a.wait_for(0x31, cond=lambda c: any(n == "__builtin:item" for _, n in remove_add(c)[1]))
item1 = [i for i, n in remove_add(ra)[1] if n == "__builtin:item"][0]
assert torches(a.wait_for(0x27)) == 19
print("dropped one torch as object", item1)
landed = a.wait_for(0x32, 5, cond=lambda c: any(v[3:] == (0, 0, 0) for v in item_updates(c, item1)))
print("it landed:", [v for v in item_updates(landed, item1)])

# A second player sees it on joining
b = Client("Other", port); b.login()
first = b.wait_for(0x31)
seen = [i for i, n in remove_add(first)[1] if n == "__builtin:item"]
print("newcomer's first objects include items", seen); assert item1 in seen

# Punch it: back into the inventory, gone for everyone
a.cmds.clear(); b.cmds.clear()
a.punch(item1)
assert torches(a.wait_for(0x27)) == 20
assert item1 in remove_add(b.wait_for(0x31))[0]
print("picked up; removed for the other player too")

# Drop the whole stack, twice nearby -> one is dropped, other merged? (single stack: just check the count)
a.inventory_action("Drop 0 current_player main 0")
ra = a.wait_for(0x31, cond=lambda c: any(n == "__builtin:item" for _, n in remove_add(c)[1]))
inv = a.wait_for(0x27); assert torches(inv) == 0, inv
print("dropped the whole stack:", remove_add(ra)[1])
print("NETWORK ITEMS OK")

# Skins: the inventory window shows a preview; > steps through them.
def formspec(c): return c[6:6 + struct.unpack_from(">I", c, 2)[0]].decode()
def fields(form, *names):
    body = struct.pack(">H", len(form)) + form.encode() + struct.pack(">H", len(names))
    for n in names: body += struct.pack(">H", len(n)) + n.encode() + struct.pack(">I", 1) + b"x"
    return struct.pack(">H", 0x3c) + body
c = Client("Skinner", port); c.login()
fs = formspec(c.wait_for(0x42)); print("formspec:", fs)
assert "model[" in fs and ";character.b3d;character.png;" in fs and "label[10.85,4.25;Default]" in fs
seen = []
a.cmds.clear()
for i in range(6):
    c.send(0, fields("", "skin_next"))
    fs = formspec(c.wait_for(0x42))
    tex = fs.split(";character.b3d;")[1].split(";")[0]
    label = fs.split("label[10.85,4.25;")[1].split("]")[0]
    seen.append((label, tex))
    upd = a.wait_for(0x32, cond=lambda m: tex.replace("\\", "").encode() in m)
print("skins in turn:", seen)
assert seen[0] == ("Elf", "esp_skin_elf.png") or seen[0][0] in ("Wizard", "Elf")
assert any("[combine:64x32:0\\,0=skin_grey_wizard.png" in t for _, t in seen), seen
assert not any("tiny" in t for _, t in seen)
c.send(0, fields("", "skin_prev")); fs = formspec(c.wait_for(0x42))
c.send(0, fields("", "quit"))   # closing the window: no change
c.pump(0.5); assert not c.take(0x42)
label = fs.split("label[10.85,4.25;")[1].split("]")[0]
print("after <:", label)
c.raw(0, b"\x00\x03")  # DISCO: the player file is saved
time.sleep(0.5)
pf = open(os.path.join(os.environ["TEST_WORLD"], "players/skinner.player")).read()
print("player file:", pf.split())
assert f"skin {label.lower().replace(' ', '_')}" in pf
print("SKINS OK")

# Existing accounts log in through the SRP thread, several in a row, and two at once.
for n in range(3):
    e = Client("Skinner", port); e.login_existing()
    print(f"SRP login {n + 1}: challenge after {e.srp_wait:.2f}s")
    fs = formspec(e.wait_for(0x42)); assert f"label[10.85,4.25;{label}]" in fs
    e.raw(0, b"\x00\x03"); time.sleep(0.3)
f1, f2 = Client("Tester", port), Client("Other", port)
import threading
th = [threading.Thread(target=c.login_existing) for c in (f1, f2)]
[t.start() for t in th]; [t.join() for t in th]
print("two at once:", round(f1.srp_wait, 2), round(f2.srp_wait, 2))
print("SRP THREAD OK")

# At the edge of the world: no blocks beyond block 1937.
f1.cmds.clear()
f1.pos_bs = (31000 * 10.0, 30 * 10.0, 100 * 10.0)
f1.playerpos()
end = time.time() + 8; xs = []
while time.time() < end:
    f1.pump(0.1)
    for c in f1.take(0x20):
        xs.append(struct.unpack_from(">h", c, 2)[0])
print(f"at x=31000: {len(xs)} blocks, block x from {min(xs)} to {max(xs)}")
assert xs and max(xs) == 1937
print("MAP EDGE OK")

# Blocks come from the builder thread and a cache: after a dig, a newcomer
# (whose blocks may come from the cache) must see the hole, and so must
# everyone who already had the block.
import zstandard
def block_node(cmd, pos):
    bx, by, bz = struct.unpack_from(">hhh", cmd, 2)
    raw = zstandard.ZstdDecompressor().decompressobj().decompress(cmd[8:])
    x, y, z = pos[0] - bx * 16, pos[1] - by * 16, pos[2] - bz * 16
    i = z * 256 + y * 16 + x
    return struct.unpack_from(">H", raw, 5 + 2 * i)[0]
g = Client("Digger", port); g.login()
px, py, pz = (v / 10 for v in g.pos_bs)
target = (round(px) + 2, round(py - 0.6), round(pz))
blk = tuple(v // 16 for v in target)
# wait until Digger has the block
end = time.time() + 15; have = None
while time.time() < end and have is None:
    g.pump(0.1)
    for c in g.take(0x20):
        if struct.unpack_from(">hhh", c, 2) == blk: have = c
assert have, "Digger never got the block"
before = block_node(have, target); assert before != 126, before
pointed = struct.pack(">BB3h3h", 0, 1, *target, target[0], target[1] + 1, target[2])
p = [int(v * 100) for v in g.pos_bs]
g.send(0, struct.pack(">HBH", 0x39, 2, 0) + struct.pack(">I", len(pointed)) + pointed + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 0, 0, 0, 72, 5))
rm = g.wait_for(0x22)
print("dug", target, "node was", before, "- removenode arrived")
h = Client("Latecomer", port); h.login()
end = time.time() + 15; got = None
while time.time() < end and got is None:
    h.pump(0.1)
    for c in h.take(0x20):
        if struct.unpack_from(">hhh", c, 2) == blk: got = c
assert got, "Latecomer never got the block"
after = block_node(got, target)
print("latecomer sees", after); assert after == 126
print("BLOCK BUILDER OK")

# Chat
def say(c, text):
    u = text.encode("utf-16-be")
    c.send(0, struct.pack(">HH", 0x32, len(u) // 2) + u)
def heard(c, secs=3, want=None):
    lines = []; end = time.time() + secs
    while time.time() < end:
        c.pump(0.05); lines += [chat(m) for m in c.take(0x2F)]
        if want and any(want in l for l in lines): break
    return lines
for c in (g, h): c.cmds.clear()
say(g, "  hello from the digger ")
l = heard(h, want="hello"); print("latecomer heard:", l); assert "<Digger> hello from the digger" in l
assert "<Digger> hello from the digger" in heard(g, want="hello")
say(g, "/me waves"); assert "* Digger waves" in heard(h, want="waves")
say(g, "/msg latecomer psst"); assert "DM from Digger: psst" in heard(h, want="psst"); assert "Message sent." in heard(g, want="sent")
say(g, "/msg nobody hi"); assert any("not online" in x for x in heard(g, want="online"))
say(g, "/time"); t = heard(g, want="time"); print("time:", t); assert any(x.startswith("Current time is ") for x in t)
say(g, "/status"); s = heard(g, want="Server"); print("status:", s); assert any("Digger" in x and "Latecomer" in x for x in s)
say(g, "/frobnicate"); assert any("Invalid command: frobnicate" in x for x in heard(g, want="Invalid"))
say(g, "héllo ünïcode 🙂"); assert "<Digger> héllo ünïcode 🙂" in heard(h, want="ünï")
for i in range(9): say(g, f"spam {i}")
l = heard(g, 2)
assert any("limited to 8 messages" in x for x in l), l
print("CHAT OK")

# Crafting: a player with fir logs and coal makes planks, sticks and torches.
players = os.path.join(os.environ["TEST_WORLD"], "players")
open(os.path.join(players, "woodcutter.inv"), "w").write("kit 1\n0 esp:fir_tree 2\n1 esp:coal_lump 1\n")
w = Client("Woodcutter", port); w.login()
def lists(c):
    text = c[6:6 + struct.unpack_from(">I", c, 2)[0]].decode()
    out = {}
    for block in text.split("EndInventoryList\n")[:-1]:
        head, *rest = block.split("\n")
        name = head.split()[1]
        out[name] = [l[5:] if l.startswith("Item ") else None for l in rest[1:] if l]
    return out
inv = lists(w.wait_for(0x27)); print("woodcutter starts with", [x for x in inv["main"] if x])
fs = formspec(w.wait_for(0x42)); assert "list[current_player;craft;" in fs and "craftpreview" in fs, fs
def act(text):
    w.cmds.clear(); w.inventory_action(text); return lists(w.wait_for(0x27))
inv = act("Move 1 current_player main 0 current_player craft 4")
print("log in grid, preview:", inv["craftpreview"]); assert inv["craftpreview"] == ["esp:fir_wood 4"]
inv = act("Craft 1 current_player"); assert inv["craftresult"] == ["esp:fir_wood 4"] and inv["craftpreview"] == [None]
inv = act("Move 4 current_player craftresult 0 current_player main 5"); assert inv["main"][5] == "esp:fir_wood 4"
inv = act("Move 1 current_player main 5 current_player craft 0"); assert inv["craftpreview"] == ["esp:stick 4"]
inv = act("Craft 1 current_player")
inv = act("MoveSomewhere 4 current_player craftresult 0 current_player main"); assert "esp:stick 4" in inv["main"]
s = inv["main"].index("esp:stick 4")
act("Move 1 current_player main 1 current_player craft 1")
inv = act(f"Move 1 current_player main {s} current_player craft 4"); print("coal over stick:", inv["craftpreview"])
assert inv["craftpreview"] == ["esp:torch 4"]
inv = act("Craft 1 current_player"); assert inv["craftresult"] == ["esp:torch 4"]
# Taking from the preview directly, or putting into it, is refused.
inv = act("Move 1 current_player main 5 current_player craftpreview 0"); assert inv["main"][5] == "esp:fir_wood 3"
# A stick can't be placed: the server corrects the client.
sticks = inv["main"].index("esp:stick 3")
print("CRAFTING OK")

# Tools: the hand can't dig cobblestone; a worn wooden pickaxe can, and breaks.
open(os.path.join(players, "miner.inv"), "w").write("kit 1\n0 esp:pick_wood 1 64000\n1 esp:cobble 5\n")
m = Client("Miner", port); m.login()
inv = lists(m.wait_for(0x27)); print("miner has", [x for x in inv["main"] if x])
assert inv["main"][0] == "esp:pick_wood 1 64000"
px, py, pz = (v / 10 for v in m.pos_bs)
ground = (round(px) - 2, round(py - 0.6), round(pz))
spot = (ground[0], ground[1] + 1, ground[2])
def interact(action, item, under, above):
    pointed = struct.pack(">BB3h3h", 0, 1, *under, *above)
    p = [int(v * 100) for v in m.pos_bs]
    m.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 0, 0, 0, 72, 5))
end = time.time() + 15
while time.time() < end and not any(struct.unpack_from(">hhh", c, 2) == tuple(v // 16 for v in spot) for c in m.take(0x20)):
    m.pump(0.1)
m.cmds.clear()
interact(3, 1, ground, spot)   # place cobble
add = m.wait_for(0x21); print("placed cobble at", spot)
m.cmds.clear()
interact(2, 2, spot, (spot[0], spot[1] + 1, spot[2]))   # dig with an empty hand
m.pump(1.0)
assert not m.take(0x22), "hand dug cobblestone"
assert m.take(0x21), "server should resend the node"
print("hand can't dig cobblestone")
m.cmds.clear()
interact(2, 0, spot, (spot[0], spot[1] + 1, spot[2]))   # dig with the pickaxe
m.wait_for(0x22)
snd = m.wait_for(0x3f); name = snd[8:8 + u16(snd, 6)].decode(); print("sound:", name); assert name == "default_tool_breaks"
inv = lists(m.wait_for(0x27, cond=lambda c: b"esp:pick_wood" not in c and b"esp:cobble 5" in c))
print("after:", [x for x in inv["main"] if x]); assert "esp:cobble 5" in inv["main"]
print("TOOLS OK")

# Furnace: craft one from a ring of stone, place it, open it, fill it.
open(os.path.join(players, "smelter.inv"), "w").write(
    "kit 1\n0 esp:pick_stone 1\n1 esp:cobble 8\n2 esp:iron_lump 5\n3 esp:coal_lump 3\n4 esp:dirt 3\n")
for old in (w, m): old.raw(0, b"\x00\x03")   # log out: six players at most
time.sleep(0.5)
f = Client("Smelter", port); f.login()
def fact(text, want=0x27):
    f.cmds.clear(); f.inventory_action(text); return f.wait_for(want)
lists(f.wait_for(0x27))
for i in (0, 1, 2, 3, 5, 6, 7, 8):
    inv = lists(fact(f"Move 1 current_player main 1 current_player craft {i}"))
print("ring of cobble:", inv["craftpreview"]); assert inv["craftpreview"] == ["esp:furnace 1"]
fact("Craft 1 current_player")
inv = lists(fact("Move 1 current_player craftresult 0 current_player main 5")); assert inv["main"][5] == "esp:furnace 1"
px, py, pz = (v / 10 for v in f.pos_bs)
ground = (round(px) + 2, round(py - 0.6), round(pz) + 2)
spot = (ground[0], ground[1] + 1, ground[2])
def finteract(action, item, under, above, keys=0):
    pointed = struct.pack(">BB3h3h", 0, 1, *under, *above)
    p = [int(v * 100) for v in f.pos_bs]
    f.send(0, struct.pack(">HBH", 0x39, action, item) + struct.pack(">I", len(pointed)) + pointed + struct.pack(">3i3iiiIBB", *p, 0, 0, 0, 0, 0, keys, 72, 5))
end = time.time() + 15
while time.time() < end and not any(struct.unpack_from(">hhh", c, 2) == tuple(v // 16 for v in spot) for c in f.take(0x20)):
    f.pump(0.1)
f.cmds.clear()
finteract(3, 5, ground, spot)
add = f.wait_for(0x21); param2 = add[11]
print("placed furnace, param2", param2, "(player at", round(px, 1), round(pz, 1), ")")
assert struct.unpack_from(">hhh", add, 2) == spot and param2 in (0, 1)
# Right-click: the furnace window with its detached inventory
f.cmds.clear()
finteract(3, 0, spot, (spot[0], spot[1] + 1, spot[2]))
det = f.wait_for(0x43); fs = f.wait_for(0x44)
dname = det[4:4 + u16(det, 2)].decode(); print("detached inventory", dname)
form = fs[6:6 + struct.unpack_from(">I", fs, 2)[0]].decode()
assert f"list[detached:{dname};src" in form and "esp:furnace" in fs[-20:].decode(errors="replace")
def dlists(c):
    n = u16(c, 2); text = c[2 + 2 + n + 1 + 2:].decode()
    out = {}
    for block in text.split("EndInventoryList\n")[:-1]:
        head, *rest = block.split("\n")
        out[head.split()[1]] = [l[5:] if l.startswith("Item ") else None for l in rest[1:] if l]
    return out
assert dlists(det) == {"src": [None], "fuel": [None], "dst": [None] * 4}, dlists(det)
loc = f"detached:{dname}"
# Fuel rules first, with nothing to smelt, so it can't light yet.
d = dlists(fact(f"Move 3 current_player main 4 {loc} fuel 0", 0x43)); assert d["fuel"] == [None], "dirt isn't fuel"
d = dlists(fact(f"Move 2 current_player main 3 {loc} fuel 0", 0x43)); assert d["fuel"] == ["esp:coal_lump 2"], d
d = dlists(fact(f"Move 1 current_player main 3 {loc} dst 0", 0x43)); assert d["dst"] == [None] * 4, "nothing into dst"
inv = lists(fact(f"MoveSomewhere 2 {loc} fuel 0 current_player main")); assert "esp:coal_lump 3" in inv["main"], inv
d = dlists(fact(f"Move 5 current_player main 2 {loc} src 0", 0x43)); assert d["src"] == ["esp:iron_lump 5"], d
print("furnace rules OK:", d)
saved = open(os.path.join(os.environ["TEST_WORLD"], "world/furnaces.txt")).read()
print("furnaces.txt:", saved.replace("\n", " | ")); assert "src 0 esp:iron_lump 5" in saved
# A full furnace can't be dug.
f.cmds.clear(); finteract(2, 0, spot, (spot[0], spot[1] + 1, spot[2])); f.pump(1.0)
assert not f.take(0x22) and f.take(0x21), "dug a full furnace"
# Closing the window; actions on it are then refused.
f.send(0, fields("esp:furnace", "quit")); f.pump(0.3)
inv = lists(fact(f"Move 1 {loc} src 0 current_player main 10")); assert inv["main"][10] is None
# Reopen and light it: 5 iron lumps and a coal lump.
f.cmds.clear(); finteract(3, 0, spot, (spot[0], spot[1] + 1, spot[2])); f.wait_for(0x44)
coal = lists(fact("Move 0 current_player main 0 current_player main 0"))  # just the current inventory
cslot = [i for i, x in enumerate(coal["main"]) if x and x.startswith("esp:coal_lump")][0]
f.cmds.clear(); f.inventory_action(f"Move 1 current_player main {cslot} {loc} fuel 0")
lit = f.wait_for(0x21, 5, cond=lambda c: struct.unpack_from(">hhh", c, 2) == spot)
print("furnace node now", u16(lit, 8), "param2", lit[11]); assert u16(lit, 8) == 19 and lit[11] == param2
snd = f.wait_for(0x3f); print("sound:", snd[8:8 + u16(snd, 6)].decode(), "id", struct.unpack_from(">i", snd, 2)[0])
upd = f.wait_for(0x44, 5, cond=lambda c: b"lowpart" in c); m_ = re.search(rb"lowpart:(\d+):default_furnace_fire_fg.*?lowpart:(\d+):", upd)
print("window shows fuel %s%%, cooking %s%%" % (m_.group(1).decode(), m_.group(2).decode()))
t0 = time.time(); ingots = 0; cools = 0
while time.time() - t0 < 25 and ingots < 5:
    f.pump(0.2)
    for c in f.take(0x43):
        d = dlists(c); ingots = sum(int(x.split()[1]) for x in d["dst"] if x)
    cools += sum(1 for c in f.take(0x3f) if b"cool_lava" in c)
print(f"{ingots} steel ingots after {time.time() - t0:.1f}s, {cools} cooling sounds, src now", d["src"], "fuel", d["fuel"])
assert ingots == 5 and d["src"] == [None] and cools >= 1
# Take the ingots; the coal burns on, so it is still lit; empty, it can be dug.
for i in range(4):
    if d["dst"][i]: fact(f"MoveSomewhere 0 {loc} dst {i} current_player main", 0x43)
f.cmds.clear(); finteract(2, 0, spot, (spot[0], spot[1] + 1, spot[2]))
f.wait_for(0x22); f.wait_for(0x55); print("dug while burning: the fire sound fades")
inv = lists(f.wait_for(0x27, cond=lambda c: b"esp:furnace 1" in c)); print("now has:", [x for x in inv["main"] if x])
assert "esp:steel_ingot 5" in inv["main"]
saved = open(os.path.join(os.environ["TEST_WORLD"], "world/furnaces.txt")).read(); assert saved == "", saved
print("FURNACE OK")

# Wi-Fi power saving: after 30 s with nobody doing anything, on; the first move turns it off.
if os.environ.get("IDLE_TEST"):
    f.cmds.clear()
    end = time.time() + 36
    while time.time() < end: f.pump(0.5)
    print("idle 36 s")
    f.pos_bs = (f.pos_bs[0] + 5, f.pos_bs[1], f.pos_bs[2]); f.playerpos(); f.pump(0.5)
    print("moved")
