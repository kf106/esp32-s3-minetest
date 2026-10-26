# Building and flashing

You need the Rust toolchain for Xtensa, plus `git` and Python 3 with the `venv` module. The ESP-IDF build creates its own Python environment inside `.embuild/` on the first run; you don't need to set one up. On Ubuntu: `sudo apt install git python3 python3-venv`.

1. Install the Rust Xtensa toolchain with [espup](https://github.com/esp-rs/espup), then load its environment (in every new shell, or from your shell profile):
   ```
   cargo install espup
   espup install
   . ~/export-esp.sh
   ```
   `rust-toolchain.toml` selects that toolchain (`esp`) for this project.
2. Install the linker wrapper and the flashing tool:
   ```
   cargo install ldproxy espflash
   ```
3. Connect the board by USB, then build and flash. Give your Wi-Fi details as environment variables the first time. The firmware writes them to `wifi.txt` on the SD card when it starts (see [Wi-Fi networks](setup.md#wi-fi-networks)), and they are not stored in the repository.
   ```
   WIFI_SSID='your network' WIFI_PASS='your password' cargo run --release
   ```
   `cargo run` flashes the board with `espflash`, with the partition table in `partitions.csv`, and opens the serial monitor.

   For the ES3C28P or ES3N28P, add `BOARD=es3c28p`:
   ```
   BOARD=es3c28p WIFI_SSID='your network' WIFI_PASS='your password' cargo run --release
   ```
   Switching between boards needs no `cargo clean`, but always give `BOARD` when building for the ES3C28P: a build without it is for the Waveshare.

**The first build is slow.** It downloads ESP-IDF v5.5.1 and its tools into `.embuild/` inside the project and compiles them.

**Rebuilding after configuration changes:** run `cargo clean` before building if you change `sdkconfig.defaults` or the ESP-IDF version (set in `.cargo/config.toml`). Neither change is picked up by an incremental build.

## Configuration

The ESP-IDF settings that matter, in `sdkconfig.defaults`:
- **Flash and PSRAM:** 16 MB flash and 8 MB octal PSRAM at 80 MHz, with PSRAM available to `malloc`.
- **Main task stack:** 32 KB.
- **Wi-Fi and network buffers in PSRAM:** `CONFIG_SPIRAM_TRY_ALLOCATE_WIFI_LWIP` lets them live there where possible. (ESP-IDF's iperf settings, with the buffers in internal RAM, were tried: no faster here, and internal RAM ran out under load.) Every minute the log reports the internal RAM left (`Internal RAM: ... KB free, lowest so far ... KB`) and warns below 16 KB.
- **CPU statistics:** `CONFIG_FREERTOS_GENERATE_RUN_TIME_STATS` lets the status screen show the load on each core.
- **UDP receive queue:** `CONFIG_LWIP_UDP_RECVMBOX_SIZE=32` lengthens it, so bursts of client packets aren't dropped while the server is busy.

## Build options

### Starting afresh

To wipe the world and start over, build with `RESET_SD=1`:

```
RESET_SD=1 WIFI_SSID='your network' WIFI_PASS='your password' cargo run --release
```

When that firmware starts, it deletes `/sd/auth`, `/sd/world` and `/sd/players`: every account, the world's changes, and all inventories and player files. The world is then generated afresh from the seed. Extra media in `/sd/media` are kept.

It happens once. Each `RESET_SD=1` build has its own token, which it writes to `/sd/reset-done` after the reset, so rebooting the board doesn't wipe the card again. Build without `RESET_SD` afterwards as usual. Alternatively, delete those three folders by hand with the card in a PC.

### Test kit

To try out crafting and building without gathering everything first, build with `TEST=1`:

```
TEST=1 WIFI_SSID='your network' WIFI_PASS='your password' cargo run --release
```

Every player then gets, once, 50 each of cobblestone, stone, sand, sandstone, the three kinds of tree trunk and planks, sticks, coal lumps, iron lumps, steel ingots, apples, wheat and cotton seeds, wheat and cotton. The value of `TEST` is the kit's name, recorded in the player's inventory file: rebuilding with `TEST=1` gives nothing more, and `TEST=2` (or any new value) hands out a fresh kit. A test build also grows saplings, apples and crops ten times faster, so a field of wheat ripens in a few minutes rather than an hour. Build without `TEST` for normal play.

### Performance logs

For load testing (see [Development](development.md#load-testing)), build with `PERF=1`:

```
PERF=1 cargo run --release
```

The serial log then says every 10 seconds how busy each core and the main tasks were, where the server's main loop spent its time, and how many map blocks were built and sent; and every minute how much PSRAM is free. Build without `PERF` for normal play: nothing is measured then.
