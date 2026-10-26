//! Drawing the status screen into a framebuffer. No hardware here, so the
//! layout can be rendered and checked on a PC; lcd.rs sends the result to
//! the panel.

use std::time::Duration;

use crate::font::{Font, BIG, SMALL};
use crate::status::{Hotspot, Status, UsbDrive};

/// Landscape: the panel is 172x320, used sideways.
pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 172;

/// RGB565 from 8-bit components.
pub const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

const BACKGROUND: u16 = rgb(16, 20, 24);
const HEADER: u16 = rgb(46, 125, 50);
const TEXT: u16 = rgb(232, 234, 237);
const LABEL: u16 = rgb(138, 145, 153);
const GOOD_RGB: [u8; 3] = [129, 199, 132];
const EVENT_RGB: [u8; 3] = [255, 213, 79];
const BAD_RGB: [u8; 3] = [239, 83, 80];
const GOOD: u16 = rgb(GOOD_RGB[0], GOOD_RGB[1], GOOD_RGB[2]);
const EVENT: u16 = rgb(EVENT_RGB[0], EVENT_RGB[1], EVENT_RGB[2]);
const BAD: u16 = rgb(BAD_RGB[0], BAD_RGB[1], BAD_RGB[2]);
/// The USB drive page's header: the orange of the BOOT hold bar.
const DRIVE_HEADER: u16 = rgb(251, 140, 0);

/// The chip's temperature bands: the ESP32-S3 is rated for 85 C around
/// it; ordinary SD cards for about 70 C, and the card sits right over the
/// chip. Their colours, on the Device page and in the LED's breathing:
/// green, amber from CHIP_WARM, red from CHIP_HOT.
pub const CHIP_WARM: f32 = 60.0;
pub const CHIP_HOT: f32 = 75.0;

pub fn chip_colour(celsius: f32) -> [u8; 3] {
    if celsius < CHIP_WARM {
        GOOD_RGB
    } else if celsius < CHIP_HOT {
        EVENT_RGB
    } else {
        BAD_RGB
    }
}
const RULE: u16 = rgb(52, 58, 64);

/// Things the display thread measures itself.
#[derive(Clone, Copy, Default)]
pub struct System {
    pub internal_free_kb: u32,
    /// The least internal RAM there has been free since start.
    pub internal_min_kb: u32,
    /// The biggest single piece of internal RAM free (new tasks need one).
    pub internal_largest_kb: u32,
    pub psram_free_kb: u32,
    pub psram_min_kb: u32,
    /// Wi-Fi signal strength and channel, if connected.
    pub rssi: Option<i8>,
    pub channel: Option<u8>,
    /// Devices on the board's own hotspot, when it runs one.
    pub hotspot_clients: Option<u8>,
    pub packets_in_per_s: f32,
    pub packets_out_per_s: f32,
    /// Load on each CPU core over the last second, 0..=100, if known.
    pub cpu: [Option<f32>; 2],
    /// Share of the last second the server loop spent working, 0..=100.
    pub server_busy: f32,
    /// Average time to build a map block over the last second (or the
    /// whole run, if none were built just now), in milliseconds.
    pub block_ms: Option<f32>,
    /// SD card size and free space, in bytes.
    pub sd: Option<(u64, u64)>,
    /// The chip's own temperature sensor, in degrees Celsius.
    pub chip_temp: Option<f32>,
}

/// The status screens, paged through with the BOOT button. Join, the
/// hotspot's QR code, comes before the rest and is only there when the
/// board runs its own hotspot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Page {
    Join,
    Overview,
    Device,
    Chat,
}

impl Page {
    /// The pages in order, with or without the hotspot's Join page.
    fn all(hotspot: bool) -> &'static [Page] {
        if hotspot {
            &[Page::Join, Page::Overview, Page::Device, Page::Chat]
        } else {
            &[Page::Overview, Page::Device, Page::Chat]
        }
    }

    pub fn next(self, hotspot: bool) -> Self {
        let pages = Self::all(hotspot);
        let i = pages.iter().position(|&p| p == self).unwrap_or(0);
        pages[(i + 1) % pages.len()]
    }
}

/// How to draw text: font, colours, and the x at which to clip. Grouped
/// so the drawing functions take few arguments: on the ESP32 a seventh
/// argument word goes on the stack, which once went wrong (see
/// server::Setup).
#[derive(Clone, Copy)]
struct Pen {
    font: &'static Font,
    fg: u16,
    bg: u16,
    clip: usize,
}

impl Pen {
    const fn new(font: &'static Font, fg: u16, bg: u16, clip: usize) -> Self {
        Self { font, fg, bg, clip }
    }
}

pub struct Frame {
    pub pixels: Vec<u16>,
}

impl Frame {
    pub fn new() -> Self {
        Self { pixels: vec![BACKGROUND; WIDTH * HEIGHT] }
    }

    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, colour: u16) {
        for row in y..(y + h).min(HEIGHT) {
            let start = row * WIDTH + x.min(WIDTH);
            let end = row * WIDTH + (x + w).min(WIDTH);
            self.pixels[start..end].fill(colour);
        }
    }

    /// Draw text with its top-left corner at (x, y) over a plain background
    /// colour, clipped at `max_x`. Returns the x after the last character.
    fn text(&mut self, pen: &Pen, x: usize, y: usize, s: &str) -> usize {
        let (font, fg, bg, max_x) = (pen.font, pen.fg, pen.bg, pen.clip);
        let mut cx = x;
        for c in s.chars() {
            if cx + font.w > max_x.min(WIDTH) {
                break;
            }
            for gy in 0..font.h {
                let py = y + gy;
                if py >= HEIGHT {
                    break;
                }
                for gx in 0..font.w {
                    let a = font.alpha(c, gx, gy);
                    if a != 0 {
                        self.pixels[py * WIDTH + cx + gx] = blend(bg, fg, a);
                    }
                }
            }
            cx += font.w;
        }
        cx
    }

    /// Text ending at `right`.
    fn text_right(&mut self, pen: &Pen, right: usize, y: usize, s: &str) {
        let font = pen.font;
        let w = s.chars().count() * font.w;
        self.text(&Pen { clip: right, ..*pen }, right.saturating_sub(w), y, s);
    }

    /// A grey label followed by its value, at (x, y).
    fn pair(&mut self, at: &(usize, usize), label: &str, value: &str) {
        let x = self.text(&Pen::new(&SMALL, LABEL, BACKGROUND, WIDTH), at.0, at.1, label);
        self.text(&Pen::new(&SMALL, TEXT, BACKGROUND, WIDTH), x, at.1, value);
    }

    /// Rows of the frame as big-endian RGB565 bytes, as the panel expects.
    pub fn write_be(&self, rows: std::ops::Range<usize>, out: &mut [u8]) {
        let pixels = &self.pixels[rows.start * WIDTH..rows.end * WIDTH];
        for (p, o) in pixels.iter().zip(out.chunks_exact_mut(2)) {
            o.copy_from_slice(&p.to_be_bytes());
        }
    }
}

/// Mix two RGB565 colours: alpha 0 gives `bg`, 15 gives `fg`.
fn blend(bg: u16, fg: u16, alpha: u8) -> u16 {
    let a = alpha as u32;
    let mix = |shift: u32, mask: u32| {
        let b = (bg as u32 >> shift) & mask;
        let f = (fg as u32 >> shift) & mask;
        ((b * (15 - a) + f * a + 7) / 15) << shift
    };
    (mix(11, 0x1F) | mix(5, 0x3F) | mix(0, 0x1F)) as u16
}

fn uptime(d: Duration) -> String {
    let s = d.as_secs();
    let (days, h, m) = (s / 86400, s / 3600 % 24, s / 60 % 60);
    if days > 0 {
        format!("up {days}d {h}h")
    } else if h > 0 {
        format!("up {h}h {m:02}m")
    } else {
        format!("up {m}m {:02}s", s % 60)
    }
}

fn kb(v: u32) -> String {
    if v >= 1024 {
        format!("{:.1}M", v as f32 / 1024.0)
    } else {
        format!("{v}K")
    }
}

/// What to draw, gathered in one struct (see Pen for why).
pub struct View<'a> {
    pub page: Page,
    pub status: &'a Status,
    pub system: &'a System,
}

/// Draw the whole screen.
pub fn render(f: &mut Frame, view: &View) {
    // As a USB drive, its own page, and no other.
    if let Some(drive) = &view.status.usb_drive {
        render_drive(f, drive);
        return;
    }
    match (view.page, &view.status.hotspot) {
        (Page::Join, Some(hotspot)) => render_join(f, view.status, hotspot),
        (Page::Overview | Page::Join, _) => render_overview(f, view.status, view.system),
        (Page::Device, _) => render_device(f, view.status, view.system),
        (Page::Chat, _) => render_chat(f, view.status),
    }
    // Which page this is, as dots along the bottom edge, kept clear of the
    // panel's rounded corner.
    let pages = Page::all(view.status.hotspot.is_some());
    let (dot, gap) = (4, 6);
    let x0 = WIDTH - 11 - pages.len() * (dot + gap) + gap;
    for (i, &p) in pages.iter().enumerate() {
        let colour = if p == view.page { TEXT } else { RULE };
        f.fill(x0 + i * (dot + gap), HEIGHT - dot - 3, dot, dot, colour);
    }
    // BOOT held at start-up: a blue bar growing along the bottom edge
    // towards a hotspot, then an orange one over it towards the USB drive.
    if let Some(held) = view.status.boot_hold {
        let width = |part: f32| (part.clamp(0.0, 1.0) * WIDTH as f32).round() as usize;
        f.fill(0, HEIGHT - HOLD_BAR, width(held), HOLD_BAR, HOLD_BLUE);
        if held > 1.0 {
            f.fill(0, HEIGHT - HOLD_BAR, width(held - 1.0), HOLD_BAR, HOLD_ORANGE);
        }
    }
}

/// The BOOT hold bar's height and colours.
const HOLD_BAR: usize = 8;
const HOLD_BLUE: u16 = rgb(30, 136, 229);
const HOLD_ORANGE: u16 = rgb(251, 140, 0);

/// A bar `w` pixels wide, filled to `percent`, coloured by how full it is.
fn bar(f: &mut Frame, at: (usize, usize), w: usize, percent: f32) {
    let h = SMALL.h - 7;
    let (x, y) = (at.0, at.1 + 4);
    f.fill(x, y, w, h, RULE);
    let filled = ((percent.clamp(0.0, 100.0) / 100.0) * w as f32).round() as usize;
    let colour = if percent < 60.0 { GOOD } else if percent < 85.0 { EVENT } else { BAD };
    f.fill(x, y, filled, h, colour);
}

fn gb(bytes: u64) -> String {
    let g = bytes as f64 / 1e9;
    if g >= 10.0 { format!("{g:.0}G") } else { format!("{g:.1}G") }
}

/// A size in megabytes (the board's own flash is a few of them).
fn mb(bytes: u64) -> String {
    let m = bytes as f64 / 1e6;
    if m >= 10.0 { format!("{m:.0}M") } else { format!("{m:.1}M") }
}

/// The second page: how hard the device is working, and its resources.
fn render_device(f: &mut Frame, st: &Status, sys: &System) {
    let margin = 6;
    let right = WIDTH - margin;
    f.fill(0, 0, WIDTH, HEIGHT, BACKGROUND);

    let header_h = BIG.h + 4;
    f.fill(0, 0, WIDTH, header_h, HEADER);
    f.text(&Pen::new(&BIG, TEXT, HEADER, WIDTH), margin, 2, "Device");
    let up = uptime(st.started.elapsed());
    f.text_right(&Pen::new(&SMALL, TEXT, HEADER, WIDTH), right, 2 + (BIG.h - SMALL.h) / 2 + 1, &up);

    let line = SMALL.h + 2;
    let mut y = header_h + 4;
    let label = Pen::new(&SMALL, LABEL, BACKGROUND, WIDTH);
    let value = Pen::new(&SMALL, TEXT, BACKGROUND, WIDTH);
    let col2 = WIDTH / 2 + 4;

    // CPU cores, with bars
    for (core, load) in sys.cpu.iter().enumerate() {
        let x0 = if core == 0 { margin } else { col2 };
        let x = f.text(&label, x0, y, &format!("CPU{core} "));
        match load {
            Some(p) => {
                bar(f, (x, y), 60, *p);
                f.text(&value, x + 64, y, &format!("{p:>3.0}%"));
            }
            None => {
                f.text(&value, x, y, "n/a");
            }
        }
    }
    y += line;

    // The server itself
    f.pair(&(margin, y), "Server ", &format!("{:.0}% busy", sys.server_busy));
    let block = sys.block_ms.map_or("-".to_string(), |ms| format!("{ms:.0} ms"));
    f.pair(&(col2, y), "Block ", &block);
    y += line;

    // Memory
    f.pair(&(margin, y), "RAM ", &format!("{} free", kb(sys.internal_free_kb)));
    f.pair(&(col2, y), "low ", &format!("{} big {}", kb(sys.internal_min_kb), kb(sys.internal_largest_kb)));
    y += line;
    f.pair(&(margin, y), "PSRAM ", &format!("{} free", kb(sys.psram_free_kb)));
    f.pair(&(col2, y), "low ", &kb(sys.psram_min_kb));
    y += line;

    // Network
    // "doze": power saving, nobody connected (see server.rs).
    let wifi = match (sys.rssi, sys.channel) {
        _ if st.hotspot.is_some() => match (sys.hotspot_clients, sys.channel) {
            (Some(n), Some(c)) => format!("AP ch{c}, {n} on"),
            (Some(n), None) => format!("AP, {n} on"),
            _ => "AP".to_string(),
        },
        (Some(r), _) if st.wifi_dozing => format!("{r} dBm doze"),
        (Some(r), Some(c)) => format!("{r} dBm ch {c}"),
        (Some(r), None) => format!("{r} dBm"),
        _ => "down".to_string(),
    };
    f.pair(&(margin, y), "Wi-Fi ", &wifi);
    f.pair(&(col2, y), "Send fails ", &st.send_failures.to_string());
    y += line;

    // Storage: the SD card, or the board's own flash without one.
    let (store, size): (&str, fn(u64) -> String) = if crate::status::on_flash() { ("Flash ", mb) } else { ("SD card ", gb) };
    let sd = sys.sd.map_or("-".to_string(), |(total, free)| format!("{} free of {}", size(free), size(total)));
    f.pair(&(margin, y), store, &sd);
    y += line;

    // Heat (see chip_colour).
    let x = f.text(&label, margin, y, "Chip ");
    match sys.chip_temp {
        Some(t) => {
            let [r, g, b] = chip_colour(t);
            let colour = rgb(r, g, b);
            f.text(&Pen::new(&SMALL, colour, BACKGROUND, WIDTH), x, y, &format!("{t:.0} °C"));
        }
        None => {
            f.text(&value, x, y, "-");
        }
    }
}

/// Split `text` into lines of at most `width` characters, breaking at
/// spaces where possible. The first line has `first` characters less room
/// (for the speaker's name).
fn wrap(text: &str, width: usize, first: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let room = |lines: &Vec<String>| if lines.is_empty() { width.saturating_sub(first).max(1) } else { width };
    for word in text.split(' ') {
        let mut word = word.to_string();
        loop {
            let used = line.chars().count();
            let gap = if used == 0 { 0 } else { 1 };
            let fits = room(&lines).saturating_sub(used + gap);
            let len = word.chars().count();
            if len <= fits {
                if gap == 1 {
                    line.push(' ');
                }
                line.push_str(&word);
                break;
            }
            if used > 0 {
                lines.push(std::mem::take(&mut line));
                continue;
            }
            // A word longer than a whole line: cut it.
            let cut: String = word.chars().take(fits.max(1)).collect();
            word = word.chars().skip(fits.max(1)).collect();
            lines.push(cut);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// The third page: recent public chat, newest at the bottom, wrapped to
/// the screen's width. (The font has only plain ASCII characters; others
/// show as "?".)
fn render_chat(f: &mut Frame, st: &Status) {
    let margin = 6;
    let right = WIDTH - margin;
    f.fill(0, 0, WIDTH, HEIGHT, BACKGROUND);
    let header_h = BIG.h + 4;
    f.fill(0, 0, WIDTH, header_h, HEADER);
    f.text(&Pen::new(&BIG, TEXT, HEADER, WIDTH), margin, 2, "Chat");
    let up = uptime(st.started.elapsed());
    f.text_right(&Pen::new(&SMALL, TEXT, HEADER, WIDTH), right, 2 + (BIG.h - SMALL.h) / 2 + 1, &up);

    let line = SMALL.h + 1;
    let top = header_h + 4;
    // Leave the bottom row's right end clear for the page dots.
    let rows = (HEIGHT - 6 - top) / line;
    let width = (right - margin) / SMALL.w;
    if st.chat.is_empty() {
        f.text(&Pen::new(&SMALL, LABEL, BACKGROUND, right), margin, top, "Nothing said yet.");
        return;
    }
    // Lay out from the newest backwards until the screen is full.
    // Each row: (name shown first?, text, colour).
    let mut layout: Vec<(Option<String>, String, u16)> = Vec::new();
    for c in st.chat.iter().rev() {
        let (prefix, colour) = if c.action { (format!("* {} ", c.name), EVENT) } else { (format!("<{}> ", c.name), TEXT) };
        let mut wrapped = wrap(&c.text, width, prefix.chars().count());
        let first = wrapped.remove(0);
        let mut rows_for: Vec<(Option<String>, String, u16)> = vec![(Some(prefix), first, colour)];
        rows_for.extend(wrapped.into_iter().map(|t| (None, t, colour)));
        // Only whole messages, unless even the newest alone doesn't fit
        // (then its start).
        if layout.len() + rows_for.len() > rows {
            if layout.is_empty() {
                rows_for.truncate(rows);
                layout.extend(rows_for.into_iter().rev());
            }
            break;
        }
        layout.extend(rows_for.into_iter().rev());
    }
    layout.reverse();
    let mut y = top;
    for (prefix, text, colour) in layout {
        let mut x = margin;
        if let Some(p) = prefix {
            let name_colour = if colour == EVENT { EVENT } else { GOOD };
            x = f.text(&Pen::new(&SMALL, name_colour, BACKGROUND, right), x, y, &p);
        }
        f.text(&Pen::new(&SMALL, colour, BACKGROUND, right), x, y, &text);
        y += line;
    }
}

/// Text cut into pieces of at most `width` characters, for names and
/// passwords too long for one line (they have no spaces to break at).
fn chunks(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars.chunks(width.max(1)).map(|c| c.iter().collect()).collect()
}

/// The USB drive's page: an orange header, what the PC sees, the files at
/// the top of it (as when it started), and how to get back to the game.
fn render_drive(f: &mut Frame, d: &UsbDrive) {
    let margin = 6;
    let right = WIDTH - margin;
    f.fill(0, 0, WIDTH, HEIGHT, BACKGROUND);
    let header_h = BIG.h + 4;
    let (title, header) = if d.error.is_some() { ("USB Drive Failed", BAD) } else { ("USB Drive Active", DRIVE_HEADER) };
    f.fill(0, 0, WIDTH, header_h, header);
    f.text(&Pen::new(&BIG, BACKGROUND, header, WIDTH), margin, 2, title);

    let line = SMALL.h + 1;
    let mut y = header_h + 4;
    let footer_y = HEIGHT - line - 2;
    let text = Pen::new(&SMALL, TEXT, BACKGROUND, right);
    if let Some(e) = &d.error {
        for part in chunks(e, (right - margin) / SMALL.w).iter().take((footer_y - y) / line) {
            f.text(&text, margin, y, part);
            y += line;
        }
        f.text(&Pen::new(&SMALL, EVENT, BACKGROUND, right), margin, footer_y, "Press BOOT to restart");
        return;
    }
    f.pair(&(margin, y), &format!("{} ", d.what), &size(d.bytes));
    y += line + 3;
    f.fill(margin, y - 2, right - margin, 1, RULE);

    // The files, in two columns; "+N more" in the last place if they don't
    // all fit.
    let rows = (footer_y - y) / line;
    let col_w = (right - margin) / 2;
    let places = rows * 2;
    let shown = if d.files.len() > places { places - 1 } else { d.files.len() };
    let mut items: Vec<String> = d.files[..shown].to_vec();
    if shown < d.files.len() {
        items.push(format!("+{} more", d.files.len() - shown));
    }
    if d.files.is_empty() {
        items.push("(empty)".to_string());
    }
    for (i, name) in items.iter().enumerate() {
        let (col, row) = (i / rows, i % rows);
        let x = margin + col * col_w;
        let colour = if name.ends_with('/') { GOOD } else { TEXT };
        f.text(&Pen::new(&SMALL, colour, BACKGROUND, x + col_w - SMALL.w), x, y + row * line, name);
    }

    let (note, colour) = if d.ejected { ("Ejected: restarting...", GOOD) } else { ("Eject it on the PC to play again", EVENT) };
    f.text(&Pen::new(&SMALL, colour, BACKGROUND, right), margin, footer_y, note);
}

/// A size in bytes, for people.
fn size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= 1024.0 * MB {
        format!("{:.1} GB", b / (1024.0 * MB))
    } else {
        format!("{:.1} MB", b / MB)
    }
}

/// The hotspot page: a QR code that a phone camera offers to join, and the
/// same details written out for a laptop.
fn render_join(f: &mut Frame, st: &Status, hotspot: &Hotspot) {
    let margin = 6;
    let right = WIDTH - margin;
    f.fill(0, 0, WIDTH, HEIGHT, BACKGROUND);
    let header_h = BIG.h + 4;
    f.fill(0, 0, WIDTH, header_h, HEADER);
    f.text(&Pen::new(&BIG, TEXT, HEADER, WIDTH), margin, 2, "Join");
    let up = uptime(st.started.elapsed());
    f.text_right(&Pen::new(&SMALL, TEXT, HEADER, WIDTH), right, 2 + (BIG.h - SMALL.h) / 2 + 1, &up);

    // The code, dark on white with a quiet border two modules wide (the
    // standard asks for four, but phones read two happily, and the bigger
    // modules matter more on a small screen).
    let top = header_h + 4;
    let room = HEIGHT - top - 4;
    let mut text_x = margin;
    if let Some(qr) = &hotspot.qr {
        let quiet = 2;
        let span = qr.size + 2 * quiet;
        let scale = (room / span).max(1);
        let side = span * scale;
        let (x0, y0) = (margin, top + (room - side.min(room)) / 2);
        f.fill(x0, y0, side, side, rgb(255, 255, 255));
        for my in 0..qr.size {
            for mx in 0..qr.size {
                if qr.dark(mx, my) {
                    f.fill(x0 + (quiet + mx) * scale, y0 + (quiet + my) * scale, scale, scale, rgb(0, 0, 0));
                }
            }
        }
        text_x = x0 + side + 8;
    }

    // The details beside it.
    let width = (right - text_x) / SMALL.w;
    let line = SMALL.h + 1;
    let label = Pen::new(&SMALL, LABEL, BACKGROUND, right);
    let value = Pen::new(&SMALL, TEXT, BACKGROUND, right);
    let mut y = top;
    f.text(&label, text_x, y, "Wi-Fi");
    y += line;
    for part in chunks(&hotspot.ssid, width).iter().take(2) {
        f.text(&value, text_x, y, part);
        y += line;
    }
    f.text(&label, text_x, y, "Password");
    y += line;
    if hotspot.password.is_empty() {
        f.text(&value, text_x, y, "none (open)");
        y += line;
    } else {
        for part in chunks(&hotspot.password, width).iter().take(2) {
            f.text(&value, text_x, y, part);
            y += line;
        }
    }
    // The address, with its label if both fit above the page dots.
    if y + 3 + 2 * line <= HEIGHT - 6 {
        y += 3;
        // The same address in a browser opens the settings page
        // (config_page.rs).
        f.text(&label, text_x, y, "IP address");
        y += line;
    }
    f.text(&Pen::new(&SMALL, GOOD, BACKGROUND, right), text_x, y, &hotspot.ip);
}

/// The first page: who is playing and what is happening.
fn render_overview(f: &mut Frame, st: &Status, sys: &System) {
    let margin = 6;
    let right = WIDTH - margin;
    f.fill(0, 0, WIDTH, HEIGHT, BACKGROUND);

    // Header: the address to connect to, and uptime
    let header_h = BIG.h + 4;
    f.fill(0, 0, WIDTH, header_h, HEADER);
    let up = uptime(st.started.elapsed());
    let up_x = right - up.len() * SMALL.w;
    // A long IP address plus a long uptime may not fit; the port is
    // always the default, so it goes first.
    let room = (up_x - SMALL.w - margin) / BIG.w;
    let address = if st.address.len() <= room {
        st.address.as_str()
    } else {
        st.address.rsplit_once(':').map_or(st.address.as_str(), |(ip, _)| ip)
    };
    f.text(&Pen::new(&BIG, TEXT, HEADER, up_x - SMALL.w), margin, 2, address);
    f.text_right(&Pen::new(&SMALL, TEXT, HEADER, WIDTH), right, 2 + (BIG.h - SMALL.h) / 2 + 1, &up);

    let line = SMALL.h + 1;
    let mut y = header_h + 4;

    // Players and signal
    let x = f.text(&Pen::new(&SMALL, LABEL, BACKGROUND, WIDTH), margin, y, "Players ");
    let mut count = st.players.len().to_string();
    if st.joining > 0 {
        count.push_str(&format!(" (+{} joining)", st.joining));
    }
    f.text(&Pen::new(&SMALL, TEXT, BACKGROUND, WIDTH), x, y, &count);
    if st.hotspot.is_some() {
        let n = sys.hotspot_clients.unwrap_or(0);
        let devices = if n == 1 { "device" } else { "devices" };
        f.text_right(&Pen::new(&SMALL, GOOD, BACKGROUND, WIDTH), right, y, &format!("Hotspot, {n} {devices}"));
    } else if let Some(rssi) = sys.rssi {
        let colour = if rssi >= -67 { GOOD } else if rssi >= -80 { EVENT } else { BAD };
        f.text_right(&Pen::new(&SMALL, colour, BACKGROUND, WIDTH), right, y, &format!("Wi-Fi {rssi} dBm"));
    }
    y += line;
    if st.players.is_empty() {
        f.text(&Pen::new(&SMALL, LABEL, BACKGROUND, right), margin + 2 * SMALL.w, y, "nobody yet");
    } else {
        let room = (right - margin) / SMALL.w - 2;
        f.text(&Pen::new(&SMALL, GOOD, BACKGROUND, right), margin + 2 * SMALL.w, y, &name_list(&st.players, room));
    }
    y += line + 2;

    // Traffic and world
    let col2 = WIDTH / 2 + 4;
    f.pair(&(margin, y), "In  ", &format!("{:.0} pkt/s", sys.packets_in_per_s));
    f.pair(&(col2, y), "Out ", &format!("{:.0} pkt/s", sys.packets_out_per_s));
    y += line;
    f.pair(&(margin, y), "Blocks ", &st.blocks_sent.to_string());
    f.pair(&(col2, y), "Edits ", &st.edits.to_string());
    y += line;
    f.pair(&(margin, y), "RAM ", &kb(sys.internal_free_kb));
    f.pair(&(col2, y), "PSRAM ", &kb(sys.psram_free_kb));
    y += line + 3;

    // Recent events
    f.fill(margin, y - 2, WIDTH - 2 * margin, 1, RULE);
    let room = (HEIGHT - y) / line;
    let skip = st.events.len().saturating_sub(room);
    for e in st.events.iter().skip(skip) {
        f.text(&Pen::new(&SMALL, EVENT, BACKGROUND, right), margin, y, e);
        y += line;
    }
}

/// As many names as fit in `max` characters, then "+N" for the rest.
fn name_list(names: &[String], max: usize) -> String {
    let all = names.join(", ");
    if all.len() <= max {
        return all;
    }
    let mut shown = String::new();
    for (i, n) in names.iter().enumerate() {
        let more = format!(", +{}", names.len() - i);
        let candidate = if shown.is_empty() { n.clone() } else { format!("{shown}, {n}") };
        // Keep room for the "+N" that the remaining names will need
        let tail = if i + 1 < names.len() { format!(", +{}", names.len() - i - 1).len() } else { 0 };
        if candidate.len() + tail > max {
            return if shown.is_empty() { format!("+{}", names.len()) } else { shown + &more };
        }
        shown = candidate;
    }
    shown
}
