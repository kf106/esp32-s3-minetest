#!/usr/bin/env python3
"""Generate mob textures for esp32-s3-minetest: mossling, hexer (64x32 character.b3d skins)
and wisp (animated sprite strip), plus a preview sheet; and the hexer's orb.

    python3 tools/make_mob_skins.py media      (needs Pillow, in a venv)

By Keir Finlow-Bates; the textures are CC BY-SA 3.0 (media/LICENSE-mobs.txt).
The hexer's face: eyes burning red, red-rimmed, with dark veins spreading
from them over a grey face, as if it had drained the life from the land."""
import math, os, sys
from PIL import Image, ImageDraw

OUT = sys.argv[1] if len(sys.argv) > 1 else "out"
os.makedirs(OUT, exist_ok=True)

def h(u, v, s=0):
    n = (u * 374761393 + v * 668265263 + s * 2147483647) & 0xFFFFFFFF
    n = ((n ^ (n >> 13)) * 1274126177) & 0xFFFFFFFF
    return ((n ^ (n >> 16)) & 0xFFFF) / 65535.0

def jit(c, u, v, s, amt=8):
    d = int((h(u, v, s) - 0.5) * 2 * amt)
    return tuple(max(0, min(255, x + d)) for x in c[:3]) + ((c[3],) if len(c) > 3 else (255,))

# --- 64x32 classic layout -----------------------------------------------------
HEAD = dict(top=(8,0,8,8), bottom=(16,0,8,8), right=(0,8,8,8), front=(8,8,8,8), left=(16,8,8,8), back=(24,8,8,8))
BODY = dict(top=(20,16,8,4), bottom=(28,16,8,4), right=(16,20,4,12), front=(20,20,8,12), left=(28,20,4,12), back=(32,20,8,12))
ARM  = dict(top=(44,16,4,4), bottom=(48,16,4,4), right=(40,20,4,12), front=(44,20,4,12), left=(48,20,4,12), back=(52,20,4,12))
LEG  = dict(top=(4,16,4,4), bottom=(8,16,4,4), right=(0,20,4,12), front=(4,20,4,12), left=(8,20,4,12), back=(12,20,4,12))

def paint(img, rect, fn):
    x, y, w, hh = rect
    for v in range(hh):
        for u in range(w):
            img.putpixel((x + u, y + v), fn(u, v, w, hh))

def ascii_face(img, rect, rows, pal, seed, amt=6):
    x, y, w, hh = rect
    assert len(rows) == hh and all(len(r) == w for r in rows), rows
    for v, row in enumerate(rows):
        for u, ch in enumerate(row):
            img.putpixel((x + u, y + v), jit(pal[ch], u + x, v + y, seed, amt if ch.islower() or ch in "BMHR" else 3))

# ============================ MOSSLING =========================================
MP = {
    'B': (52, 38, 26), 'b': (80, 59, 38), 'c': (108, 82, 52),
    'M': (48, 82, 32), 'm': (74, 114, 42), 'n': (110, 148, 58),
    'E': (214, 255, 96), 'e': (128, 176, 52),
    'T': (222, 212, 172), 'K': (22, 15, 10),
    'R': (38, 28, 19), 'C': (150, 132, 96), 'L': (156, 166, 124),
}

def bark(u, v, s):
    g = h(u, 0, s)           # vertical grain per column
    k = h(u, v, s + 1)
    if g < 0.25 or k < 0.08: base = MP['B']
    elif g > 0.8 or k > 0.93: base = MP['c']
    else: base = MP['b']
    return jit(base, u, v, s + 2, 6)

def moss(u, v, s):
    k = h(u, v, s)
    base = MP['M'] if k < 0.3 else MP['n'] if k > 0.82 else MP['m']
    return jit(base, u, v, s + 3, 7)

def moss_cover(depth_rows):
    """moss on top rows with a ragged lower edge, plus occasional lichen"""
    def fn(u, v, w, hh, s):
        edge = depth_rows + (1 if h(u, 7, s) > 0.55 else 0) - (1 if h(u, 9, s) > 0.8 else 0)
        if v < edge: return moss(u, v, s)
        if h(u, v, s + 5) > 0.95: return jit(MP['L'], u, v, s, 5)
        return bark(u, v, s)
    return fn

def mossling():
    img = Image.new("RGBA", (64, 32), (0, 0, 0, 0))
    ascii_face(img, HEAD['front'], [
        "nmMnmnMm",
        "MmbcbbmM",
        "bcBcbBcb",
        "beEbbEeb",
        "bBbccbBb",
        "cKTKKTKc",
        "bKKTTKKb",
        "BbbcbcbB",
    ], MP, 11)
    s = 20
    paint(img, HEAD['top'], lambda u, v, w, hh: moss(u, v, s) if h(u, v, s+9) > 0.12 else jit(MP['n'], u, v, s, 4))
    paint(img, HEAD['bottom'], lambda u, v, w, hh: bark(u, v, s + 1))
    for name, sd in (('left', 30), ('right', 31)):
        paint(img, HEAD[name], lambda u, v, w, hh, sd=sd: moss_cover(3)(u, v, w, hh, sd))
    # pointed ears suggested on the sides
    for name, ux in (('left', 2), ('right', 4)):
        x, y, _, _ = HEAD[name]
        for (du, dv, ch) in ((0, 3, 'c'), (1, 3, 'c'), (0, 4, 'c'), (1, 4, 'b'), (1, 2, 'c')):
            img.putpixel((x + ux + du, y + dv), MP[ch] + (255,))
    paint(img, HEAD['back'], lambda u, v, w, hh: moss_cover(5)(u, v, w, hh, 33))

    # body: moss mantle over shoulders, bark belly with lichen
    def body_fn(sd, rows=4):
        return lambda u, v, w, hh: moss_cover(rows)(u, v, w, hh, sd)
    paint(img, BODY['front'], body_fn(40, 3))
    paint(img, BODY['back'], body_fn(41, 6))
    paint(img, BODY['left'], body_fn(42, 4))
    paint(img, BODY['right'], body_fn(43, 4))
    paint(img, BODY['top'], lambda u, v, w, hh: moss(u, v, 44))
    paint(img, BODY['bottom'], lambda u, v, w, hh: bark(u, v, 45))
    # a hanging vine and a knot on the chest
    x, y, _, _ = BODY['front']
    for dv in range(3, 9):
        img.putpixel((x + 6, y + dv), jit(MP['M'] if dv % 2 else MP['m'], 6, dv, 46))
    img.putpixel((x + 2, y + 6), MP['B'] + (255,)); img.putpixel((x + 3, y + 6), MP['B'] + (255,))
    img.putpixel((x + 2, y + 7), MP['c'] + (255,))

    # arms: mossy shoulders, root hands with pale claws
    def arm_fn(face, sd):
        def fn(u, v, w, hh):
            if v >= 9:
                if v == 11 and face in ('front', 'right', 'left') and u % 2 == 0: return jit(MP['C'], u, v, sd, 4)
                return jit(MP['R'], u, v, sd, 5)
            return moss_cover(3)(u, v, w, hh, sd)
        return fn
    for face, sd in (('front', 50), ('back', 51), ('left', 52), ('right', 53)):
        paint(img, ARM[face], arm_fn(face, sd))
    paint(img, ARM['top'], lambda u, v, w, hh: moss(u, v, 54))
    paint(img, ARM['bottom'], lambda u, v, w, hh: jit(MP['C'], u, v, 55, 4) if (u + v) % 2 else jit(MP['R'], u, v, 55))

    # legs: bark with root feet
    def leg_fn(face, sd):
        def fn(u, v, w, hh):
            if v >= 10:
                if v == 11 and face == 'front': return jit(MP['C'] if u % 2 else MP['R'], u, v, sd, 4)
                return jit(MP['R'], u, v, sd, 5)
            if v < 2 and h(u, v, sd) > 0.4: return moss(u, v, sd)
            return bark(u, v, sd)
        return fn
    for face, sd in (('front', 60), ('back', 61), ('left', 62), ('right', 63)):
        paint(img, LEG[face], leg_fn(face, sd))
    paint(img, LEG['top'], lambda u, v, w, hh: bark(u, v, 64))
    paint(img, LEG['bottom'], lambda u, v, w, hh: jit(MP['R'], u, v, 65))
    return img

# ============================= HEXER ===========================================
XP = {
    'H': (22, 15, 32), 'h': (40, 27, 56), 'i': (60, 41, 82), 'j': (86, 62, 112),
    'F': (158, 166, 146), 'f': (110, 118, 102), 'g': (70, 76, 66), 'G': (18, 16, 20),
    'V': (236, 150, 255), 'v': (140, 66, 190),
    'E': (255, 72, 48), 'o': (128, 22, 30), 'x': (58, 20, 40),
    'S': (196, 40, 52), 's': (112, 26, 38),
    'k': (30, 24, 22), 'y': (160, 118, 62),
    'P': (194, 188, 162), 'p': (136, 130, 112),
    'r': (30, 21, 42), 'R': (50, 35, 68),
}

def robe(u, v, s, light=0.0):
    g = h(u, 0, s)
    if g < 0.3: base = XP['H']
    elif g > 0.75: base = XP['i']
    else: base = XP['h']
    if h(u, v, s + 1) > 0.94: base = XP['j']
    return jit(base, u, v, s + 2, 5)

def hexer():
    img = Image.new("RGBA", (64, 32), (0, 0, 0, 0))
    ascii_face(img, HEAD['front'], [
        "jhihhihj",
        "hHHHHHHh",
        "hHoGGoHh",
        "hoEggEoh",
        "hxofFoxh",
        "hgxfgxgh",
        "hHgxGgHh",
        "hhHfgHhh",
    ], XP, 71, 4)
    paint(img, HEAD['top'], lambda u, v, w, hh: jit(XP['i'] if (u == 3 or u == 4) and v < 3 else XP['h'], u, v, 72, 5))
    paint(img, HEAD['bottom'], lambda u, v, w, hh: jit(XP['G'] if 1 <= u <= 6 and v >= 1 else XP['H'], u, v, 73, 3))
    for name, sd in (('left', 74), ('right', 75)):
        paint(img, HEAD[name], lambda u, v, w, hh, sd=sd: robe(u, v, sd) if u > 0 else jit(XP['j'], u, v, sd, 4))
    def hood_back(u, v, w, hh):
        if u in (3, 4) and v < 6: return jit(XP['H'], u, v, 76, 3)   # seam
        if v >= 6 and (u + v) % 3 == 0: return jit(XP['j'], u, v, 76, 4)
        return robe(u, v, 77)
    paint(img, HEAD['back'], hood_back)

    ascii_face(img, BODY['front'], [
        "hjhhhhjh",
        "jhihhihj",
        "hRrrrrRh",
        "hRrsSrRh",
        "hrSrrSrh",
        "hRrSSrRh",
        "hRrsrrRh",
        "kkkyykkk",
        "hRrrHrRh",
        "hRrrHrRh",
        "hRrrHrRh",
        "hrRrHRrh",
    ], XP, 80, 5)
    paint(img, BODY['back'], lambda u, v, w, hh: jit(XP['j'], u, v, 81, 4) if v < 2 and u in (2, 5) else (jit(XP['k'], u, v, 81, 3) if v == 7 else robe(u, v, 82)))
    for name, sd in (('left', 83), ('right', 84)):
        paint(img, BODY[name], lambda u, v, w, hh, sd=sd: jit(XP['k'], u, v, sd, 3) if v == 7 else robe(u, v, sd))
    paint(img, BODY['top'], lambda u, v, w, hh: jit(XP['h'], u, v, 85, 5))
    paint(img, BODY['bottom'], lambda u, v, w, hh: jit(XP['H'], u, v, 86, 3))

    # sleeves with tattered cuffs and bony hands
    def arm_fn(face, sd):
        def fn(u, v, w, hh):
            if v >= 10:
                if face == 'front' and v == 11: return jit(XP['P'] if u % 2 == 0 else XP['G'], u, v, sd, 3)
                return jit(XP['P'] if (u + v) % 3 else XP['p'], u, v, sd, 4)
            if v == 9:
                return jit(XP['H'] if h(u, v, sd) > 0.5 else XP['P'], u, v, sd, 3)
            if v == 8: return jit(XP['j'] if u % 2 else XP['i'], u, v, sd, 4)
            return robe(u, v, sd)
        return fn
    for face, sd in (('front', 90), ('back', 91), ('left', 92), ('right', 93)):
        paint(img, ARM[face], arm_fn(face, sd))
    paint(img, ARM['top'], lambda u, v, w, hh: jit(XP['i'], u, v, 94, 5))
    paint(img, ARM['bottom'], lambda u, v, w, hh: jit(XP['p'], u, v, 95, 4))

    # robe skirt over the legs with a ragged hem
    def leg_fn(face, sd):
        def fn(u, v, w, hh):
            hem = 9 + (1 if h(u, 3, sd) > 0.5 else 0)
            if v > hem: return jit(XP['G'], u, v, sd, 3)
            if v == hem: return jit(XP['j'], u, v, sd, 4)
            if face == 'front' and u == 3: return jit(XP['H'], u, v, sd, 3)
            return robe(u, v, sd)
        return fn
    for face, sd in (('front', 100), ('back', 101), ('left', 102), ('right', 103)):
        paint(img, LEG[face], leg_fn(face, sd))
    paint(img, LEG['top'], lambda u, v, w, hh: jit(XP['h'], u, v, 104))
    paint(img, LEG['bottom'], lambda u, v, w, hh: jit(XP['G'], u, v, 105, 3))
    return img

# ============================== WISP ===========================================
def wisp(frames_idle=4, frames_charge=4, size=16):
    total = frames_idle + frames_charge
    img = Image.new("RGBA", (size, size * total), (0, 0, 0, 0))
    for f in range(total):
        charging = f >= frames_idle
        ph = f % 4
        scale = (1.0 + 0.06 * math.sin(ph * math.pi / 2)) if not charging else (1.12 if ph % 2 == 0 else 0.94)
        cx, cy = 7.5, 9.0
        for y in range(size):
            for x in range(size):
                dx, dy = x - cx, y - cy
                ang = math.atan2(-dy, dx)                # 0 = right, pi/2 = up
                up = max(0.0, math.sin(ang))
                tongue = up ** 2 * (2.2 + 1.6 * h(int((ang + 4) * 3), f, 7)) + 0.4 * h(x, y + f * 16, 9)
                r = (4.6 + tongue) * scale
                d = math.hypot(dx, dy * (1.0 if dy > 0 else 0.85)) / r
                if d < 0.28 * (1.25 if charging else 1.0): col = (238, 252, 255, 255)
                elif d < 0.5: col = (176, 230, 255, 255)
                elif d < 0.72: col = (96, 176, 255, 235)
                elif d < 1.0: col = (52, 112, 236, 170)
                elif d < 1.35: col = (40, 80, 210, int(110 * (1.35 - d) / 0.35))
                else: continue
                img.putpixel((x, y + f * size), col)
        if charging:
            # sparks and two dark slit eyes appearing in the core
            for k in range(5):
                sx, sy = int(h(k, f, 13) * 15), int(h(f, k, 14) * 15)
                if img.getpixel((sx, sy + f * size))[3] < 200:
                    img.putpixel((sx, sy + f * size), (220, 245, 255, 255))
            for (ex, ey) in ((5, 8), (6, 9), (10, 8), (9, 9)):
                img.putpixel((ex, ey + f * size), (16, 30, 80, 255))
    return img

# ============================ PREVIEW ==========================================
def project(p, s, ox, oy):
    x, y, z = p
    t, f = math.radians(32), math.radians(22)
    sx = x * math.cos(t) - z * math.sin(t)
    depth = x * math.sin(t) + z * math.cos(t)
    return (ox + sx * s, oy - y * math.cos(f) * s + depth * math.sin(f) * s)

def draw_box(dr, tex, regions, box, s, ox, oy, mirror=False, skip_top=False):
    x0, x1, y0, y1, z0, z1 = box
    def texel(face, u, v):
        rx, ry, rw, rh = regions[face]
        return tex.getpixel((rx + u, ry + v))
    def shade(c, k):
        return tuple(int(ch * k) for ch in c[:3]) + (255,)
    def quad(pts, col):
        poly = [project(p, s, ox, oy) for p in pts]
        dr.polygon(poly, fill=col, outline=col)
    w, hgt, d = x1 - x0, y1 - y0, z1 - z0
    # front (z = z1)
    for v in range(hgt):
        for u in range(w):
            uu = (w - 1 - u) if mirror else u
            c = texel('front', uu, v)
            if c[3] < 128: continue
            xa = x0 + u; ya = y1 - v
            quad([(xa, ya, z1), (xa + 1, ya, z1), (xa + 1, ya - 1, z1), (xa, ya - 1, z1)], shade(c, 0.92))
    # +x side: character's left; mirrored limbs show the outer 'right' face
    side = 'right' if mirror else 'left'
    for v in range(hgt):
        for u in range(d):
            uu = (d - 1 - u) if mirror else u
            c = texel(side, uu, v)
            if c[3] < 128: continue
            za = z1 - u; ya = y1 - v
            quad([(x1, ya, za), (x1, ya, za - 1), (x1, ya - 1, za - 1), (x1, ya - 1, za)], shade(c, 0.7))
    if not skip_top:
        for v in range(d):
            for u in range(w):
                uu = (w - 1 - u) if mirror else u
                c = texel('top', uu, v)
                if c[3] < 128: continue
                xa = x0 + u; za = z0 + v
                quad([(xa, y1, za), (xa + 1, y1, za), (xa + 1, y1, za + 1), (xa, y1, za + 1)], shade(c, 1.05 if c[0] < 240 else 1.0))

def render_iso(tex, s=7):
    W, H = int(40 * s), int(48 * s)
    im = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    dr = ImageDraw.Draw(im)
    ox, oy = W / 2, H - 9 * s
    draw_box(dr, tex, LEG, (-4, 0, 0, 12, -2, 2), s, ox, oy, skip_top=True)
    draw_box(dr, tex, LEG, (0, 4, 0, 12, -2, 2), s, ox, oy, mirror=True, skip_top=True)
    draw_box(dr, tex, ARM, (-8, -4, 12, 24, -2, 2), s, ox, oy)
    draw_box(dr, tex, BODY, (-4, 4, 12, 24, -2, 2), s, ox, oy, skip_top=True)
    draw_box(dr, tex, ARM, (4, 8, 12, 24, -2, 2), s, ox, oy, mirror=True)
    draw_box(dr, tex, HEAD, (-4, 4, 24, 32, -4, 4), s, ox, oy)
    return im

def flat(tex, which, s=6):
    im = Image.new("RGBA", (16 * s, 32 * s), (0, 0, 0, 0))
    def put(regions, face, dx, dy, mirror=False):
        rx, ry, rw, rh = regions[face]
        part = tex.crop((rx, ry, rx + rw, ry + rh))
        if mirror: part = part.transpose(Image.FLIP_LEFT_RIGHT)
        im.alpha_composite(part.resize((rw * s, rh * s), Image.NEAREST), (dx * s, dy * s))
    f = which
    put(HEAD, f, 4, 0); put(BODY, f, 4, 8)
    put(ARM, f, 0, 8, mirror=(f == 'back')); put(ARM, f, 12, 8, mirror=(f == 'front'))
    put(LEG, f, 4, 20, mirror=(f == 'back')); put(LEG, f, 8, 20, mirror=(f == 'front'))
    return im

def checker(w, hh, cell=8):
    im = Image.new("RGBA", (w, hh))
    for y in range(0, hh, cell):
        for x in range(0, w, cell):
            c = (58, 58, 66, 255) if (x // cell + y // cell) % 2 else (48, 48, 56, 255)
            ImageDraw.Draw(im).rectangle([x, y, x + cell - 1, y + cell - 1], fill=c)
    return im

def orb(size=16):
    """The hexer's orb: a dark red ember in a smoky violet halo, glowing."""
    im = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    c = (size - 1) / 2
    for y in range(size):
        for x in range(size):
            r = math.hypot(x - c, y - c) / c
            n = h(x, y, 300)
            if r < 0.35:
                col, a = (255, 120 + int(60 * (0.35 - r) / 0.35), 90), 255
            elif r < 0.6:
                col, a = (200, 30, 48), 255
            elif r < 0.95:
                col, a = (96, 30, 120), int(200 * (0.95 - r) / 0.35 * (0.7 + 0.3 * n))
            else:
                continue
            im.putpixel((x, y), jit(col + (a,), x, y, 301, 10))
    return im

def label(dr, xy, text, col=(225, 225, 235)):
    dr.text(xy, text, fill=col)

if __name__ == "__main__":
    m, x, w = mossling(), hexer(), wisp()
    m.save(f"{OUT}/mobs_mossling.png"); x.save(f"{OUT}/mobs_hexer.png"); w.save(f"{OUT}/mobs_wisp.png")
    orb().save(f"{OUT}/mobs_hexer_orb.png")

    BG = (24, 24, 32, 255)
    sheet = Image.new("RGBA", (1280, 900), BG)
    dr = ImageDraw.Draw(sheet)
    row_y = [20, 440]
    for (name, tex), ry in zip((("MOSSLING", m), ("HEXER", x)), row_y):
        label(dr, (20, ry), name)
        iso = render_iso(tex)
        sheet.alpha_composite(iso, (10, ry + 20))
        sheet.alpha_composite(flat(tex, 'front'), (310, ry + 20))
        sheet.alpha_composite(flat(tex, 'back'), (420, ry + 20))
        label(dr, (330, ry + 220), "front"); label(dr, (445, ry + 220), "back")
        raw = tex.resize((64 * 8, 32 * 8), Image.NEAREST)
        cb = checker(512, 256)
        cb.alpha_composite(raw)
        sheet.alpha_composite(cb, (545, ry + 20))
        label(dr, (545, ry + 282), "64x32 texture (character.b3d UV layout)")
    # wisp frames along the right edge
    label(dr, (1090, 20), "WISP  idle 0-3 / charging 4-7")
    for f in range(8):
        fr = w.crop((0, f * 16, 16, f * 16 + 16)).resize((96, 96), Image.NEAREST)
        bg = Image.new("RGBA", (96, 96), (8, 8, 14, 255))
        bg.alpha_composite(fr)
        sheet.alpha_composite(bg, (1080 + (f % 2) * 100, 40 + (f // 2) * 104))
    label(dr, (1080, 460), "shown on night sky")
    sheet.save(f"{OUT}/preview.png")
    print("ok")
