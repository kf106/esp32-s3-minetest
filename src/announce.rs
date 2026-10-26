//! Announcing the server on the public server list (servers.luanti.org),
//! as a Luanti server with `server_announce = true` does, so players find
//! it in the client's Join Game tab.
//!
//! For now, the settings: kept in `server.txt` on the SD card (or flash),
//! in Luanti's "name = value" style with Luanti's names where it has them,
//! changed on the settings page (config_page.rs) or by hand:
//!
//! ```text
//! server_announce = false
//! server_name = Minetest on a Stick
//! server_description = A small world on an ESP32. Be nice.
//! server_url = https://example.org/our-server
//! server_address = mc.example.org
//! server_announce_port = 30000
//! server_announce_send_players = true
//! serverlist_url = https://servers.luanti.org
//! ```
//!
//! - `server_name` and `server_description`: required to announce.
//! - `server_url`: optional, a web page about the server (http or https).
//! - `server_address`: optional, a domain name players connect to. Left
//!   out, the list uses the address the announcement came from, which is
//!   right for most homes. No port, no "http://".
//! - `server_announce_port` (ours): the port players connect to from
//!   outside, if the router forwards a different one to the board's
//!   30000. Luanti always announces the port it listens on.
//! - `server_announce_send_players`: whether the list shows the players'
//!   names, or only how many there are (Luanti's, default true).
//! - `serverlist_url`: Luanti's, for a list of your own; rarely changed.
//!
//! The checks are the list server's own (luanti-org/serverlist, server.py:
//! checkRequestAddress and checkRequest), so a setting it would refuse is
//! refused here, where the user can see why, rather than by the list.
//! With anything wrong, the server doesn't announce (and says why).

use std::net::IpAddr;
use std::path::Path;

use log::warn;

use crate::storage;

pub const DEFAULT_LIST_URL: &str = "https://servers.luanti.org";
pub const MAX_NAME: usize = 80;
pub const MAX_DESCRIPTION: usize = 400;
pub const MAX_URL: usize = 200;
const MAX_ADDRESS: usize = 255;

#[derive(Clone, Debug, PartialEq)]
pub struct Announce {
    /// Whether to announce (the switch).
    pub enabled: bool,
    pub name: String,
    pub description: String,
    /// Empty: none.
    pub url: String,
    /// Empty: the address the announcement comes from.
    pub address: String,
    /// None: the port the server listens on.
    pub port: Option<u16>,
    pub send_players: bool,
    pub list_url: String,
}

impl Default for Announce {
    fn default() -> Self {
        Announce {
            enabled: false,
            name: String::new(),
            description: String::new(),
            url: String::new(),
            address: String::new(),
            port: None,
            send_players: true,
            list_url: DEFAULT_LIST_URL.to_string(),
        }
    }
}

fn flag(v: &str) -> bool {
    matches!(v.trim(), "true" | "yes" | "1" | "on")
}

/// Read `server.txt`'s text. Unknown names are warned about and skipped.
pub fn parse(text: &str) -> Announce {
    let mut a = Announce::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            warn!("server.txt line {}: no \"=\"", n + 1);
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "server_announce" => a.enabled = flag(&v),
            "server_name" => a.name = v,
            "server_description" => a.description = v,
            "server_url" => a.url = v,
            "server_address" => a.address = v,
            "server_announce_port" => {
                a.port = match v.parse::<u16>() {
                    Ok(p) if p > 0 => Some(p),
                    _ if v.is_empty() => None,
                    _ => {
                        warn!("server.txt line {}: server_announce_port must be 1 to 65535", n + 1);
                        None
                    }
                }
            }
            "server_announce_send_players" => a.send_players = flag(&v),
            "serverlist_url" => a.list_url = if v.is_empty() { DEFAULT_LIST_URL.to_string() } else { v },
            other => warn!("server.txt line {}: unknown setting \"{other}\"", n + 1),
        }
    }
    a
}

const HEADER: &str = "\
# Announcing this server on the public server list (servers.luanti.org),
# where Luanti's Join Game tab finds it. Change it on the settings page
# (the board's hotspot, then http://192.168.4.1) or here.
#
# To be listed, the server must be reachable from the internet: forward
# UDP port 30000 on your router to the board. Anyone can then join and
# make an account.
";

/// The file's text.
pub fn format(a: &Announce) -> String {
    let mut t = String::from(HEADER);
    t.push_str(&format!("\nserver_announce = {}\n", a.enabled));
    t.push_str(&format!("server_name = {}\n", a.name));
    t.push_str(&format!("server_description = {}\n", a.description));
    t.push_str("\n# Optional: a web page, a domain name, and the port players use from\n# outside if not the board's own (30000).\n");
    t.push_str(&format!("server_url = {}\n", a.url));
    t.push_str(&format!("server_address = {}\n", a.address));
    t.push_str(&format!("server_announce_port = {}\n", a.port.map(|p| p.to_string()).unwrap_or_default()));
    t.push_str("\n# Show the players' names on the list (otherwise only how many).\n");
    t.push_str(&format!("server_announce_send_players = {}\n", a.send_players));
    if a.list_url != DEFAULT_LIST_URL {
        t.push_str(&format!("\nserverlist_url = {}\n", a.list_url));
    }
    t
}

/// The settings in `path`: the defaults if there is no file.
pub fn read(path: &Path) -> Announce {
    match storage::read(path) {
        Ok(d) => parse(&String::from_utf8_lossy(&d)),
        Err(_) => Announce::default(),
    }
}

/// The settings to use at start-up: as `read`, but not announcing if the
/// details are incomplete or wrong (with a warning saying why).
pub fn load(path: &Path) -> Announce {
    let mut a = read(path);
    if a.enabled {
        if let Err(why) = check(&a) {
            warn!("Not announcing on the server list: {why} (in {})", path.display());
            a.enabled = false;
        }
    }
    a
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// The list server's characters that may not appear in URLs, names and
/// such (server.py's BAD_CHARS).
fn has_bad_char(s: &str) -> bool {
    s.chars().any(|c| matches!(c, ' ' | '\t' | '\x0b' | '\r' | '\n' | '\0' | '\''))
}

/// Whether the details are complete and the list would take them; if
/// not, what to fix. (The switch itself doesn't matter here.)
pub fn check(a: &Announce) -> Result<(), String> {
    let name = a.name.trim();
    if name.is_empty() {
        return Err("the server needs a name".into());
    }
    if name.chars().count() > MAX_NAME || has_control(name) {
        return Err(format!("the name must be one line of at most {MAX_NAME} characters"));
    }
    let desc = a.description.trim();
    if desc.is_empty() {
        return Err("the server needs a description".into());
    }
    if desc.chars().count() > MAX_DESCRIPTION || has_control(desc) {
        return Err(format!("the description must be one line of at most {MAX_DESCRIPTION} characters"));
    }
    if !a.url.is_empty() {
        check_url(&a.url).map_err(|why| format!("the website {why}"))?;
    }
    if !a.address.is_empty() {
        check_address(&a.address)?;
    }
    if a.port == Some(0) {
        return Err("the port must be 1 to 65535".into());
    }
    check_url(&a.list_url).map_err(|why| format!("serverlist_url {why}"))?;
    Ok(())
}

fn check_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("must start with http:// or https://".into());
    }
    if url.len() > MAX_URL || has_bad_char(url) || url.len() <= "https://".len() {
        return Err(format!("must be a web address of at most {MAX_URL} characters, without spaces or quotes"));
    }
    Ok(())
}

/// server.py's checkRequestAddress, with REJECT_PRIVATE_ADDRESSES on (as
/// the public list has it).
fn check_address(address: &str) -> Result<(), String> {
    let name = address.trim().to_lowercase();
    let invalid = || "the address must be a domain name (like mc.example.org) or a public IP address, without a port or http://".to_string();
    let private = || "the address is a private or local one, which only works on your own network: leave it empty to use your public address".to_string();
    if name == "game.minetest.net" || [".example.com", ".example.net", ".example.org"].iter().any(|s| name.ends_with(s)) {
        return Err("the address is an example one".into());
    }
    if name.len() > MAX_ADDRESS || name.contains(|c| " @#/*\"'\t\x0b\r\n\0".contains(c)) || name.starts_with('-') {
        return Err(invalid());
    }
    if !name.is_ascii() {
        return Err("the address must use the punycode form of an international domain name (xn--...)".into());
    }
    if (name.contains('.') && name.contains(':')) || (name.contains(':') && name.contains('[')) {
        return Err(invalid());
    }
    let is_domain = match name.rsplit_once('.') {
        Some((a, b)) => !a.is_empty() && b.chars().next().is_some_and(|c| c.is_ascii_alphabetic()),
        None => false,
    };
    if is_domain {
        if name == "localhost" || [".localhost", ".local", ".internal"].iter().any(|s| name.ends_with(s)) {
            return Err(private());
        }
        return Ok(());
    }
    match name.parse::<IpAddr>() {
        Ok(ip) if ip.is_multicast() || is_link_local(&ip) => Err(invalid()),
        Ok(ip) if !is_global(&ip) => Err(private()),
        Ok(_) => Ok(()),
        Err(_) => Err(invalid()),
    }
}

fn is_link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_link_local(),
        IpAddr::V6(v) => (v.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// Python's ipaddress `is_global`, for the cases that matter: not
/// private, shared (carrier-grade NAT), loopback, unspecified,
/// documentation, benchmarking or reserved.
fn is_global(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (18..20).contains(&o[1]))
                || o[0] >= 240
                || (o[0] == 192 && o[1] == 0 && o[2] == 0))
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            !(v.is_loopback()
                || v.is_unspecified()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || v.to_ipv4_mapped().is_some())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Announce {
        Announce { name: "Stick".into(), description: "A small world".into(), ..Announce::default() }
    }

    #[test]
    fn round_trip() {
        let a = Announce {
            enabled: true,
            name: "Minetest on a Stick".into(),
            description: "A small world on an ESP32. Be nice.".into(),
            url: "https://example.org/x".into(),
            address: "mc.example.org".into(),
            port: Some(30001),
            send_players: false,
            list_url: DEFAULT_LIST_URL.into(),
        };
        assert_eq!(parse(&format(&a)), a);
        assert_eq!(parse(&format(&Announce::default())), Announce::default());
        let custom = Announce { list_url: "http://list.lan".into(), ..a };
        assert_eq!(parse(&format(&custom)), custom);
    }

    #[test]
    fn needs_name_and_description() {
        assert!(check(&good()).is_ok());
        assert!(check(&Announce { name: " ".into(), ..good() }).unwrap_err().contains("name"));
        assert!(check(&Announce { description: "".into(), ..good() }).unwrap_err().contains("description"));
        assert!(check(&Announce { name: "x".repeat(MAX_NAME + 1), ..good() }).is_err());
    }

    #[test]
    fn urls() {
        assert!(check(&Announce { url: "https://example.org/a?b=c".into(), ..good() }).is_ok());
        assert!(check(&Announce { url: "example.org".into(), ..good() }).is_err());
        assert!(check(&Announce { url: "https://exa mple.org".into(), ..good() }).is_err());
        assert!(check(&Announce { url: "https://".into(), ..good() }).is_err());
    }

    #[test]
    fn addresses_as_the_list_checks_them() {
        let ok = |s: &str| check(&Announce { address: s.into(), ..good() }).is_ok();
        assert!(ok("mc.example.co.uk") && ok("play.zalmanac.com") && ok("8.8.8.8") && ok("2a00:1450:4001::1"));
        // A port, a scheme, an example, private and local ones, garbage.
        for bad in ["mc.example.com", "1.2.3.4:30000", "http://x.org", "192.168.1.10", "10.0.0.1", "100.70.1.1", "127.0.0.1",
                    "fd00::1", "localhost", "box.local", "nodot", "-x.org", "1.2.3", "x.org/a", "[::1]:30000", "pölö.fi", "224.0.0.1"] {
            assert!(!ok(bad), "{bad} accepted");
        }
    }
}
