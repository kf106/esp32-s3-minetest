// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2026 Keir Finlow-Bates
// Translated from Luanti's built-in chat commands (builtin/game/chat.lua):
//   Copyright (C) 2010-2024 celeron55, Perttu Ahola <celeron55@gmail.com>, and Luanti contributors
// Licensed under the GNU Lesser General Public License, version 2.1 or (at
// your option) any later version: see COPYING.LESSER.

//! Chat commands, as Luanti's built-in ones (builtin/game/chat.lua), with
//! the same names, parameters, privileges and replies, for the commands
//! that make sense here. A child of server.rs, so it works on the server
//! directly.

use std::net::SocketAddr;

use log::info;

use super::*;

/// A command: its name, parameters and description (for /help), and the
/// privileges it needs.
struct Command {
    name: &'static str,
    params: &'static str,
    description: &'static str,
    privs: &'static [&'static str],
}

const fn cmd(name: &'static str, params: &'static str, description: &'static str, privs: &'static [&'static str]) -> Command {
    Command { name, params, description, privs }
}

/// In alphabetical order, as /help lists them.
const COMMANDS: &[Command] = &[
    cmd("admin", "", "Show the name of the server owner", &[]),
    cmd("ban", "[<name>]", "Ban the IP of a player or show the ban list", &["ban"]),
    cmd("clearinv", "[<name>]", "Clear the inventory of yourself or another player", &["give"]),
    cmd("clearpassword", "<name>", "Set empty password for a player", &["password"]),
    cmd("fire", "[on | off]", "Show whether fire spreads, or turn it on or off", &[]),
    cmd("give", "<name> <ItemString> [<count>]", "Give item to player", &["give"]),
    cmd("giveme", "<ItemString> [<count>]", "Give item to yourself", &["give"]),
    cmd("grant", "<name> (<privilege> [, <privilege2> [<...>]] | all)", "Give privileges to player", &[]),
    cmd("grantme", "<privilege> [, <privilege2> [<...>]] | all", "Grant privileges to yourself", &[]),
    cmd("haspriv", "<privilege>", "Return list of all online players with privilege", &["basic_privs"]),
    cmd("help", "[all | <cmd>]", "Get help for commands", &[]),
    cmd("home", "", "Teleport you to your home point", &["home"]),
    cmd("kick", "<name> [<reason>]", "Kick a player", &["kick"]),
    cmd("kill", "[<name>]", "Kill player or yourself", &["server"]),
    cmd("me", "<action>", "Show chat action (e.g., '/me orders a pizza' displays '<player name> orders a pizza')", &["shout"]),
    cmd("mobs", "[on | off | spawn [mossling | wisp | hexer]]", "Show whether hostile mobs spawn, or turn them on or off, or bring a pack", &[]),
    cmd("mode", "[classic | survival | creative]", "Show the game mode, or set it: classic (Minetest Game), survival (with mobs) or creative", &[]),
    cmd("msg", "<name> <message>", "Send a direct message to a player", &["shout"]),
    cmd("privs", "[<name>]", "Show privileges of yourself or another player", &[]),
    cmd("revoke", "<name> (<privilege> [, <privilege2> [<...>]] | all)", "Remove privileges from player", &[]),
    cmd("revokeme", "<privilege> [, <privilege2> [<...>]] | all", "Revoke privileges from yourself", &[]),
    cmd("sethome", "", "Set your home point", &["home"]),
    cmd("setpassword", "<name> <password>", "Set player's password (sent unencrypted, thus insecure)", &["password"]),
    cmd("status", "", "Show server status", &[]),
    cmd("teleport", "<X>,<Y>,<Z> | <to_name> | <name> <X>,<Y>,<Z> | <name> <to_name>", "Teleport to position or player", &["teleport"]),
    cmd("time", "[<0..23>:<0..59> | <0..24000>]", "Show or set time of day", &[]),
    cmd("tnt", "[on | off]", "Show whether TNT can be lit, or turn it on or off", &[]),
    cmd("unban", "<name> | <IP_address>", "Remove IP ban belonging to a player/IP", &["ban"]),
];

/// The edge of the map (MAX_MAP_GENERATION_LIMIT).
const MAP_LIMIT: f32 = 31007.0;

/// "(x, y, z)" with one decimal, as core.pos_to_string(p, 1).
fn pos_text(p: [f32; 3]) -> String {
    format!("({:.1}, {:.1}, {:.1})", p[0], p[1], p[2])
}

/// The item a /give names: our own name ("esp:torch"), or the same name
/// with any other mod prefix ("default:torch") or none ("torch").
fn item_named(name: &str) -> Option<u16> {
    content::id_of(name).or_else(|| {
        let bare = name.rsplit_once(':').map_or(name, |(_, n)| n);
        content::id_of(&format!("esp:{bare}"))
    })
    .filter(|&id| id != world::CONTENT_AIR)
}

impl Server {
    /// Run a chat command (the text after the "/").
    pub(super) fn chat_command(&mut self, from: SocketAddr, name: &str, command: &str) {
        let (word, param) = command.split_once(' ').unwrap_or((command, ""));
        let param = param.trim();
        if word.is_empty() {
            self.tell(from, "-!- Empty command.");
            return;
        }
        let Some(command) = COMMANDS.iter().find(|c| c.name == word) else {
            self.tell(from, &format!("-!- Invalid command: {word}. Type /help for a list of commands."));
            return;
        };
        let mine = self.privs_of(name);
        let missing: Vec<&str> = command.privs.iter().copied().filter(|p| !mine.has(p)).collect();
        if !missing.is_empty() {
            self.tell(from, &format!("You don't have permission to run this command (missing privileges: {}).", missing.join(", ")));
            return;
        }
        info!("{name} runs /{word} {param}");
        let reply = match word {
            "admin" => Some(match self.privs.admin() {
                Some(admin) => format!("The administrator of this server is {admin}."),
                None => "There is no administrator yet: the first player to log in becomes it.".to_string(),
            }),
            "ban" => Some(self.cmd_ban(name, param)),
            "clearinv" => Some(self.cmd_clearinv(name, param)),
            "clearpassword" if param.is_empty() => Some("Name field required.".to_string()),
            "clearpassword" => Some(self.set_password(name, param, None)),
            "fire" => Some(self.cmd_fire(name, param)),
            "give" => Some(match param.split_once(' ') {
                Some((to, item)) => self.give(name, to, item.trim()),
                None => "Name and ItemString required.".to_string(),
            }),
            "giveme" if param.is_empty() => Some("ItemString required.".to_string()),
            "giveme" => Some(self.give(name, name, param)),
            "grant" | "revoke" => Some(match param.split_once(' ') {
                Some((who, what)) => self.change_privs(name, who, what, word == "grant"),
                None => format!("Invalid parameters (see /help {word})."),
            }),
            "grantme" | "revokeme" if param.is_empty() => Some(format!("Invalid parameters (see /help {word}).")),
            "grantme" => Some(self.change_privs(name, name, param, true)),
            "revokeme" => Some(self.change_privs(name, name, param, false)),
            "haspriv" => Some(self.cmd_haspriv(param)),
            "help" => Some(self.cmd_help(name, param)),
            // minetest_game's sethome mod: one home per player, kept in
            // their player file.
            "home" => Some(match self.peers.get(&from).and_then(|p| p.home) {
                Some(home) => {
                    self.move_player(from, home);
                    "Teleported to home!".to_string()
                }
                None => "Set a home using /sethome".to_string(),
            }),
            "sethome" => Some(match self.peers.get_mut(&from) {
                Some(peer) => {
                    let pos = peer.pos_bs.map(|v| v / 10.0);
                    peer.home = Some(pos);
                    peer.save(&self.players);
                    info!("{name} set their home at {}", pos_text(pos));
                    "Home set!".to_string()
                }
                None => "Player not found!".to_string(),
            }),
            "kick" => Some(self.cmd_kick(name, param)),
            "kill" => Some(self.cmd_kill(name, if param.is_empty() { name } else { param })),
            "me" if param.is_empty() => Some("-!- Invalid command usage: /me <action>".to_string()),
            "me" => {
                self.tell_all(&format!("* {name} {param}"));
                status::lock(&self.status).chat_line();
                None
            }
            "mobs" => Some(self.cmd_mobs(name, param)),
            "mode" => Some(self.cmd_mode(name, param)),
            "msg" => Some(self.cmd_msg(name, param)),
            "privs" => {
                let who = if param.is_empty() { name } else { param };
                Some(if self.exists(who) { privs::describe(who, self.privs_of(who)) } else { format!("Player {who} does not exist.") })
            }
            "setpassword" => Some(match param.split_once(' ') {
                Some((who, password)) => self.set_password(name, who, Some(password.trim())),
                None if !param.is_empty() => self.set_password(name, param, None),
                None => "Name field required.".to_string(),
            }),
            "status" => Some(self.status_line()),
            "teleport" => Some(self.cmd_teleport(name, param)),
            "time" => Some(self.cmd_time(name, param)),
            "tnt" => Some(self.cmd_tnt(name, param)),
            "unban" => Some(if self.privs.unban(param) {
                info!("{name} unbans {param}");
                format!("Unbanned {param}.")
            } else {
                "Failed to unban player/IP.".to_string()
            }),
            _ => None,
        };
        if let Some(reply) = reply {
            self.tell(from, &reply);
        }
    }

    // -------------------------------------------------------------------
    // Players and their privileges
    // -------------------------------------------------------------------

    /// A player in the game, by name (any case).
    fn online(&self, name: &str) -> Option<SocketAddr> {
        self.peers.iter().find(|(_, p)| p.ready && p.display_name().eq_ignore_ascii_case(name)).map(|(a, _)| *a)
    }

    /// Whether there is an account with this name.
    fn exists(&self, name: &str) -> bool {
        auth::check_name(name).is_ok() && self.auth.exists(name)
    }

    /// What a player may do: as they logged in with, if online.
    pub(super) fn privs_of(&self, name: &str) -> Privs {
        match self.online(name).and_then(|a| self.peers.get(&a)) {
            Some(p) => p.privs,
            None => self.privs.privs(name),
        }
    }

    /// Store a player's privileges, and tell their client if they are
    /// online (it shows fly, fast and noclip by them).
    fn set_privs_of(&mut self, name: &str, privs: Privs) -> Result<(), String> {
        self.privs.set_privs(name, privs).map_err(|e| format!("Saving the privileges failed: {e}"))?;
        let privs = self.privs.privs(name);
        if let Some(addr) = self.online(name) {
            if let Some(peer) = self.peers.get_mut(&addr) {
                peer.privs = privs;
            }
            self.send_privileges(addr);
            // Granted or revoked creative (the creative mod's on_grant
            // and on_revoke).
            self.update_creative(addr);
        }
        Ok(())
    }

    /// /grant, /grantme, /revoke and /revokeme (chat.lua's
    /// handle_grant_command and handle_revoke_command).
    fn change_privs(&mut self, caller: &str, target: &str, what: &str, grant: bool) -> String {
        let caller_privs = self.privs_of(caller);
        if !(caller_privs.has("privs") || caller_privs.has("basic_privs")) {
            return "Your privileges are insufficient.".to_string();
        }
        if !self.exists(target) {
            return format!("Player {target} does not exist.");
        }
        let current = self.privs_of(target);
        let request = match privs::parse_request(what, if grant { Privs::EVERY } else { current }) {
            Ok(r) => r,
            Err(unknown) => return unknown,
        };
        let verb = if grant { "grant" } else { "revoke" };
        if let Err(why) = privs::may_change(caller_privs, request, verb) {
            return why;
        }
        if grant {
            if let Err(e) = self.set_privs_of(target, current.with(request)) {
                return e;
            }
            info!("{caller} granted ({}) privileges to {target}", request.to_text());
            if !target.eq_ignore_ascii_case(caller) {
                if let Some(addr) = self.online(target) {
                    self.tell(addr, &format!("{caller} granted you privileges: {}", request.to_text()));
                }
            }
        } else {
            if self.privs.is_admin(target) {
                // The administrator always has every privilege.
                return format!("Note: Cannot revoke from admin: {}\nNo privileges were revoked.", request.to_text());
            }
            if request.is_empty() {
                return "No privileges were revoked.".to_string();
            }
            if let Err(e) = self.set_privs_of(target, current.without(request)) {
                return e;
            }
            info!("{caller} revoked ({}) privileges from {target}", request.to_text());
            if !target.eq_ignore_ascii_case(caller) {
                if let Some(addr) = self.online(target) {
                    self.tell(addr, &format!("{caller} revoked privileges from you: {}", request.to_text()));
                }
            }
        }
        privs::describe(target, self.privs_of(target))
    }

    fn cmd_haspriv(&self, param: &str) -> String {
        if param.is_empty() {
            return "Invalid parameters (see /help haspriv).".to_string();
        }
        if Privs::of(param).is_none() {
            return "Unknown privilege!".to_string();
        }
        let mut names: Vec<&str> = self.peers.values().filter(|p| p.ready && p.privs.has(param)).map(|p| p.display_name()).collect();
        names.sort_unstable_by_key(|n| n.to_ascii_lowercase());
        if names.is_empty() {
            format!("No online player has the \"{param}\" privilege.")
        } else {
            format!("Players online with the \"{param}\" privilege: {}", names.join(", "))
        }
    }

    /// /setpassword and /clearpassword. A new password needs one SRP
    /// exponentiation, which holds the server up for a moment.
    fn set_password(&mut self, caller: &str, target: &str, password: Option<&str>) -> String {
        if auth::check_name(target).is_err() {
            return format!("Player {target} does not exist.");
        }
        let account = match password {
            Some(p) => {
                let (salt, verifier) = srp::new_verifier(target, p);
                Account { salt, verifier }
            }
            None => Account { salt: Vec::new(), verifier: Vec::new() },
        };
        if let Err(e) = self.auth.set(target, &account) {
            return format!("Saving the password failed: {e}");
        }
        let (told, log, reply) = match password {
            Some(_) => (format!("Your password was set by {caller}."), "sets", format!("Password of player \"{target}\" set.")),
            None => (format!("Your password was cleared by {caller}."), "clears", format!("Password of player \"{target}\" cleared.")),
        };
        info!("{caller} {log} password of {target}.");
        if !target.eq_ignore_ascii_case(caller) {
            if let Some(addr) = self.online(target) {
                self.tell(addr, &told);
            }
        }
        reply
    }

    // -------------------------------------------------------------------
    // Kicking and banning
    // -------------------------------------------------------------------

    /// Disconnect a player with a message (core.kick_player): the client
    /// shows it and returns to the menu.
    fn disconnect(&mut self, addr: SocketAddr, message: &str) {
        let denied = build_access_denied(AccessDenied::CustomString, message, false);
        // Unreliable, then DISCO, as reject_stale: the peer is forgotten
        // straight away, so nothing would resend a reliable copy.
        self.send_raw(addr, &unreliable_datagram(0, &denied));
        self.send_raw(addr, &disco_datagram());
        self.remove_peer(addr, &format!("was disconnected: {message}"), "was kicked");
    }

    fn cmd_kick(&mut self, caller: &str, param: &str) -> String {
        let (target, reason) = match param.split_once(' ') {
            Some((t, r)) => (t, Some(r.trim())),
            None => (param, None),
        };
        let Some(addr) = self.online(target).filter(|_| !self.privs.is_admin(target)) else {
            return format!("Failed to kick player {target}.");
        };
        let message = match reason {
            Some(r) if !r.is_empty() => format!("Kicked: {r}"),
            _ => "Kicked.".to_string(),
        };
        info!("{caller} kicks {target}{}", reason.map(|r| format!(" with reason \"{r}\"")).unwrap_or_default());
        self.disconnect(addr, &message);
        format!("Kicked {target}.")
    }

    fn cmd_ban(&mut self, caller: &str, param: &str) -> String {
        if param.is_empty() {
            let list = self.privs.ban_list();
            return if list.is_empty() { "The ban list is empty.".to_string() } else { format!("Ban list: {list}") };
        }
        if self.privs.is_admin(param) {
            return "Failed to ban player.".to_string();
        }
        let Some(addr) = self.online(param) else {
            return "Player is not online.".to_string();
        };
        let display = self.peers.get(&addr).map_or(param.to_string(), |p| p.display_name().to_string());
        self.privs.ban(&display, addr.ip());
        let desc = format!("{}|{display}", addr.ip());
        info!("{caller} bans {desc}.");
        self.disconnect(addr, &format!("Your IP is banned. Banned name was {display}"));
        format!("Banned {desc}.")
    }

    // -------------------------------------------------------------------
    // Teleporting
    // -------------------------------------------------------------------

    /// Move a player (player:set_pos).
    fn move_player(&mut self, addr: SocketAddr, pos: [f32; 3]) {
        let pos_bs = pos.map(|v| v * 10.0);
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        peer.pos_bs = pos_bs;
        let (pitch, yaw) = (peer.pitch, peer.yaw);
        peer.save(&self.players);
        self.send_command(addr, 0, &build_move_player(pos_bs, pitch, yaw));
        self.plan_blocks(addr, pos, MAX_VIEW_RADIUS);
        self.announce_position(addr);
    }

    /// A player's position, in nodes.
    pub(super) fn position(&self, addr: SocketAddr) -> [f32; 3] {
        self.peers.get(&addr).map_or([0.0; 3], |p| p.pos_bs.map(|v| v / 10.0))
    }

    fn teleport_to_pos(&mut self, who: &str, p: [f32; 3]) -> String {
        if p.iter().any(|v| v.abs() > MAP_LIMIT) {
            return "Cannot teleport out of map bounds!".to_string();
        }
        let Some(addr) = self.online(who) else {
            return format!("Cannot get player with name {who}.");
        };
        self.move_player(addr, p);
        format!("Teleporting {who} to {}.", pos_text(p))
    }

    /// Next to another player: the first of the four places beside them
    /// with room to stand, or where they stand (find_free_position_near,
    /// with the fix below).
    fn teleport_to_player(&mut self, who: &str, target: &str) -> String {
        if who.eq_ignore_ascii_case(target) {
            return "One does not teleport to oneself.".to_string();
        }
        let Some(addr) = self.online(who) else {
            return format!("Cannot get teleportee with name {who}.");
        };
        let Some(to) = self.online(target) else {
            return format!("Cannot get target player with name {target}.");
        };
        let at = self.position(to);
        // The node the body is in: a little above the feet. (Luanti looks
        // at the node at the feet, and a player standing on the ground is
        // reported a hair below its top, so it finds the ground beside
        // them solid every way, and puts the newcomer inside them.) And
        // room for the head.
        let open = |x: i32, y: i32, z: i32| !content::walkable(self.world.node((x, y, z))) && !content::walkable(self.world.node((x, y + 1, z)));
        let free = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]]
            .iter()
            .map(|d| [at[0] + d[0], at[1], at[2] + d[2]])
            .find(|p| open(p[0].round() as i32, (p[1] + 0.6).round() as i32, p[2].round() as i32))
            .unwrap_or(at);
        self.move_player(addr, free);
        format!("Teleporting {who} to {target} at {}.", pos_text(free))
    }

    fn cmd_teleport(&mut self, caller: &str, param: &str) -> String {
        let here = self.online(caller).map_or([0.0; 3], |a| self.position(a));
        if let Some(p) = privs::parse_position(param, here) {
            return self.teleport_to_pos(caller, p);
        }
        let words: Vec<&str> = param.split_whitespace().collect();
        if words.len() == 1 && !param.contains(',') {
            return self.teleport_to_player(caller, words[0]);
        }
        let may_bring = self.privs_of(caller).has("bring");
        let missing_bring = "You don't have permission to teleport other players (missing privilege: bring).".to_string();
        if let Some((who, rest)) = param.split_once(' ') {
            let rest = rest.trim();
            let there = self.online(who).map_or(here, |a| self.position(a));
            if let Some(p) = privs::parse_position(rest, there) {
                return if may_bring { self.teleport_to_pos(who, p) } else { missing_bring };
            }
            if !rest.contains([' ', ',']) && !rest.is_empty() {
                return if may_bring { self.teleport_to_player(who, rest) } else { missing_bring };
            }
        }
        "Invalid parameters (see /help teleport).".to_string()
    }

    // -------------------------------------------------------------------
    // Items, time, help and the rest
    // -------------------------------------------------------------------

    /// /give and /giveme (chat.lua's handle_give_command).
    fn give(&mut self, giver: &str, receiver: &str, stackstring: &str) -> String {
        let mut words = stackstring.split_whitespace();
        let Some(item) = words.next() else {
            return "Cannot give an empty item.".to_string();
        };
        let count = match words.next().map(str::parse::<u32>) {
            None => 1,
            Some(Ok(n)) => n.clamp(1, u16::MAX as u32) as u16,
            Some(Err(_)) => return "Cannot give an unknown item.".to_string(),
        };
        let Some(id) = item_named(item) else {
            return "Cannot give an unknown item.".to_string();
        };
        let Some(addr) = self.online(receiver) else {
            return format!("{receiver} is not a known player.");
        };
        info!("{giver} invoked /give, stackstring=\"{stackstring}\"");
        let left = self.peers.get_mut(&addr).map_or(count, |p| p.inventory.add(Stack::new(id, count)));
        if left < count {
            self.inventory_changed(addr);
        }
        let given = if count == 1 { content::name_of(id).to_string() } else { format!("{} {count}", content::name_of(id)) };
        let how = match left {
            0 => "",
            l if l == count => " could not be",
            _ => " partially",
        };
        let own = format!("{given}{how} added to inventory.");
        if giver.eq_ignore_ascii_case(receiver) {
            return own;
        }
        self.tell(addr, &own);
        format!("{given}{how} added to inventory of {receiver}.")
    }

    fn cmd_clearinv(&mut self, caller: &str, param: &str) -> String {
        let other = !param.is_empty() && !param.eq_ignore_ascii_case(caller);
        if other && !self.privs_of(caller).has("server") {
            return "You don't have permission to clear another player's inventory (missing privilege: server).".to_string();
        }
        let who = if other { param } else { caller };
        let Some(addr) = self.online(who) else {
            return "Player must be online to clear inventory!".to_string();
        };
        let display = match self.peers.get_mut(&addr) {
            Some(p) => {
                for list in [List::Main, List::Craft, List::CraftResult] {
                    p.inventory.list_mut(list).fill(None);
                }
                p.display_name().to_string()
            }
            None => who.to_string(),
        };
        self.inventory_changed(addr);
        if other {
            self.tell(addr, &format!("{caller} cleared your inventory."));
        }
        info!("{caller} clears {display}'s inventory");
        format!("Cleared {display}'s inventory.")
    }

    fn cmd_kill(&mut self, caller: &str, victim: &str) -> String {
        let Some(addr) = self.online(victim) else {
            return format!("Player {victim} is not online.");
        };
        let hp = self.peers.get(&addr).map_or(0, |p| p.hp);
        if hp == 0 {
            return if victim.eq_ignore_ascii_case(caller) { "You are already dead.".to_string() } else { format!("{victim} is already dead.") };
        }
        if !victim.eq_ignore_ascii_case(caller) {
            info!("{caller} killed {victim}");
        }
        self.harm_player(addr, hp, "was killed", true);
        format!("{victim} has been killed.")
    }

    fn cmd_msg(&mut self, name: &str, param: &str) -> String {
        let Some((to, message)) = param.split_once(' ').map(|(t, m)| (t, m.trim())).filter(|(_, m)| !m.is_empty()) else {
            return "Invalid usage, see /help msg.".to_string();
        };
        let Some(addr) = self.online(to) else {
            return format!("The player {to} is not online.");
        };
        info!("DM from {name} to {to}: {message}");
        self.tell(addr, &format!("DM from {name}: {message}"));
        "Message sent.".to_string()
    }

    /// /fire (ours: minetest_game has the enable_fire setting): whether
    /// fire spreads, and how many flames burn; with "on" or "off" (and the
    /// server privilege), turn spreading on or off, kept in the world.
    fn cmd_fire(&mut self, caller: &str, param: &str) -> String {
        let on = match param {
            "" => {
                let world = &self.world;
                let burning = self.flames.iter().filter(|&&p| world.node(p) == fire::BASIC_FLAME).count();
                let state = if self.settings.fire() { "on" } else { "off" };
                return format!("Fire spreading is {state}; {burning} flames burning (at most {}).", fire::MAX_FLAMES);
            }
            "on" => true,
            "off" => false,
            _ => return "Invalid parameters (see /help fire).".to_string(),
        };
        if !self.moderates(caller) {
            return "You don't have permission to run this command (missing privilege: server or ban).".to_string();
        }
        self.settings.set_fire(on);
        info!("{caller} turns fire spreading {param}");
        format!("Fire spreading is now {param}.")
    }

    /// Whether a player may turn fire and TNT on and off: an admin
    /// (server) or a moderator (ban).
    fn moderates(&self, name: &str) -> bool {
        let privs = self.privs_of(name);
        privs.has("server") || privs.has("ban")
    }

    /// /mobs (ours): whether hostile mobs spawn, and how many there are;
    /// with "on" or "off" (and the server or ban privilege), turn them on
    /// or off, kept in the world (off, those about are gone at once); with
    /// "spawn" (and the server privilege), bring a pack of mosslings (or,
    /// "spawn wisp" or "spawn hexer", a wisp or a hexer) in front of you.
    /// /mode: the game mode, a shortcut for the settings it amounts to:
    /// classic (no mobs, no creative mode), survival (mobs) or creative
    /// (creative mode, no mobs). Setting it needs the server privilege.
    fn cmd_mode(&mut self, caller: &str, param: &str) -> String {
        use crate::creative::Mode;
        let now = Mode::of(self.settings.creative(), self.settings.mobs());
        if param.is_empty() {
            return format!("The game mode is {}.", now.describe());
        }
        let Some(mode) = Mode::parse(param) else {
            return "Invalid parameters (see /help mode).".to_string();
        };
        if !self.privs_of(caller).has("server") {
            return "You don't have permission to run this command (missing privilege: server).".to_string();
        }
        if mode == now {
            return format!("The game mode is already {}.", mode.name());
        }
        self.settings.set_creative(mode == Mode::Creative);
        self.settings.set_mobs(mode == Mode::Survival);
        if mode != Mode::Survival {
            self.clear_mobs();
        }
        self.update_creative_all();
        info!("{caller} sets the game mode to {}", mode.name());
        self.tell_all(&format!("{caller} set the game mode to {}.", mode.describe()));
        format!("The game mode is now {}.", mode.name())
    }

    fn cmd_mobs(&mut self, caller: &str, param: &str) -> String {
        let on = match param {
            "" => {
                let state = if self.settings.mobs() { "on" } else { "off" };
                return format!("Mobs are {state}; {} about (at most {}).", self.mobs.len(), crate::mobs::MAX_MOBS);
            }
            "on" => true,
            "off" => false,
            "spawn" | "spawn mossling" | "spawn wisp" | "spawn hexer" => {
                if !self.privs_of(caller).has("server") {
                    return "You don't have permission to run this command (missing privilege: server).".to_string();
                }
                if !self.settings.mobs() {
                    return "Mobs are off (/mobs on).".to_string();
                }
                use crate::mobs::Kind;
                let kind = match param {
                    "spawn wisp" => Kind::Wisp,
                    "spawn hexer" => Kind::Hexer,
                    _ => Kind::Mossling,
                };
                return match (self.spawn_mobs_near(caller, kind), kind) {
                    (0, _) => "There is no room for them in front of you.".to_string(),
                    (_, Kind::Wisp) => "A wisp comes.".to_string(),
                    (_, Kind::Hexer) => "A hexer comes.".to_string(),
                    (n, _) => format!("{n} mosslings come."),
                };
            }
            _ => return "Invalid parameters (see /help mobs).".to_string(),
        };
        if !self.moderates(caller) {
            return "You don't have permission to run this command (missing privilege: server or ban).".to_string();
        }
        self.settings.set_mobs(on);
        if !on {
            self.clear_mobs();
        }
        info!("{caller} turns mobs {param}");
        format!("Mobs are now {param}.")
    }

    /// /tnt (ours: minetest_game has the enable_tnt setting): whether TNT
    /// and gunpowder can be lit; with "on" or "off" (and the server or ban
    /// privilege), turn them on or off, kept in the world. Off, lit TNT
    /// goes out rather than off.
    fn cmd_tnt(&mut self, caller: &str, param: &str) -> String {
        let on = match param {
            "" => return format!("TNT is {}.", if self.settings.tnt() { "on" } else { "off" }),
            "on" => true,
            "off" => false,
            _ => return "Invalid parameters (see /help tnt).".to_string(),
        };
        if !self.moderates(caller) {
            return "You don't have permission to run this command (missing privilege: server or ban).".to_string();
        }
        self.settings.set_tnt(on);
        info!("{caller} turns TNT {param}");
        format!("TNT is now {param}.")
    }

    fn cmd_time(&mut self, caller: &str, param: &str) -> String {
        if param.is_empty() {
            return format!("Current time is {}.", self.clock.hh_mm());
        }
        if !self.privs_of(caller).has("settime") {
            return "You don't have permission to run this command (missing privilege: settime).".to_string();
        }
        match privs::parse_time(param) {
            Ok(t) => {
                self.clock.set_time_of_day(t);
                self.send_to_players(&build_time_of_day(t, clock::TIME_SPEED));
                info!("{caller} sets time to {t}");
                "Time of day changed.".to_string()
            }
            Err(e) => e,
        }
    }

    /// /help: the commands this player may run, or one command's usage.
    fn cmd_help(&self, name: &str, param: &str) -> String {
        let mine = self.privs_of(name);
        let allowed = |c: &Command| c.privs.iter().all(|p| mine.has(p));
        if param.is_empty() || param == "all" {
            let list: Vec<&str> = COMMANDS.iter().filter(|c| param == "all" || allowed(c)).map(|c| c.name).collect();
            let head = if param == "all" { "All commands" } else { "Available commands" };
            return format!("{head}: {}\nUse '/help <cmd>' to get more information, or '/help all' to list everything.", list.join(" "));
        }
        let word = param.trim_start_matches('/');
        match COMMANDS.iter().find(|c| c.name == word) {
            Some(c) => {
                let usage = if c.params.is_empty() { format!("/{}: {}", c.name, c.description) } else { format!("/{} {}: {}", c.name, c.params, c.description) };
                if allowed(c) { usage } else { format!("{usage} (no permission)") }
            }
            None => format!("Command not available: {word}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_table() {
        // Sorted, unique, and every privilege named exists.
        assert!(COMMANDS.windows(2).all(|w| w[0].name < w[1].name));
        for c in COMMANDS {
            for p in c.privs {
                assert!(Privs::of(p).is_some(), "/{} needs unknown privilege {p}", c.name);
            }
        }
        assert_eq!(pos_text([1.0, -2.25, 3.0]), "(1.0, -2.2, 3.0)");
        assert_eq!(item_named("default:torch"), content::id_of("esp:torch"));
        assert_eq!(item_named("torch"), content::id_of("esp:torch"));
        assert_eq!(item_named("air"), None);
        assert_eq!(item_named("nothing"), None);
    }
}
