//! What the server reports to the status screen. The server updates it as
//! things happen; the display thread copies it once a second.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// How many recent events the screen keeps.
const EVENTS: usize = 3;
/// A QR code, as rows of modules (true for dark), `size` on a side.
pub struct Qr {
    pub size: usize,
    pub dark: Vec<bool>,
}

impl Qr {
    pub fn dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.dark[y * self.size + x]
    }
}

/// The board's own hotspot, when it runs one: what players need to join
/// it, for the Join page.
pub struct Hotspot {
    pub ssid: String,
    /// Empty for an open hotspot.
    pub password: String,
    /// The server's address on the hotspot.
    pub ip: String,
    /// The phone camera's Wi-Fi join code, if it could be made.
    pub qr: Option<Qr>,
}

#[derive(Clone)]
pub struct Status {
    pub started: Instant,
    /// "address:port" clients connect to.
    pub address: String,
    /// Players in the game, in joining order.
    pub players: Vec<String>,
    /// Connections still logging in or loading.
    pub joining: usize,
    pub packets_in: u64,
    pub packets_out: u64,
    pub blocks_sent: u64,
    /// Nodes dug or placed since the server started.
    pub edits: u64,
    /// Recent happenings, newest last.
    pub events: VecDeque<String>,
    /// Time the server loop spent working rather than waiting for
    /// packets, in microseconds since start.
    pub busy_us: u64,
    /// Map blocks built (generated and compressed), and the time that took
    /// in microseconds. Blocks are built once and may be sent to several
    /// players.
    pub blocks_built: u64,
    pub block_us: u64,
    /// Packets that couldn't be sent (Wi-Fi transmit buffers full).
    pub send_failures: u64,
    /// Wi-Fi power saving is on (nobody connected).
    pub wifi_dozing: bool,
    /// Running counts of players joining and leaving and of chat lines,
    /// for the LED to flash on.
    pub joins: u64,
    pub leaves: u64,
    pub chats: u64,
    /// Set when the board runs its own hotspot rather than joining Wi-Fi.
    pub hotspot: Option<Arc<Hotspot>>,
    /// While BOOT is watched at start-up (for a hotspot or the USB drive),
    /// and while the board is a USB drive: the button doesn't page through
    /// the screens then, and the screen stays on.
    #[cfg_attr(not(target_os = "espidf"), allow(dead_code))]
    pub boot_window: bool,
    /// How long BOOT has been held at start-up, shown as a bar along the
    /// bottom of the screen: 0 to 1 towards a hotspot (blue), then 1 to 2
    /// towards the USB drive (orange). It stays at 1 from when the hotspot
    /// is chosen until it is up.
    pub boot_hold: Option<f32>,
    /// Set while the board is a USB drive (or failed to become one): the
    /// screen shows its own page then.
    pub usb_drive: Option<UsbDrive>,
    /// Whether the log is kept in files (filelog.rs), for the Logs page.
    pub log_kept: bool,
    /// The Logs page's download page (config_page.rs, log_server), as
    /// lcd.rs shows it: how far BOOT has been held towards opening it (0
    /// to 1), and, while it is open, its address and how long it has left.
    pub log_hold: Option<f32>,
    pub log_server: Option<LogServer>,
}

/// The log download page, for the Logs page.
#[derive(Clone)]
pub struct LogServer {
    /// What to type in a browser: "192.168.68.136/K7Q3XZ".
    pub address: String,
    pub left_secs: u32,
}

/// The USB drive, for its screen.
#[derive(Clone)]
#[cfg_attr(not(target_os = "espidf"), allow(dead_code))]
pub struct UsbDrive {
    /// What the PC sees: "SD card" or "Flash".
    pub what: &'static str,
    /// Its size in bytes.
    pub bytes: u64,
    /// The files and folders at the top, as they were when it started
    /// (folders end in "/", and come first).
    pub files: Vec<String>,
    /// Why it couldn't start.
    pub error: Option<String>,
    /// Ejected on the PC: the board is restarting.
    pub ejected: bool,
}

/// Problems (such as failing to save to the SD card), counted from
/// anywhere in the server, for the LED to flash red on.
/// (32 bits: the ESP32's Xtensa cores have no 64-bit atomic operations.)
static PROBLEMS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Note that something went wrong (it should also be logged).
pub fn problem() {
    PROBLEMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn problems() -> u64 {
    PROBLEMS.load(std::sync::atomic::Ordering::Relaxed) as u64
}

/// Set at start-up when there is no SD card and the game is kept in the
/// board's own flash instead.
static ON_FLASH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_on_flash() {
    ON_FLASH.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn on_flash() -> bool {
    ON_FLASH.load(std::sync::atomic::Ordering::Relaxed)
}

pub type SharedStatus = Arc<Mutex<Status>>;

impl Status {
    pub fn new(address: String) -> SharedStatus {
        Arc::new(Mutex::new(Self {
            started: Instant::now(),
            address,
            players: Vec::new(),
            joining: 0,
            packets_in: 0,
            packets_out: 0,
            blocks_sent: 0,
            edits: 0,
            events: VecDeque::new(),
            busy_us: 0,
            blocks_built: 0,
            block_us: 0,
            send_failures: 0,
            wifi_dozing: false,
            joins: 0,
            leaves: 0,
            chats: 0,
            hotspot: None,
            boot_window: false,
            boot_hold: None,
            usb_drive: None,
            log_kept: false,
            log_hold: None,
            log_server: None,
        }))
    }

    /// A line of public chat, counted for the LED. (The chat itself is in
    /// the log.)
    pub fn chat_line(&mut self) {
        self.chats += 1;
    }

    pub fn event(&mut self, text: String) {
        if self.events.len() == EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(text);
    }
}

/// Lock the status, carrying on even if a thread panicked while holding it
/// (the data is only for display).
pub fn lock(s: &SharedStatus) -> std::sync::MutexGuard<'_, Status> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}
