# Setting up and connecting

What the board keeps on its SD card (or in flash), how it finds Wi-Fi or makes its own hotspot, and how players connect.

## The SD card

Format the card as FAT32 and insert it before powering up. Any size will do: a world takes a few megabytes. The server creates what it needs:

```
/sd/auth/       accounts (salt and SRP verifier per player), <name>.privs (privileges, where not the default),
                admin.txt (the administrator's name) and ipban.txt (banned players)
/sd/world/      the world: nodes.log (every change made), furnaces.txt and chests.txt (their contents, and those of bones), items.txt (dropped items), doors.txt (steel doors' and trapdoors' owners),
                beds_spawns (where players respawn),
                env_meta.txt (the time of day, as Luanti keeps it) and the generator version
/sd/players/    <name>.inv (inventory) and <name>.player (position, health, skin, home)
/sd/media/      optional extra media (see below)
/sd/motd.txt    optional welcome message (see below)
/sd/wifi.txt    the Wi-Fi networks to join, and the hotspot (see below)
```

All of these are plain text or simple binary files, so you can inspect or edit them on a PC. If a new version changes the terrain generator, the world starts afresh: the old change log is kept as `nodes-v<n>.log`, and the chests', furnaces' and doors' files, bed spawns, signs' text and dropped items as `<file>.v<n>` (`chests.txt.v4`, say). Players keep their inventories; anyone standing inside the new ground is moved up onto it.

### Without an SD card

If no card is found at start-up, the server keeps everything in the board's own flash instead: an 11 MB partition (`storage` in `partitions.csv`), formatted as FAT the first time and mounted in the card's place, with ESP-IDF's wear levelling spreading the writes. The same files are kept there, and the Device page shows "Flash" in place of "SD card". Things to know:
- **Getting files in and out:** there is no card to put in a PC, but the board can become a USB drive instead (see [As a USB drive](#as-a-usb-drive)). That is how to edit `wifi.txt` or `motd.txt`, add skins, back the world up, or move it to a card or a PC.
- **The partition table:** `cargo run` writes it with the firmware (see `.cargo/config.toml`). Ordinary reflashing keeps the game, but flashing with a different table (for instance if the firmware ever outgrows its 5 MB), or erasing the whole flash, wipes it.
- **Space:** 11 MB. A world takes a few megabytes, so it is plenty for play, but leaves little room for extra media.
- **Wear:** flash lasts about 100,000 erases per sector. Wear levelling spreads the writes over the whole partition, so at the server's rate (changes saved once a second while people play) it should last many years, but unlike a card it can't be replaced.
- **A card and flash are separate:** a world started in flash stays there, and isn't seen while a card is in. One board holds one world in flash; cards let you swap between worlds.
- `RESET_SD=1` resets whichever is in use.

For tinkering or more than one world, use a card; flash suits a fixed setup or a demo.

### As a USB drive

The board can show its files to a PC as a USB drive: the SD card if there is one, otherwise the flash. That saves taking the card out to add a Wi-Fi network to `wifi.txt`, and it is the only way at the files in flash. Plug it into the PC, and when the screen says "Hold BOOT now for a hotspot", keep BOOT held for three seconds: the blue bar fills (that far means the hotspot), then an orange one fills over it. When the orange bar is full, the server doesn't start, and the files appear on the PC: `world`, `players`, `auth`, `wifi.txt` and the rest. Let go while only the blue bar is full and you get the hotspot instead.

The screen changes to an orange "USB Drive Active" page, with what the PC sees (the card or the flash, and its size) and the files and folders at the top of it. Copy or change what you like, and eject the drive on the PC: the page says "Ejected: restarting...", and the board restarts into the game. BOOT does nothing while it is a drive. (Unplugging also works once the drive is ejected.) If the drive couldn't start, the page turns red and says why; BOOT then restarts the board. Things to know:
- **Don't change the game's files unless you know what you are doing.** `wifi.txt` and `motd.txt` are meant to be edited. Deleting or editing the rest can lose accounts, inventories or the world's changes, or let someone else become the administrator. Copying them off as a backup is always safe.
- **Eject, don't just unplug.** The PC may still be writing when it says the copy is done; pulling the plug before ejecting can damage the files.
- **No server meanwhile.** Only one side can use the files at a time, so nobody can play while it is a drive.
- **No serial log meanwhile.** The board's USB carries the drive instead of the log. Flashing still works as usual.
- **A world from a PC:** the PC build keeps the same files in its `--data` folder. Copy them over (in flash, the 11 MB limit applies), and the board plays that world.
- **Speed:** the board's USB is the slow kind (12 Mbit/s), so expect about 1 MB/s at best: fine for `wifi.txt` or a world, slow for a whole card of extra media.

## Wi-Fi networks

The server joins Wi-Fi using `/sd/wifi.txt`, a plain text file you can edit on a PC:

```
network My Home Wi-Fi
password correct horse battery staple

network Office
password hunter2
```

Each `network` line starts an entry. Its `password` line is optional; leave it out for an open network. Everything after the first space is the value, spaces included. Lines starting with `#` are ignored.

How the server chooses a network:
- **Which one:** at start-up it scans, then joins the networks it can see, in the file's order. If none are in range it also tries the rest, in case one is hidden, and keeps retrying every 10 seconds. The LCD shows what it is doing.
- **If the connection drops** (the router restarts, or the signal goes), the server notices within a couple of seconds, tries the same network again, and then looks for any known network as at start-up. The game keeps running meanwhile; the LCD header shows "Wi-Fi..." and the status light blinks amber until it is back. Players whose clients haven't given up (after 30 seconds) carry on where they were.
- **Power saving:** after 30 seconds with nobody connected, the radio dozes between the router's beacons, which saves power and keeps the board's 3.3 V regulator cooler. The first packet of the next login wakes it, and it stays fully awake while anyone is connected, even standing still: a dozing radio only hears packets at the beacons that announce them, about every 100 ms on many routers but every 300 ms or more on some (a TP-Link Deco, for one), which a player would feel. The Device page shows "doze" in place of the channel while it saves.
- **Building with Wi-Fi details:** if you build with `WIFI_SSID` and `WIFI_PASS`, that network is written to the top of `wifi.txt` the first time the new firmware starts, or its password is updated if it is already listed. After that the file is yours: later edits are kept, even through reflashing the same firmware. It is only rewritten if you build with a different network or password.
- **Building without them:** plain `cargo run --release` uses whatever `wifi.txt` holds.

## Its own hotspot

Where there is no Wi-Fi to join, the board can make its own network. Switch it on, and when the screen says "Hold BOOT now for a hotspot", hold the BOOT button for a second (it waits three seconds for this). A blue bar grows along the bottom of the screen while you hold it; when it reaches the end, the hotspot starts and the screen shows its QR code. Let go before then and the bar goes, and the board joins Wi-Fi as usual. (Holding on past the blue bar starts an orange one, towards [the USB drive](#as-a-usb-drive): let go once the blue bar is full for the hotspot.) Don't hold BOOT while plugging the board in: the chip then waits to be flashed instead of starting.

The hotspot is set in `wifi.txt` with a `hotspot` line, and the `password` line after it:

```
hotspot MTG-0BC4
password minetest
```

If the file has no `hotspot` entry, this default is added to the end of it at start-up (the rest of the file is left as it was). The four characters after "MTG-" are the last four hex digits of the board's hotspot MAC address, so boards near each other have different names; the screen's Join page shows the name. A file that still has the old default (`hotspot Luanti` with the password `LuantiLuanti`) gets the new one in its place; a name or password you set yourself is left alone. WPA2 needs a password of 8 to 63 characters; leave the `password` line out for an open hotspot. If the entry can't be used (a password too short, say), the server warns and runs the default instead.

On the hotspot, the server is at **192.168.4.1** (port 30000, Luanti's default), and hands out addresses to up to 8 devices. The screen opens on the Join page: point a phone's camera at the QR code to join, or type the name and password on a laptop, then connect Luanti to 192.168.4.1 (or open that address in a browser for [the settings page](#the-settings-page)). There is no internet on the hotspot, so a phone may complain or drift back to mobile data; tell it to stay connected. The power saving described above is off on the hotspot, since the board is the access point.

The passwords are stored on the card in plain text, so treat the card accordingly.

## The settings page

On the hotspot, the board also serves a settings page: join the hotspot with a phone or laptop and open **http://192.168.4.1** in a browser (the IP address on the Join page). It changes the files described in this guide, without a PC or the USB drive:

- **Game rules:** fire, TNT, mobs and creative mode, as toggles (`world/settings.txt`, the same settings as `/fire`, `/tnt`, `/mobs` and `/mode` in the game).
- **Welcome message:** `motd.txt`. Empty means the built-in welcome.
- **Public server list:** a name, a description and, optionally, a website, a domain name and the port players use from outside (`server.txt`, see [below](#the-public-server-list)). The switch to announce the server stays greyed out until the details are complete and would pass the list's own checks.
- **Wi-Fi networks:** add, remove, rename and reorder them (`wifi.txt`). Saved passwords are never shown: a password left empty keeps the one saved, and a new network with no password is an open one.
- **The hotspot's name and password.**

Saving writes the files, then the board saves the game and restarts, so that it reads them. Players connected at the time are told the server is restarting, and their clients offer to reconnect. Choose where it restarts:

- **Save and restart on the hotspot:** it comes straight back on the hotspot, without holding BOOT. If you changed the hotspot's name or password, join the new one.
- **Save and restart on Wi-Fi:** it joins the networks on the list, as at an ordinary start. This is the way to set up a new board: start the hotspot, add your home network, and restart on Wi-Fi.

Things to know:

- **Only on the hotspot:** the page isn't there when the board has joined a network, so nobody on your home network or the internet can reach it.
- **Anyone on the hotspot can use it:** there is no separate login. The hotspot's password is the lock, and the screen shows it, so anyone who can see the board can change its settings. Don't run the hotspot where that matters.
- **The PC build has it too,** without the Wi-Fi and hotspot sections, on its own computer only: see [docs/pc.md](pc.md#the-settings-page).
- **`wifi.txt` is rewritten:** saving writes it out afresh, so comments of your own in it are replaced by the standard ones.

## The public server list

The server can be listed on Luanti's public server list, where anyone finds it in the Join Game tab. The settings are in `/sd/server.txt`, set on [the settings page](#the-settings-page) or by hand, in Luanti's own "name = value" style:

```
server_announce = true
server_name = Minetest on a Stick
server_description = A small world on an ESP32. Be nice.
server_url =
server_address =
server_announce_port =
server_announce_send_players = true
```

- `server_announce`: whether to announce. It only takes effect when the name and description are filled in and everything passes the list's checks; otherwise the server says why in its log and doesn't announce.
- `server_name`, `server_description`: what the list shows (one line each, up to 80 and 400 characters).
- `server_url`: optional, a web page about the server, starting `http://` or `https://`.
- `server_address`: optional, a domain name that points at your home connection. Leave it empty and the list uses the address the announcement comes from, which is right for most homes. No port and no `http://`. A private address (192.168.x.x and the like) is refused, since nobody outside could use it.
- `server_announce_port`: optional, the port players use from outside, if your router forwards a different one to the board's 30000.
- `server_announce_send_players`: whether the list shows the names of the players online, or only how many there are.
- `serverlist_url`: not on the page; only for announcing to a list of your own (the default is `https://servers.luanti.org`).

For players outside your home to join, your router must forward **UDP** port 30000 to the board, and your connection needs its own public address: most mobile connections, and some home ones, share one between many customers ("carrier-grade NAT"), and then nothing outside can reach the board. Once listed, anyone can join and make an account.

How it works, as a Luanti server does it:

- **When:** a few seconds after start-up the server announces itself ("start"), then again every 5 minutes ("update", with the players online and how long it has been up). The list drops a server it hasn't heard from for five and a half minutes, so a board that is switched off falls off by itself. Switching announcing off on the settings page (or changing the port or domain name) takes the old entry off at once when the board restarts. Not from the hotspot: it has no internet.
- **What the list shows:** the name, description and website; the players (or only how many); creative mode, or the heart for damage (players can't hurt each other here, so never the sword); and the version as `esp32-s3-minetest-0.1.0`, since this isn't Luanti itself. Only Luanti clients that speak protocol 53 (Luanti 5.17 tested) see it, as those are the only ones the server lets in.
- **The check:** after each announcement the list tries the game port from outside, as a client would. If that fails (no port forwarding, or a shared public address), the board isn't listed, and the next announcement hears why: the log says "did not respond to ping", and the status screen shows "Server list: can't reach us". The status screen also shows "Server list: announced" when it starts, and "Server list: no answer" if the list can't be reached (it then tries again every minute).
- **Its cost:** each announcement is an HTTPS request, a second or two of work in the background. The board stays fully awake (no Wi-Fi power saving) for half a minute after each, so it answers the list's check promptly.

## Welcome message

Players see a short welcome in the chat area when they join: what the server is, where the source is, and who else is playing. To use your own text instead, put it in `/sd/motd.txt`; each line becomes one chat line. The list of other players is still added at the end.

## Extra media and skins

Textures and models are built into the firmware. A file in `/sd/media` with the same name as a built-in one replaces it, and new files are offered to clients as well.

To add a skin, put `skin_<name>.png` in `/sd/media` and restart. It appears in the skin chooser as "<Name>": for example, `skin_red_knight.png` appears as "Red knight". Both skin formats are accepted:
- **64×32:** the classic Minetest and old Minecraft layout;
- **64×64:** modern Minecraft skins. Luanti's player model has no place for their overlay layers, so only the top half is used.

## Connecting

1. Watch the serial monitor or the LCD for the board's IP address.
2. In Luanti, open **Join Game** and enter that address with port **30000**.
3. Pick a name and password; the first login creates the account.

The first player to log in to a new world becomes its administrator, so log in yourself before anyone else does.
