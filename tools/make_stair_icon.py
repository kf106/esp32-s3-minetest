"""Draw the settings page's creative-mode icon: an inner birch wood stair,
seen as Luanti shows nodes in the inventory (top, front left and front
right faces, the sides darker), from the game's own birch wood texture.

    python3 tools/make_stair_icon.py   (needs Pillow: pip install pillow,
                                        in a venv)

Writes src/web/stair_birch_inner.png, which config_page.rs builds in.
"""
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
TEXTURE = ROOT / "media" / "esp_birch_wood.png"
OUT = ROOT / "src" / "web" / "stair_birch_inner.png"

SIZE = 64          # the icon, in pixels
A = 28             # half the width of a whole block on screen
H = 1.15 * A       # a whole block's height on screen
SHADE = {"top": 1.0, "x": 0.8, "z": 0.62}

tex = Image.open(TEXTURE).convert("RGBA")
T = tex.width


def screen(x, y, z):
    """Block coordinates (0..1) to pixels: +x down to the right, +z down
    to the left, +y up."""
    return (SIZE / 2 + (x - z) * A, 2 + A / 2 * (x + z) + (1 - y) * H - 0.0)


# The stair in half-block cubes (i, j, k) along x, y, z: the bottom half
# whole, the top half an L, missing the quarter nearest the viewer, so the
# inner corner shows.
cubes = {(i, j, k) for i in (0, 1) for j in (0, 1) for k in (0, 1)} - {(1, 1, 1)}

img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
px = img.load()


def face(corner, du, dv, uv0, kind):
    """Fill the parallelogram corner + s*du + t*dv (s, t in 0..1), whose
    texture is the half-size square starting at texture fraction uv0."""
    p0 = screen(*corner)
    pu = screen(*(c + d for c, d in zip(corner, du)))
    pv = screen(*(c + d for c, d in zip(corner, dv)))
    ex, ey = pu[0] - p0[0], pu[1] - p0[1]
    fx, fy = pv[0] - p0[0], pv[1] - p0[1]
    det = ex * fy - ey * fx
    xs = [p0[0], pu[0], pv[0], pu[0] + fx]
    ys = [p0[1], pu[1], pv[1], pu[1] + fy]
    for y in range(int(min(ys)) - 1, int(max(ys)) + 2):
        for x in range(int(min(xs)) - 1, int(max(xs)) + 2):
            if not (0 <= x < SIZE and 0 <= y < SIZE):
                continue
            cx, cy = x + 0.5 - p0[0], y + 0.5 - p0[1]
            s = (cx * fy - cy * fx) / det
            t = (ex * cy - ey * cx) / det
            if 0 <= s < 1 and 0 <= t < 1:
                u = int((uv0[0] + s / 2) * T) % T
                v = int((uv0[1] + t / 2) * T) % T
                r, g, b, a = tex.getpixel((u, v))
                k = SHADE[kind]
                px[x, y] = (int(r * k), int(g * k), int(b * k), 255)


# Far to near, so nearer faces paint over farther ones.
for (i, j, k) in sorted(cubes, key=lambda c: (c[0] + c[2], c[1])):
    x0, y0, z0 = i / 2, j / 2, k / 2
    if (i, j + 1, k) not in cubes:   # top
        face((x0, y0 + 0.5, z0), (0.5, 0, 0), (0, 0, 0.5), (x0, z0), "top")
    if (i + 1, j, k) not in cubes:   # +x side, front right
        face((x0 + 0.5, y0 + 0.5, z0), (0, 0, 0.5), (0, -0.5, 0), (z0, 0.5 - y0), "x")
    if (i, j, k + 1) not in cubes:   # +z side, front left
        face((x0, y0 + 0.5, z0 + 0.5), (0.5, 0, 0), (0, -0.5, 0), (x0, 0.5 - y0), "z")

OUT.parent.mkdir(exist_ok=True)
img.save(OUT, optimize=True)
print(f"wrote {OUT.relative_to(ROOT)} ({OUT.stat().st_size} bytes)")
