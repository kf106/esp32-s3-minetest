//! The boards the firmware can be built for, chosen when building with the
//! BOARD environment variable (see build.rs):
//!
//! - `BOARD=waveshare` (the default): Waveshare ESP32-S3-LCD-1.47, with a
//!   1.47" 172x320 ST7789 screen.
//! - `BOARD=es3c28p` (or `es3n28p`): LCDwiki's ES3C28P / ES3N28P, with a
//!   2.8" 240x320 ILI9341V screen (touch on the C, unused here), a speaker
//!   amplifier, and a battery connector. Pins from its specification
//!   (ES3C28P_ES2N28P_Specification_V1.0, lcdwiki.com).
//! - `BOARD=tdisplays3`: LilyGO's T-Display-S3, with a 1.9" 170x320 ST7789V
//!   screen on an 8-bit parallel (Intel 8080) bus rather than SPI, and no
//!   SD card slot: the game is kept in flash. Pins from LilyGO's
//!   pin_config.h (Xinyuan-LilyGO/T-Display-S3).
//!
//! All are ESP32-S3s with 16 MB of flash and 8 MB of octal PSRAM. The SD
//! card and the SPI bus for the screen are given their pins in main.rs,
//! where the pins are typed; everything else is here.

/// The screen's controller. Each build uses only its own board's, so the
/// other is never made.
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(dead_code)]
pub enum Controller {
    St7789,
    Ili9341,
}

/// A screen on an 8-bit parallel (Intel 8080) bus: its data lines D0 to
/// D7, the write strobe, and the read strobe (held high: nothing is read).
pub struct I80 {
    pub data: [i32; 8],
    pub wr: i32,
    pub rd: i32,
}

pub struct Board {
    /// Shown in the welcome message and the log.
    pub name: &'static str,
    pub lcd_controller: Controller,
    /// The screen's bus: None for SPI (its clock and data pins are in
    /// main.rs), or the parallel bus's pins.
    pub lcd_i80: Option<I80>,
    pub lcd_cs: i32,
    pub lcd_dc: i32,
    /// -1: the screen resets with the chip.
    pub lcd_reset: i32,
    /// Active high.
    pub lcd_backlight: i32,
    /// The status pages are 320x172. The panel's other axis: 172 on the
    /// Waveshare (in the middle of the controller's 240), 240 on the
    /// ES3C28P, where the pages sit in the middle for now.
    pub lcd_rows: i32,
    /// Where the pages start on that axis, in the controller's 240.
    pub lcd_gap: i32,
    /// IPS panels want their colours inverted.
    pub lcd_invert: bool,
    /// Red and blue the other way round.
    pub lcd_bgr: bool,
    /// Landscape is the controller's axes swapped, then mirrored to suit
    /// how the panel is mounted: left to right, and top to bottom. Pages
    /// upside down: flip both.
    pub lcd_mirror_x: bool,
    pub lcd_mirror_y: bool,
    /// One WS2812-style RGB LED; -1: none.
    pub led: i32,
    /// Its colour order: red, green, blue (the Waveshare's), or green, red,
    /// blue (the WS2812's own).
    pub led_grb: bool,
    /// The BOOT button: pulled up, low while pressed.
    pub button: i32,
    /// A speaker amplifier's enable pin, held so that it stays off; and
    /// the level that keeps it off.
    pub amp_off: Option<(i32, u32)>,
    /// A pin held high to power the screen and peripherals (needed on
    /// battery).
    pub power_on: Option<i32>,
    /// Whether the board has an SD card slot. Without one, the game is kept
    /// in flash.
    pub sd_slot: bool,
}

// The default, and the PC builds' (which set no board).
#[cfg(not(any(board = "es3c28p", board = "tdisplays3")))]
pub const BOARD: Board = Board {
    name: "Waveshare ESP32-S3-LCD-1.47",
    lcd_controller: Controller::St7789,
    lcd_i80: None,
    lcd_cs: 42,
    lcd_dc: 41,
    lcd_reset: 39,
    lcd_backlight: 48,
    lcd_rows: 172,
    lcd_gap: 34,
    lcd_invert: true,
    lcd_bgr: false,
    lcd_mirror_x: true,
    lcd_mirror_y: false,
    led: 38,
    led_grb: false,
    button: 0,
    amp_off: None,
    power_on: None,
    sd_slot: true,
};

#[cfg(board = "es3c28p")]
pub const BOARD: Board = Board {
    name: "LCDwiki ES3C28P/ES3N28P",
    lcd_controller: Controller::Ili9341,
    lcd_i80: None,
    lcd_cs: 10,
    lcd_dc: 46,
    lcd_reset: -1,
    lcd_backlight: 45,
    lcd_rows: 240,
    lcd_gap: 34,
    lcd_invert: true,
    lcd_bgr: true,
    // Mounted the other way round from the Waveshare's.
    lcd_mirror_x: true,
    lcd_mirror_y: true,
    led: 42,
    led_grb: true,
    button: 0,
    // The amplifier is enabled by a low level on GPIO 1.
    amp_off: Some((1, 1)),
    power_on: None,
    sd_slot: true,
};

#[cfg(board = "tdisplays3")]
pub const BOARD: Board = Board {
    name: "LilyGO T-Display-S3",
    lcd_controller: Controller::St7789,
    lcd_i80: Some(I80 { data: [39, 40, 41, 42, 45, 46, 47, 48], wr: 8, rd: 9 }),
    lcd_cs: 6,
    lcd_dc: 7,
    lcd_reset: 5,
    lcd_backlight: 38,
    // The panel's 170 columns sit in the middle of the controller's 240
    // (from 35): with the pages' 172 rows at 34, their top and bottom rows
    // fall just outside it.
    lcd_rows: 240,
    lcd_gap: 34,
    lcd_invert: true,
    lcd_bgr: false,
    // As LilyGO's own examples turn it to landscape.
    lcd_mirror_x: false,
    lcd_mirror_y: true,
    led: -1,
    led_grb: false,
    button: 0,
    amp_off: None,
    power_on: Some(15),
    sd_slot: false,
};
