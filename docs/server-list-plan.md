# Handoff: announcing to the public server list

Status: researched, not started. No code has been written for this.

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

## Proposed design

- **A new module, `src/announce.rs`,** running its own thread. It does a
  `start` once Wi-Fi is up and the server is ready, then an `update` every
  300 s. It sends `delete` only if the server shuts down cleanly (rare on the
  board; purging covers the rest).
- **Threading:**
  - Pin the thread to **core 0**, the same core as the server loop, at low
    priority. Then no Arc or atomic crosses between the cores. See "No Arc
    shared between the cores" in development.md: atomics in PSRAM aren't
    atomic between the cores on the ESP32-S3.
  - The server loop hands over a by-value snapshot (player names, uptime,
    game time, settings flags) through a static Mutex, which lives in
    internal RAM.
  - The TLS handshake (about 1 s of CPU) then never stalls the server loop or
    the block builder.
  - Allow about 10 KB of stack for TLS.
- **The JSON:** the project has `src/jsonstring.rs` for string escaping.
  - `version`: honest, e.g. "esp32-s3-minetest <version>".
  - `proto_min`/`proto_max`: from SERVER_PROTO_MIN/MAX in messages.rs (37
    and 53 at present; see the open questions).
  - `gameid`: "minetest".
  - `clients_max`: server::MAX_PLAYERS (6), or the PC build's
    Setup::max_players.
  - `creative` and `damage`: from world settings (settings.rs).
  - `password`: false, unless we refuse empty passwords.
  - `dedicated`: true.
  - `mapgen`: "v7".
  - `mods`: optional; leave it out, or list the built-in MTG mod names.
- **Power saving:** an empty server dozes (now only when nobody is
  connected). On the TP-Link Deco a dozing board answered pings 300 to 400 ms
  late, which would inflate the listed ping (the ranking penalises over
  0.4 s). Keep Wi-Fi awake for about 10 s after each announcement, so the
  probe gets a prompt reply.
- **The PC build:** pc/ could use the same code via a std-only HTTP client,
  or skip announcing at first.
- **Logging:** log the list server's reply body. It returns
  human-readable errors, such as "did not respond to ping" or "server_address
  ... is private", which help the user debug port forwarding. Show "listed"
  or the error on the screen's Device page.

## Settings (to decide)

A new file such as `/sd/server.txt`, in Luanti's "name = value" style, made
by the user editing over USB drive mode, like wifi.txt:

- `server_announce` (default false)
- `server_name`
- `server_description`
- `server_url` (optional)
- `server_address` (optional, only with a domain name)
- `announce_port`: the outside port. Luanti announces its bind port, but if
  the router forwards a different outside port to 30000, the board must
  announce that outside port.
- `server_announce_send_players` (default true, as in Luanti)

## Open questions for Keir

1. **Settings:** where they live (a new server.txt, or extra lines in
   wifi.txt), and the names to use.
2. **Protocol range:** announce 37 to 53 as now, or only 53 until older
   clients are tested? It is untested how far below 53 the server really
   works, and a public listing invites older clients.
3. **The PC build:** whether it should announce too.
4. **Admission:** with a public listing, anyone can register an account.
   Consider a setting to refuse new accounts, or a whitelist, before listing
   publicly.

## User-side requirements (for docs/setup.md when built)

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
