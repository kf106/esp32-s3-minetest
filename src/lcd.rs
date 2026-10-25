//! The board's LCD, driven through ESP-IDF's esp_lcd component, and the
//! thread that redraws the status screens. The BOOT button pages through
//! them. Pins and panel settings are in board.rs.
//!
//! - Waveshare ESP32-S3-LCD-1.47: a 172x320 ST7789. The panel's 172
//!   columns sit in the middle of the controller's 240, hence the
//!   34-pixel gap.
//! - ES3C28P/ES3N28P: a 240x320 ILI9341V. It takes the ST7789's commands
//!   for everything esp_lcd sends, and after esp_lcd's set-up gets the
//!   power and gamma settings an ILI9341 needs (as Espressif's ILI9341
//!   driver sends them). For now the 320x172 pages are drawn in the
//!   middle of its 240 rows.

use std::ptr::null_mut;
use std::time::{Duration, Instant};

use esp_idf_svc::hal::gpio::{AnyIOPin, OutputPin};
use esp_idf_svc::hal::spi::{Dma, SpiAnyPins, SpiDriver, SpiDriverConfig};
use esp_idf_svc::sys::*;
use log::{info, warn};

use crate::board::{Controller, BOARD};
use crate::indicator::{self, Event, Indicator, State};
use crate::screen::{self, Frame, Page, System, View, HEIGHT, WIDTH};
use crate::status::{self, SharedStatus};

const PIN_CS: i32 = BOARD.lcd_cs;
const PIN_DC: i32 = BOARD.lcd_dc;
const PIN_RST: i32 = BOARD.lcd_reset;
const PIN_BACKLIGHT: i32 = BOARD.lcd_backlight;
const PIXEL_CLOCK_HZ: u32 = 40_000_000;
/// Where the pages start in the controller's 240 rows.
const GAP: i32 = BOARD.lcd_gap;
/// How landscape is mirrored for the board's panel (board.rs).
const MIRROR_X: bool = BOARD.lcd_mirror_x;
const MIRROR_Y: bool = BOARD.lcd_mirror_y;

/// The ILI9341's power, VCOM, frame rate and gamma settings, after
/// esp_lcd's sleep-out, MADCTL and COLMOD (Espressif's esp_lcd_ili9341
/// vendor_specific_init_default, as Adafruit's driver sends them too).
const ILI9341_INIT: &[(i32, &[u8])] = &[
    (0xCF, &[0x00, 0xC1, 0x30]),                   // power control B
    (0xED, &[0x64, 0x03, 0x12, 0x81]),             // power on sequence
    (0xE8, &[0x85, 0x00, 0x78]),                   // driver timing A
    (0xCB, &[0x39, 0x2C, 0x00, 0x34, 0x02]),       // power control A
    (0xF7, &[0x20]),                               // pump ratio
    (0xEA, &[0x00, 0x00]),                         // driver timing B
    (0xC0, &[0x23]),                               // power control 1
    (0xC1, &[0x10]),                               // power control 2
    (0xC5, &[0x3E, 0x28]),                         // VCOM control 1
    (0xC7, &[0x86]),                               // VCOM control 2
    (0xB1, &[0x00, 0x18]),                         // frame rate: 79 Hz
    (0xB6, &[0x08, 0x82, 0x27]),                   // display function
    (0xF2, &[0x00]),                               // 3-gamma off
    (0x26, &[0x01]),                               // gamma curve 1
    (0xE0, &[0x0F, 0x31, 0x2B, 0x0C, 0x0E, 0x08, 0x4E, 0xF1, 0x37, 0x07, 0x10, 0x03, 0x0E, 0x09, 0x00]),
    (0xE1, &[0x00, 0x0E, 0x14, 0x03, 0x11, 0x07, 0x31, 0xC1, 0x48, 0x08, 0x0F, 0x0C, 0x31, 0x36, 0x0F]),
];

/// Rows sent per transfer, from a buffer in DMA-capable internal RAM.
const STRIP_ROWS: usize = 20;
const STRIP_BYTES: usize = WIDTH * STRIP_ROWS * 2;
const ST7789_NOP: i32 = 0x00;

const REFRESH: Duration = Duration::from_secs(1);
/// The screen switches off after this long without a button press (it
/// warms the board, and nobody watches it all the time). A press wakes it.
const SCREEN_TIMEOUT: Duration = Duration::from_secs(60);
/// The board's RGB LED: one addressable (WS2812-style) LED.
const PIN_LED: i32 = BOARD.led;
/// The BOOT button: pulled up, low while pressed.
const PIN_BUTTON: i32 = BOARD.button;
/// How often the button is checked; also its debounce time.
const BUTTON_POLL: Duration = Duration::from_millis(30);
/// The SD card's free space is looked up this often (it can be slow).
const SD_INTERVAL: Duration = Duration::from_secs(30);
const SD_MOUNT: &str = "/sd";
/// Measured peak use is about 3.5 KB; this leaves a wide margin.
const DISPLAY_STACK: usize = 12 * 1024;

pub struct Lcd {
    _spi: SpiDriver<'static>,
    io: esp_lcd_panel_io_handle_t,
    panel: esp_lcd_panel_handle_t,
    strip: *mut u8,
}

// The handles are only used from the display thread.
unsafe impl Send for Lcd {}

impl Lcd {
    pub fn new(
        spi: impl SpiAnyPins + 'static,
        sclk: impl OutputPin + 'static,
        mosi: impl OutputPin + 'static,
    ) -> Result<Self, EspError> {
        let spi = SpiDriver::new(
            spi,
            sclk,
            mosi,
            None::<AnyIOPin>,
            &SpiDriverConfig::new().dma(Dma::Auto(STRIP_BYTES)),
        )?;

        let io_config = esp_lcd_panel_io_spi_config_t {
            cs_gpio_num: PIN_CS,
            dc_gpio_num: PIN_DC,
            spi_mode: 0,
            pclk_hz: PIXEL_CLOCK_HZ,
            trans_queue_depth: 10,
            lcd_cmd_bits: 8,
            lcd_param_bits: 8,
            ..Default::default()
        };
        let mut io: esp_lcd_panel_io_handle_t = null_mut();
        esp!(unsafe { esp_lcd_new_panel_io_spi(spi.host() as _, &io_config, &mut io) })?;

        // RGB element order and big-endian data are the zero defaults.
        let panel_config = esp_lcd_panel_dev_config_t {
            reset_gpio_num: PIN_RST,
            bits_per_pixel: 16,
            ..Default::default()
        };
        let mut panel: esp_lcd_panel_handle_t = null_mut();
        esp!(unsafe { esp_lcd_new_panel_st7789(io, &panel_config, &mut panel) })?;
        unsafe {
            // With no reset pin, this sends the controller a software reset.
            esp!(esp_lcd_panel_reset(panel))?;
            esp!(esp_lcd_panel_init(panel))?;
            if BOARD.lcd_controller == Controller::Ili9341 {
                for (cmd, params) in ILI9341_INIT {
                    esp!(esp_lcd_panel_io_tx_param(io, *cmd, params.as_ptr().cast(), params.len()))?;
                }
            }
            // An IPS panel: colours are inverted unless told otherwise.
            esp!(esp_lcd_panel_invert_color(panel, BOARD.lcd_invert))?;
            // Landscape: swap axes plus one mirror is a 90 degree turn.
            esp!(esp_lcd_panel_swap_xy(panel, true))?;
            esp!(esp_lcd_panel_mirror(panel, MIRROR_X, MIRROR_Y))?;
            if BOARD.lcd_bgr {
                // Blue and red the other way round: MADCTL as esp_lcd just
                // set it (MV for the swap, MX or MY for the mirror), plus
                // the BGR bit.
                let madctl: u8 = 0x20 | (if MIRROR_X { 0x40 } else { 0 }) | (if MIRROR_Y { 0x80 } else { 0 }) | 0x08;
                esp!(esp_lcd_panel_io_tx_param(io, 0x36, [madctl].as_ptr().cast(), 1))?;
            }
        }

        let strip = unsafe { heap_caps_malloc(STRIP_BYTES, MALLOC_CAP_DMA | MALLOC_CAP_INTERNAL) } as *mut u8;
        if strip.is_null() {
            return Err(EspError::from(ESP_ERR_NO_MEM as esp_err_t).unwrap());
        }
        let lcd = Self { _spi: spi, io, panel, strip };

        // Clear the whole panel before switching the display and backlight
        // on, so its random power-up contents never show: then the pages
        // go at the gap (after the swap, the 172-pixel axis is y).
        lcd.clear_rows(BOARD.lcd_rows)?;
        unsafe {
            esp!(esp_lcd_panel_set_gap(panel, 0, GAP))?;
        }
        lcd.show(&Frame::new())?;
        unsafe {
            esp!(esp_lcd_panel_disp_on_off(panel, true))?;
            esp!(gpio_reset_pin(PIN_BACKLIGHT as _))?;
            esp!(gpio_set_direction(PIN_BACKLIGHT as _, gpio_mode_t_GPIO_MODE_OUTPUT))?;
            esp!(gpio_set_level(PIN_BACKLIGHT as _, 1))?;
        }
        Ok(lcd)
    }

    /// Black over the first `rows` rows, with no gap set: all of a panel
    /// taller than the pages.
    fn clear_rows(&self, rows: i32) -> Result<(), EspError> {
        let buf = unsafe { std::slice::from_raw_parts_mut(self.strip, STRIP_BYTES) };
        buf.fill(0);
        let mut y = 0;
        while y < rows {
            let n = (STRIP_ROWS as i32).min(rows - y);
            unsafe {
                esp!(esp_lcd_panel_draw_bitmap(self.panel, 0, y, WIDTH as i32, y + n, self.strip as *const _))?;
                esp!(esp_lcd_panel_io_tx_param(self.io, ST7789_NOP, std::ptr::null(), 0))?;
            }
            y += n;
        }
        Ok(())
    }

    /// Switch the backlight and the panel on or off.
    pub fn set_on(&self, on: bool) -> Result<(), EspError> {
        unsafe {
            esp!(gpio_set_level(PIN_BACKLIGHT as _, on as u32))?;
            esp!(esp_lcd_panel_disp_on_off(self.panel, on))?;
        }
        Ok(())
    }

    /// Send a whole frame, a strip at a time.
    pub fn show(&self, frame: &Frame) -> Result<(), EspError> {
        let buf = unsafe { std::slice::from_raw_parts_mut(self.strip, STRIP_BYTES) };
        let mut y = 0;
        while y < HEIGHT {
            let rows = STRIP_ROWS.min(HEIGHT - y);
            frame.write_be(y..y + rows, &mut buf[..rows * WIDTH * 2]);
            unsafe {
                esp!(esp_lcd_panel_draw_bitmap(self.panel, 0, y as i32, WIDTH as i32, (y + rows) as i32, self.strip as *const _))?;
                // draw_bitmap only queues the transfer. Any command waits
                // for queued pixel data to finish first, so a NOP makes it
                // safe to refill the buffer.
                esp!(esp_lcd_panel_io_tx_param(self.io, ST7789_NOP, std::ptr::null(), 0))?;
            }
            y += rows;
        }
        Ok(())
    }
}

/// Start the LCD and a thread that redraws the status screen every second.
/// If the LCD can't be started, the server carries on without it.
pub fn start(
    status: SharedStatus,
    spi: impl SpiAnyPins + 'static,
    sclk: impl OutputPin + 'static,
    mosi: impl OutputPin + 'static,
) {
    let lcd = match Lcd::new(spi, sclk, mosi) {
        Ok(lcd) => lcd,
        Err(e) => {
            warn!("LCD not available: {e}");
            return;
        }
    };
    info!("LCD started");
    let spawned = std::thread::Builder::new()
        .name("display".into())
        .stack_size(DISPLAY_STACK)
        .spawn(move || run(lcd, status));
    if let Err(e) = spawned {
        warn!("Starting the display thread: {e}");
    }
}

fn run(lcd: Lcd, status: SharedStatus) {
    let mut frame = Frame::new();
    let mut page = Page::Overview;
    let button = Button::new();
    let mut cpu = CpuLoad::new();
    let mut stacks_reported: Option<Instant> = None;
    let thermometer = Thermometer::new();
    let mut last = (Instant::now(), 0u64, 0u64);
    let mut last_server = (0u64, 0u64, 0u64); // busy_us, block_us, blocks
    let mut block_ms = None;
    // When the SD card was last checked, and what it said.
    let mut sd: (Option<Instant>, Option<(u64, u64)>) = (None, None);
    let mut drawn: Option<Instant> = None;
    let mut last_press = Instant::now();
    let mut screen_on = true;
    let led = RgbLed::new();
    let mut indicator = Indicator::new();
    let mut shown: Option<[u8; 3]> = None;
    let mut ticked = Instant::now();
    // Counts already flashed for (joins, leaves, chats, problems).
    let mut seen = {
        let st = status::lock(&status);
        (st.joins, st.leaves, st.chats, status::problems())
    };
    // Whether the board runs its own hotspot. Decided at start-up, after
    // this thread starts; when it is, the Join page (its QR code) is shown.
    let mut hotspot = false;
    // The hotspot hold bar as last drawn (see Status::hotspot_hold).
    let mut hold_drawn: Option<f32> = None;
    // The chip's temperature, read every few seconds for the LED's colour
    // (and shown on the Device page).
    let mut chip_temp: Option<f32> = None;
    let mut temp_read: Option<Instant> = None;
    loop {
        if temp_read.map_or(true, |t| t.elapsed() >= TEMP_INTERVAL) {
            chip_temp = thermometer.as_ref().and_then(Thermometer::read);
            temp_read = Some(Instant::now());
        }
        if stacks_reported.map_or(true, |t| t.elapsed() >= STACK_REPORT) {
            report_stacks();
            stacks_reported = Some(Instant::now());
        }
        // The LED, whether the screen is on or off: breathing and flashing
        // (see indicator.rs).
        let (state, counts, hotspot_now, window, hold) = {
            let st = status::lock(&status);
            let state = if st.address.starts_with("Wi-Fi") {
                State::NoWifi
            } else {
                // Breathing in the temperature's colour: green, amber when
                // warm, red when hot.
                let colour = chip_temp.map_or(screen::chip_colour(0.0), screen::chip_colour);
                State::Running { players: st.players.len(), colour }
            };
            (state, (st.joins, st.leaves, st.chats, status::problems()), st.hotspot.is_some(), st.hotspot_window, st.hotspot_hold)
        };
        if hotspot_now && !hotspot {
            hotspot = true;
            page = Page::Join;
            drawn = None;
            last_press = Instant::now();
            if !screen_on {
                screen_on = true;
                if let Err(e) = lcd.set_on(true) {
                    warn!("LCD on: {e}");
                }
            }
        }
        let news = [
            (counts.0 > seen.0, Event::Join),
            (counts.1 > seen.1, Event::Leave),
            (counts.2 > seen.2, Event::Chat),
            (counts.3 > seen.3, Event::Problem),
        ];
        seen = counts;
        let dt = ticked.elapsed();
        ticked = Instant::now();
        if let Some(led) = &led {
            for (new, e) in news {
                if new {
                    indicator.event(e);
                }
            }
            let c = indicator.colour(dt, state);
            let (r, g, b) = (indicator::to_byte(c.0), indicator::to_byte(c.1), indicator::to_byte(c.2));
            let bytes = if BOARD.led_grb { [g, r, b] } else { [r, g, b] };
            if shown != Some(bytes) {
                led.set(bytes);
                shown = Some(bytes);
            }
        }

        // While BOOT is watched for a hotspot at start-up, a press is the
        // start of that hold, not a page turn: the bar shows it instead.
        let pressed = button.pressed() && !window;
        let hold_changed = hold != hold_drawn;
        if hold_changed {
            hold_drawn = hold;
            last_press = Instant::now();
        }
        if pressed {
            last_press = Instant::now();
            if screen_on {
                page = page.next(hotspot);
            } else {
                // The first press only wakes the screen, on the page it showed.
                screen_on = true;
                drawn = None;
                if let Err(e) = lcd.set_on(true) {
                    warn!("LCD on: {e}");
                }
                continue;
            }
        }
        if screen_on && last_press.elapsed() >= SCREEN_TIMEOUT {
            screen_on = false;
            if let Err(e) = lcd.set_on(false) {
                warn!("LCD off: {e}");
            }
        }
        if !screen_on {
            std::thread::sleep(BUTTON_POLL);
            continue;
        }
        if !pressed && !hold_changed && drawn.is_some_and(|t| t.elapsed() < REFRESH) {
            std::thread::sleep(BUTTON_POLL);
            continue;
        }
        drawn = Some(Instant::now());

        let snapshot = status::lock(&status).clone();
        let now = Instant::now();
        let dt = now.duration_since(last.0).as_secs_f32().max(0.001);
        // Server load and block time since the last redraw.
        let busy = (snapshot.busy_us - last_server.0) as f32 / (dt * 1e6) * 100.0;
        let blocks = snapshot.blocks_built - last_server.2;
        if blocks > 0 {
            block_ms = Some((snapshot.block_us - last_server.1) as f32 / blocks as f32 / 1000.0);
        } else if block_ms.is_none() && snapshot.blocks_built > 0 {
            block_ms = Some(snapshot.block_us as f32 / snapshot.blocks_built as f32 / 1000.0);
        }
        last_server = (snapshot.busy_us, snapshot.block_us, snapshot.blocks_built);
        // Only while the Device page is up: the first look can mean reading
        // the card's whole allocation table, and the server waits for the
        // card meanwhile.
        if page == Page::Device && sd.0.map_or(true, |t| t.elapsed() >= SD_INTERVAL) {
            sd = (Some(Instant::now()), sd_space());
        }
        let (rssi, channel, hotspot_clients) = wifi(hotspot);
        let sys = unsafe {
            System {
                internal_free_kb: heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) as u32 / 1024,
                internal_min_kb: heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) as u32 / 1024,
                internal_largest_kb: heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) as u32 / 1024,
                psram_free_kb: heap_caps_get_free_size(MALLOC_CAP_SPIRAM) as u32 / 1024,
                psram_min_kb: heap_caps_get_minimum_free_size(MALLOC_CAP_SPIRAM) as u32 / 1024,
                rssi,
                channel,
                hotspot_clients,
                packets_in_per_s: (snapshot.packets_in - last.1) as f32 / dt,
                packets_out_per_s: (snapshot.packets_out - last.2) as f32 / dt,
                cpu: cpu.sample(),
                server_busy: busy.clamp(0.0, 100.0),
                block_ms,
                sd: sd.1,
                chip_temp,
            }
        };
        last = (now, snapshot.packets_in, snapshot.packets_out);

        screen::render(&mut frame, &View { page, status: &snapshot, system: &sys });
        if let Err(e) = lcd.show(&frame) {
            warn!("LCD update failed: {e}");
        }
    }
}

/// The RGB LED, driven by the RMT peripheral: each bit is a high pulse,
/// short for 0 and long for 1, 24 bits per colour. Unlike most WS2812s,
/// this board's LED takes red, green, blue in that order.
struct RgbLed {
    channel: rmt_channel_handle_t,
    encoder: rmt_encoder_handle_t,
}

impl RgbLed {
    /// RMT ticks per second: 0.1 us each.
    const RESOLUTION_HZ: u32 = 10_000_000;

    fn new() -> Option<Self> {
        // One RMT symbol: `high` ticks high, then `low` ticks low.
        let symbol = |high: u32, low: u32| rmt_symbol_word_t { val: high | 1 << 15 | low << 16 };
        let started = unsafe {
            let channel_config = rmt_tx_channel_config_t {
                gpio_num: PIN_LED,
                clk_src: soc_periph_rmt_clk_src_t_RMT_CLK_SRC_DEFAULT,
                resolution_hz: Self::RESOLUTION_HZ,
                mem_block_symbols: 48,
                trans_queue_depth: 2,
                ..Default::default()
            };
            let mut channel: rmt_channel_handle_t = null_mut();
            let mut encoder: rmt_encoder_handle_t = null_mut();
            // WS2812 timing: a 0 is 0.3 us high then 0.9 us low; a 1 the
            // other way round.
            let mut encoder_config = rmt_bytes_encoder_config_t {
                bit0: symbol(3, 9),
                bit1: symbol(9, 3),
                ..Default::default()
            };
            encoder_config.flags.set_msb_first(1);
            esp!(rmt_new_tx_channel(&channel_config, &mut channel))
                .and_then(|_| esp!(rmt_new_bytes_encoder(&encoder_config, &mut encoder)))
                .and_then(|_| esp!(rmt_enable(channel)))
                .map(|_| Self { channel, encoder })
        };
        match started {
            Ok(led) => {
                led.set([0; 3]); // it may light up at power-on
                info!("RGB LED on GPIO {}, colour order {}", PIN_LED, if BOARD.led_grb { "GRB" } else { "RGB" });
                Some(led)
            }
            Err(e) => {
                warn!("RGB LED not available: {e}");
                None
            }
        }
    }

    /// Show a colour: red, green, blue power, 0 to 255.
    fn set(&self, rgb: [u8; 3]) {
        let config = rmt_transmit_config_t::default();
        unsafe {
            if esp!(rmt_transmit(self.channel, self.encoder, rgb.as_ptr() as *const _, rgb.len(), &config)).is_ok() {
                // A few tens of microseconds; the buffer must outlive it.
                let _ = rmt_tx_wait_all_done(self.channel, 10);
            }
        }
    }
}

/// The BOOT button, with a press reported once (on going down).
struct Button {
    was_down: std::cell::Cell<bool>,
}

impl Button {
    fn new() -> Self {
        unsafe {
            gpio_reset_pin(PIN_BUTTON as _);
            gpio_set_direction(PIN_BUTTON as _, gpio_mode_t_GPIO_MODE_INPUT);
            gpio_set_pull_mode(PIN_BUTTON as _, gpio_pull_mode_t_GPIO_PULLUP_ONLY);
        }
        Self { was_down: std::cell::Cell::new(false) }
    }

    /// Whether the button has been pressed since the last call. Polled
    /// every BUTTON_POLL, which also rides out contact bounce.
    fn pressed(&self) -> bool {
        let down = unsafe { gpio_get_level(PIN_BUTTON as _) } == 0;
        let pressed = down && !self.was_down.get();
        self.was_down.set(down);
        pressed
    }
}

/// How often to report every task's unused stack.
const STACK_REPORT: Duration = Duration::from_secs(60);
/// How often the chip temperature is read for the LED.
const TEMP_INTERVAL: Duration = Duration::from_secs(2);
/// Below this much unused stack (bytes), one of our tasks is reported as a
/// warning.
const STACK_LOW: u32 = 1536;

/// Log how much of its stack each task has never used (FreeRTOS's high
/// water mark; on ESP-IDF stacks are counted in bytes). A task that runs
/// out of stack overwrites the memory next to it, so this is worth
/// watching as the server grows.
fn report_stacks() {
    let mut tasks: Vec<TaskStatus_t> = Vec::new();
    let mut total: u32 = 0;
    let n = unsafe {
        let capacity = uxTaskGetNumberOfTasks() + 4;
        tasks.reserve(capacity as usize);
        let n = uxTaskGetSystemState(tasks.as_mut_ptr(), capacity, &mut total);
        tasks.set_len(n as usize);
        n
    };
    let mut list: Vec<(String, u32)> = tasks[..n as usize]
        .iter()
        .map(|t| {
            let name = unsafe { std::ffi::CStr::from_ptr(t.pcTaskName) }.to_string_lossy().into_owned();
            (name, t.usStackHighWaterMark as u32)
        })
        .collect();
    list.sort_by_key(|(_, free)| *free);
    let text: Vec<String> = list.iter().map(|(name, free)| format!("{name} {free}")).collect();
    // Only our own tasks: some of ESP-IDF's (ipc0, ipc1, Tmr Svc) always run
    // with a few hundred bytes to spare. Rust threads show as "pthread".
    let ours = |name: &str| matches!(name, "main" | "blocks" | "srp" | "pthread");
    if list.iter().any(|(name, free)| ours(name) && *free < STACK_LOW) {
        warn!("Stack left (bytes), lowest first: {}", text.join(", "));
        crate::status::problem();
    } else {
        info!("Stack left (bytes), lowest first: {}", text.join(", "));
    }
    // The Wi-Fi driver and network stack keep their buffers in internal RAM
    // (for speed, see sdkconfig.defaults): watch how low it gets.
    let caps = MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT;
    let (free, lowest) = unsafe { (heap_caps_get_free_size(caps) / 1024, heap_caps_get_minimum_free_size(caps) / 1024) };
    if lowest < INTERNAL_LOW_KB {
        warn!("Internal RAM: {free} KB free, lowest so far {lowest} KB");
        crate::status::problem();
    } else {
        info!("Internal RAM: {free} KB free, lowest so far {lowest} KB");
    }
}

/// Below this much free internal RAM (KB), the report becomes a warning.
const INTERNAL_LOW_KB: usize = 16;

/// CPU load per core, from how long each core's FreeRTOS idle task ran
/// (needs CONFIG_FREERTOS_GENERATE_RUN_TIME_STATS; the run-time clock is
/// esp_timer, in microseconds).
struct CpuLoad {
    idle: [u32; 2],
    at: u64,
}

impl CpuLoad {
    fn new() -> Self {
        let mut c = Self { idle: [0; 2], at: 0 };
        c.sample();
        c
    }

    /// Load since the last call, in percent.
    fn sample(&mut self) -> [Option<f32>; 2] {
        let now = unsafe { esp_timer_get_time() } as u64;
        let mut tasks: Vec<TaskStatus_t> = Vec::with_capacity(32);
        let mut total: u32 = 0;
        let n = unsafe {
            let capacity = uxTaskGetNumberOfTasks() + 4;
            tasks.reserve(capacity as usize);
            let n = uxTaskGetSystemState(tasks.as_mut_ptr(), capacity, &mut total);
            tasks.set_len(n as usize);
            n
        };
        let mut idle = [None; 2];
        for t in &tasks[..n as usize] {
            let name = unsafe { std::ffi::CStr::from_ptr(t.pcTaskName) }.to_bytes();
            match name {
                b"IDLE0" => idle[0] = Some(t.ulRunTimeCounter as u32),
                b"IDLE1" => idle[1] = Some(t.ulRunTimeCounter as u32),
                _ => {}
            }
        }
        let elapsed = now.saturating_sub(self.at).max(1) as f32;
        let mut load = [None; 2];
        for core in 0..2 {
            if let Some(i) = idle[core] {
                let ran = i.wrapping_sub(self.idle[core]) as f32;
                if self.at != 0 {
                    load[core] = Some((100.0 - ran / elapsed * 100.0).clamp(0.0, 100.0));
                }
                self.idle[core] = i;
            }
        }
        self.at = now;
        load
    }
}

/// The ESP32-S3's built-in temperature sensor (on the chip itself, so it
/// reads warmer than the air around the board).
struct Thermometer {
    handle: temperature_sensor_handle_t,
}

impl Thermometer {
    fn new() -> Option<Self> {
        // The sensor has a few fixed ranges; 20 to 100 C suits a board that
        // runs warm.
        let config = temperature_sensor_config_t {
            range_min: 20,
            range_max: 100,
            // The clock source is left at 0, which the driver takes as its default.
            ..Default::default()
        };
        let mut handle: temperature_sensor_handle_t = null_mut();
        let started = unsafe {
            esp!(temperature_sensor_install(&config, &mut handle)).and_then(|_| esp!(temperature_sensor_enable(handle)))
        };
        match started {
            Ok(()) => Some(Self { handle }),
            Err(e) => {
                warn!("Temperature sensor not available: {e}");
                None
            }
        }
    }

    fn read(&self) -> Option<f32> {
        let mut celsius = 0.0f32;
        (unsafe { temperature_sensor_get_celsius(self.handle, &mut celsius) } == 0).then_some(celsius)
    }
}

/// The SD card's size and free space, in bytes.
fn sd_space() -> Option<(u64, u64)> {
    let path = std::ffi::CString::new(SD_MOUNT).ok()?;
    let (mut total, mut free) = (0u64, 0u64);
    (unsafe { esp_vfs_fat_info(path.as_ptr(), &mut total, &mut free) } == 0).then_some((total, free))
}

/// Signal strength and channel when joined to a network; on the board's
/// own hotspot, its channel and how many devices are on it.
fn wifi(hotspot: bool) -> (Option<i8>, Option<u8>, Option<u8>) {
    if hotspot {
        let mut list: wifi_sta_list_t = Default::default();
        let clients = (unsafe { esp_wifi_ap_get_sta_list(&mut list) } == 0).then_some(list.num as u8);
        let (mut primary, mut second) = (0u8, 0 as wifi_second_chan_t);
        let channel = (unsafe { esp_wifi_get_channel(&mut primary, &mut second) } == 0).then_some(primary);
        return (None, channel, clients);
    }
    let mut ap: wifi_ap_record_t = Default::default();
    if unsafe { esp_wifi_sta_get_ap_info(&mut ap) } == 0 {
        (Some(ap.rssi), Some(ap.primary), None)
    } else {
        (None, None, None)
    }
}
