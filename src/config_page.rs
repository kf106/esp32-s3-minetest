//! The settings page: a small web page for changing what the server's
//! files hold. On the board it is on the board's own hotspot only, so
//! nothing beyond the hotspot can reach it: join the hotspot (hold BOOT
//! for a second at start-up), open http://192.168.4.1, change things, and
//! save: the board saves the game and restarts, on the hotspot again or on
//! Wi-Fi. The PC build serves it on 127.0.0.1 only (the first free port
//! from 8080), without the Wi-Fi and hotspot sections, and restarts
//! itself in place on saving.
//!
//! It edits:
//! - `wifi.txt` (the board only): the networks to join, in order, and the
//!   hotspot's name and password (wifi_config.rs);
//! - `world/settings.txt`: fire, TNT, mobs and creative mode (settings.rs);
//! - `motd.txt`: the welcome message (empty: the built-in one);
//! - `server.txt`: announcing on the public server list (announce.rs).
//!   The switch stays greyed out until the details would pass the list's
//!   checks.
//!
//! Nothing changes while the server runs: the files are written, then
//! the server saves the game and restarts (Server::restart_for_settings),
//! so the files are only ever read at start-up, as before.
//!
//! The Wi-Fi networks' passwords are never sent to the page. Anyone near
//! the board can join the hotspot (its password is on the screen), and
//! they shouldn't learn the home network's password from it. A password
//! field left empty keeps the network's password; a new network with no
//! password is an open one. The hotspot's own password is shown, as the
//! screen shows it anyway.
//!
//! The server is std only (no ESP-IDF HTTP server), so the PC build runs
//! the same code (and the tests with it). It serves one connection at a
//! time, which is plenty for one person with a phone, from its own thread
//! on the server's core (see start). The page draws itself with the
//! game's textures, served from the firmware (content::embedded).

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use log::{info, warn};

use crate::announce::{self, Announce};
use crate::content;
use crate::settings::Settings;
use crate::storage;
use crate::wifi_config::{self, Network};

/// The hotspot's web port.
#[cfg_attr(not(target_os = "espidf"), allow(dead_code))] // the board only
pub const PORT: u16 = 80;

/// Set when the page has saved new settings: the server loop saves the
/// game and restarts (Server::tick). A static, so in internal RAM on the
/// board, where an atomic works between the cores.
static RESTART: AtomicBool = AtomicBool::new(false);

/// Whether the settings page has asked for a restart.
pub fn restart_requested() -> bool {
    RESTART.load(Ordering::Relaxed)
}

/// The files the page edits.
#[derive(Clone)]
pub struct Files {
    /// The Wi-Fi networks and hotspot: None in the PC build, which has no
    /// Wi-Fi of its own, and then the page leaves those sections out.
    pub wifi: Option<PathBuf>,
    pub motd: PathBuf,
    pub settings: PathBuf,
    /// The game port and announcing on the server list (announce.rs).
    pub server: PathBuf,
    /// Written to restart on the hotspot (see `take_hotspot_marker`); None
    /// in the PC build.
    pub hotspot_next: Option<PathBuf>,
    /// The game port, when not server.txt's to set (the PC build's
    /// --port): the page shows it but can't change it.
    pub fixed_port: Option<u16>,
}

impl Files {
    /// The board's page (Wi-Fi, the hotspot, two ways to restart), rather
    /// than the PC build's.
    fn board(&self) -> bool {
        self.wifi.is_some()
    }
}

/// Whether the settings page asked for the hotspot after the restart, and
/// forget it: the marker counts once.
#[cfg_attr(not(target_os = "espidf"), allow(dead_code))] // the board only
pub fn take_hotspot_marker(path: &std::path::Path) -> bool {
    let there = path.exists();
    if there {
        if let Err(e) = std::fs::remove_file(path) {
            warn!("Removing {}: {e}", path.display());
        }
    }
    there
}

/// Limits on what the page accepts.
const MAX_NETWORKS: usize = 16;
const MAX_MOTD: usize = 2000;
const MAX_HEAD: usize = 8 * 1024;
const MAX_BODY: usize = 16 * 1024;

/// Start serving the page on `bind`:`port`, in its own thread. With
/// `search`, the first free port from `port` up (the PC build: 8080 may be
/// taken). Returns the address it is at.
pub fn start(bind: IpAddr, port: u16, search: bool, files: Files) -> io::Result<SocketAddr> {
    let last = if search { port.saturating_add(99) } else { port };
    let mut tried = port;
    let listener = loop {
        match TcpListener::bind((bind, tried)) {
            Ok(l) => break l,
            Err(e) if e.kind() == io::ErrorKind::AddrInUse && tried < last => tried += 1,
            Err(e) => return Err(io::Error::new(e.kind(), format!("settings page on {bind}:{tried}: {e}"))),
        }
    };
    let addr = listener.local_addr()?;
    info!("Settings page at http://{addr}/");
    // The page itself is built on the heap.
    spawn_on_server_core(b"settings\0", 12 * 1024, move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(e) = handle(stream, &files) {
                        warn!("Settings page: {e}");
                    }
                }
                Err(e) => {
                    warn!("Settings page: {e}");
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    })?;
    Ok(addr)
}

/// Start a thread on the server loop's core (core 0), below the loop's
/// priority: the settings page's, and the server list's (announce.rs).
/// Nothing they hold is shared with the other core (an Arc in PSRAM isn't
/// safe between them). `name` ends in a NUL; `stack` comes from internal
/// RAM, which is short.
#[cfg(target_os = "espidf")]
pub fn spawn_on_server_core<F>(name: &'static [u8], stack: usize, f: F) -> io::Result<()>
where
    F: FnOnce() + Send + 'static,
{
    use esp_idf_svc::sys::{esp_pthread_get_default_config, esp_pthread_set_cfg};
    unsafe {
        let mut cfg = esp_pthread_get_default_config();
        cfg.thread_name = name.as_ptr() as *const _;
        cfg.pin_to_core = 0;
        cfg.prio = 1;
        cfg.stack_size = stack as _;
        esp_pthread_set_cfg(&cfg);
    }
    let spawned = std::thread::Builder::new().stack_size(stack).spawn(f).map(|_| ());
    unsafe {
        let cfg = esp_pthread_get_default_config();
        esp_pthread_set_cfg(&cfg);
    }
    spawned
}

#[cfg(not(target_os = "espidf"))]
pub fn spawn_on_server_core<F>(name: &'static [u8], _stack: usize, f: F) -> io::Result<()>
where
    F: FnOnce() + Send + 'static,
{
    // A PC has the room: Rust's default stack.
    let name = String::from_utf8_lossy(name.strip_suffix(b"\0").unwrap_or(name)).into_owned();
    std::thread::Builder::new().name(name).spawn(f).map(|_| ())
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

/// Read one request: the head up to its blank line, then Content-Length
/// bytes of body.
fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err(bad("request head too long"));
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(bad("connection closed mid-request"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("").to_string();
    let length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(bad("request body too long"));
    }
    let mut body = buf[head_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(Request { method, path, body })
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, extra: &str, body: &[u8]) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn handle(mut stream: TcpStream, files: &Files) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let req = read_request(&mut stream)?;
    let html = "text/html; charset=utf-8";
    let no_cache = "Cache-Control: no-store\r\n";
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => {
            let page = page(&Current::read(files), None, files.board());
            respond(&mut stream, "200 OK", html, no_cache, page.as_bytes())
        }
        ("GET", path) if path.starts_with("/t/") => match texture(&path[3..]) {
            Some(png) => respond(&mut stream, "200 OK", "image/png", "Cache-Control: max-age=86400\r\n", png),
            None => respond(&mut stream, "404 Not Found", "text/plain", "", b"Not found"),
        },
        ("POST", "/save") => {
            let form = parse_form(&req.body);
            let current = Current::read(files);
            match settle(&form, &current, files.board()) {
                Ok(new) => {
                    let saved = save(&new, files);
                    match saved {
                        Ok(()) => {
                            match (files.board(), new.restart_on_hotspot) {
                                (false, _) => info!("Settings page: saved; restarting"),
                                (true, true) => info!("Settings page: saved; restarting on the hotspot"),
                                (true, false) => info!("Settings page: saved; restarting on Wi-Fi"),
                            }
                            let page = restarting_page(&new, files.board());
                            respond(&mut stream, "200 OK", html, no_cache, page.as_bytes())?;
                            // Close our side now, so the browser shows the
                            // reply at once, and give it time to arrive
                            // before the server goes.
                            let _ = stream.shutdown(std::net::Shutdown::Write);
                            std::thread::sleep(Duration::from_millis(500));
                            RESTART.store(true, Ordering::Relaxed);
                            Ok(())
                        }
                        Err(e) => {
                            warn!("Settings page: saving: {e}");
                            let page = page(&new.as_current(), Some(&format!("Couldn't save: {e}")), files.board());
                            respond(&mut stream, "200 OK", html, no_cache, page.as_bytes())
                        }
                    }
                }
                Err((shown, why)) => {
                    let page = page(&shown, Some(&why), files.board());
                    respond(&mut stream, "200 OK", html, no_cache, page.as_bytes())
                }
            }
        }
        ("GET", "/favicon.ico") => respond(&mut stream, "200 OK", "image/png", "Cache-Control: max-age=86400\r\n", texture("default_meselamp.png").unwrap_or(&[])),
        _ => respond(&mut stream, "404 Not Found", "text/plain", "", b"Not found"),
    }
}

/// The creative-mode icon: an inner birch wood stair, drawn from the
/// game's texture by tools/make_stair_icon.py (the game has no picture of
/// a stair: clients draw stairs in 3D).
const STAIR_ICON: &[u8] = include_bytes!("web/stair_birch_inner.png");

/// A built-in texture (or the page's own icon), for the page's look.
fn texture(name: &str) -> Option<&'static [u8]> {
    if !name.ends_with(".png") || name.contains('/') {
        return None;
    }
    if name == "stair_birch_inner.png" {
        return Some(STAIR_ICON);
    }
    content::embedded(name)
}

/// `application/x-www-form-urlencoded` pairs, in order.
fn parse_form(body: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (url_decode(k), url_decode(v))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// The settings
// ---------------------------------------------------------------------------

/// What the files hold now.
struct Current {
    networks: Vec<Network>,
    hotspot: Network,
    fire: bool,
    tnt: bool,
    mobs: bool,
    creative: bool,
    no_empty_passwords: bool,
    motd: String,
    announce: Announce,
    /// Files::fixed_port.
    fixed_port: Option<u16>,
}

impl Current {
    fn read(files: &Files) -> Self {
        let text = files.wifi.as_ref().and_then(|w| storage::read(w).ok()).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default();
        let parsed = wifi_config::parse(&text);
        let settings = Settings::read(&files.settings);
        Current {
            networks: parsed.networks,
            // The file always has one after start-up (wifi_config::load).
            hotspot: parsed.hotspot.unwrap_or(Network { ssid: String::new(), password: String::new() }),
            fire: settings.fire(),
            tnt: settings.tnt(),
            mobs: settings.mobs(),
            creative: settings.creative(),
            no_empty_passwords: settings.no_empty_passwords(),
            motd: std::fs::read_to_string(&files.motd).unwrap_or_default(),
            announce: announce::read(&files.server),
            fixed_port: files.fixed_port,
        }
    }
}

/// The settings to save, from the form.
struct New {
    networks: Vec<Network>,
    hotspot: Network,
    fire: bool,
    tnt: bool,
    mobs: bool,
    creative: bool,
    no_empty_passwords: bool,
    motd: String,
    announce: Announce,
    restart_on_hotspot: bool,
    fixed_port: Option<u16>,
}

impl New {
    /// As the page shows it (after a failed save).
    fn as_current(&self) -> Current {
        Current {
            networks: self.networks.clone(),
            hotspot: self.hotspot.clone(),
            fire: self.fire,
            tnt: self.tnt,
            mobs: self.mobs,
            creative: self.creative,
            no_empty_passwords: self.no_empty_passwords,
            motd: self.motd.clone(),
            announce: self.announce.clone(),
            fixed_port: self.fixed_port,
        }
    }
}

/// Whether Wi-Fi can use this password: none (an open network), a WPA
/// passphrase of 8 to 63 characters, or a 64-digit hex key.
fn password_ok(pw: &str) -> bool {
    let hex_key = pw.len() == 64 && pw.bytes().all(|b| b.is_ascii_hexdigit());
    pw.is_empty() || hex_key || (8..64).contains(&pw.len())
}

/// Turn the form into settings to save, or say what is wrong (with the
/// form's values, to show again; passwords are never shown).
fn settle(form: &[(String, String)], current: &Current, board: bool) -> Result<New, (Current, String)> {
    let get = |name: &str| form.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    let on = |name: &str| get(name).is_some();

    // Networks: rows ssid0, pass0, orig0, ssid1... in the page's order. A
    // row with no name is dropped; an empty password keeps the password
    // of the network the row started as (orig), if any.
    let mut networks = Vec::new();
    let mut problem: Option<String> = None;
    // (The PC build has no Wi-Fi: no rows.)
    for i in 0..if board { 64 } else { 0 } {
        let Some(ssid) = get(&format!("ssid{i}")) else {
            continue;
        };
        let ssid = ssid.trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let typed = get(&format!("pass{i}")).unwrap_or("");
        let orig = get(&format!("orig{i}")).unwrap_or("");
        let password = if typed.is_empty() {
            current.networks.iter().find(|n| !orig.is_empty() && n.ssid == orig).map(|n| n.password.clone()).unwrap_or_default()
        } else {
            typed.to_string()
        };
        if ssid.len() > 32 {
            problem.get_or_insert(format!("The network name \"{ssid}\" is too long: at most 32 characters."));
        } else if !password_ok(&password) {
            problem.get_or_insert(format!("The password for \"{ssid}\" must be 8 to 63 characters (or empty for an open network)."));
        } else if networks.iter().any(|n: &Network| n.ssid == ssid) {
            problem.get_or_insert(format!("\"{ssid}\" is listed twice."));
        }
        networks.push(Network { ssid, password });
    }
    if networks.len() > MAX_NETWORKS {
        problem.get_or_insert(format!("At most {MAX_NETWORKS} networks."));
    }

    let hotspot = Network {
        ssid: get("hs_ssid").unwrap_or("").trim().to_string(),
        password: get("hs_pass").unwrap_or("").to_string(),
    };
    // (The PC build's hotspot is unused: not shown, not saved.)
    if let (true, Err(why)) = (board, wifi_config::check_hotspot(&hotspot)) {
        problem.get_or_insert(format!("The hotspot can't be used: {why}."));
    }

    let motd = get("motd").unwrap_or("").replace("\r\n", "\n");
    if motd.chars().count() > MAX_MOTD {
        problem.get_or_insert(format!("The welcome message is too long: at most {MAX_MOTD} characters."));
    }

    // The server list. One-line fields: control characters (a line break
    // from a hand-made request) would break the file.
    let line = |name: &str| get(name).unwrap_or("").chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string();

    // The game port, unless --port sets it (the PC build).
    let game_port = match current.fixed_port {
        Some(_) => current.announce.game_port,
        None => {
            let text = line("port");
            match text.parse::<u16>() {
                _ if text.is_empty() => announce::DEFAULT_PORT,
                Ok(p) if p > 0 => p,
                _ => {
                    problem.get_or_insert("The game port must be a number from 1 to 65535.".to_string());
                    current.announce.game_port
                }
            }
        }
    };
    // On a PC, another program may have the port (the board has it to
    // itself): say so now, rather than fail to restart.
    if !board && game_port != current.announce.game_port {
        if let Err(e) = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, game_port)) {
            problem.get_or_insert(format!("Port {game_port} can't be used on this computer: {e}."));
        }
    }

    let port_text = line("ann_port");
    let port = match port_text.parse::<u16>() {
        _ if port_text.is_empty() => None,
        Ok(p) if p > 0 => Some(p),
        _ => {
            problem.get_or_insert("The port players use must be a number from 1 to 65535.".to_string());
            None
        }
    };
    let announce = Announce {
        game_port,
        enabled: on("ann_on"),
        name: line("ann_name"),
        description: line("ann_desc"),
        url: line("ann_url"),
        address: line("ann_addr"),
        port,
        send_players: on("ann_players"),
        list_url: current.announce.list_url.clone(),
    };
    if announce.enabled {
        if let Err(why) = announce::check(&announce) {
            problem.get_or_insert(format!("Can't announce on the server list: {why}."));
        }
    }

    let restart_on_hotspot = board && get("restart") != Some("wifi");
    if board && !restart_on_hotspot && networks.is_empty() {
        problem.get_or_insert("There are no Wi-Fi networks to join: add one, or restart on the hotspot.".to_string());
    }

    let new = New {
        networks,
        hotspot,
        fire: on("fire"),
        tnt: on("tnt"),
        mobs: on("mobs"),
        creative: on("creative"),
        no_empty_passwords: on("nopw"),
        motd,
        announce,
        restart_on_hotspot,
        fixed_port: current.fixed_port,
    };
    match problem {
        None => Ok(new),
        Some(why) => Err((new.as_current(), why)),
    }
}

/// Write the files. The welcome message's file is removed when it is
/// empty, so the built-in one is used.
fn save(new: &New, files: &Files) -> io::Result<()> {
    if let Some(wifi) = &files.wifi {
        storage::write(wifi, wifi_config::format(&new.networks, &new.hotspot).as_bytes())?;
    }
    let mut settings = Settings::read(&files.settings);
    settings.set_all(new.fire, new.tnt, new.mobs, new.creative, new.no_empty_passwords);
    if new.motd.trim().is_empty() {
        match std::fs::remove_file(&files.motd) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    } else {
        let mut text = new.motd.trim_end().to_string();
        text.push('\n');
        storage::write(&files.motd, text.as_bytes())?;
    }
    storage::write(&files.server, announce::format(&new.announce).as_bytes())?;
    if let Some(marker) = &files.hotspot_next {
        if new.restart_on_hotspot {
            std::fs::write(marker, b"hotspot\n")?;
        } else if marker.exists() {
            std::fs::remove_file(marker)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const STYLE: &str = r#"
:root{--ink:#f2efe6;--dim:#b9b3a3;--panel:rgba(22,20,18,.88);--edge:#000;--hi:#6b6b6b;--lo:#2b2b2b;--on:#4c9a2a;--onhi:#7cc04b;--warn:#e8b23a;--bad:#d9534f}
*{box-sizing:border-box}
html{background:#2a1d12 url(/t/esp_dirt.png) repeat;background-size:64px;image-rendering:pixelated}
body{margin:0;color:var(--ink);font:16px/1.4 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;padding-bottom:120px}
img{image-rendering:pixelated}
input[type=checkbox]{accent-color:var(--on)}
header{background:url(/t/esp_grass_side.png) repeat-x;background-size:48px;height:48px;border-bottom:4px solid #000}
.title{max-width:720px;margin:0 auto;padding:18px 16px 6px;display:flex;align-items:center;gap:12px}
.title img{width:48px;height:48px}
h1{margin:0;font-size:26px;letter-spacing:.5px;text-shadow:3px 3px 0 #000}
.sub{color:var(--dim);margin:0;text-shadow:2px 2px 0 #000;font-size:14px}
main{max-width:720px;margin:0 auto;padding:8px 16px}
section{background:var(--panel);border:3px solid var(--edge);box-shadow:inset 2px 2px 0 var(--hi),inset -2px -2px 0 var(--lo);margin:16px 0;padding:14px}
h2{margin:0 0 10px;font-size:19px;display:flex;align-items:center;gap:10px;text-shadow:2px 2px 0 #000}
h2 img{width:32px;height:32px}
p.note{color:var(--dim);font-size:14px;margin:6px 0 0}
.toggles{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:10px}
.toggle{position:relative;display:block;cursor:pointer;background:#3b3b3b;border:3px solid #000;box-shadow:inset 2px 2px 0 #777,inset -2px -2px 0 #1c1c1c;padding:12px 10px 10px;text-align:center;user-select:none}
.toggle input{position:absolute;opacity:0;pointer-events:none}
.toggle img{width:48px;height:48px;display:block;margin:0 auto 6px;filter:grayscale(1) brightness(.6)}
.toggle b{display:block;font-size:16px;text-shadow:2px 2px 0 #000}
.toggle .state{display:inline-block;margin-top:6px;min-width:56px;padding:2px 8px;font-size:13px;font-weight:700;background:#222;border:2px solid #000;color:var(--dim)}
.toggle .state::after{content:"OFF"}
.toggle input:checked~img{filter:none}
.toggle input:checked~.state{background:var(--on);color:#fff;box-shadow:inset 2px 2px 0 var(--onhi)}
.toggle input:checked~.state::after{content:"ON"}
.toggle input:focus-visible~b{outline:2px dashed var(--warn)}
.toggle small{display:block;color:var(--dim);font-size:12px;margin-top:4px}
.toggle.wide{display:grid;grid-template-columns:48px 1fr auto;gap:12px;align-items:center;text-align:left;margin-bottom:6px}
.toggle.wide img{margin:0}
.toggle.wide .state{margin:0}
.toggle input:disabled~*{opacity:.45}
.toggle:has(input:disabled){cursor:not-allowed;background:#2e2e2e}
p.hint{margin:4px 0 8px;font-size:14px;color:var(--warn);min-height:1.4em}
.two{display:grid;grid-template-columns:1fr 1fr;gap:0 10px}
@media (max-width:520px){.two{grid-template-columns:1fr}}
label.field{display:block;font-size:14px;color:var(--dim);margin:8px 0 4px}
input[type=text],input[type=password],textarea{width:100%;font:inherit;color:var(--ink);background:#111;border:2px solid #000;box-shadow:inset 2px 2px 0 #000,inset -1px -1px 0 #444;padding:8px 10px;border-radius:0}
input:focus,textarea:focus{outline:2px solid var(--warn);outline-offset:0}
textarea{min-height:110px;resize:vertical}
.net{display:grid;grid-template-columns:1fr 1fr auto;gap:8px;align-items:end;padding:10px 0;border-top:2px solid #000}
.net:first-of-type{border-top:0}
.net .ctl{display:flex;gap:4px}
.btn{font:inherit;font-weight:700;color:var(--ink);cursor:pointer;background:#5a5a5a;border:3px solid #000;box-shadow:inset 2px 2px 0 #8a8a8a,inset -2px -2px 0 #333;padding:8px 14px;text-shadow:2px 2px 0 #000}
.btn:active{box-shadow:inset 2px 2px 0 #333,inset -2px -2px 0 #8a8a8a}
.btn.small{padding:4px 10px;min-width:38px}
a.btn{display:inline-block;text-decoration:none}
.btn.go{background:#3f7f24;box-shadow:inset 2px 2px 0 #73b84a,inset -2px -2px 0 #24521a}
.btn.water{background:#2a5fb0 url(/t/default_water.png);background-size:48px;box-shadow:inset 2px 2px 0 #6b9ee6,inset -2px -2px 0 #173a73}
.err{background:#5a1a17;border:3px solid #000;padding:10px 12px;margin:16px 0;box-shadow:inset 2px 2px 0 #8c3a35}
.bar{position:fixed;left:0;right:0;bottom:0;background:var(--panel);border-top:3px solid #000;padding:10px 16px}
.bar div{max-width:720px;margin:0 auto;display:flex;gap:10px;flex-wrap:wrap}
.bar .btn{flex:1 1 220px}
.busy{position:fixed;inset:0;z-index:9;display:none;align-items:center;justify-content:center;background:rgba(0,0,0,.6);padding:16px}
.busy div{background:var(--panel);border:3px solid #000;box-shadow:inset 2px 2px 0 var(--hi),inset -2px -2px 0 var(--lo);padding:18px 20px;max-width:420px;text-align:center}
.busy img{width:48px;height:48px;animation:spin 1.2s steps(8) infinite}
.busy b{display:block;font-size:19px;margin:10px 0 6px;text-shadow:2px 2px 0 #000}
.busy p{margin:0;color:var(--dim);font-size:14px}
body.saving .busy{display:flex}
@keyframes spin{to{transform:rotate(360deg)}}
@media (max-width:520px){.net{grid-template-columns:1fr}.net .ctl{justify-content:flex-end}}
"#;

const SCRIPT: &str = r#"
// The network list (the board's page only).
const list=document.getElementById('nets'),tmpl=document.getElementById('blank');
if(list){
function add(){const r=tmpl.content.firstElementChild.cloneNode(true);list.appendChild(r);r.querySelector('input').focus();}
list.addEventListener('click',e=>{const b=e.target.closest('button[data-do]');if(!b)return;e.preventDefault();const r=b.closest('.net');
if(b.dataset.do==='up'&&r.previousElementSibling)list.insertBefore(r,r.previousElementSibling);
if(b.dataset.do==='down'&&r.nextElementSibling)list.insertBefore(r.nextElementSibling,r);
if(b.dataset.do==='del')r.remove();});
document.getElementById('add').addEventListener('click',e=>{e.preventDefault();add();});
document.getElementById('show').addEventListener('change',e=>{document.getElementById('hs_pass').type=e.target.checked?'text':'password';});}
document.getElementById('f').addEventListener('submit',e=>{
if(document.body.classList.contains('saving')){e.preventDefault();return;}
if(list)list.querySelectorAll('.net').forEach((r,i)=>{
r.querySelector('[data-n=ssid]').name='ssid'+i;r.querySelector('[data-n=pass]').name='pass'+i;r.querySelector('[data-n=orig]').name='orig'+i;});
const to=e.submitter&&e.submitter.value;
document.getElementById('busy_what').textContent=to==='wifi'?'Saving, then restarting on Wi-Fi':to==='hotspot'?'Saving, then restarting on the hotspot':'Saving, then restarting';
document.body.classList.add('saving');});
function annProblem(){
const v=id=>document.getElementById(id).value.trim();
const name=v('ann_name'),desc=v('ann_desc'),url=v('ann_url'),addr=v('ann_addr').toLowerCase(),port=v('ann_port');
const ctl=/[\u0000-\u001f\u007f]/;
if(!name)return'Give the server a name to switch this on.';
if(name.length>MAXN||ctl.test(name))return'The name is too long.';
if(!desc)return'Give the server a description to switch this on.';
if(desc.length>MAXD||ctl.test(desc))return'The description is too long.';
if(url&&!/^https?:\/\/[^\s'"]+$/.test(url))return'The website must start with http:// or https://.';
if(port&&!(/^\d+$/.test(port)&&+port>=1&&+port<=65535))return'The port must be a number from 1 to 65535.';
if(addr){
if(/[\s@#\/*"']/.test(addr)||addr.startsWith('-')||/[^\x00-\x7f]/.test(addr))return'The address must be a domain name or a public IP address, without http:// or a port.';
if(addr.includes('.')&&addr.includes(':'))return'Leave the port out of the address: it has its own box.';
const m=addr.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
if(m){const a=+m[1],b=+m[2];
if(a==10||a==127||a==0||a>=224||(a==172&&b>=16&&b<32)||(a==192&&b==168)||(a==100&&b>=64&&b<128)||(a==169&&b==254))return'That is a private or local address: leave it empty to use your public one.';}
else if(!addr.includes(':')){const tld=addr.split('.').pop();
if(!addr.includes('.')||!/^[a-z]/.test(tld))return'The address must be a domain name or a public IP address.';
if(/(^localhost|\.localhost|\.local|\.internal)$/.test(addr))return'That is a local name: leave it empty to use your public address.';
if(/(\.example\.(com|net|org))$/.test(addr))return'That is an example address.';}}
return'';}
function annCheck(){const why=annProblem(),sw=document.getElementById('ann_on');
sw.disabled=!!why;if(why)sw.checked=false;document.getElementById('ann_hint').textContent=why;}
['ann_name','ann_desc','ann_url','ann_addr','ann_port'].forEach(i=>document.getElementById(i).addEventListener('input',annCheck));
annCheck();
document.getElementById('ann_on').addEventListener('change',e=>{if(e.target.checked)document.getElementById('nopw').checked=true;});
"#;

/// One network's row. `i` numbers the fields for a browser without the
/// script (which renumbers them on saving, after moving rows about).
fn network_row(i: usize, ssid: &str, known: bool) -> String {
    let placeholder = if known { "unchanged" } else { "none: an open network" };
    format!(
        r#"<div class="net"><input type="hidden" data-n="orig" name="orig{i}" value="{orig}">
<div><label class="field">Network name</label><input type="text" data-n="ssid" name="ssid{i}" value="{ssid}" maxlength="32" autocomplete="off" autocapitalize="off" spellcheck="false"></div>
<div><label class="field">Password</label><input type="password" data-n="pass" name="pass{i}" value="" maxlength="64" placeholder="{placeholder}" autocomplete="new-password"></div>
<div class="ctl"><button class="btn small" data-do="up" title="Try sooner" aria-label="Move up">&#9650;</button><button class="btn small" data-do="down" title="Try later" aria-label="Move down">&#9660;</button><button class="btn small" data-do="del" title="Remove" aria-label="Remove">&#10005;</button></div></div>"#,
        orig = if known { esc(ssid) } else { String::new() },
        ssid = esc(ssid),
    )
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn toggle(name: &str, icon: &str, title: &str, note: &str, on: bool) -> String {
    format!(
        r#"<label class="toggle"><input type="checkbox" name="{name}"{checked}><img src="/t/{icon}" alt=""><b>{title}</b><small>{note}</small><span class="state"></span></label>"#,
        checked = if on { " checked" } else { "" },
    )
}

fn page(c: &Current, error: Option<&str>, board: bool) -> String {
    // The Wi-Fi networks and the hotspot: the board's only.
    let wifi = if board {
        let mut rows = String::new();
        for (i, net) in c.networks.iter().enumerate() {
            // A network the files already know keeps its password when the
            // field is left empty.
            rows.push_str(&network_row(i, &net.ssid, true));
        }
        // One empty row to fill in, for a browser without the script.
        rows.push_str(&network_row(c.networks.len(), "", false));
        format!(
            r#"<section><h2><img src="/t/default_mese_crystal.png" alt="">Wi-Fi networks</h2>
<div id="nets">{rows}</div>
<template id="blank">{blank}</template>
<p><button class="btn" id="add">+ Add a network</button></p>
<p class="note">The board joins the first network on the list that it can see. Saved passwords are never shown here: leave a password empty to keep it.</p></section>
<section><h2><img src="/t/esp_torch_on_floor.png" alt="">The board's hotspot</h2>
<label class="field">Hotspot name</label><input type="text" name="hs_ssid" value="{hs_ssid}" maxlength="32" required autocomplete="off" autocapitalize="off" spellcheck="false">
<label class="field">Hotspot password</label><input type="password" id="hs_pass" name="hs_pass" value="{hs_pass}" maxlength="64" autocomplete="off">
<label class="field"><input type="checkbox" id="show"> Show the password</label>
<p class="note">8 to 63 characters, or empty for an open hotspot. If you change these, join the new hotspot after the restart.</p></section>
"#,
            blank = network_row(0, "", false),
            hs_ssid = esc(&c.hotspot.ssid),
            hs_pass = esc(&c.hotspot.password),
        )
    } else {
        String::new()
    };
    let (sub, buttons, busy_note, machine) = if board {
        (
            "Minetest Game on an ESP32. Changes are saved when the board restarts.",
            r#"<button class="btn go" name="restart" value="hotspot">Save and restart on the hotspot</button><button class="btn water" name="restart" value="wifi">Save and restart on Wi-Fi</button>"#,
            "Don't switch the board off. This takes a few seconds; the board then restarts and this page shows what happens next.",
            "the board",
        )
    } else {
        (
            "Minetest Game on this computer. Changes are saved when the server restarts.",
            r#"<button class="btn go" name="restart" value="pc">Save and restart the server</button>"#,
            "This takes a few seconds; the server then restarts, and this page comes back by itself.",
            "this computer",
        )
    };
    let error = error.map(|e| format!(r#"<div class="err"><b>Not saved.</b> {}</div>"#, esc(e))).unwrap_or_default();
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Server settings</title><link rel="icon" href="/t/default_meselamp.png"><style>{STYLE}</style></head><body>
<header></header>
<div class="title"><img src="/t/default_meselamp.png" alt=""><div><h1>Server settings</h1><p class="sub">{sub}</p></div></div>
<main><form id="f" method="post" action="/save">{error}
<section><h2><img src="/t/default_tool_diamondpick.png" alt="">Game rules</h2>
<div class="toggles">
{fire}{tnt}{mobs}{creative}
</div></section>
<section><h2><img src="/t/default_sign_wood.png" alt="">Welcome message</h2>
<textarea name="motd" maxlength="{MAX_MOTD}" placeholder="Leave empty for the built-in welcome.">{motd}</textarea>
<p class="note">Each line is one line in the chat when a player joins. The list of players online is added after it.</p></section>
<section><h2><img src="/t/doors_item_steel.png" alt="">Game port</h2>
<div class="two"><div><label class="field" for="port">Port (UDP)</label><input type="text" id="port" name="port" value="{game_port}" maxlength="5" inputmode="numeric" autocomplete="off" placeholder="{default_port}"{port_dis}></div></div>
<p class="note">{port_note}</p></section>
<section><h2><img src="/t/map_mapping_kit.png" alt="">Public server list</h2>
<label class="toggle wide"><input type="checkbox" id="ann_on" name="ann_on"{ann_on}{ann_dis}><img src="/t/map_mapping_kit.png" alt=""><span><b>Announce this server</b><small>List it in Luanti's Join Game tab, for anyone to find.</small></span><span class="state"></span></label>
<p class="hint" id="ann_hint">{ann_hint}</p>
<label class="toggle wide"><input type="checkbox" id="nopw" name="nopw"{nopw}><img src="/t/keys_key.png" alt=""><span><b>Require passwords</b><small>New players must choose a password, so nobody else can log in as them. Turned on with announcing.</small></span><span class="state"></span></label>
<label class="field" for="ann_name">Server name</label><input type="text" id="ann_name" name="ann_name" value="{ann_name}" maxlength="{max_name}" autocomplete="off">
<label class="field" for="ann_desc">Description</label><input type="text" id="ann_desc" name="ann_desc" value="{ann_desc}" maxlength="{max_desc}" autocomplete="off" placeholder="What players will find, and any rules">
<label class="field" for="ann_url">Website (optional)</label><input type="text" id="ann_url" name="ann_url" value="{ann_url}" maxlength="{max_url}" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="https://...">
<div class="two"><div><label class="field" for="ann_addr">Domain name (optional)</label><input type="text" id="ann_addr" name="ann_addr" value="{ann_addr}" maxlength="255" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="your public address"></div>
<div><label class="field" for="ann_port">Port players use (optional)</label><input type="text" id="ann_port" name="ann_port" value="{ann_port}" maxlength="5" inputmode="numeric" autocomplete="off" placeholder="{game_port}"></div></div>
<label class="field"><input type="checkbox" name="ann_players"{ann_players}> Show the names of the players online</label>
<p class="note">For players outside your home to join, your router must forward UDP port {game_port} to {machine} (and your internet connection must have its own public address: most mobile ones don't). Once listed, anyone can join and make an account. Leave the domain name empty unless you have one pointing at your home; the port too, unless your router forwards a different one.</p></section>
{wifi}</main>
<div class="bar"><div>{buttons}</div></div>
</form>
<div class="busy" role="status" aria-live="polite"><div><img src="/t/default_mese_crystal.png" alt=""><b id="busy_what">Saving</b><p>{busy_note}</p></div></div>
<script>const MAXN={max_name},MAXD={max_desc};{SCRIPT}</script></body></html>"#,
        fire = toggle("fire", "fire_basic_flame.png", "Fire", "Spreads and burns", c.fire),
        tnt = toggle("tnt", "tnt_side.png", "TNT", "Can be lit", c.tnt),
        mobs = toggle("mobs", "mobs_hexer_orb.png", "Mobs", "Hostile mobs spawn", c.mobs),
        creative = toggle("creative", "stair_birch_inner.png", "Creative", "Unlimited blocks", c.creative),
        motd = esc(&c.motd),
        game_port = c.announce.game_port,
        default_port = announce::DEFAULT_PORT,
        port_dis = if c.fixed_port.is_some() { " disabled" } else { "" },
        port_note = match c.fixed_port {
            Some(p) => format!("Set to {p} by <code>--port</code> when the server was started, which takes the place of this setting."),
            None => format!("The port players connect to. Luanti fills in {}, its usual one; with another, players type it in Join Game after the address. Port forwarding on your router must follow it.", announce::DEFAULT_PORT),
        },
        nopw = if c.no_empty_passwords { " checked" } else { "" },
        ann_on = if c.announce.enabled && announce::check(&c.announce).is_ok() { " checked" } else { "" },
        ann_dis = if announce::check(&c.announce).is_err() { " disabled" } else { "" },
        ann_hint = announce::check(&c.announce).err().map(|why| esc(&capitalise(&why))).unwrap_or_default(),
        ann_name = esc(&c.announce.name),
        ann_desc = esc(&c.announce.description),
        ann_url = esc(&c.announce.url),
        ann_addr = esc(&c.announce.address),
        ann_port = c.announce.port.map(|p| p.to_string()).unwrap_or_default(),
        ann_players = if c.announce.send_players { " checked" } else { "" },
        max_name = announce::MAX_NAME,
        max_desc = announce::MAX_DESCRIPTION,
        max_url = announce::MAX_URL,
    )
}

fn restarting_page(new: &New, board: bool) -> String {
    let comes_back = !board || new.restart_on_hotspot;
    let (what, then) = if !board {
        (
            "The server is restarting with the new settings.".to_string(),
            "Players connected are told, and can reconnect in a few seconds. This page comes back by itself.".to_string(),
        )
    } else if new.restart_on_hotspot {
        (
            format!("The board is restarting on its hotspot, <b>{}</b>.", esc(&new.hotspot.ssid)),
            "In half a minute or so, rejoin the hotspot if your phone or computer has left it. This page goes back to the settings by itself once the board answers.".to_string(),
        )
    } else {
        let first = new.networks.first().map(|n| esc(&n.ssid)).unwrap_or_default();
        (
            "The board is restarting on Wi-Fi.".to_string(),
            format!("This hotspot goes away. The board joins the first network it can see (<b>{first}</b> first); its screen shows its new address."),
        )
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Restarting</title><style>{STYLE}{inline}</style></head><body><header></header>
<div class="title"><img src="{lamp}" alt=""><div><h1>Saved</h1><p class="sub">The game is saved before the restart.</p></div></div>
<main><section><h2><img src="{crystal}" alt="">Restarting</h2><p>{what}</p><p class="note">{then}</p>{back}</section></main>
<script>
// A refresh asks for the settings again, rather than sending the form a
// second time.
history.replaceState(null,'','/');
{poll}</script></body></html>"#,
        // Where the page will be there again (the PC build, or the board
        // back on its hotspot): a way back, and a check every 1.5 s that
        // goes back by itself once the server answers. Not at once: the
        // old server answers until it stops.
        back = if comes_back {
            r#"<p class="hint" id="wait"></p><p><a class="btn go" href="/">Back to the settings</a></p>"#.to_string()
        } else {
            String::new()
        },
        poll = if comes_back {
            format!(
                "const w=document.getElementById('wait');let n=0;\
                 setTimeout(()=>{{const t=setInterval(()=>{{n++;w.textContent='Waiting for the {who} to come back'+'.'.repeat(n%4);\
                 fetch('/',{{cache:'no-store'}}).then(r=>{{if(r.ok){{clearInterval(t);w.textContent='It is back. Opening the settings...';location.replace('/');}}}}).catch(()=>{{}});}},1500);}},4000);",
                who = if board { "board" } else { "server" },
            )
        } else {
            String::new()
        },
        // The server is gone by the time the browser asks for textures, so
        // this page carries its own.
        inline = format!(
            "html{{background-image:url({})}}header{{background-image:url({})}}",
            data_url("esp_dirt.png"),
            data_url("esp_grass_side.png")
        ),
        lamp = data_url("default_meselamp.png"),
        crystal = data_url("default_mese_crystal.png"),
    )
}

/// A built-in texture as a `data:` URL, for a page that must not need
/// the server once it has arrived.
fn data_url(name: &str) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let data = texture(name).unwrap_or(&[]);
    let mut out = String::from("data:image/png;base64,");
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(ssid: &str, password: &str) -> Network {
        Network { ssid: ssid.into(), password: password.into() }
    }

    fn current() -> Current {
        Current {
            networks: vec![net("Home", "secret123"), net("Cafe", "")],
            hotspot: net("MTG-0BC4", "minetest"),
            fire: true,
            tnt: true,
            mobs: false,
            creative: false,
            no_empty_passwords: false,
            motd: String::new(),
            announce: Announce::default(),
            fixed_port: None,
        }
    }

    #[test]
    fn game_port() {
        let c = current();
        let with = |port: &str| form(&[("ssid0", "Home"), ("hs_ssid", "Den"), ("hs_pass", "drawbridge"), ("restart", "wifi"), ("port", port)]);
        assert_eq!(settle(&with("30001"), &c, true).ok().unwrap().announce.game_port, 30001);
        assert_eq!(settle(&with(""), &c, true).ok().unwrap().announce.game_port, 30000);
        for bad in ["0", "65536", "abc", "-1"] {
            assert!(settle(&with(bad), &c, true).err().unwrap().1.contains("game port"), "{bad}");
        }
        // Set by --port: shown, not changed.
        let fixed = Current { fixed_port: Some(30002), ..current() };
        assert_eq!(settle(&with("30009"), &fixed, true).ok().unwrap().announce.game_port, 30000);
        let html = page(&fixed, None, false);
        assert!(html.contains(r#"name="port" value="30000" maxlength="5" inputmode="numeric" autocomplete="off" placeholder="30000" disabled>"#));
        assert!(html.contains("Set to 30002 by <code>--port</code>"));
        let c = Current { announce: Announce { game_port: 30005, ..Announce::default() }, ..current() };
        let html = page(&c, None, true);
        assert!(html.contains(r#"name="port" value="30005""#) && html.contains("forward UDP port 30005 to"), "the page uses the game port");
    }

    fn form(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn decodes_forms() {
        let f = parse_form(b"ssid0=My+Home%21&pass0=a%26b%3Dc&motd=line1%0D%0Aline2&fire=on");
        assert_eq!(f[0], ("ssid0".into(), "My Home!".into()));
        assert_eq!(f[1], ("pass0".into(), "a&b=c".into()));
        assert_eq!(f[2], ("motd".into(), "line1\r\nline2".into()));
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
        assert_eq!(url_decode("%C3%A4"), "ä");
    }

    #[test]
    fn keeps_unchanged_passwords_and_order() {
        let f = form(&[
            ("ssid0", "Cafe"), ("pass0", ""), ("orig0", "Cafe"),
            ("ssid1", "Home"), ("pass1", ""), ("orig1", "Home"),
            ("ssid2", "Office"), ("pass2", "hunter22"), ("orig2", ""),
            ("ssid3", ""), ("pass3", ""), ("orig3", ""),
            ("hs_ssid", "Den"), ("hs_pass", "drawbridge"),
            ("fire", "on"), ("creative", "on"), ("motd", "Hi\r\nthere"),
            ("restart", "wifi"),
        ]);
        let new = settle(&f, &current(), true).ok().unwrap();
        assert_eq!(new.networks, vec![net("Cafe", ""), net("Home", "secret123"), net("Office", "hunter22")]);
        assert_eq!(new.hotspot, net("Den", "drawbridge"));
        assert_eq!((new.fire, new.tnt, new.mobs, new.creative), (true, false, false, true));
        assert_eq!(new.motd, "Hi\nthere");
        assert!(!new.restart_on_hotspot);
    }

    #[test]
    fn renamed_network_keeps_its_password() {
        let f = form(&[("ssid0", "Home 5G"), ("pass0", ""), ("orig0", "Home"), ("hs_ssid", "Den"), ("hs_pass", "")]);
        let new = settle(&f, &current(), true).ok().unwrap();
        assert_eq!(new.networks, vec![net("Home 5G", "secret123")]);
        assert!(new.restart_on_hotspot);
    }

    #[test]
    fn refuses_bad_settings() {
        let base = [("hs_ssid", "Den"), ("hs_pass", "drawbridge")];
        let bad = |extra: &[(&str, &str)]| {
            let mut f = form(&base);
            f.extend(form(extra));
            settle(&f, &current(), true).err().map(|(_, why)| why)
        };
        assert!(bad(&[("ssid0", "Office"), ("pass0", "short")]).unwrap().contains("8 to 63"));
        assert!(bad(&[("ssid0", "A"), ("ssid1", "A")]).unwrap().contains("twice"));
        assert!(bad(&[("ssid0", &"x".repeat(33))]).unwrap().contains("too long"));
        assert!(bad(&[("restart", "wifi")]).unwrap().contains("no Wi-Fi networks"));
        let mut f = form(&[("hs_ssid", "Den"), ("hs_pass", "short")]);
        f.push(("ssid0".into(), "Home".into()));
        assert!(settle(&f, &current(), true).err().unwrap().1.contains("hotspot"));
        // Fine: an open network, a hex key.
        assert!(bad(&[("ssid0", "Open"), ("ssid1", "Hex"), ("pass1", &"a".repeat(64))]).is_none());
    }

    #[test]
    fn base64() {
        // The textures' data URLs: standard base64, padded.
        let url = data_url("esp_dirt.png");
        assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"));
        assert_eq!((url.len() - "data:image/png;base64,".len()) % 4, 0);
    }

    #[test]
    fn pc_page_has_no_wifi() {
        // No Wi-Fi rows or hotspot to check, and one way to restart.
        let f = form(&[("ssid0", "x"), ("pass0", "short"), ("hs_ssid", ""), ("fire", "on"), ("restart", "pc")]);
        let new = settle(&f, &current(), false).ok().unwrap();
        assert!(new.networks.is_empty() && !new.restart_on_hotspot && new.fire);
        let html = page(&current(), None, false);
        assert!(!html.contains("Wi-Fi networks") && !html.contains("The board's hotspot") && html.contains("Save and restart the server"));
        assert!(html.contains("to this computer"));
        let html = page(&current(), None, true);
        assert!(html.contains("Wi-Fi networks") && html.contains("Save and restart on the hotspot") && html.contains("to the board"));
    }

    #[test]
    fn page_never_shows_network_passwords() {
        let c = current();
        let html = page(&c, Some("<oops>"), true);
        assert!(!html.contains("secret123"));
        assert!(html.contains("minetest")); // the hotspot's, as on the screen
        assert!(html.contains("&lt;oops&gt;"));
        assert!(html.contains(r#"value="Home""#));
    }

    #[test]
    fn saves_files() {
        let dir = std::env::temp_dir().join(format!("config_page_test_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("world")).unwrap();
        let files = Files {
            wifi: Some(dir.join("wifi.txt")),
            motd: dir.join("motd.txt"),
            settings: dir.join("world/settings.txt"),
            server: dir.join("server.txt"),
            hotspot_next: Some(dir.join("boot-hotspot")),
            fixed_port: None,
        };
        let new = New {
            networks: vec![net("Home", "secret123")],
            hotspot: net("Den", "drawbridge"),
            fire: false,
            tnt: true,
            mobs: true,
            creative: false,
            no_empty_passwords: true,
            motd: "Welcome!\n".into(),
            announce: Announce { enabled: true, name: "Stick".into(), description: "Small".into(), game_port: 30004, ..Announce::default() },
            restart_on_hotspot: true,
            fixed_port: None,
        };
        save(&new, &files).unwrap();
        let parsed = wifi_config::parse(&std::fs::read_to_string(files.wifi.as_ref().unwrap()).unwrap());
        assert_eq!(parsed.networks, new.networks);
        assert_eq!(parsed.hotspot, Some(new.hotspot.clone()));
        let s = Settings::open(&files.settings);
        assert_eq!((s.fire(), s.tnt(), s.mobs(), s.creative(), s.no_empty_passwords()), (false, true, true, false, true));
        assert_eq!(std::fs::read_to_string(&files.motd).unwrap(), "Welcome!\n");
        assert_eq!(announce::read(&files.server), new.announce);
        let marker = files.hotspot_next.clone().unwrap();
        assert!(take_hotspot_marker(&marker));
        assert!(!take_hotspot_marker(&marker));
        // An empty message goes back to the built-in one.
        let new = New { motd: "  ".into(), restart_on_hotspot: false, ..new };
        save(&new, &files).unwrap();
        assert!(!files.motd.exists());
        assert!(!marker.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
