"""Parse the server's content packets the way the Luanti 5.17 client does."""
import hashlib, struct, sys, io, zstandard
from pathlib import Path

out = Path(sys.argv[1]); media_dir = Path(sys.argv[2])

class R:
    def __init__(s, b): s.b, s.p = b, 0
    def take(s, n):
        assert s.p + n <= len(s.b), f"read past end ({s.p}+{n}>{len(s.b)})"
        v = s.b[s.p:s.p+n]; s.p += n; return v
    def u8(s): return s.take(1)[0]
    def u16(s): return struct.unpack(">H", s.take(2))[0]
    def s16(s): return struct.unpack(">h", s.take(2))[0]
    def u32(s): return struct.unpack(">I", s.take(4))[0]
    def u64(s): return struct.unpack(">Q", s.take(8))[0]
    def f32(s): return struct.unpack(">f", s.take(4))[0]
    def str16(s): return s.take(s.u16())
    def str32(s): return s.take(s.u32())
    def can(s): return s.p < len(s.b)
    def done(s): assert s.p == len(s.b), f"{len(s.b)-s.p} trailing bytes"

def unzstd(b):
    return zstandard.ZstdDecompressor().decompressobj().decompress(b)

PROTO = 53
def image(r):
    name = r.str16(); anim = r.u8(); assert anim == 0; return name
def sound(r):
    n = r.str16(); g = r.f32(); r.f32(); r.f32(); return (n.decode(), round(g, 2))

# ---- ITEMDEF
p = R((out/"cmd_3d.bin").read_bytes()); assert p.u16() == 0x3d
raw = unzstd(p.str32()); p.done()
m = R(raw); assert m.u8() == 0
for _ in range(m.u16()):
    d = R(m.str16())
    assert d.u8() >= 6; t = d.u8(); name = d.str16(); desc = d.str16()
    inv = image(d); wield = image(d); scale = (d.f32(), d.f32(), d.f32())
    stack = d.s16(); d.u8(); d.u8(); tc = d.str16()
    if name == b"":
        assert tc, "hand has no ToolCapabilities (client aborts)"
    if tc:
        t2 = R(tc); assert t2.u8() >= 4; t2.f32(); t2.s16()
        caps = []
        for _ in range(t2.u32()):
            g = t2.str16().decode(); t2.s16(); t2.s16()
            caps.append((g, [(t2.s16(), round(t2.f32(), 2)) for _ in range(t2.u32())]))
        dmg = [(t2.str16().decode(), t2.s16()) for _ in range(t2.u32())]
        t2.u16(); t2.done(); print(f"    tool caps {caps} damage {dmg}")
    for _ in range(d.u16()): d.str16(); d.s16()
    pred = d.str16(); sound(d); sound(d); rng = d.f32(); d.str16(); color = d.u32()
    image(d); image(d)
    # optional sections, read only while bytes remain (as the client does)
    if d.can(): d.str16()
    if d.can(): sound(d); sound(d)
    if d.can():
        if d.u8(): d.u8()
    # >= 5.9: wallmounted_rotate_vertical, touch_interaction,
    # pointabilities, wear bar (only the bug net sends these)
    if d.can():
        d.u8(); assert all(d.u8() <= 2 for _ in range(3))
        pt = d.str16()
        if pt:
            q = R(pt); assert q.u8() == 0
            maps = [[(q.str16().decode(), q.u8()) for _ in range(q.u32())] for _ in range(4)]
            q.done(); print(f"    pointabilities {maps}")
            assert maps[1] == [("catchable", 1)], maps
        assert d.u8() == 0
    assert not d.can(), "unexpected section"
    print(f"  item type={t} name={name!r} wield={wield!r} scale={scale} stack={stack} range={rng} color={color:08x} place={pred!r}")
assert m.u16() == 0; m.done()
print("ITEMDEF OK")

# ---- NODEDEF
p = R((out/"cmd_3a.bin").read_bytes()); assert p.u16() == 0x3a
raw = unzstd(p.str32()); p.done()
m = R(raw); assert m.u8() >= 1; count = m.u16(); lst = R(m.str32()); m.done()
def tile(r):
    assert r.u8() >= 6; name = r.str16(); at = r.u8()
    anim = None
    if at == 1: anim = (r.u16(), r.u16(), round(r.f32(), 2))   # TAT_VERTICAL_FRAMES
    else: assert at == 0
    flags = r.u16()
    if flags & 8: r.take(3)
    if flags & 16: r.u8()
    if flags & 32: r.u8()
    return name, flags, anim
def in_bs(b):
    # Boxes go in BS units (10 to a node): one in node units is tiny.
    # (the smallest real box: an insect's, 0.1 of a node either way)
    assert max(abs(v) for v in b) >= 1, ("box in node units, not BS?", b)
def nodebox(r):
    assert r.u8() == 6
    t = r.u8()
    if t == 2:  # NODEBOX_WALLMOUNTED: top, bottom, side boxes (v3f32 min, max each)
        boxes = [[r.f32() for _ in range(6)] for _ in range(3)]
        for b in boxes: assert all(b[i] < b[i + 3] for i in range(3)), b; in_bs(b)
        return boxes
    if t == 1:  # NODEBOX_FIXED: u16 count, then boxes
        boxes = [[r.f32() for _ in range(6)] for _ in range(r.u16())]
        for b in boxes: assert all(b[i] < b[i + 3] for i in range(3)), b; in_bs(b)
        return boxes
    if t == 4:  # NODEBOX_CONNECTED: 15 lists
        lists = [[[r.f32() for _ in range(6)] for _ in range(r.u16())] for _ in range(15)]
        for l in lists:
            for b in l: assert all(b[i] < b[i + 3] for i in range(3)), b
        return ("connected", lists)
    assert t == 0
for _ in range(count):
    cid = lst.u16(); f = R(lst.str16())
    assert f.u8() >= 13; name = f.str16()
    groups = [(f.str16(), f.s16()) for _ in range(f.u16())]
    pt, pt2, dt = f.u8(), f.u8(), f.u8(); mesh = f.str16().decode(); vs = f.f32()
    assert f.u8() == 6; tiles = [tile(f) for _ in range(6)]; [tile(f) for _ in range(6)]
    assert f.u8() == 6; [tile(f) for _ in range(6)]
    alpha_legacy = f.u8(); f.take(3); f.str16(); f.u8(); csides = f.u8()
    cto = [f.u16() for _ in range(f.u16())]
    f.u32(); f.u8()
    lprop = f.u8(); sunprop = f.u8(); light = f.u8(); f.u8()
    walk = f.u8(); point = f.u8(); dig = f.u8(); f.u8(); f.u8(); f.u8(); f.u32()
    f.u8(); f.str16(); f.str16(); f.take(5)
    nbox = nodebox(f); selbox = nodebox(f); cbox = nodebox(f)
    snd = [sound(f) for _ in range(3)]; f.u8(); f.u8()
    for nm, _ in snd:
        if nm and nm != "__group":
            assert any(m.startswith(nm + ".") for m in (p.name for p in media_dir.iterdir())), f"missing sound {nm}"
    print(f"    sounds: footstep {snd[0]}, dig {snd[1]}, dug {snd[2]}")
    pred = f.str16()
    if f.can(): f.u8()
    if f.can(): alpha = f.u8()
    if f.can(): f.u8(); f.u8()
    if f.can(): f.u8()
    f.done()
    print(f"  node id={cid} {name.decode()} groups={[(g.decode(), v) for g, v in groups]} drawtype={dt} tiles={[t[0].decode() for t in tiles]} alpha={alpha} walkable={walk}")
    if name.startswith(b"esp:torch"):
        print(f"    torch: mesh={mesh} paramtype={pt} paramtype2={pt2} light={lprop},{sunprop},{light} tile={tiles[0]} selection={[[round(v, 3) for v in b] for b in selbox]}")
        assert (pt, pt2, dt, lprop, sunprop, light, walk, alpha) == (1, 4, 16, 1, 1, 12, 0, 1)
        assert selbox is not None and tiles[0][1] & 1 == 0 and tiles[0][2] == (16, 16, 3.3)
        assert mesh == {b"esp:torch": "esp_torch_floor.obj", b"esp:torch_wall": "esp_torch_wall.obj", b"esp:torch_ceiling": "esp_torch_ceiling.obj"}[name]
        assert (media_dir/mesh).exists() and (media_dir/tiles[0][0].decode()).exists()
        torches = globals().setdefault("torches", []); torches.append(name)
    if name.startswith((b"esp:stair_", b"esp:slab_")):
        n = 3 if name.startswith(b"esp:stair_inner_") else 2 if name.startswith(b"esp:stair_") else 1
        assert (pt, pt2, dt, lprop, sunprop, walk) == (1, 3, 12, 1, 0, 1), (pt, pt2, dt, lprop, sunprop, walk)
        assert len(nbox) == n and selbox == nbox and cbox is None, (nbox, selbox, cbox)
        assert all(t[1] & 1 for t in tiles) and (media_dir/tiles[0][0].decode()).exists()
        aligned = all(t[1] & 32 for t in tiles)
        assert aligned == (b"wood" not in name and b"brick" not in name), name
        assert not any(g in (b"wood", b"stone") for g, _ in groups)
        shaped = globals().setdefault("shaped", []); shaped.append(name)
    ids = globals().setdefault("ids", {}); ids[name.decode()] = (cid, groups)
    if name.startswith(b"esp:fence_"):
        print(f"    fence: connects_to={cto} nodebox={nbox[0]} lists={[len(l) for l in nbox[1]]}")
        assert (pt, pt2, dt, lprop, sunprop, walk, csides) == (1, 0, 12, 1, 1, 1, 0)
        assert nbox[0] == "connected" and selbox == nbox and cbox[0] == "connected"
        rail = name.startswith(b"esp:fence_rail_")
        assert [len(l) for l in nbox[1]] == [2 if rail else 1, 0, 0, 2, 2, 2, 2] + [0] * 8 and [len(l) for l in cbox[1]] == [1, 0, 0, 1, 1, 1, 1] + [0] * 8
        assert cto == sorted(cto) and cid in cto
        fences = globals().setdefault("fences", []); fences.append((cid, cto, rail))
    if name.startswith(b"esp:gate_"):
        print(f"    gate: mesh={mesh} connect_sides={csides} selection={selbox} collision={cbox}")
        assert (pt, pt2, dt, lprop, sunprop, walk, csides, cto) == (1, 3, 16, 1, 1, 1, 40, [])
        assert (media_dir/mesh).exists() and mesh == ("doors_fencegate_open.obj" if name.endswith(b"_open") else "doors_fencegate_closed.obj")
        assert all(t[1] & 1 for t in tiles) and len(cbox) == (2 if name.endswith(b"_open") else 1)
        gates = globals().setdefault("gates", []); gates.append(cid)
lst.done(); assert len(torches) == 3 and len(shaped) == 136; 
assert len(fences) == 10 and len(gates) == 10
want = sorted(cid for nm, (cid, gr) in ids.items() if any(g in (b"fence", b"wood", b"tree", b"wall") for g, _ in gr))
rail_want = sorted(cid for nm, (cid, gr) in ids.items() if any(g in (b"fence", b"wall") for g, _ in gr))
assert all(cto == (rail_want if rail else want) for _, cto, rail in fences) and all(g in want for g in gates), (want, fences)
print("rails join:", [nm for nm, (cid, _) in ids.items() if cid in rail_want])
print("fences join:", [nm for nm, (cid, _) in ids.items() if cid in want])
print("NODEDEF OK")

# ---- ANNOUNCE_MEDIA
p = R((out/"cmd_3c.bin").read_bytes()); assert p.u16() == 0x3c
a = R(unzstd(p.str32())); n = a.u32(); lens = [a.u16() for _ in range(n)]
names = [a.take(l).decode() for l in lens]; a.done()
hashes = {nm: p.take(20) for nm in names}; remote = p.str16(); p.done()
for nm in names:
    assert hashes[nm] == hashlib.sha1((media_dir/nm).read_bytes()).digest(), nm
print(f"ANNOUNCE_MEDIA OK: {names}, SHA-1s match, remote={remote!r}")

# ---- MEDIA
got = {}
for f in sorted(out.glob("media_*.bin")):
    p = R(f.read_bytes()); assert p.u16() == 0x38
    nb, bi, nf = p.u16(), p.u16(), p.u32()
    for _ in range(nf):
        nm = p.str16().decode(); data = unzstd(p.str32()); got[nm] = data
    p.done(); print(f"  bunch {bi}/{nb}: {nf} files")
for nm in names: assert got[nm] == (media_dir/nm).read_bytes(), nm
print("MEDIA OK: all files arrive intact")

# ---- small packets
mv = R((out/"cmd_45.bin").read_bytes()); assert mv.u16() == 0x45; print("MOVEMENT", [round(mv.f32(), 2) for _ in range(12)]); mv.done()
t = R((out/"cmd_29.bin").read_bytes()); assert t.u16() == 0x29; print("TIME_OF_DAY", t.u16(), t.f32()); t.done()
c = R((out/"cmd_2a.bin").read_bytes()); assert c.u16() == 0x2a; print("CSM", c.u64(), c.u32()); c.done()
print("ALL CONTENT VERIFIED")
