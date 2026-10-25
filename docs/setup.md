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
- **Getting files in and out:** there is no card to put in a PC, so no editing `wifi.txt` or `motd.txt`, adding skins, or looking at the files there. Build with `WIFI_SSID` and `WIFI_PASS` (or use the hotspot) for the Wi-Fi settings; changing networks means reflashing.
- **Backups and moving a world:** a card can be copied in seconds; a world in flash belongs to that board, and there is no tool yet to copy it out.
- **The partition table:** `cargo run` writes it with the firmware (see `.cargo/config.toml`). Ordinary reflashing keeps the game, but flashing with a different table (for instance if the firmware ever outgrows its 5 MB), or erasing the whole flash, wipes it.
- **Space:** 11 MB. A world takes a few megabytes, so it is plenty for play, but leaves little room for extra media.
- **Wear:** flash lasts about 100,000 erases per sector. Wear levelling spreads the writes over the whole partition, so at the server's rate (changes saved once a second while people play) it should last many years, but unlike a card it can't be replaced.
- **A card and flash are separate:** a world started in flash stays there, and isn't seen while a card is in. One board holds one world in flash; cards let you swap between worlds.
- `RESET_SD=1` resets whichever is in use.

For tinkering, backups or more than one world, use a card; flash suits a fixed setup or a demo.

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
- **Power saving:** after 30 seconds with nobody doing anything (no one connected, or every player standing still with their map loaded), the radio dozes between the router's beacons, which saves power and keeps the board's 3.3 V regulator cooler. The first move afterwards may arrive up to a few hundred milliseconds late; then the radio stays fully awake again. The Device page shows "doze" in place of the channel while it saves.
- **Building with Wi-Fi details:** if you build with `WIFI_SSID` and `WIFI_PASS`, that network is written to the top of `wifi.txt` the first time the new firmware starts, or its password is updated if it is already listed. After that the file is yours: later edits are kept, even through reflashing the same firmware. It is only rewritten if you build with a different network or password.
- **Building without them:** plain `cargo run --release` uses whatever `wifi.txt` holds.

## Its own hotspot

Where there is no Wi-Fi to join, the board can make its own network. Switch it on, and when the screen says "Hold BOOT now for a hotspot", hold the BOOT button for a second (it waits three seconds for this). A blue bar grows along the bottom of the screen while you hold it; when it reaches the end, the hotspot starts and the screen shows its QR code. Let go before then and the bar goes, and the board joins Wi-Fi as usual. Don't hold BOOT while plugging the board in: the chip then waits to be flashed instead of starting.

The hotspot is set in `wifi.txt` with a `hotspot` line, and the `password` line after it:

```
hotspot MTG-0BC4
password minetest
```

If the file has no `hotspot` entry, this default is added to the end of it at start-up (the rest of the file is left as it was). The four characters after "MTG-" are the last four hex digits of the board's hotspot MAC address, so boards near each other have different names; the screen's Join page shows the name. A file that still has the old default (`hotspot Luanti` with the password `LuantiLuanti`) gets the new one in its place; a name or password you set yourself is left alone. WPA2 needs a password of 8 to 63 characters; leave the `password` line out for an open hotspot. If the entry can't be used (a password too short, say), the server warns and runs the default instead.

On the hotspot, the server is at **192.168.4.1** (port 30000, Luanti's default), and hands out addresses to up to 8 devices. The screen opens on the Join page: point a phone's camera at the QR code to join, or type the name and password on a laptop, then connect Luanti to 192.168.4.1. There is no internet on the hotspot, so a phone may complain or drift back to mobile data; tell it to stay connected. The power saving described above is off on the hotspot, since the board is the access point.

The passwords are stored on the card in plain text, so treat the card accordingly.

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
