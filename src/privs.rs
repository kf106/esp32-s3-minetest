// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2026 Keir Finlow-Bates
// Translated from Luanti's privileges and their chat commands (builtin/game/privileges.lua,
// builtin/game/chat.lua):
//   Copyright (C) 2010-2024 celeron55, Perttu Ahola <celeron55@gmail.com>, and Luanti contributors
// Licensed under the GNU Lesser General Public License, version 2.1 or (at
// your option) any later version: see COPYING.LESSER.

//! Privileges, as Luanti's (builtin/game/privileges.lua and chat.lua):
//! what each player may do, the server's administrator, and the ban list.
//!
//! - Every player starts with Luanti's default privileges, interact and
//!   shout. Anything more (fly, fast, noclip, teleport and so on) is granted
//!   with /grant and taken away with /revoke.
//! - The administrator has every privilege, always. Luanti names them in
//!   the "name" setting; here it is the first player to log in to a new
//!   world (or, for a world from before there was an administrator, the
//!   first to log in after the update). The name is kept in
//!   `auth/admin.txt`, which can be edited on a PC.
//! - Banned players are kept in `auth/ipban.txt`, as Luanti's ipban.txt:
//!   one `address|name` line each. A ban stops the name, and the address
//!   it last played from.
//!
//! Each player's privileges are kept in `auth/<name>.privs`, a line such as
//! `interact,shout,fly`; without the file, they have the default ones.

use std::fs;
use std::io;
use std::net::IpAddr;
use std::path::PathBuf;

use crate::storage;

/// The privileges, with Luanti's descriptions. Only ones that do something
/// here.
pub const ALL: [(&str, &str); 17] = [
    ("interact", "Can interact with things and modify the world"),
    ("shout", "Can speak in chat"),
    ("basic_privs", "Can modify basic privileges (interact, shout)"),
    ("privs", "Can modify privileges"),
    ("teleport", "Can teleport self"),
    ("bring", "Can teleport other players"),
    ("settime", "Can set the time of day using /time"),
    ("server", "Can do server maintenance stuff"),
    ("ban", "Can ban and unban players"),
    ("kick", "Can kick players"),
    ("give", "Can use /give, /giveme and /clearinv"),
    ("password", "Can use /setpassword and /clearpassword"),
    ("fly", "Can use fly mode"),
    ("fast", "Can use fast mode"),
    ("noclip", "Can fly through solid nodes using noclip mode"),
    // minetest_game's sethome mod.
    ("home", "Can use /sethome and /home"),
    // minetest_game's creative mod.
    ("creative", "Allow player to use creative inventory"),
];

/// A set of privileges: bit i is ALL[i].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Privs(u32);

impl Privs {
    pub const NONE: Privs = Privs(0);
    pub const EVERY: Privs = Privs((1 << ALL.len()) - 1);

    pub fn of(name: &str) -> Option<Privs> {
        ALL.iter().position(|(n, _)| *n == name).map(|i| Privs(1 << i))
    }

    pub fn has(self, name: &str) -> bool {
        Self::of(name).is_some_and(|p| self.0 & p.0 != 0)
    }

    pub fn with(self, other: Privs) -> Privs {
        Privs(self.0 | other.0)
    }

    pub fn without(self, other: Privs) -> Privs {
        Privs(self.0 & !other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn contains(self, other: Privs) -> bool {
        self.0 & other.0 == other.0
    }

    /// The names, in alphabetical order (as the client lists them).
    pub fn names(self) -> Vec<&'static str> {
        let mut names: Vec<&str> = ALL.iter().enumerate().filter(|(i, _)| self.0 & (1 << i) != 0).map(|(_, (n, _))| *n).collect();
        names.sort_unstable();
        names
    }

    /// "a, b, c", as core.privs_to_string(privs, ", ").
    pub fn to_text(self) -> String {
        self.names().join(", ")
    }

    /// Parse a stored line ("interact,shout"); unknown names are dropped.
    fn from_line(line: &str) -> Privs {
        line.split(',').filter_map(|n| Privs::of(n.trim())).fold(Privs::NONE, Privs::with)
    }
}

/// The server's admin has every privilege, but those not given to admins
/// (give_to_admin = false): creative.
pub fn admin_privs() -> Privs {
    Privs::EVERY.without(Privs::of("creative").unwrap())
}

/// Luanti's default_privs and basic_privs: interact and shout.
pub fn default_privs() -> Privs {
    Privs::of("interact").unwrap().with(Privs::of("shout").unwrap())
}

/// What a /grant or /revoke names: "all", or a comma-separated list (as
/// core.string_to_privs). Unknown names are the error, each as Luanti
/// words it.
pub fn parse_request(text: &str, all: Privs) -> Result<Privs, String> {
    let text = text.trim();
    if text == "all" {
        return Ok(all);
    }
    let mut privs = Privs::NONE;
    let mut unknown = Vec::new();
    for name in text.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        match Privs::of(name) {
            Some(p) => privs = privs.with(p),
            None => unknown.push(format!("Unknown privilege: {name}")),
        }
    }
    if unknown.is_empty() { Ok(privs) } else { Err(unknown.join("\n")) }
}

/// Whether someone with `caller` privileges may grant or revoke `request`:
/// "privs" allows anything, "basic_privs" only interact and shout.
pub fn may_change(caller: Privs, request: Privs, verb: &str) -> Result<(), String> {
    if caller.has("privs") {
        return Ok(());
    }
    if !caller.has("basic_privs") {
        return Err("Your privileges are insufficient.".to_string());
    }
    if !default_privs().contains(request) {
        return Err(format!(
            "Your privileges are insufficient. 'basic_privs' only allows you to {verb}: {}",
            default_privs().to_text()
        ));
    }
    Ok(())
}

/// "Privileges of X: ..." (chat.lua's privileges_of).
pub fn describe(name: &str, privs: Privs) -> String {
    if privs.is_empty() {
        format!("{name} does not have any privileges.")
    } else {
        format!("Privileges of {name}: {}", privs.to_text())
    }
}

/// A ban: the address the player last played from, and their name.
#[derive(Clone, Debug, PartialEq)]
pub struct Ban {
    pub ip: String,
    pub name: String,
}

/// Privileges, the administrator and bans, on the SD card (or flash).
pub struct Store {
    dir: PathBuf,
    admin: Option<String>,
    bans: Vec<Ban>,
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = fs::create_dir_all(&dir);
        let admin = storage::read(&dir.join("admin.txt"))
            .ok()
            .map(|d| String::from_utf8_lossy(&d).trim().to_string())
            .filter(|n| crate::auth::check_name(n).is_ok());
        let bans = storage::read(&dir.join("ipban.txt"))
            .map(|d| parse_bans(&String::from_utf8_lossy(&d)))
            .unwrap_or_default();
        Self { dir, admin, bans }
    }

    /// A store that keeps nothing on disk (for tests and the harness).
    #[allow(dead_code)]
    pub fn in_memory() -> Self {
        Self { dir: PathBuf::new(), admin: None, bans: Vec::new() }
    }

    fn on_disk(&self) -> bool {
        !self.dir.as_os_str().is_empty()
    }

    pub fn admin(&self) -> Option<&str> {
        self.admin.as_deref()
    }

    pub fn is_admin(&self, name: &str) -> bool {
        self.admin.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(name))
    }

    /// Make `name` the administrator, if there is none yet. Returns whether
    /// they now are.
    pub fn claim_admin(&mut self, name: &str) -> bool {
        if self.admin.is_some() {
            return false;
        }
        self.admin = Some(name.to_string());
        if self.on_disk() {
            if let Err(e) = storage::write(&self.dir.join("admin.txt"), format!("{name}\n").as_bytes()) {
                log::warn!("Saving the administrator's name: {e}");
            }
        }
        true
    }

    fn privs_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}.privs", name.to_ascii_lowercase()))
    }

    /// A player's privileges: every one for the administrator, otherwise
    /// what is stored, or the default ones.
    pub fn privs(&self, name: &str) -> Privs {
        if self.is_admin(name) {
            return admin_privs();
        }
        if !self.on_disk() {
            return default_privs();
        }
        match storage::read(&self.privs_path(name)) {
            Ok(d) => Privs::from_line(&String::from_utf8_lossy(&d)),
            Err(_) => default_privs(),
        }
    }

    pub fn set_privs(&self, name: &str, privs: Privs) -> io::Result<()> {
        if !self.on_disk() {
            return Ok(());
        }
        let line = privs.names().join(",") + "\n";
        storage::write(&self.privs_path(name), line.as_bytes())
    }

    /// The ban on this name or address, if any.
    pub fn ban_on(&self, name: &str, ip: IpAddr) -> Option<&Ban> {
        let ip = ip.to_string();
        self.bans.iter().find(|b| b.name.eq_ignore_ascii_case(name) || b.ip == ip)
    }

    /// "address|name, ..." (as core.get_ban_list).
    pub fn ban_list(&self) -> String {
        self.bans.iter().map(|b| format!("{}|{}", b.ip, b.name)).collect::<Vec<_>>().join(", ")
    }

    pub fn ban(&mut self, name: &str, ip: IpAddr) {
        let ip = ip.to_string();
        self.bans.retain(|b| !b.name.eq_ignore_ascii_case(name) && b.ip != ip);
        self.bans.push(Ban { ip, name: name.to_string() });
        self.save_bans();
    }

    /// Lift the bans on a name or address. Returns whether there were any.
    pub fn unban(&mut self, name_or_ip: &str) -> bool {
        let before = self.bans.len();
        self.bans.retain(|b| !b.name.eq_ignore_ascii_case(name_or_ip) && b.ip != name_or_ip);
        let changed = self.bans.len() != before;
        if changed {
            self.save_bans();
        }
        changed
    }

    fn save_bans(&self) {
        if !self.on_disk() {
            return;
        }
        let text: String = self.bans.iter().map(|b| format!("{}|{}\n", b.ip, b.name)).collect();
        if let Err(e) = storage::write(&self.dir.join("ipban.txt"), text.as_bytes()) {
            log::warn!("Saving the ban list: {e}");
        }
    }
}

fn parse_bans(text: &str) -> Vec<Ban> {
    text.lines()
        .filter_map(|l| l.trim().split_once('|'))
        .map(|(ip, name)| Ban { ip: ip.trim().to_string(), name: name.trim().to_string() })
        .filter(|b| !b.ip.is_empty() && !b.name.is_empty())
        .collect()
}

/// A teleport destination: "x,y,z" or "x y z", each a number or relative
/// to `here` ("~", "~5", "~-2"), as core.parse_coordinates.
pub fn parse_position(text: &str, here: [f32; 3]) -> Option<[f32; 3]> {
    let parts: Vec<&str> = text.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    if parts.len() != 3 {
        return None;
    }
    let mut out = [0.0; 3];
    for (i, part) in parts.iter().enumerate() {
        out[i] = match part.strip_prefix('~') {
            Some("") => here[i],
            Some(d) => here[i] + d.parse::<f32>().ok()?,
            None => part.parse::<f32>().ok()?,
        };
        if !out[i].is_finite() {
            return None;
        }
    }
    Some(out)
}

/// A new time for /time: "hh:mm", or Luanti's units (0 to 24000). The
/// error is Luanti's wording.
pub fn parse_time(text: &str) -> Result<u16, String> {
    if let Some((h, m)) = text.split_once(':') {
        let hour: u32 = h.trim().parse().map_err(|_| "Invalid time (must be between 0 and 24000).".to_string())?;
        let minute: u32 = m.trim().parse().map_err(|_| "Invalid time (must be between 0 and 24000).".to_string())?;
        if hour > 23 {
            return Err("Invalid hour (must be between 0 and 23 inclusive).".to_string());
        }
        if minute > 59 {
            return Err("Invalid minute (must be between 0 and 59 inclusive).".to_string());
        }
        return Ok(((hour * 60 + minute) * 24000 / 1440) as u16);
    }
    match text.trim().parse::<f64>() {
        Ok(t) if (0.0..=24000.0).contains(&t) => Ok((t as u32 % 24000) as u16),
        _ => Err("Invalid time (must be between 0 and 24000).".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privilege_sets() {
        let fly = Privs::of("fly").unwrap();
        let p = default_privs().with(fly);
        assert_eq!(p.to_text(), "fly, interact, shout");
        assert!(p.has("fly") && !p.has("noclip") && !p.has("nonsense"));
        assert_eq!(p.without(fly), default_privs());
        assert_eq!(Privs::from_line("shout, fly,bogus\n"), Privs::of("shout").unwrap().with(fly));
        assert_eq!(Privs::EVERY.names().len(), ALL.len());
        assert_eq!(describe("Jane", Privs::NONE), "Jane does not have any privileges.");
    }

    #[test]
    fn requests() {
        assert_eq!(parse_request("fly, fast", Privs::EVERY), Ok(Privs::of("fly").unwrap().with(Privs::of("fast").unwrap())));
        assert_eq!(parse_request("all", Privs::EVERY), Ok(Privs::EVERY));
        assert_eq!(parse_request("fly, wings", Privs::EVERY), Err("Unknown privilege: wings".to_string()));
        // basic_privs may only hand out interact and shout.
        let basic = Privs::of("basic_privs").unwrap();
        assert!(may_change(basic, default_privs(), "grant").is_ok());
        assert!(may_change(basic, Privs::of("fly").unwrap(), "grant").is_err());
        assert!(may_change(default_privs(), Privs::of("shout").unwrap(), "grant").is_err());
        assert!(may_change(Privs::of("privs").unwrap(), Privs::EVERY, "grant").is_ok());
    }

    #[test]
    fn admin_and_bans() {
        let mut s = Store::in_memory();
        assert_eq!(s.privs("Jane"), default_privs());
        assert!(s.claim_admin("Keir") && !s.claim_admin("Jane"));
        assert!(s.is_admin("keir") && s.privs("KEIR") == admin_privs() && !admin_privs().has("creative"));
        let ip: IpAddr = "192.168.4.2".parse().unwrap();
        s.ban("Griefer", ip);
        assert!(s.ban_on("griefer", "10.0.0.1".parse().unwrap()).is_some(), "by name");
        assert!(s.ban_on("Other", ip).is_some(), "by address");
        assert_eq!(s.ban_list(), "192.168.4.2|Griefer");
        assert!(s.unban("192.168.4.2") && s.ban_on("Griefer", ip).is_none() && !s.unban("Griefer"));
        assert_eq!(parse_bans("1.2.3.4|Bob\nbad line\n|x\n"), vec![Ban { ip: "1.2.3.4".into(), name: "Bob".into() }]);
    }

    #[test]
    fn positions_and_times() {
        assert_eq!(parse_position("10,-5,3.5", [0.0; 3]), Some([10.0, -5.0, 3.5]));
        assert_eq!(parse_position("~ ~10 ~-2", [1.0, 2.0, 3.0]), Some([1.0, 12.0, 1.0]));
        assert_eq!(parse_position("Jane", [0.0; 3]), None);
        assert_eq!(parse_time("6:00"), Ok(6000));
        assert_eq!(parse_time("18:30"), Ok(18500));
        assert_eq!(parse_time("12000"), Ok(12000));
        assert!(parse_time("24:00").is_err() && parse_time("25000").is_err() && parse_time("noon").is_err());
    }
}
