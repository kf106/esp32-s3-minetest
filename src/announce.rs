//! Announcing the server on the public server list (servers.luanti.org),
//! as a Luanti server with `server_announce = true` does, so players find
//! it in the client's Join Game tab.
//!
//! The settings are kept in `server.txt` on the SD card (or flash), in
//! Luanti's "name = value" style with Luanti's names where it has them,
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
//!
//! The announcing is Luanti's (src/server/serverlist.cpp, and the timer in
//! server.cpp): an HTTP POST to `<serverlist_url>/announce`, a multipart
//! form whose one field, `json`, holds the server's description. "start"
//! first, then "update" every 300 s with the players and uptime; "delete"
//! when the server stops announcing. The list then tries the game port (a
//! Luanti hello, which the server answers as any) and lists the server if
//! that works; it drops a server it hasn't heard from for 330 s, so a board
//! that loses power falls off by itself.
//!
//! The posting runs in its own thread on the server loop's core (an HTTPS
//! request takes a second or two, mostly the TLS handshake), fed by the
//! server loop through a channel (Announcer). The board posts with
//! ESP-IDF's HTTP client and its bundle of root certificates; the PC build
//! with ureq.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use log::{info, warn};

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

// ---------------------------------------------------------------------------
// Announcing
// ---------------------------------------------------------------------------

/// How often the server tells the list it is still there (Luanti's 300 s;
/// the list forgets a server after 330 s without word).
pub const EVERY: Duration = Duration::from_secs(300);
/// The first announcement, this long after start-up.
const FIRST_AFTER: Duration = Duration::from_secs(5);
/// After a failed announcement (no internet yet, the list down), sooner.
const RETRY: Duration = Duration::from_secs(60);
/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(20);
/// How long a restart waits for the "delete" to go.
const WITHDRAW_WAIT: Duration = Duration::from_secs(8);
/// The announcing thread's stack (the board's internal RAM): room for a
/// TLS handshake.
const STACK: usize = 12 * 1024;

/// What the server is like now, for an announcement.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// The players in the game.
    pub players: Vec<String>,
    pub max_players: usize,
    /// The world's age, in seconds (Luanti's game_time).
    pub game_time: u64,
    pub creative: bool,
    /// New accounts must have a password (disallow_empty_password): the
    /// list's "password" flag.
    pub passwords_required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Start,
    Update,
    Delete,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Action::Start => "start",
            Action::Update => "update",
            Action::Delete => "delete",
        }
    }
}

/// The program's name and version, as the list shows it (it takes out
/// spaces, so none): "esp32-s3-minetest-0.1.0", or "-pc-" for the PC
/// build. Not Luanti's version: this isn't Luanti.
pub fn version() -> String {
    format!("{}-{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

/// A JSON string, quoted.
fn js(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The announcement, as Luanti's ServerList::sendAnnounce builds it, with
/// what this server is. `port` is the one players use (the listening port,
/// or server_announce_port); `uptime` in seconds.
///
/// Left out, as not ours to claim: Luanti's mod list (the game is built
/// in, not Lua mods), the map generator's name (ours is v7-like, not v7),
/// and can_see_far_names. Damage is on unless creative mode is (as with
/// Luanti's enable_damage); players can't hurt each other, so no PvP.
/// "password" is Luanti's disallow_empty_password.
pub fn json(a: &Announce, port: u16, action: Action, uptime: u64, s: &Snapshot) -> String {
    let mut f: Vec<(&str, String)> = vec![("action", js(action.name())), ("port", port.to_string())];
    if !a.address.is_empty() {
        f.push(("address", js(&a.address)));
    }
    if action != Action::Delete {
        f.push(("name", js(&a.name)));
        f.push(("description", js(&a.description)));
        f.push(("version", js(&version())));
        f.push(("proto_min", crate::messages::SERVER_PROTO_MIN.to_string()));
        f.push(("proto_max", crate::messages::SERVER_PROTO_MAX.to_string()));
        if !a.url.is_empty() {
            f.push(("url", js(&a.url)));
        }
        f.push(("creative", s.creative.to_string()));
        f.push(("damage", (!s.creative).to_string()));
        f.push(("password", s.passwords_required.to_string()));
        f.push(("pvp", "false".into()));
        f.push(("uptime", uptime.to_string()));
        f.push(("gameid", js("minetest")));
        f.push(("game_time", s.game_time.to_string()));
        f.push(("clients", s.players.len().to_string()));
        f.push(("clients_max", s.max_players.to_string()));
        if a.send_players {
            let names: Vec<String> = s.players.iter().map(|n| js(n)).collect();
            f.push(("clients_list", format!("[{}]", names.join(","))));
        }
    }
    if action == Action::Start {
        f.push(("dedicated", "true".into()));
        f.push(("rollback", "false".into()));
        f.push(("privs", js("interact, shout")));
    }
    let fields: Vec<String> = f.into_iter().map(|(k, v)| format!("{}:{v}", js(k))).collect();
    format!("{{{}}}", fields.join(","))
}

/// The form Luanti posts: multipart, one field "json". Returns the
/// content type (with its boundary) and the body.
pub fn form(json: &str) -> (String, Vec<u8>) {
    let mut boundary = String::from("esp32-s3-minetest-announce");
    while json.contains(&boundary) {
        boundary.push('x');
    }
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"json\"\r\n\r\n{json}\r\n--{boundary}--\r\n"
    );
    (format!("multipart/form-data; boundary={boundary}"), body.into_bytes())
}

/// What the list said (server.py's replies).
#[derive(Debug, PartialEq)]
pub enum Reply {
    /// Taken ("Request has been filed."): the list now tries the game
    /// port, and says how that went in its reply to the next request.
    Filed,
    /// Taken, but the last request went wrong, or had a warning: what.
    /// (Not reachable: "did not respond to ping".)
    FiledBut(String),
    /// An "update" for a server the list doesn't have: start again.
    NotListed,
    /// Taken off ("Removed from server list.", or wasn't on it).
    Removed,
    /// Refused: why (bad settings, banned, too big).
    Refused(String),
    /// No answer, or an unexpected one.
    Failed(String),
}

pub fn understand(action: Action, result: Result<(u16, String), String>) -> Reply {
    let (status, text) = match result {
        Ok(r) => r,
        Err(e) => return Reply::Failed(e),
    };
    let text = text.trim().to_string();
    match (action, status) {
        (Action::Delete, 200) => Reply::Removed,
        (_, 202) => Reply::Filed,
        (_, 409) => Reply::FiledBut(text),
        (Action::Update, 200) if text.contains("not found") => Reply::NotListed,
        (_, 400 | 403 | 413) => Reply::Refused(text),
        _ => Reply::Failed(format!("HTTP {status}: {text}")),
    }
}

/// POST `body` to `url`: the reply's status and text, or what went wrong.
#[cfg(target_os = "espidf")]
fn post(url: &str, content_type: &str, body: &[u8]) -> Result<(u16, String), String> {
    use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
    use esp_idf_svc::http::Method;
    let e = |e: esp_idf_svc::sys::EspError| e.to_string();
    let mut c = EspHttpConnection::new(&Configuration {
        timeout: Some(TIMEOUT),
        // ESP-IDF's bundle of the common root certificates.
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        ..Default::default()
    })
    .map_err(e)?;
    let length = body.len().to_string();
    c.initiate_request(Method::Post, url, &[("Content-Type", content_type), ("Content-Length", length.as_str())]).map_err(e)?;
    c.write_all(body).map_err(e)?;
    c.initiate_response().map_err(e)?;
    let status = c.status();
    let mut text = Vec::new();
    let mut buf = [0u8; 256];
    while text.len() < 2048 {
        match c.read(&mut buf).map_err(e)? {
            0 => break,
            n => text.extend_from_slice(&buf[..n]),
        }
    }
    Ok((status, String::from_utf8_lossy(&text).into_owned()))
}

#[cfg(not(target_os = "espidf"))]
fn post(url: &str, content_type: &str, body: &[u8]) -> Result<(u16, String), String> {
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
    match agent.post(url).set("Content-Type", content_type).send_bytes(body) {
        Ok(r) => Ok((r.status(), r.into_string().unwrap_or_default())),
        Err(ureq::Error::Status(code, r)) => Ok((code, r.into_string().unwrap_or_default())),
        Err(e) => Err(e.to_string()),
    }
}

/// Send one announcement.
fn send(a: &Announce, port: u16, action: Action, uptime: u64, s: &Snapshot) -> Reply {
    let url = format!("{}/announce", a.list_url.trim_end_matches('/'));
    let (kind, body) = form(&json(a, port, action, uptime, s));
    understand(action, post(&url, &kind, &body))
}

enum Job {
    Announce { snapshot: Snapshot, uptime: u64 },
    /// Taken off the list; the sender hears when that is done.
    Withdraw(Sender<()>),
}

/// How an announcement went, for the server loop: whether to wait the
/// full EVERY, and a line for the status screen.
struct Outcome {
    ok: bool,
    note: Option<String>,
}

/// The announcing thread: one request at a time, as the server loop asks.
fn run(a: Announce, port: u16, jobs: Receiver<Job>, outcomes: Sender<Outcome>) {
    let mut listed = false; // whether the list has had our "start"
    let mut announced_once = false;
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Announce { snapshot, uptime } => {
                let action = if listed { Action::Update } else { Action::Start };
                let mut reply = send(&a, port, action, uptime, &snapshot);
                if reply == Reply::NotListed {
                    // The list forgot us (it restarted, or we were silent
                    // too long): start again.
                    info!("Server list: not on the list any more; announcing afresh");
                    reply = send(&a, port, Action::Start, uptime, &snapshot);
                }
                let outcome = match reply {
                    Reply::Filed => {
                        listed = true;
                        if !announced_once {
                            info!("Server list: announced \"{}\" to {}", a.name, a.list_url);
                        }
                        announced_once = true;
                        Outcome { ok: true, note: (action == Action::Start).then(|| "Server list: announced".to_string()) }
                    }
                    Reply::FiledBut(why) => {
                        listed = true;
                        warn!("Server list: {why}");
                        let note = if why.contains("did not respond to ping") {
                            // Port forwarding, most likely.
                            "Server list: can't reach us"
                        } else {
                            "Server list: see the log"
                        };
                        Outcome { ok: true, note: Some(note.to_string()) }
                    }
                    Reply::Refused(why) => {
                        listed = false;
                        warn!("Server list: refused: {why}");
                        Outcome { ok: false, note: Some("Server list: refused".into()) }
                    }
                    Reply::Failed(why) => {
                        warn!("Server list: {action:?} failed: {why}");
                        Outcome { ok: false, note: Some("Server list: no answer".into()) }
                    }
                    Reply::NotListed | Reply::Removed => Outcome { ok: false, note: None },
                };
                if outcomes.send(outcome).is_err() {
                    return;
                }
            }
            Job::Withdraw(done) => {
                if listed {
                    match send(&a, port, Action::Delete, 0, &Snapshot::default()) {
                        Reply::Removed => info!("Server list: taken off the list"),
                        other => warn!("Server list: taking it off: {other:?}"),
                    }
                    listed = false;
                }
                let _ = done.send(());
            }
        }
    }
}

/// The server loop's side of the announcing: when to announce, and the
/// thread that does it.
pub struct Announcer {
    settings: Announce,
    path: PathBuf,
    port: u16,
    jobs: Sender<Job>,
    outcomes: Receiver<Outcome>,
    born: Instant,
    next: Instant,
    /// An announcement is on its way.
    busy: bool,
}

/// What Announcer::tick did.
#[derive(Default)]
pub struct Tick {
    /// An announcement went off: the list will try the game port soon.
    pub sent: bool,
    /// A line for the status screen.
    pub note: Option<String>,
}

impl Announcer {
    /// Start announcing with these settings (from `path`, server.txt), if
    /// they say to; `listen_port` is the server's own port. None: not
    /// announcing (the reason is in the log).
    pub fn start(settings: Announce, path: PathBuf, listen_port: u16) -> Option<Self> {
        if !settings.enabled {
            return None;
        }
        let port = settings.port.unwrap_or(listen_port);
        let (jobs, job_rx) = mpsc::channel();
        let (outcome_tx, outcomes) = mpsc::channel();
        let a = settings.clone();
        if let Err(e) = crate::config_page::spawn_on_server_core(b"announce\0", STACK, move || run(a, port, job_rx, outcome_tx)) {
            warn!("Server list: can't start announcing: {e}");
            return None;
        }
        info!("Server list: announcing \"{}\" on port {port} to {} every {} s", settings.name, settings.list_url, EVERY.as_secs());
        let now = Instant::now();
        Some(Announcer { settings, path, port, jobs, outcomes, born: now, next: now + FIRST_AFTER, busy: false })
    }

    /// Called from the server loop: announce when it's time, with what
    /// `snapshot` says the server is like.
    pub fn tick(&mut self, snapshot: impl FnOnce() -> Snapshot) -> Tick {
        let mut tick = Tick::default();
        while let Ok(outcome) = self.outcomes.try_recv() {
            self.busy = false;
            self.next = Instant::now() + if outcome.ok { EVERY } else { RETRY };
            tick.note = outcome.note.or(tick.note);
        }
        if !self.busy && Instant::now() >= self.next {
            let job = Job::Announce { snapshot: snapshot(), uptime: self.born.elapsed().as_secs() };
            if self.jobs.send(job).is_ok() {
                self.busy = true;
                tick.sent = true;
            }
        }
        tick
    }

    /// Before a restart for new settings: if server.txt no longer
    /// announces this server as it is now (switched off, or a different
    /// port or address), take it off the list rather than leave it there
    /// for the list to time out. Waits a few seconds at most.
    pub fn withdraw_if_changed(&self) {
        let new = load(&self.path);
        let same = new.enabled && new.port.unwrap_or(self.port) == self.port && new.address == self.settings.address && new.list_url == self.settings.list_url;
        if same {
            return;
        }
        let (done, wait) = mpsc::channel();
        if self.jobs.send(Job::Withdraw(done)).is_ok() && wait.recv_timeout(WITHDRAW_WAIT).is_err() {
            warn!("Server list: no word on taking it off; the list drops it within a few minutes anyway");
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
    fn announcement_json() {
        let a = Announce { address: "mc.example.org".into(), send_players: true, ..good() };
        let s = Snapshot { players: vec!["Ann".into(), "Bo \"B\"".into()], max_players: 6, game_time: 5040, creative: false, passwords_required: true };
        let j = json(&a, 30001, Action::Start, 75, &s);
        for want in [r#""action":"start""#, r#""port":30001"#, r#""address":"mc.example.org""#, r#""name":"Stick""#,
                     r#""proto_min":53"#, r#""proto_max":53"#, r#""damage":true"#, r#""creative":false"#, r#""pvp":false"#, r#""password":true"#,
                     r#""uptime":75"#, r#""gameid":"minetest""#, r#""game_time":5040"#, r#""clients":2"#, r#""clients_max":6"#,
                     r#""clients_list":["Ann","Bo \"B\""]"#, r#""dedicated":true"#, r#""privs":"interact, shout""#] {
            assert!(j.contains(want), "{want} not in {j}");
        }
        assert!(!j.contains("\"url\""), "no empty url");
        let u = json(&Announce { send_players: false, ..a.clone() }, 30001, Action::Update, 375, &Snapshot { creative: true, ..s.clone() });
        assert!(u.contains(r#""action":"update""#) && u.contains(r#""damage":false"#) && !u.contains("clients_list") && !u.contains("dedicated"));
        let d = json(&a, 30001, Action::Delete, 0, &s);
        assert_eq!(d, r#"{"action":"delete","port":30001,"address":"mc.example.org"}"#);
        let (kind, body) = form(&j);
        let body = String::from_utf8(body).unwrap();
        let b = kind.split("boundary=").nth(1).unwrap();
        assert!(body.starts_with(&format!("--{b}\r\n")) && body.ends_with(&format!("\r\n--{b}--\r\n")) && body.contains(&j));
    }

    #[test]
    fn replies() {
        let ok = |s: u16, t: &str| Ok((s, t.to_string()));
        assert_eq!(understand(Action::Start, ok(202, "Request has been filed.")), Reply::Filed);
        assert!(matches!(understand(Action::Update, ok(409, "... did not respond to ping")), Reply::FiledBut(t) if t.contains("ping")));
        assert_eq!(understand(Action::Update, ok(200, "Server to update not found.")), Reply::NotListed);
        assert_eq!(understand(Action::Delete, ok(200, "Removed from server list.")), Reply::Removed);
        assert!(matches!(understand(Action::Start, ok(400, "Incorrect JSON data.")), Reply::Refused(_)));
        assert!(matches!(understand(Action::Start, Err("dns".into())), Reply::Failed(_)));
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
