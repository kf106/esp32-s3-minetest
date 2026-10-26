# Running on a PC or VPS

The same server, with the same game and world generator, also builds as an ordinary program for Linux, to run on a PC or a rented server (a VPS) rather than the board. It is in `pc/`: a small `main.rs` that takes its settings from the command line and uses the firmware's code in `src/` unchanged. Windows should follow (the code avoids Linux-only calls), but hasn't been built or tried yet.

It needs only stable Rust ([rustup](https://rustup.rs/)), not the ESP-IDF tools. `pc/` has its own `.cargo/config.toml` (build for this computer, not the ESP32) and `rust-toolchain.toml` (stable Rust), so build it from inside that folder:

```
cd pc
cargo build --release
./target/x86_64-unknown-linux-gnu/release/minetest-server
```

(The folder under `target` is your computer's platform: `aarch64-unknown-linux-gnu` on an ARM server, for instance.) Then join it in Luanti at the computer's address, port 30000.

```
minetest-server [--port 30000] [--bind 0.0.0.0] [--data minetest-data]
                [--max-players 15] [--log info]
```

- `--data` is the folder for the world, accounts and players, laid out as the board's SD card: `auth/`, `world/`, `players/`, and optionally `media/` for extra skins and `motd.txt` for your own welcome message (see [The SD card](setup.md#the-sd-card)). It is made if it isn't there. To start afresh, stop the server and delete it.
- `--max-players` defaults to 15, Luanti's own default; a PC has the power the board lacks.
- It keeps much larger caches than the board (up to about 60 MB), so players returning to a place find it already worked out. Built for a 32-bit machine (a router, say), or with `MINETEST_SMALL_CACHES=1` set while building, it keeps the board's small ones.
- `--log` sets how much goes to the log, on stderr: `error`, `warn`, `info` (the default) or `debug`.
- `--config-page <port>` serves the board's hotspot settings page (see [setup.md](setup.md#the-settings-page)) on that TCP port, editing `wifi.txt`, `motd.txt` and `world/settings.txt` in the data folder. Saving stops the server rather than restarting it. It is there for the tests; on a PC, editing the files is simpler.

Like the board, it saves as it goes (world changes every second, inventories within 5 seconds), so stopping it with Ctrl+C loses at most that much. As on the board, the first player to log in becomes the administrator.

On a VPS, as a service:

1. Build it (on the server, or on a machine with the same platform) and copy `minetest-server` to `/usr/local/bin/`.
2. Make a user and a data folder for it: `sudo useradd --system --home /var/lib/minetest-server --create-home minetest`.
3. Copy `pc/minetest-server.service` to `/etc/systemd/system/`, adjust the paths and port if you like, then `sudo systemctl daemon-reload && sudo systemctl enable --now minetest-server`.
4. Open UDP port 30000 in the firewall (`sudo ufw allow 30000/udp`, or your provider's firewall settings). Luanti uses UDP only.
5. Log in yourself first, to become the administrator. Follow the log with `journalctl -u minetest-server -f`.

What the board does that the PC build doesn't: Wi-Fi set-up and the hotspot, the status screen and light, `RESET_SD` and the test kit (`TEST=`), which are all part of the firmware. The world is generated from the same fixed seed, so a world on a PC looks like one on the board.
