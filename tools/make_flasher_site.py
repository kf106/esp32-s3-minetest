#!/usr/bin/env python3
"""Put together the web flasher (GitHub Pages) from built firmware.

    python3 tools/make_flasher_site.py --version 0.1.0 --firmware out --out _site \\
        [--repo https://github.com/kf106/esp32-s3-minetest]

--firmware holds one merged image per board, <board>.bin (from espflash
save-image --merge --skip-padding, written at address 0). The site gets
flasher/index.html, an ESP Web Tools manifest and the image for each board
under firmware/, the textures the page uses (from media/) and the boards'
photos (from assets/).

The release workflow (.github/workflows/release.yml) runs this; it also
works locally, to look at the page before releasing (serve _site over
http://localhost, which browsers treat as secure, for Web Serial).
"""
import argparse
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Board id (as BOARD= when building), its name, and its photo in assets/
# (None: the page shows the board without one).
BOARDS = [
    ("waveshare", "Waveshare ESP32-S3-LCD-1.47", "server-001.jpg"),
    ("es3c28p", "LCDwiki ES3C28P / ES3N28P", "server-101.jpg"),
    ("tdisplays3", "LilyGO T-Display-S3", None),
]

TEXTURES = ["esp_dirt.png", "esp_grass_side.png", "default_meselamp.png", "default_tool_diamondpick.png",
            "default_sign_wood.png", "map_mapping_kit.png", "default_mese_crystal.png"]


def manifest(board: str, name: str, version: str) -> dict:
    """ESP Web Tools' manifest (esp-web-tools src/const.ts, Manifest):
    one ESP32-S3 build, the merged image at 0. ESP Web Tools picks a build
    by chip family only, so each board has its own manifest and button."""
    return {
        "name": f"Minetest on a Stick ({name})",
        "version": version,
        # Ask whether to erase: yes for a new board, no for an update (a
        # game kept in flash would go).
        "new_install_prompt_erase": True,
        # The firmware doesn't speak Improv Wi-Fi: don't wait for it.
        "new_install_improv_wait_time": 0,
        "builds": [{"chipFamily": "ESP32-S3", "parts": [{"path": f"{board}.bin", "offset": 0}]}],
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--version", required=True)
    ap.add_argument("--firmware", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--repo", default="https://github.com/kf106/esp32-s3-minetest")
    a = ap.parse_args()

    out = a.out
    if out.exists():
        shutil.rmtree(out)
    (out / "firmware").mkdir(parents=True)
    (out / "t").mkdir()
    (out / "boards").mkdir()

    for board, name, photo in BOARDS:
        image = a.firmware / f"{board}.bin"
        if not image.is_file():
            print(f"make_flasher_site: no {image}", file=sys.stderr)
            return 1
        shutil.copy(image, out / "firmware" / f"{board}.bin")
        (out / "firmware" / f"{board}.json").write_text(json.dumps(manifest(board, name, a.version), indent=2) + "\n")
        if photo:
            shutil.copy(ROOT / "assets" / photo, out / "boards" / f"{board}.jpg")
    for t in TEXTURES:
        shutil.copy(ROOT / "media" / t, out / "t" / t)

    repo = a.repo.rstrip("/")
    page = (ROOT / "flasher" / "index.html").read_text()
    for key, value in {"VERSION": a.version, "REPO": repo, "REPO_NAME": repo.removeprefix("https://"),
                       "RELEASES": f"{repo}/releases/latest"}.items():
        page = page.replace("{{" + key + "}}", value)
    if "{{" in page:
        print("make_flasher_site: a {{...}} left in the page", file=sys.stderr)
        return 1
    (out / "index.html").write_text(page)
    print(f"Flasher site for {a.version} in {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
