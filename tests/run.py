#!/usr/bin/env python3
"""Run the tests on a PC: the unit tests, the host checks, the content
verifier, and the network tests, each against the server running on this
machine.

    python3 tests/run.py                  everything
    python3 tests/run.py bones panes      only the network tests named
    python3 tests/run.py --list           list the network tests
    python3 tests/run.py --no-net         unit tests, checks, verifier and PC build only
    python3 tests/run.py --no-pc          everything but the PC build (pc/)

Needs Rust (stable, via rustup) and Python 3 with zstandard
(pip install zstandard, or apt install python3-zstandard). See
tests/README.md.
"""
import argparse, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TESTS = ROOT / "tests"
NET = TESTS / "net"
# Outside the project: its .cargo/config.toml builds for the ESP32.
WORK = Path(os.environ.get("TEST_WORKDIR", Path(tempfile.gettempdir()) / "esp32-s3-minetest-tests"))
WORLD = WORK / "out" / "serveworld"
FIRST_PORT = 30700

# The network tests: name, extra environment for the server, extra
# arguments ("{plant:kelp}" and so on are filled in from FIND_PLANTS), and
# whether the world starts at 22:00 (beds need night).
NETWORK = [
    ("network", {}, [], False),       # netclient.py itself: login, map, items, skins, chat, crafting, furnace
    ("admin", {}, [], False),
    ("bones", {}, [], False),
    ("panes", {}, [], False),
    ("fence", {}, [], False),
    ("walls", {}, [], False),
    ("signs", {}, [], False),
    ("shelf", {}, [], False),
    ("books", {}, [], False),
    ("vessels", {}, [], False),
    ("keys", {}, [], False),
    ("craftguide", {}, [], False),
    ("weather", {}, [], False),
    ("fire", {"LUANTI_FIRE_FAST": "1"}, [], False),
    ("tnt", {}, [], False),
    ("mobs", {}, [], True),
    ("wisp", {}, [], True),
    ("hexer", {}, [], True),
    ("door", {}, [], False),
    ("falling", {}, [], False),
    ("chest", {}, [], False),
    ("stair", {}, [], False),
    ("bed", {}, [], True),
    ("scoop", {}, [], False),
    ("lava", {}, [], False),
    ("drown", {}, [], False),
    ("deep", {}, [], False),
    ("flow", {}, [], False),
    ("decay", {}, [], False),
    ("grass", {}, [], False),
    ("turf", {"LUANTI_GRASS_FAST": "1"}, [], False),
    ("split", {}, [], False),
    ("water", {}, [], False),
    ("apple", {"LUANTI_GROW_SECS": "2"}, [], False),
    ("farm", {"LUANTI_GROW_SECS": "2", "LUANTI_SOIL_SECS": "2"}, [], False),
    ("plants", {}, ["{kelp}", "{coral}", "{berries}"], False),
    ("flowers", {}, ["{water}"], False),
    ("moss", {"LUANTI_MOSS_FAST": "1"}, ["{water}"], False),
    ("dungeon", {}, ["{chest}", "{front}"], False),
]


def script(name):
    return NET / ("netclient.py" if name == "network" else f"{name}test.py")


def copy_if_changed(src: Path, dst: Path):
    """Copy, keeping the old file (and its time) if nothing changed, so
    Cargo only rebuilds what did."""
    if dst.exists() and dst.read_bytes() == src.read_bytes():
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dst)


def prepare():
    """The host crate: the server's sources (not the ESP32-only main.rs and
    lcd.rs), the harness as its main, and the media."""
    (WORK / "src").mkdir(parents=True, exist_ok=True)
    for f in (ROOT / "src").glob("*.rs"):
        if f.name not in ("main.rs", "lcd.rs"):
            copy_if_changed(f, WORK / "src" / f.name)
    copy_if_changed(TESTS / "host" / "harness.rs", WORK / "src" / "main.rs")
    copy_if_changed(TESTS / "host" / "Cargo.toml", WORK / "Cargo.toml")
    copy_if_changed(TESTS / "host" / "data" / "skin_grey_wizard.png", WORK / "data" / "skin_grey_wizard.png")
    for f in (ROOT / "media").iterdir():
        if f.is_file():
            copy_if_changed(f, WORK / "media" / f.name)
    # Media since removed from the project go from the build folder too.
    for f in (WORK / "media").iterdir():
        if f.is_file() and not (ROOT / "media" / f.name).exists():
            f.unlink()
    (WORK / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "stable"\n')


def step(title, cmd, **kw):
    print(f"\n=== {title}", flush=True)
    t = time.time()
    r = subprocess.run(cmd, cwd=WORK, **kw)
    ok = r.returncode == 0
    print(f"=== {title}: {'passed' if ok else 'FAILED'} ({time.time() - t:.0f} s)", flush=True)
    return ok, r


def find_plants(binary):
    """Where the generated world has kelp, coral, blueberries, open water
    and a dungeon chest, for the plant and dungeon tests."""
    r = subprocess.run([binary], cwd=WORK, env={**os.environ, "FIND_PLANTS": "1"}, capture_output=True, text=True)
    found = {}
    for line in r.stdout.splitlines():
        if line.startswith("FOUND "):
            _, what, x, y, z = line.split()
            found[what] = f"{x},{y},{z}"
    # And a dungeon's loot chest, with the air in front of it.
    r = subprocess.run([binary], cwd=WORK, env={**os.environ, "DUNGEONS": "1"}, capture_output=True, text=True)
    for line in r.stdout.splitlines():
        if line.startswith("FOUND chest "):
            w = line.split()
            found["chest"], found["front"] = ",".join(w[2:5]), ",".join(w[6:9])
    return found


def run_network(name, env, args, night, port, binary, plants):
    """Start the server on a fresh world, run one test against it."""
    shutil.rmtree(WORLD, ignore_errors=True)
    server_env = {**os.environ, "SERVE": str(port), **env}
    if night:
        (WORLD / "world").mkdir(parents=True)
        (WORLD / "world" / "env_meta.txt").write_text("game_time = 0\ntime_of_day = 22000\nEnvArgsEnd\n")
        server_env["KEEP_WORLD"] = "1"
    # TEST_LOGS=1 keeps each server's log, in out/<test>.log.
    log = open(WORK / "out" / f"{name}.log", "w") if os.environ.get("TEST_LOGS") else subprocess.DEVNULL
    server = subprocess.Popen([binary], cwd=WORK, env=server_env, stdout=log, stderr=log)
    try:
        time.sleep(1.5)
        filled = [a.format(**plants) for a in args]
        test_env = {**os.environ, "TEST_WORLD": str(WORLD)}
        t = time.time()
        r = subprocess.run([sys.executable, str(script(name)), str(port), str(NET / "netclient.py"), *filled],
                           cwd=NET, env=test_env, capture_output=True, text=True, timeout=600)
        out = (r.stdout + r.stderr).strip().splitlines()
        last = out[-1] if out else ""
        ok = r.returncode == 0 and last.endswith("OK")
        print(f"  {'pass' if ok else 'FAIL'}  {name:<8} {time.time() - t:5.0f} s  {last[:90]}", flush=True)
        if not ok:
            print("\n".join("        " + l for l in out[-15:]))
        return ok
    except subprocess.TimeoutExpired:
        print(f"  FAIL  {name:<8} timed out")
        return False
    finally:
        server.kill()
        server.wait()


def pc_build():
    """The PC build (pc/), built where its own .cargo/config.toml and
    rust-toolchain.toml apply; its target folder is kept here, outside
    the project. Returns the program, or None."""
    env = {**os.environ, "CARGO_TARGET_DIR": str(WORK / "pc-target")}
    print("\n=== PC build", flush=True)
    t = time.time()
    r = subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT / "pc", env=env)
    print(f"=== PC build: {'passed' if r.returncode == 0 else 'FAILED'} ({time.time() - t:.0f} s)", flush=True)
    if r.returncode != 0:
        return None
    host = next(l.split()[1] for l in subprocess.run(["rustc", "-vV"], cwd=ROOT / "pc", capture_output=True, text=True).stdout.splitlines()
                if l.startswith("host:"))
    return WORK / "pc-target" / host / "release" / ("minetest-server" + (".exe" if os.name == "nt" else ""))


def run_pc(program, port):
    """The PC build as a server, on a fresh data folder, with a player."""
    data = WORK / "out" / "pcdata"
    shutil.rmtree(data, ignore_errors=True)
    server = subprocess.Popen([str(program), "--port", str(port), "--bind", "127.0.0.1", "--data", str(data), "--max-players", "3"],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(1.5)
        t = time.time()
        r = subprocess.run([sys.executable, str(NET / "pctest.py"), str(port), str(NET / "netclient.py")],
                           cwd=NET, capture_output=True, text=True, timeout=120)
        out = (r.stdout + r.stderr).strip().splitlines()
        ok = r.returncode == 0 and out and out[-1].endswith("OK")
        print(f"  {'pass' if ok else 'FAIL'}  {'pc':<8} {time.time() - t:5.0f} s  {(out or [''])[-1][:90]}", flush=True)
        if not ok:
            print("\n".join("        " + l for l in out[-15:]))
        return ok
    except subprocess.TimeoutExpired:
        print("  FAIL  pc       timed out")
        return False
    finally:
        server.kill()
        server.wait()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("only", nargs="*", help="network tests to run (default: all)")
    ap.add_argument("--list", action="store_true", help="list the network tests")
    ap.add_argument("--no-net", action="store_true", help="skip the network tests")
    ap.add_argument("--net-only", action="store_true", help="skip the unit tests, checks and verifier")
    ap.add_argument("--no-pc", action="store_true", help="skip building and trying the PC build (pc/)")
    a = ap.parse_args()
    if a.list:
        print("\n".join(n for n, *_ in NETWORK))
        return 0
    unknown = [n for n in a.only if n not in {n for n, *_ in NETWORK}]
    if unknown:
        sys.exit(f"Unknown test(s): {', '.join(unknown)}. Try --list.")

    print(f"Building in {WORK}")
    prepare()
    results = []
    binary = str(WORK / "target" / "release" / ("esp32-minetest-host" + (".exe" if os.name == "nt" else "")))
    if not a.net_only:
        ok, _ = step("Unit tests", ["cargo", "test", "--release", "--quiet"])
        results.append(("unit tests", ok))
    ok, _ = step("Build", ["cargo", "build", "--release", "--quiet"])
    if not ok:
        return 1
    if not a.net_only:
        ok, _ = step("Host checks", [binary])
        results.append(("host checks", ok))
        ok, _ = step("Content verifier", [sys.executable, str(NET / "verify.py"), str(WORK / "out"), str(ROOT / "media")],
                     stdout=subprocess.DEVNULL)
        results.append(("content verifier", ok))
    pc = None
    if not a.no_pc and not a.only:
        pc = pc_build()
        results.append(("PC build", pc is not None))
    if not a.no_net:
        chosen = [t for t in NETWORK if not a.only or t[0] in a.only]
        plants = find_plants(binary) if any(t[2] for t in chosen) else {}
        print(f"\n=== Network tests ({len(chosen)})")
        for i, (name, env, args, night) in enumerate(chosen):
            results.append((name, run_network(name, env, args, night, FIRST_PORT + i, binary, plants)))
        if pc is not None:
            results.append(("pc", run_pc(pc, FIRST_PORT - 1)))
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)} of {len(results)} passed" + (f"; failed: {', '.join(failed)}" if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
