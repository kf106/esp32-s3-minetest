# Tests

Everything here runs on a PC, without the board: the server's code is built for the PC and run on a UDP port, and the network tests play against it as Luanti clients would.

```
.venv/bin/python3 tests/run.py
```

(after the one-off setup below).

That runs, in order:

1. **Unit tests:** the `#[cfg(test)]` tests in `src/`.
2. **Host checks:** `tests/host/harness.rs`, the server's code exercised directly, checking crafting, tools, item physics, trees, the Wi-Fi file and so on.
3. **Content verifier:** `tests/net/verify.py` reads the item and node definitions, media and other packets the server sends at login the way the Luanti 5.17 client does, and checks they parse.
4. **PC build:** `pc/`, the server as a program for Linux (see [docs/pc.md](../docs/pc.md)), built in its own folder with its own settings, and later started on a fresh data folder for a player to log in to (`tests/net/pctest.py`), and again for `tests/net/configtest.py` to save new settings through its settings page, check the files, and check that the server restarts in place with them (`config`); and once more for `tests/net/announcetest.py`, which runs a stand-in for the public server list (the real list's checks on an announcement, and its probe of the game port) and checks that the server announces itself, answers the probe, and takes itself off the list when announcing is switched off (`announce`).
5. **Network tests:** for each, a server on a fresh world, and a script in `tests/net/` that logs in one or more players and checks what happens: digging, building, crafting, chests, doors, beds, water and lava, farming, plants, admin commands, bones, signs, bookshelves, writing in books, vessels and the vessels shelf, keys, the screwdriver, binoculars and the mapping kit, the crafting guide, the weather and ambient sounds, fire, TNT and gunpowder (and the particles of a blast, read as the client reads them), butterflies and fireflies (hiding and coming out, catching, bottling), mosslings (spawning, hunting, being killed, sinking at dawn), wisps (bursting, with TNT on and off, being put out), hexers (casting, draining the land, being killed, vanishing at dawn), boats (placing on water, sitting in one, steering and cruising, being seen in one by someone joining, getting out, picking it up), carts (laying rails, pushing a cart along them and its picking up a dropped item, riding, switching at a junction, picking it up with a sneak-punch), game modes and creative mode (the creative inventory's tabs, search, taking and trash, the creative hand, unlimited placing, no damage), players' positions passed on to each other (together, once a server step), grass spreading and dying, dungeon loot, and more.

It ends with a count, and exits non-zero if anything failed. A full run takes about 10 minutes, mostly the network tests.

## Setting up

- **Rust, stable,** through rustup: `rustup toolchain install stable` if you only have the ESP toolchain. The tests are built for your PC, not the ESP32.
- **Python 3** with **zstandard**, best in a virtual environment in the project folder (`.venv`, which git ignores):
  ```
  python3 -m venv .venv
  .venv/bin/pip install -r tests/requirements.txt
  ```
  Then run the tests with `.venv/bin/python3 tests/run.py`; the test scripts it starts use the same Python. (Or install it system-wide: `sudo apt install python3-zstandard`.)

The ESP-IDF tools aren't needed.

## Choosing what to run

With the venv's Python (or `python3` once the venv is activated with `source .venv/bin/activate`):

```
python3 tests/run.py --list            the network tests
python3 tests/run.py bones panes       just those network tests (after the unit tests and checks)
python3 tests/run.py --net-only bones  just that network test
python3 tests/run.py --no-net          unit tests, checks, verifier and PC build only
python3 tests/run.py --no-pc           everything but the PC build
```

## How it works

The project's `.cargo/config.toml` makes Cargo build for the ESP32, so `run.py` builds the PC version in a folder outside the project: `esp32-s3-minetest-tests` in the system's temporary folder (`/tmp` on Linux), or wherever `TEST_WORKDIR` points. It copies in the server's sources (all of `src/` except the ESP32-only `main.rs` and `lcd.rs`), `tests/host/harness.rs` as the program's `main.rs`, `tests/host/Cargo.toml`, and the media. Files that haven't changed aren't copied again, so later builds are quick.

With `SERVE=<port>` the harness runs the server on that port, with its world in `out/serveworld` in that folder. The network tests write starting inventories there (`players/<name>.inv`), so each test's player has what it needs, and find it through `TEST_WORLD`. Some servers are started differently:

- the bed test's world starts at 22:00;
- the apple and farming tests grow things in seconds rather than minutes (`LUANTI_GROW_SECS`, `LUANTI_SOIL_SECS`);
- the turf test's grass spreads and dies every second rather than one time in 50 every 6 or 8 s (`LUANTI_GRASS_FAST`);
- the fire test's fire spreads and burns every second, every time, and its flames go out after 1 to 2 s rather than 30 to 60 (`LUANTI_FIRE_FAST`);
- the insects test's butterflies and fireflies look at the light every second rather than every 10 s (`LUANTI_INSECTS_FAST`);
- the plant and flower tests are pointed at kelp, coral, blueberries and open water found by the harness (`FIND_PLANTS=1`), and the dungeon test at the dungeon chest nearest the spawn (`DUNGEONS=1`).

Ports 30700 upwards are used, so a Luanti server on 30000 can keep running.

The servers' logs are thrown away, unless `TEST_LOGS=1` is set: then each is kept in `out/<test>.log` in the build folder, for when a test fails (`TEST_LOGS=1 python3 tests/run.py keys`).

## Other harness modes

The harness has a few tools besides the checks, chosen with an environment variable, run from the build folder (`/tmp/esp32-s3-minetest-tests`, after a `run.py`):

- `BIOMES=1`, `ORE_STATS=1`, `COAL_STATS=1`, `IRON_STATS=1`: statistics about the generated world.
- `FIND_PLANTS=1`: where the plant tests' plants are.
- `RIVERS=1`: how much of the land the river valleys and the icesheet take, the nearest river to the spawn, the emergent jungle trees, and a map in `out/rivers.bin`.
- `DUNGEONS=1`: how many mapchunks near the spawn have dungeons, how big they are and how long they take to work out, and the nearest loot chest.
- `BENCH=1`: how long map blocks take to build, at the surface and underground.
- `SERVE=30000`: the server on the usual port, for a real Luanti client on this PC to join.

For example `SERVE=30000 ./target/release/esp32-minetest-host`, then join 127.0.0.1 in Luanti.
