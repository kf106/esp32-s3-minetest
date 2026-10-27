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

## Releases and the web installer

Released firmware is built on GitHub, and can be installed from a browser at **https://kf106.github.io/esp32-s3-minetest/**, with [ESP Web Tools](https://github.com/esphome/esp-web-tools): Chrome, Edge or Opera on a computer, which can talk to the board over USB (Web Serial). The page is in `flasher/index.html`.

### Making a release

1. Set `version` in `Cargo.toml` (the firmware reports it, on the server list too) and commit.
2. Tag the commit with that version and push the tag:
   ```
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. The **Release** workflow (`.github/workflows/release.yml`) then:
   - builds the firmware for both boards (`BOARD=waveshare` and `BOARD=es3c28p`), without Wi-Fi details;
   - makes one image per board with `espflash save-image --merge --skip-padding`: the bootloader, the partition table and the app, from address 0, as `cargo run` writes them, but not padded to the flash size, so it leaves the storage partition (a game kept in flash) alone;
   - makes a GitHub release with the two images (`minetest-on-a-stick-<board>-<version>.bin`) and their SHA-256 sums;
   - publishes the web installer to GitHub Pages, with those images and an ESP Web Tools manifest for each board (`tools/make_flasher_site.py`).

A tag that doesn't match `Cargo.toml`'s version stops the workflow. The first run takes about half an hour (ESP-IDF is downloaded and built); later ones reuse the cached ESP-IDF. To check that the firmware builds without releasing anything, run the workflow by hand (Actions, Release, Run workflow): it only builds, and the images are downloadable from the run's page.

### Setting it up on GitHub (once)

- **Pages:** Settings, Pages, Build and deployment, Source: **GitHub Actions**.
- **Deploying from tags:** Settings, Environments, **github-pages**, Deployment branches and tags: add a rule for tags matching `v*`. Without it, GitHub only lets the default branch deploy to Pages, and the release's last step fails with "not allowed to deploy to github-pages due to environment protection rules".

### Things to know

- **The toolchain is pinned:** `XTENSA_TOOLCHAIN` at the top of the workflow sets the Xtensa Rust version (espup's numbering, such as `1.97.0.0`), so a new toolchain can't break a release. Keep it in step with what you build with locally (`rustc +esp --version`).
- **Two boards, two buttons:** ESP Web Tools picks firmware by chip, and both boards are ESP32-S3s, so the page has a button and a manifest for each.
- **Erasing:** the installer asks whether to erase the flash first. Erase on a new board; not on an update, or a game kept in the board's own flash is lost. The SD card is never touched.
- **To look at the page locally:** `python3 tools/make_flasher_site.py --version test --firmware <folder with waveshare.bin and es3c28p.bin> --out _site`, then `python3 -m http.server -d _site` and open http://localhost:8000 (Web Serial works on localhost as on https).

