# Development

## Project layout

```
src/main.rs        start-up: SD card or flash, Wi-Fi, LCD, and the memory allocator
src/board.rs       the boards it can be built for, and their pins
src/server.rs      the server loop: peers, reliability, and all game logic
src/protocol.rs    Luanti's UDP packet framing
src/messages.rs    building and parsing protocol commands
src/content.rs     node and item definitions, media
src/world.rs       the world: generated blocks, player changes, lighting
src/terrain.rs     terrain, biomes, caves and ores
src/dungeons.rs    dungeons, mossy cobblestone, and dungeon loot
src/meta.rs        node metadata (a sign's text) and signs
src/books.rs       writing in books (item metadata is in inventory.rs)
src/items.rs       dropped items and their physics
src/inventory.rs   inventories and inventory actions
src/crafting.rs    crafting recipes, the craft grid, and fuel
src/furnace.rs     furnaces, and moving items between inventories
src/chest.rs       chests (and the contents of bones and shelves)
src/bones.rs       bones
src/nature.rs      leaf decay, saplings, apples, grass spreading and moss
src/clock.rs       the time of day, kept over restarts
src/stairs.rs      stairs and slabs
src/doors.rs       doors and trapdoors
src/keys.rs        skeleton keys and keys
src/screwdriver.rs the screwdriver
src/binoculars.rs  binoculars and the mapping kit
src/craftguide.rs  the crafting guide (the inventory's Recipes tab)
src/weather.rs     the weather: clouds and shadows
src/env_sounds.rs  running water and lava sounds near players
src/fire.rs        fire: flames, flint and steel, spreading
src/insects.rs     butterflies and fireflies, the bug net, fireflies in bottles
src/boats.rs       boats: floating, steering, saving
src/tnt.rs         TNT and gunpowder, and blasts
src/settings.rs    the world's settings: fire, TNT and mobs on or off
src/mobs.rs        hostile mobs: the Mossling, the Wisp and the Hexer
src/farming.rs     farming: soil, seeds, crops, hoes
src/beds.rs        beds, sleeping and respawn points
src/fences.rs      fences, fence rails, fence gates and walls
src/panes.rs       glass panes and steel bars
src/plants.rs      cactus, papyrus, bushes, ferns, marram grass, kelp, corals
src/falling.rs     falling sand and gravel
src/colours.rs     flowers, dyes and wool
src/liquids.rs     flowing water and lava
src/privs.rs       privileges, the administrator and bans
src/commands.rs    chat commands
src/player.rs      saved player state
src/skins.rs       player skins
src/auth.rs        accounts; src/srp.rs, sha256.rs, sha1.rs: login crypto
src/zstd.rs        a small zstd compressor for map blocks
src/lcd.rs, screen.rs, font.rs, status.rs   the status display
src/storage.rs     safe file writes on FAT
media/             built-in textures and models
partitions.csv     the flash layout: firmware, and room for the game without an SD card
tools/loadtest.py  bot players for load testing (see below)
tools/make_mob_sounds.py  the mobs' placeholder sounds
tools/make_mob_skins.py   the mobs' textures
pc/                the PC (Linux, later Windows) build: main.rs, a systemd unit
tests/             the tests: tests/run.py runs them all (see Testing below)
```

## Testing

The server can be tested on a PC, without the board. `tests/run.py` builds the server's code for the PC, runs the unit tests and a set of checks, verifies what the server sends at login the way the Luanti client reads it, and then runs about 35 network tests, each a scripted player (or two) on a fresh world:

```
python3 -m venv .venv && .venv/bin/pip install -r tests/requirements.txt   # once
.venv/bin/python3 tests/run.py
```

It needs stable Rust (through rustup) and Python 3, not the ESP-IDF tools, and takes about 10 minutes. See [tests/README.md](../tests/README.md) for running single tests and how it works.

## Load testing

`tools/loadtest.py` starts bot players that log in and walk off in
different directions, so the server has to generate and send fresh map
blocks for each, as players exploring would. It needs only Python 3:

```
python3 tools/loadtest.py 192.168.68.137 --bots 5 --minutes 10
```

Every 10 seconds it prints each bot's distance walked, map blocks received
(in total, per second, and in KB per second), round trip time, longest silence from the
server, and packets it had to resend. Watch the board's Device page
meanwhile for CPU load and memory. The bots are saved like any player, so
each run carries on in new terrain. Real players count towards the limit
of six; a bot over the limit is turned away. `--help` lists the other
options (speed, how far apart they join, view range, names).

**Measuring Wi-Fi throughput.** Walking bots are limited by how fast the
board can generate terrain. To measure sending alone, have one bot stand
still where the terrain is already built:

```
python3 tools/loadtest.py 192.168.68.137 --bots 1 --speed 0 --minutes 1 --prefix Still
python3 tools/loadtest.py 192.168.68.137 --bots 1 --speed 0 --minutes 1 --prefix Still
```

The first run builds the blocks around the bot (and the server keeps up to
600 of them); the second gets them all from that store, so it measures how
fast they go out. At the end it prints how long the whole view took to
arrive, in blocks and KB per second. Run it the same way, from the same
place, before and after a change.

## Notes for hackers

- **Read the engine source.** The protocol has no separate specification. The Luanti source (`src/network/`, `src/server/`, `src/client/`) is the reference, and the comments in this code point to the functions they follow.
- **The Xtensa argument limit.** Functions here are kept to at most six argument words, counting a hidden pointer for large return values. A seventh argument is passed on the stack, and in one build it arrived corrupted, which crashed the server on the first packet. Group arguments into a struct instead; `server::Setup` is the example.
- **Two cores.** The server loop (network and game logic) runs on core 0, next to the Wi-Fi driver. Map blocks, the expensive part, are generated and compressed by a thread on core 1, working from a snapshot of the world's changes. Built blocks are cached, so players near each other share them. A block the world changed around while it was being built is built again.
- **Memory.** The global allocator in `main.rs` puts the server's data in PSRAM. Left to the default, small allocations fill the internal RAM, and then Wi-Fi sends fail and threads can't start.
