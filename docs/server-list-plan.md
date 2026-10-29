# Handoff: announcing to the public server list

Status: built. The settings (`server.txt` and the settings page) and the announcing itself (src/announce.rs) are done and tested from the PC build; the board's build of the announcing is not yet tested on a board.

Goal: let the board announce itself to the public Luanti server list
(servers.luanti.org), so players see it in the client's Join Game tab, as a
Luanti server with `server_announce = true` does.

## How the list works (read from the sources, October 2026)

Sources read: Luanti `src/server/serverlist.cpp` and the announce timer in
`src/server.cpp`; the list server, luanti-org/serverlist `server.py` and
`config-example.py`; `builtin/mainmenu/serverlistmgr.lua` and `common.lua`;
ESP-IDF v5.5.1 `components/mbedtls/Kconfig`.

### What a server sends

- An HTTP POST to `<serverlist_url>/announce`, where `serverlist_url`
  defaults to `https://servers.luanti.org`. The body is multipart, with a
  single form field `json` holding a JSON object.
- `action` is `start` (once, when the server starts), `update` (every 300 s:
  Luanti's `m_masterserver_timer`) or `delete` (on shutdown).
- **Every action:** `action`, `port`, and `address` (only if the
  `server_address` setting exists).
- **Start and update:** `name`, `description`, `version`, `proto_min`,
  `proto_max`, `url`, `creative`, `damage`, `password`
  (disallow_empty_password), `pvp`, `uptime` (whole seconds), `gameid`,
  `game_time`, `clients`, `clients_max`, and `clients_list` (the player
  names, if `server_announce_send_players`, which defaults to true).
- **Start only:** `dedicated`, `rollback`, `mapgen`, `privs` (default_privs),
  `can_see_far_names`, `mods` (a list of names).
- **Update only:** `lag` (float, only if non-zero).

### What the list server checks (server.py)

- **Size:** the JSON must be at most 11 KB. A request from a banned IP
  address or server is refused.
- **Required fields** (type-checked; strings are accepted for ints and
  bools): `clients`, `clients_max`, `uptime`, `game_time`, `version`,
  `proto_min`, `proto_max`, `gameid`, `name`, `description`.
- **Numbers:** none negative, and `proto_min` <= `proto_max`.
- **Text:** `url` must start with http:// or https:// (otherwise it is
  dropped). Player and mod names must be non-empty, with no spaces, control
  characters or quote marks.
- **Address:** if `address` is missing, it becomes the IP the POST came from.
  With no domain name, leave it out. A given address must be a domain or an
  IP, with no port and no Unicode; private addresses are rejected on the
  public list.
- **Updates:** an `update` for an unknown server is refused, so the server
  must send `start` first. `uptime` must not go backwards between updates.
- **Reachability probe** (`serverUp`): sends one empty ORIGINAL packet
  (`4f457403 0000 00 01`) by UDP to the address and port, and expects a
  reliable CONTROL SET_PEER_ID back within 3 s. It then sends DISCO. The
  reply time becomes the listed `ping`. If there is no reply, the server is
  not listed, and the error is kept for 10 minutes.
- **Purging:** entries are dropped 330 s (PURGE_TIME) after their last start
  or update. A board that loses power drops off on its own.
- **Duplicates:** the same address and port announced from a different IP
  address is a duplicate; the better-verified one stays.
- **Ranking:**
  - points for players and age;
  - a penalty of 8 points per second of ping over 0.4 s;
  - a penalty for clients_max over 200;
  - points reduced to 60% if proto_min <= 32 while proto_max > 36.

### What the client does

- It fetches `/list?proto_version_min=..&proto_version_max=..`.
- `is_server_protocol_compat` keeps the servers whose proto_min..proto_max
  range overlaps the client's.

## How our server fares

- **The probe already works.** The list server's `serverUp` code was run
  against the PC build: the correct SET_PEER_ID reply came back, and DISCO
  removed the peer ("New peer 2 ... Peer 2 (?) disconnected"). On the board,
  each probe shows as a peer that connects and disconnects every 5 minutes.
- **HTTPS is likely needed.** Whether plain http://servers.luanti.org
  accepts a POST is unknown (it couldn't be reached from the research
  session). Luanti itself uses https, so plan for HTTPS. Check plain HTTP
  once: if it works, TLS isn't needed at all.
- **TLS memory is the one real risk.** ESP-IDF's default is
  MBEDTLS_INTERNAL_MEM_ALLOC: about 16 KB of input buffer, plus output
  buffers and handshake/certificate memory, all from internal RAM. A running
  server has about 106 KB of internal RAM free, but the largest free block is
  only about 35 KB. Set `CONFIG_MBEDTLS_EXTERNAL_MEM_ALLOC=y` in
  sdkconfig.defaults to put all mbedTLS allocations in PSRAM: slower, but
  fine every 5 minutes.
- **Certificates and the clock are fine.** The certificate bundle
  (`CONFIG_MBEDTLS_CERTIFICATE_BUNDLE`) is on by default. Its default is the
  full bundle; the "common" option (CMN) halves it. Certificate expiry checks
  are off by default (`MBEDTLS_HAVE_TIME_DATE=n`), so the board needs no
  SNTP.
- **Flash is fine.** The firmware partition is 5 MB and the firmware about
  2.6 MB.
- **The HTTP client:** esp-idf-svc's `http::client::EspHttpConnection` with
  `crt_bundle_attach`, or esp_http_client directly through esp-idf-sys.

## As built (October 2026)

- **src/announce.rs:** the settings (below), the announcement and the
  `Announcer`.
  - `json(...)` builds Luanti's fields.
  - `form(...)` wraps them as Luanti's multipart form (one field, `json`).
  - `understand(...)` reads server.py's replies: 202 filed, 409 filed but
    the last one failed (for instance "did not respond to ping"), 200 "not
    found" (update for an unknown server: start again), 400/403/413 refused.
- **Threading:** a thread `announce`, started by `Announcer::start` with
  `config_page::spawn_on_server_core` (core 0, priority 1, a 12 KB stack on
  the board for TLS).
  - The server loop (`Server::announce`, each tick) sends it a by-value
    `Snapshot` through an mpsc channel when an announcement is due, and
    reads back how it went: a full EVERY (300 s) after success, RETRY (60 s)
    after failure.
  - Both ends are on core 0, so the channel's PSRAM memory never crosses
    cores. The first announcement goes 5 s after start-up.
- **Posting:**
  - The board uses `esp_idf_svc::http::client::EspHttpConnection` with
    `crt_bundle_attach: esp_crt_bundle_attach` and a 20 s timeout.
  - The PC build and the tests use ureq (rustls).
  - sdkconfig.defaults sets `CONFIG_MBEDTLS_DEFAULT_MEM_ALLOC=y`, so TLS's
    big buffers go to PSRAM.
- **The fields:**
  - `version` "esp32-s3-minetest-0.1.0" (or "-pc-"; the list strips
    spaces).
  - `proto_min` and `proto_max` from SERVER_PROTO_MIN/MAX, now **53 and 53**:
    the server accepts protocol 53 only, as tested (Keir's decision).
  - `gameid` "minetest"; `creative` from world settings; `damage` = not
    creative; `pvp` false (players can't hurt each other); `password` false.
  - `clients`, `clients_max`, `clients_list` (if
    `server_announce_send_players`), `uptime` (since start-up), `game_time`.
  - On start: `dedicated` true, `rollback` false, `privs` "interact, shout",
    `mapgen` "esp-gen" (our own generator's name), and `mods`: Minetest
    Game's mods as rebuilt here, plus our mobs, as `esp:default`,
    `esp:doors` and so on (Keir's decision: the list's Mods hover then
    shows what's in the game).
  - Left out: `can_see_far_names`, `lag`.
- **Power saving:** each announcement calls `Server::wake`, so the radio
  stays awake for 30 s and answers the list's probe promptly.
- **Delete:** at a restart for new settings
  (`Server::restart_for_settings`), `Announcer::withdraw_if_changed` reads
  server.txt again. If announcing is now off, or the port, address or list
  changed, it sends "delete" and waits up to 8 s. Otherwise the restarted
  server's "start" replaces the entry. Power loss: the list purges after
  330 s.
- **Not from the hotspot** (main.rs): no internet there.
- **Reporting:**
  - The log has every reply.
  - The status screen's events show "Server list: announced", "Server
    list: can't reach us" (the probe failed: port forwarding), "Server list:
    refused" and "Server list: no answer".
- **The PC build** announces too (Keir's decision), from its data folder's
  server.txt, on its `--port` unless `server_announce_port` is set.

### Tested

- **Against the real list server:** a local copy of luanti-org/serverlist
  (server.py under Flask, its config-example). The PC build's "start" was
  filed (202), the list probed the game port and listed it, with all the
  fields above and a ping. Switching announcing off on the settings page
  sent "delete" during the restart ("Server was deleted"), and switching it
  back on announced again.
- **In the suite:**
  - `tests/net/announcetest.py` (`announce`) runs a stand-in list with
    server.py's schema and its exact probe packets: start, the probe
    answered, delete when switched off.
  - Unit tests cover the JSON, the form and the replies.
  - The `config` test's server.txt points at a dead local address, so the
    suite never announces to the real list.
- **Not tested here:** the board build. The ESP toolchain isn't available
  in the session that wrote it, so the board's posting code (the esp-idf-svc
  HTTP client calls, the certificate bundle, the 12 KB stack, the mbedTLS
  memory setting) hasn't been compiled or run. Nor the real
  servers.luanti.org, which that session couldn't reach. First things to
  check on the board: that it builds, that "Server list: announced" appears,
  and the internal RAM figure in the log while announcing.

## Settings (decided and built)

`/sd/server.txt`, read by src/announce.rs (parse, format, check and load),
edited on the hotspot's settings page or by hand; see docs/setup.md, "The
public server list". Luanti's names where it has them:

- `port` (Luanti's; default 30000): the game port, added later; the PC
  build's `--port` overrides it
- `server_announce` (default false)
- `server_name`, `server_description` (required; one line, up to 80 and
  400 characters)
- `server_url` (optional; http or https)
- `server_address` (optional; checked as server.py's checkRequestAddress
  with REJECT_PRIVATE_ADDRESSES on)
- `server_announce_port` (ours; optional, the outside port)
- `server_announce_send_players` (default true)
- `serverlist_url` (default https://servers.luanti.org; not on the page)

`announce::load` gives the settings to use at start-up: with the switch on
but the details failing `check`, it warns and turns announcing off.

## Open questions for Keir

1. ~~Settings~~: decided, see above.
2. ~~Protocol range~~: 53 only, decided; the server accepts no other.
3. ~~The PC build~~: announces too, decided.
4. **Admission:** with a public listing, anyone can register an account.
   Consider a setting to refuse new accounts, or a whitelist, before listing
   publicly.

## User-side requirements (now in docs/setup.md)

- Forward **UDP** 30000 to the board (on a Deco: More, Advanced, NAT
  Forwarding, Port Forwarding), and give the board a fixed address (More,
  Advanced, Address Reservation).
- Not behind carrier-grade NAT: the Deco's WAN IP address must match the
  public IP address. Otherwise the probe fails, and only a tunnel such as
  playit.gg (UDP) can work. With a tunnel, `server_address` and
  `announce_port` must be the tunnel's, and the announce must come from an
  IP address the address resolves to, or the list marks it with a lower
  verify level.
- Forward only UDP 30000; a DMZ adds nothing and exposes more.

## Related context

- **Wi-Fi diagnosis (October 2026):** the Deco performs as well as an Archer
  AP once the board is awake (5 bots: 16.7 to 20 blocks/s, no resends). The
  bad Deco session was the board's idle power saving.
  - The Deco's DTIM is 1, the same as the Archer, but a dozing board waited
    300 to 400 ms for its packets. This is likely because the Deco buffers
    according to the board's listen interval of 3 (307 ms).
  - The fix: power saving now goes on only after 30 s with nobody connected.
- **Load figures:** 5 bots give about 16 to 20 blocks/s, with the block
  builder on core 1 (about 84% busy) as the limit.
