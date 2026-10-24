# Administrator and privileges

Privileges work as in Luanti. Each player has a set of them, and chat commands check them.

- **New players** get Luanti's defaults, `interact` (dig, build, use and move items) and `shout` (chat). No fly, fast or noclip until the administrator grants them.
- **The administrator** has every privilege, always: they can't be revoked, kicked or banned. It is the first player to log in to a new world, or, for a world from before there was an administrator, the first to log in after the update. The name is kept in `/sd/auth/admin.txt`; to hand the role on, put another name there on a PC (one line) and restart. `/admin` shows who it is.
- **Privileges:** `interact`, `shout`, `fly`, `fast`, `noclip` (pass through blocks while flying), `teleport` (self), `bring` (teleport others), `settime`, `give`, `kick`, `ban`, `password`, `server`, `privs` (grant and revoke anything), `basic_privs` (grant and revoke only `interact` and `shout`, for a moderator) and `home` (`/sethome` and `/home`).

The commands, with Luanti's wording:

| Command | Privilege | What it does |
|---|---|---|
| `/help [all \| <cmd>]` | | The commands you may use, or how to use one |
| `/admin` | | Who the administrator is |
| `/privs [<name>]` | | Your privileges, or another player's |
| `/me <action>`, `/msg <name> <message>` | `shout` | An action, or a private message |
| `/status`, `/time` | | Players online and uptime; the time of day |
| `/grant <name> <privs> \| all`, `/revoke ...` | `privs` or `basic_privs` | Give or take away privileges, comma-separated: `/grant Jane fly, fast` |
| `/grantme ...`, `/revokeme ...` | `privs` or `basic_privs` | The same for yourself |
| `/haspriv <privilege>` | `basic_privs` | Who online has a privilege |
| `/sethome`, `/home` | `home` | Remember where you stand; go back there (one home each, kept over restarts) |
| `/time <hh:mm \| 0..24000>` | `settime` | Set the time of day |
| `/fire [on \| off]` | `server` or `ban` (to change it) | Whether fire spreads and burns things, and how many flames burn; turn it on or off (ours: Minetest Game has the `enable_fire` setting) |
| `/mobs [on \| off \| spawn [mossling \| wisp]]` | `server` or `ban` (to turn them on or off), `server` (to spawn) | Whether hostile mobs spawn, and how many there are; turn them on or off (off in a new world), or bring a pack of mosslings or a wisp in front of you (ours: Minetest Game has no mobs) |
| `/tnt [on \| off]` | `server` or `ban` (to change it) | Whether TNT and gunpowder can be lit; turn them on or off, to stop a griefer (ours: Minetest Game has `enable_tnt`) |
| `/teleport <x>,<y>,<z> \| <name>` | `teleport` | Go to a place (`~` for where you are: `~ ~20 ~`) or to a player |
| `/teleport <name> <x>,<y>,<z> \| <name2>` | `bring` | Move another player (to a player: beside them, with room to stand) |
| `/give <name> <item> [<count>]`, `/giveme <item> [<count>]` | `give` | Items, by their name: `esp:torch`, `default:torch` or just `torch` |
| `/clearinv [<name>]` | `give` (`server` for others) | Empty an inventory |
| `/kick <name> [<reason>]` | `kick` | Disconnect a player |
| `/ban [<name>]`, `/unban <name \| address>` | `ban` | Ban an online player (their name and address), list bans, or lift one |
| `/setpassword <name> <password>` | `password` | Set a player's password, for one who has forgotten theirs |
| `/clearpassword <name>` | `password` | Clear it: their next login sets a new one, like a new account |
| `/kill [<name>]` | `server` | Kill a player, or yourself |

Privileges are kept in `/sd/auth/<name>.privs`, one line such as `interact,shout,fly`, and bans in `/sd/auth/ipban.txt`, one `address|name` line each; both can be edited on a PC. A banned player is turned away by name, and anyone from their address too (on the board's hotspot, addresses are handed out afresh, so the name is what counts). Setting a password holds the server up for about a third of a second.
