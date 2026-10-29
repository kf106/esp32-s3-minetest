//! The log, kept in files as well as on the serial port: `logs/` on the SD
//! card (or the flash, or the PC build's data folder), one file a day,
//! `logs/2026-10-28.log`, with each line stamped with the date and time
//! (UTC). Switched on with `log_to_file = true` in server.txt (announce.rs),
//! on the settings page, and read at start-up.
//!
//! The board's clock is only right once it has been set from the internet
//! (SNTP, main.rs); until then, and on the hotspot (no internet), lines
//! carry no date and go to `logs/undated.log`, after a start-up marker.
//!
//! Lines are gathered in a small buffer in internal RAM, from whichever
//! task logs (Wi-Fi, lwIP and the rest log too), and written out by a
//! thread of their own every few seconds, so a task that logs never waits
//! for the card. If the buffer fills before then, lines are dropped and
//! the log says how many. The newest KEEP_DAYS days are kept.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Days of log files kept.
pub const KEEP_DAYS: usize = 14;
/// The buffer between the tasks that log and the writer.
const BUFFER: usize = 4096;
/// Written out when this much is waiting, or after FLUSH_EVERY.
const FLUSH_AT: usize = BUFFER / 2;
const FLUSH_EVERY: Duration = Duration::from_secs(10);
/// After start-up, lines wait this long (unless the buffer fills) for the
/// clock to be set, so that they go into the day's file.
const HOLD_AT_START: Duration = Duration::from_secs(45);
/// undated.log is moved to undated-old.log at this size.
const UNDATED_MAX: u64 = 1024 * 1024;
/// The longest line kept (longer ones are cut).
const LINE_MAX: usize = 256;
/// The writer's stack: file writes on FAT.
const WRITER_STACK: usize = 6 * 1024;
/// Before this (2024-01-01), the clock hasn't been set.
const CLOCK_SET_AFTER: u64 = 1_704_067_200;

static ON: AtomicBool = AtomicBool::new(false);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static PENDING: Mutex<Pending> = Mutex::new(Pending { data: [0; BUFFER], len: 0 });
/// Where the files go, once started.
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

struct Pending {
    data: [u8; BUFFER],
    len: usize,
}

/// Whether lines are being kept (so that callers can skip formatting).
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Gather lines from start-up on, before the settings have been read (the
/// board): `start` then keeps them, `stop` forgets them.
#[cfg_attr(not(target_os = "espidf"), allow(dead_code))]
pub fn capture() {
    ON.store(true, Ordering::Relaxed);
}

/// Not keeping the log after all: forget what `capture` gathered.
#[cfg_attr(not(target_os = "espidf"), allow(dead_code))]
pub fn stop() {
    ON.store(false, Ordering::Relaxed);
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).len = 0;
}

/// Start keeping the log in `dir` (made if needed), with a thread that
/// writes it out. `what` names the start-up in the log, ahead of any lines
/// gathered since `capture`.
pub fn start(dir: PathBuf, what: &str) {
    *DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.clone());
    {
        // The start-up marker first.
        let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        let marker = format!("--- {what} started ---\n");
        let n = marker.len();
        if p.len + n <= BUFFER {
            let len = p.len;
            p.data.copy_within(..len, n);
            p.data[..n].copy_from_slice(marker.as_bytes());
            p.len += n;
        }
    }
    ON.store(true, Ordering::Relaxed);
    let started = crate::config_page::spawn_on_server_core(b"filelog\0", WRITER_STACK, move || run(&dir));
    if let Err(e) = started {
        ON.store(false, Ordering::Relaxed);
        log::warn!("Log files: can't start the writer: {e}");
    }
}

/// A Rust log record: "I server: text".
pub fn push_record(r: &log::Record) {
    if !enabled() {
        return;
    }
    let marker = match r.level() {
        log::Level::Error => "E",
        log::Level::Warn => "W",
        log::Level::Info => "I",
        log::Level::Debug => "D",
        log::Level::Trace => "V",
    };
    // The module, without the crate's name.
    let target = r.target().rsplit("::").next().unwrap_or(r.target());
    let mut line = Line::default();
    let _ = write!(line, "{marker} {target}: {}", r.args());
    push_text(line.as_bytes());
}

/// A line of text as printed (ESP-IDF's own log lines come this way):
/// colour codes and line ends are taken out, and the time put in front.
pub fn push_text(text: &[u8]) {
    if !enabled() {
        return;
    }
    let stamp = wall_secs().map(stamp);
    let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let clean = strip(text);
    let need = stamp.as_ref().map_or(0, |s| s.len() + 1) + clean.len().min(LINE_MAX) + 1;
    if clean.is_empty() {
        return;
    }
    if p.len + need > BUFFER {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let mut put = |bytes: &[u8]| {
        let at = p.len;
        p.data[at..at + bytes.len()].copy_from_slice(bytes);
        p.len += bytes.len();
    };
    if let Some(s) = &stamp {
        put(s.as_bytes());
        put(b" ");
    }
    put(&clean[..clean.len().min(LINE_MAX)]);
    put(b"\n");
}

/// Write out what is waiting now (before a restart).
pub fn flush() {
    if !enabled() {
        return;
    }
    let dir = DIR.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(dir) = dir {
        write_out(&dir, &take());
    }
}

/// The writer thread.
fn run(dir: &Path) {
    let started = Instant::now();
    let mut last = Instant::now();
    let mut clock_noted = false;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if !clock_noted && wall_secs().is_some() {
            clock_noted = true;
            push_text(b"Clock set: lines from here on carry the date and time (UTC)");
        }
        let waiting = PENDING.lock().unwrap_or_else(|e| e.into_inner()).len;
        let holding = !clock_noted && started.elapsed() < HOLD_AT_START;
        if waiting >= FLUSH_AT || (!holding && waiting > 0 && last.elapsed() >= FLUSH_EVERY) {
            write_out(dir, &take());
            last = Instant::now();
        }
    }
}

/// What is waiting, and a note of any lines dropped.
fn take() -> Vec<u8> {
    let mut out = {
        let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        let out = p.data[..p.len].to_vec();
        p.len = 0;
        out
    };
    let dropped = DROPPED.swap(0, Ordering::Relaxed);
    if dropped > 0 {
        let note = format!("[{dropped} log lines dropped: more were logged than could be written]\n");
        out.extend_from_slice(note.as_bytes());
    }
    out
}

/// Append the lines to their files: a line stamped with a date to that
/// day's file; an unstamped one (from before the clock was set) to today's
/// if the clock is set by now, else to undated.log.
fn write_out(dir: &Path, text: &[u8]) {
    if text.is_empty() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        // Not logged: that would only add to what can't be written.
        eprintln!("Log files: {}: {e}", dir.display());
        return;
    }
    let today = wall_secs().map(|s| stamp(s)[..10].to_string());
    let mut runs: Vec<(String, Vec<u8>)> = Vec::new();
    for line in text.split_inclusive(|&b| b == b'\n') {
        let file = match day_of(line) {
            Some(day) => format!("{day}.log"),
            None => today.as_ref().map_or_else(|| "undated.log".to_string(), |d| format!("{d}.log")),
        };
        match runs.last_mut() {
            Some((f, buf)) if *f == file => buf.extend_from_slice(line),
            _ => runs.push((file, line.to_vec())),
        }
    }
    let mut new_day = false;
    for (file, buf) in runs {
        let path = dir.join(&file);
        if file == "undated.log" {
            rotate_undated(dir);
        } else if !path.exists() {
            new_day = true;
        }
        let written = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(&buf));
        if let Err(e) = written {
            eprintln!("Log files: {}: {e}", path.display());
        }
    }
    if new_day {
        prune(dir, KEEP_DAYS);
    }
}

/// undated.log, once too big, becomes undated-old.log (replacing the last).
fn rotate_undated(dir: &Path) {
    let path = dir.join("undated.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > UNDATED_MAX) {
        let old = dir.join("undated-old.log");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&path, &old);
    }
}

/// Keep the newest `keep` day files.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut days: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.len() == 14 && n.ends_with(".log") && day_of(n.as_bytes()).is_some())
        .collect();
    days.sort();
    let extra = days.len().saturating_sub(keep);
    for name in &days[..extra] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// Whether `name` is one of the log's files: a day's (2026-10-28.log) or
/// undated.log (or its old one). Nothing else is served (config_page.rs).
pub fn is_log_name(name: &str) -> bool {
    (name.len() == 14 && name.ends_with(".log") && day_of(name.as_bytes()).is_some()) || name == "undated.log" || name == "undated-old.log"
}

/// The date a line (or file name) starts with: "2026-10-28".
fn day_of(line: &[u8]) -> Option<&str> {
    let d = line.get(..10)?;
    let ok = d.iter().enumerate().all(|(i, &b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    ok.then(|| std::str::from_utf8(d).ok()).flatten()
}

/// Seconds since 1970, if the clock has been set.
fn wall_secs() -> Option<u64> {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    (s >= CLOCK_SET_AFTER).then_some(s)
}

/// "2026-10-28 21:40:12" (UTC).
fn stamp(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rest / 3600, rest / 60 % 60, rest % 60)
}

/// Howard Hinnant's days-to-date: the Gregorian date `days` after
/// 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The text without terminal colour codes ("\x1b[0;32m") and line ends.
fn strip(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len().min(LINE_MAX + 16));
    let mut i = 0;
    while i < text.len() && out.len() < LINE_MAX {
        match text[i] {
            0x1b => {
                // ESC [ ... a letter
                i += 1;
                if text.get(i) == Some(&b'[') {
                    i += 1;
                    while i < text.len() && !text[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                }
            }
            b'\r' | b'\n' => {}
            b => out.push(b),
        }
        i += 1;
    }
    while out.last() == Some(&b' ') {
        out.pop();
    }
    out
}

/// A line formatted on the stack, cut at LINE_MAX.
struct Line {
    buf: [u8; LINE_MAX],
    len: usize,
}

impl Default for Line {
    fn default() -> Self {
        Line { buf: [0; LINE_MAX], len: 0 }
    }
}

impl Line {
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl std::fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let n = s.len().min(LINE_MAX - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00");
        assert_eq!(stamp(1_793_223_612), "2026-10-28 21:40:12");
        assert_eq!(stamp(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(day_of(b"2026-10-28 21:40:12 I server: x"), Some("2026-10-28"));
        assert_eq!(day_of(b"I (123) wifi: x"), None);
        assert_eq!(day_of(b"2026-10-28.log"), Some("2026-10-28"));
    }

    #[test]
    fn names() {
        assert!(is_log_name("2026-10-28.log") && is_log_name("undated.log") && is_log_name("undated-old.log"));
        for bad in ["../server.txt", "2026-10-28.log/../x", "x.log", "2026-10-28.txt", "", "2026-1-28.log", "/etc/passwd"] {
            assert!(!is_log_name(bad), "{bad}");
        }
    }

    #[test]
    fn strips_colours() {
        assert_eq!(strip(b"\x1b[0;32mI (47258937) wifi:Set ps type: 1\x1b[0m\n"), b"I (47258937) wifi:Set ps type: 1");
        assert_eq!(strip(b"plain\r\n"), b"plain");
        assert_eq!(strip(&[b'x'; 400]).len(), LINE_MAX);
    }

    #[test]
    fn files_by_day_and_pruned() {
        let dir = std::env::temp_dir().join(format!("filelog-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_out(&dir, b"2026-10-27 23:59:59 I a: one\n2026-10-28 00:00:01 I a: two\n2026-10-28 00:00:02 I a: three\n");
        assert_eq!(std::fs::read_to_string(dir.join("2026-10-27.log")).unwrap(), "2026-10-27 23:59:59 I a: one\n");
        assert_eq!(std::fs::read_to_string(dir.join("2026-10-28.log")).unwrap().lines().count(), 2);
        // Unstamped lines go to today's file (a PC's clock is set).
        write_out(&dir, b"I (5) boot: early\n");
        let today = stamp(wall_secs().unwrap())[..10].to_string();
        assert!(std::fs::read_to_string(dir.join(format!("{today}.log"))).unwrap().contains("I (5) boot: early"));
        for d in 1..=20 {
            std::fs::write(dir.join(format!("2025-01-{d:02}.log")), "x\n").unwrap();
        }
        std::fs::write(dir.join("undated.log"), "x\n").unwrap();
        prune(&dir, 5);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left.len(), 6, "{left:?}"); // five days and undated.log
        assert!(left.contains(&"undated.log".to_string()) && left.contains(&format!("{today}.log")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn buffered_and_dropped() {
        ON.store(true, Ordering::Relaxed);
        let _ = take();
        push_text(b"\x1b[0;33mW (1) x: hello\x1b[0m\n");
        let line = String::from_utf8(take()).unwrap();
        assert!(line.ends_with(" W (1) x: hello\n"), "{line:?}");
        for _ in 0..100 {
            push_text(&[b'y'; 100]);
        }
        let out = String::from_utf8(take()).unwrap();
        assert!(out.len() <= BUFFER + 100 && out.contains("log lines dropped"), "{}", out.len());
        ON.store(false, Ordering::Relaxed);
    }
}
