mod auth;
mod beds;
mod binoculars;
mod board;
mod books;
mod bones;
mod chest;
mod clock;
mod colours;
mod config_page;
mod content;
mod crafting;
mod craftguide;
mod doors;
mod env_sounds;
mod dungeons;
mod falling;
mod fire;
mod farming;
mod fences;
mod fasthash;
mod font;
mod furnace;
mod indicator;
mod boats;
mod carts;
mod creative;
mod insects;
mod inventory;
mod items;
mod jsonstring;
mod keys;
mod lcd;
mod liquids;
mod messages;
mod mobs;
mod meta;
mod nature;
mod panes;
mod plants;
mod privs;
mod player;
mod protocol;
mod screen;
mod screwdriver;
mod server;
mod settings;
mod sha1;
mod sha256;
mod skins;
mod srp;
mod stairs;
mod status;
mod storage;
mod terrain;
mod tnt;
mod weather;
mod wifi_config;
mod world;
mod zstd;

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs;
use std::net::UdpSocket;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::sd::mmc::{SdMmcHostConfiguration, SdMmcHostDriver};
use esp_idf_svc::hal::sd::{SdCardConfiguration, SdCardDriver};
use esp_idf_svc::io::vfs::MountedFatfs;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AccessPointConfiguration, BlockingWifi, ClientConfiguration, Configuration, EspWifi, WifiDriver};
use log::info;

const MOUNT: &str = "/sd";
/// Wi-Fi networks, editable on a PC (see wifi_config.rs).
const WIFI_FILE: &str = "/sd/wifi.txt";
/// Which built-in network was last written to WIFI_FILE.
const WIFI_WRITTEN: &str = "/sd/wifi-written";
/// Left by the settings page to restart on the hotspot (config_page.rs).
const HOTSPOT_NEXT: &str = "/sd/boot-hotspot";
/// How long to wait before looking for known networks again.
const WIFI_RETRY: std::time::Duration = std::time::Duration::from_secs(10);
/// How often to check that the Wi-Fi connection is still there.
const WIFI_CHECK: std::time::Duration = std::time::Duration::from_secs(2);
const PORT: u16 = 30000;
/// The BOOT button: GPIO 0, pulled up, low while pressed.
const PIN_BOOT: i32 = 0;
/// How long after start-up the BOOT button is watched, and how long it
/// must be held, to run the board's own hotspot, or to become a USB
/// drive. (Holding it while the board powers up
/// doesn't work: the chip then waits to be flashed.)
const BOOT_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);
const HOTSPOT_HOLD: std::time::Duration = std::time::Duration::from_secs(1);
const DRIVE_HOLD: std::time::Duration = std::time::Duration::from_secs(3);
/// The hotspot's address (the gateway of its own 192.168.4.x network) and
/// radio channel, and how many devices may join it.
const HOTSPOT_IP: std::net::Ipv4Addr = std::net::Ipv4Addr::new(192, 168, 4, 1);
const HOTSPOT_CHANNEL: u8 = 1;
const HOTSPOT_MAX_DEVICES: u16 = 8;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    // The USB pins to the serial port, whatever a USB drive session before
    // the last restart left set (see give_usb_to_serial).
    give_usb_to_serial();
    esp_idf_svc::log::EspLogger::initialize_default();
    log_heap("start");

    let p = Peripherals::take()?;
    let pins = p.pins;

    info!("Built for the {}", board::BOARD.name);
    // A speaker amplifier stays off: nothing here makes sound.
    if let Some((pin, off)) = board::BOARD.amp_off {
        unsafe {
            esp_idf_svc::sys::gpio_reset_pin(pin);
            esp_idf_svc::sys::gpio_set_direction(pin, esp_idf_svc::sys::gpio_mode_t_GPIO_MODE_OUTPUT);
            esp_idf_svc::sys::gpio_set_level(pin, off);
        }
    }

    // Storage: the SD card (its slot's 4-bit SDMMC pins depend on the
    // board, see board.rs) if there is one, otherwise the board's own
    // flash. Either is mounted at /sd, so the rest of the server doesn't
    // know which.
    let card = (|| -> anyhow::Result<_> {
        #[cfg(not(board = "es3c28p"))]
        let host = SdMmcHostDriver::new_4bits(
            p.sdmmc1,
            pins.gpio15, // CMD
            pins.gpio14, // CLK
            pins.gpio16, // D0
            pins.gpio18, // D1
            pins.gpio17, // D2
            pins.gpio21, // D3
            None::<AnyIOPin>, // no card-detect line
            None::<AnyIOPin>, // no write-protect line
            &SdMmcHostConfiguration::new(),
        )?;
        #[cfg(board = "es3c28p")]
        let host = SdMmcHostDriver::new_4bits(
            p.sdmmc1,
            pins.gpio40, // CMD
            pins.gpio38, // CLK
            pins.gpio39, // D0
            pins.gpio41, // D1
            pins.gpio48, // D2
            pins.gpio47, // D3
            None::<AnyIOPin>, // no card-detect line
            None::<AnyIOPin>, // no write-protect line
            &SdMmcHostConfiguration::new(),
        )?;
        Ok(SdCardDriver::new_mmc(host, &SdCardConfiguration::new())?)
    })();
    // The card's driver stays here, where it doesn't move: the mount
    // borrows it, and the USB drive reads the card through it.
    let mut card = card;
    let sd = match card.as_mut() {
        Ok(card) => {
            let info = card.card() as *const esp_idf_svc::sys::sdmmc_card_t;
            Fatfs::new_sdcard(0, card)
                .and_then(|fatfs| MountedFatfs::mount(fatfs, MOUNT, 4))
                .map(|mounted| (mounted, info))
                .map_err(anyhow::Error::from)
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    };
    // The mount is kept until the server stops: dropping it would unmount
    // the card. And what the USB drive would show: the card, or the flash.
    let (sd, drive) = match sd {
        Ok((mounted, info)) => {
            info!("SD card mounted at {MOUNT}");
            (Some(mounted), DriveStorage::Card(info))
        }
        Err(e) => {
            log::warn!("No SD card ({e}): keeping the game in the board's own flash");
            let flash = mount_flash()?;
            status::set_on_flash();
            (None, DriveStorage::Flash(flash))
        }
    };
    reset_sd_if_asked();
    for entry in fs::read_dir(MOUNT)? {
        info!("  {:?}", entry?.file_name());
    }

    // The status screen starts now, so it can say what Wi-Fi is doing.
    let status = status::Status::new("Wi-Fi...".to_string());
    // The screen's SPI bus: SCLK and MOSI.
    #[cfg(not(board = "es3c28p"))]
    lcd::start(status.clone(), p.spi2, pins.gpio40, pins.gpio45);
    #[cfg(board = "es3c28p")]
    lcd::start(status.clone(), p.spi2, pins.gpio12, pins.gpio11);

    // Wi-Fi: the networks in wifi.txt, plus one given when building
    // (WIFI_SSID, WIFI_PASS), which is written into wifi.txt; or, if BOOT
    // is held just after start-up, the hotspot in wifi.txt.
    let built_in = option_env!("WIFI_SSID").filter(|s| !s.is_empty()).map(|ssid| wifi_config::Network {
        ssid: ssid.to_string(),
        password: option_env!("WIFI_PASS").unwrap_or("").to_string(),
    });
    let config = wifi_config::load(&wifi_config::Files { networks: WIFI_FILE, written: WIFI_WRITTEN }, built_in, &board_id());
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    // Kept until the server stops: dropping it would switch the hotspot off.
    let mut _hotspot_wifi = None;
    // Restarted by the settings page, which asked for the hotspot again:
    // no BOOT window. Otherwise BOOT decides.
    let boot = if config_page::take_hotspot_marker(std::path::Path::new(HOTSPOT_NEXT)) {
        info!("Back on the hotspot, as the settings page asked");
        Boot::Hotspot
    } else {
        boot_choice(&status)
    };
    if boot == Boot::Drive {
        // The server's own use of the files stops first.
        drop(sd);
        usb_drive(drive, &status);
    }
    let _sd = sd;
    let ip = if boot == Boot::Hotspot {
        let driver = WifiDriver::new(p.modem, sysloop.clone(), Some(nvs))?;
        let wifi = start_hotspot(driver, sysloop, &config.hotspot, &status)?;
        let ip = wifi.wifi().ap_netif().get_ip_info()?.ip;
        _hotspot_wifi = Some(wifi);
        ip
    } else {
        let mut wifi = BlockingWifi::wrap(
            EspWifi::new(p.modem, sysloop.clone(), Some(nvs))?,
            sysloop,
        )?;
        // No network yet, just scanning. (The default asks for WPA2 with an
        // empty password, which the driver warns about.)
        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            auth_method: esp_idf_svc::wifi::AuthMethod::None,
            ..Default::default()
        }))?;
        wifi.start()?;
        let networks = config.networks;
        let joined = join_wifi(&mut wifi, &networks, &status);
        let ip = wifi.wifi().sta_netif().get_ip_info()?.ip;
        info!("Connected to {joined}, IP address {ip}");
        {
            let mut st = status::lock(&status);
            st.address = format!("{ip}:{PORT}");
            st.event(format!("Joined Wi-Fi {joined}"));
        }
        // Power saving makes the radio doze between beacons, which delays and
        // drops outgoing packets. A server should stay awake.
        esp_idf_svc::sys::esp!(unsafe {
            esp_idf_svc::sys::esp_wifi_set_ps(esp_idf_svc::sys::wifi_ps_type_t_WIFI_PS_NONE)
        })?;
        // ESP-IDF doesn't rejoin a network on its own: a thread watches the
        // connection and rejoins if it drops (the router restarts, or the board
        // loses signal). The server carries on meanwhile, and players whose
        // clients don't time out (30 s) carry on playing.
        std::thread::Builder::new()
            .name("wifi".into())
            .stack_size(8 * 1024)
            .spawn({
                let status = status.clone();
                move || watch_wifi(wifi, &networks, &status, joined)
            })?;
        ip
    };
    log_heap("after Wi-Fi");

    // Luanti server
    let auth = auth::AuthStore::open(format!("{MOUNT}/auth"))?;
    let world = world::World::open(format!("{MOUNT}/world"))?;
    let inventories = inventory::InventoryStore::open(format!("{MOUNT}/players"))?;
    let players = player::PlayerStore::open(format!("{MOUNT}/players"))?;
    let furnaces = furnace::Furnaces::open(format!("{MOUNT}/world/furnaces.txt"));
    let chests = chest::Chests::open(format!("{MOUNT}/world/chests.txt"));
    let items = items::Items::open(format!("{MOUNT}/world/items.txt"));
    let clock = clock::Clock::open(format!("{MOUNT}/world/env_meta.txt"));
    let door_owners = doors::Owners::open(format!("{MOUNT}/world/doors.txt"));
    let bed_spawns = beds::Spawns::open(format!("{MOUNT}/world/beds_spawns"));
    let settings = settings::Settings::open(format!("{MOUNT}/world/settings.txt"));
    let media = content::Media::load(&format!("{MOUNT}/media"));
    let socket = UdpSocket::bind(("0.0.0.0", PORT))?;
    let sent = std::sync::Arc::as_ptr(&status) as usize;
    // An optional welcome message of your own, one chat line per line.
    let motd = fs::read_to_string(format!("{MOUNT}/motd.txt")).ok();
    if motd.is_some() {
        info!("Welcome message from {MOUNT}/motd.txt");
    }
    let mut server = server::Server::new(server::Setup { socket, motd, auth, world, inventories, players, furnaces, chests, items, clock, door_owners, bed_spawns, settings, media, status, max_players: server::MAX_PLAYERS });
    // Guard against the compiler problem that once lost an argument on the
    // way into Server::new (see server::Setup).
    if server.status_data_addr() != sent {
        log::error!("Server was built with a corrupted status pointer; expect a crash");
    }
    // The settings page, on the hotspot only (see config_page.rs).
    if boot == Boot::Hotspot {
        let files = config_page::Files {
            wifi: WIFI_FILE.into(),
            motd: format!("{MOUNT}/motd.txt").into(),
            settings: format!("{MOUNT}/world/settings.txt").into(),
            hotspot_next: HOTSPOT_NEXT.into(),
        };
        if let Err(e) = config_page::start(config_page::PORT, files) {
            log::warn!("No settings page: {e}");
        }
    }
    log_heap("server ready");
    info!("Luanti server listening on {ip}:{PORT}");
    server.run()?;

    Ok(())
}

/// The server's own memory (map caches, packets, inventories and so on)
/// goes in the 8 MB of PSRAM. Left to the default, every allocation under
/// 16 KB goes in the ESP32's 300 KB or so of internal RAM until that is
/// full, and then Wi-Fi buffers and new tasks (which must be in internal
/// RAM) run short: sends fail with "Not enough space" and threads can't
/// start. Internal RAM is used only if PSRAM is full.
struct PreferPsram;

const PSRAM: u32 = esp_idf_svc::sys::MALLOC_CAP_SPIRAM | esp_idf_svc::sys::MALLOC_CAP_8BIT;
/// heap_caps_malloc's alignment (as Rust's own allocator assumes on ESP-IDF).
const MALLOC_ALIGN: usize = 4;

unsafe impl GlobalAlloc for PreferPsram {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        use esp_idf_svc::sys::*;
        let p: *mut u8 = if layout.align() <= MALLOC_ALIGN {
            heap_caps_malloc(layout.size(), PSRAM).cast()
        } else {
            heap_caps_aligned_alloc(layout.align(), layout.size(), PSRAM).cast()
        };
        if p.is_null() {
            System.alloc(layout)
        } else {
            p
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // Frees from any heap, PSRAM or internal, aligned or not.
        esp_idf_svc::sys::heap_caps_free(ptr.cast());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.align() <= MALLOC_ALIGN {
            // Moves the block to PSRAM if it was in internal RAM.
            let p: *mut u8 = esp_idf_svc::sys::heap_caps_realloc(ptr.cast(), new_size, PSRAM).cast();
            if !p.is_null() {
                return p;
            }
        }
        let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
        let new = self.alloc(new_layout);
        if !new.is_null() {
            std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
            self.dealloc(ptr, layout);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: PreferPsram = PreferPsram;

/// What BOOT, held just after start-up, asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Boot {
    /// Not held: join Wi-Fi.
    Normal,
    /// The board's own hotspot.
    Hotspot,
    /// The game's files (the card, or the flash) as a USB drive on a PC.
    Drive,
}

/// What the USB drive shows a PC.
enum DriveStorage {
    /// The SD card, through its driver (kept in main).
    Card(*const esp_idf_svc::sys::sdmmc_card_t),
    /// The board's flash, through its wear-levelling handle (unmounted
    /// from the server's side first).
    Flash(esp_idf_svc::sys::wl_handle_t),
}

/// Watch BOOT for BOOT_WINDOW after start-up (and for as long as a hold
/// that began then goes on). Held for HOTSPOT_HOLD, then let go, the board
/// runs its own hotspot; held on to DRIVE_HOLD, it becomes a USB drive
/// instead. The screen shows the hold as a bar: blue, then orange.
fn boot_choice(status: &status::SharedStatus) -> Boot {
    use esp_idf_svc::sys::*;
    use std::time::{Duration, Instant};
    unsafe {
        gpio_set_direction(PIN_BOOT as _, gpio_mode_t_GPIO_MODE_INPUT);
        gpio_set_pull_mode(PIN_BOOT as _, gpio_pull_mode_t_GPIO_PULLUP_ONLY);
    }
    const HINTS: [&str; 2] = ["Hold BOOT now for a hotspot", "Hold it 3s for a USB drive"];
    {
        let mut st = status::lock(status);
        for hint in HINTS {
            st.event(hint.to_string());
        }
        st.boot_window = true;
    }
    // Once the choice is made, the hints go from the screen's messages.
    let done = |boot: Boot| {
        status::lock(status).events.retain(|e| !HINTS.contains(&e.as_str()));
        boot
    };
    info!("Hold BOOT for {}s now to run a hotspot", HOTSPOT_HOLD.as_secs());
    let show = |hold: Option<f32>| status::lock(status).boot_hold = hold;
    // How far along the bar a hold of this long is (see Status::boot_hold).
    let bar = |held: Duration| {
        if held < HOTSPOT_HOLD {
            held.as_secs_f32() / HOTSPOT_HOLD.as_secs_f32()
        } else {
            1.0 + (held - HOTSPOT_HOLD).as_secs_f32() / (DRIVE_HOLD - HOTSPOT_HOLD).as_secs_f32()
        }
    };
    let start = Instant::now();
    let mut held_since: Option<Instant> = None;
    while start.elapsed() < BOOT_WINDOW || held_since.is_some() {
        let down = unsafe { gpio_get_level(PIN_BOOT as _) } == 0;
        match (down, held_since) {
            (true, None) => held_since = Some(Instant::now()),
            (false, Some(t)) => {
                held_since = None;
                if t.elapsed() >= HOTSPOT_HOLD {
                    // Let go between the two: the hotspot. The bar stays
                    // full blue until it is up (start_hotspot).
                    show(Some(1.0));
                    return done(Boot::Hotspot);
                }
                show(None);
            }
            _ => {}
        }
        if let Some(t) = held_since {
            let held = t.elapsed();
            if held >= DRIVE_HOLD {
                show(Some(2.0));
                return done(Boot::Drive);
            }
            show(Some(bar(held)));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    {
        let mut st = status::lock(status);
        st.boot_window = false;
        st.boot_hold = None;
    }
    done(Boot::Normal)
}

/// Show the game's files to a PC as a USB drive (ESP-IDF's TinyUSB mass
/// storage), instead of running the server: the whole SD card, or the
/// flash partition through its wear levelling. Only one side can write a
/// FAT file system at a time, so the server's own use of them has stopped
/// (the card is unmounted by the caller, the flash here). Copy or edit the
/// files, and eject the drive: the board restarts into the game. BOOT
/// does nothing meanwhile (unplugging, once ejected, also works). The
/// board's USB carries the drive instead of the serial log meanwhile.
fn usb_drive(storage: DriveStorage, status: &status::SharedStatus) -> ! {
    use esp_idf_svc::sys;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    // Set when the PC ejects the drive: esp_tinyusb then mounts it on the
    // board's side again, and says so.
    static EJECTED: AtomicBool = AtomicBool::new(false);
    unsafe extern "C" fn mount_changed(_: *mut sys::tinyusb_msc_event_t) {
        EJECTED.store(true, Ordering::Relaxed);
    }
    let what = match storage {
        DriveStorage::Card(_) => "SD card",
        DriveStorage::Flash(_) => "Flash",
    };
    // What the screen shows: the files at the top, and the size.
    let mut files: Vec<String> = Vec::new();
    let mut bytes = 0u64;
    let started = (|| -> anyhow::Result<()> {
        // Mounted on the board's side when esp_tinyusb takes it (where
        // it mounts it again after an eject), then handed to the PC.
        let mut mount_config: sys::esp_vfs_fat_mount_config_t = unsafe { std::mem::zeroed() };
        mount_config.max_files = 4;
        match storage {
            DriveStorage::Card(card) => {
                let mut config: sys::tinyusb_msc_sdmmc_config_t = unsafe { std::mem::zeroed() };
                config.card = card as *mut _;
                config.mount_config.max_files = mount_config.max_files;
                sys::esp!(unsafe { sys::tinyusb_msc_storage_init_sdmmc(&config) })?;
            }
            DriveStorage::Flash(flash) => {
                let base = std::ffi::CString::new(MOUNT)?;
                sys::esp!(unsafe { sys::esp_vfs_fat_spiflash_unmount_rw_wl(base.as_ptr(), flash) })?;
                let label = std::ffi::CString::new(FLASH_PARTITION)?;
                let partition = unsafe {
                    sys::esp_partition_find_first(
                        sys::esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
                        sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_FAT,
                        label.as_ptr(),
                    )
                };
                anyhow::ensure!(!partition.is_null(), "no \"{FLASH_PARTITION}\" partition");
                let mut handle: sys::wl_handle_t = 0;
                sys::esp!(unsafe { sys::wl_mount(partition, &mut handle) })?;
                let mut config: sys::tinyusb_msc_spiflash_config_t = unsafe { std::mem::zeroed() };
                config.wl_handle = handle;
                config.mount_config.max_files = mount_config.max_files;
                sys::esp!(unsafe { sys::tinyusb_msc_storage_init_spiflash(&config) })?;
            }
        }
        // Kept for the life of the board: esp_tinyusb keeps the pointer.
        let base: &'static std::ffi::CStr = Box::leak(std::ffi::CString::new(MOUNT)?.into_boxed_c_str());
        sys::esp!(unsafe { sys::tinyusb_msc_storage_mount(base.as_ptr()) })?;
        // While it is still the board's, a look at what is on it.
        let mut dirs = Vec::new();
        for entry in fs::read_dir(MOUNT)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(format!("{name}/"));
            } else {
                files.push(name);
            }
        }
        dirs.sort_by_key(|n| n.to_lowercase());
        files.sort_by_key(|n| n.to_lowercase());
        files.splice(0..0, dirs);
        bytes = unsafe { sys::tinyusb_msc_storage_get_sector_count() as u64 * sys::tinyusb_msc_storage_get_sector_size() as u64 };
        sys::esp!(unsafe { sys::tinyusb_msc_storage_unmount() })?;
        sys::esp!(unsafe {
            sys::tinyusb_msc_register_callback(
                sys::tinyusb_msc_event_type_t_TINYUSB_MSC_EVENT_MOUNT_CHANGED,
                Some(mount_changed),
            )
        })?;
        // All zero: the built-in PHY, and the default descriptors (from
        // sdkconfig.defaults).
        let usb: sys::tinyusb_config_t = unsafe { std::mem::zeroed() };
        sys::esp!(unsafe { sys::tinyusb_driver_install(&usb) })?;
        Ok(())
    })();
    // The screen's USB drive page.
    let error = match started {
        Ok(()) => {
            info!("The game's files ({what}) are a USB drive now: eject it to go back to the game");
            None
        }
        Err(e) => {
            log::warn!("USB drive: {e}");
            Some(e.to_string())
        }
    };
    {
        let mut st = status::lock(status);
        st.boot_hold = None;
        st.usb_drive = Some(status::UsbDrive { what, bytes, files, error, ejected: false });
    }
    // Ejected on the PC: restart into the game. BOOT does nothing while
    // the PC has the drive (restarting under it could damage the files);
    // only if the drive couldn't start does a press restart the board.
    let failed = status::lock(status).usb_drive.as_ref().is_some_and(|d| d.error.is_some());
    let down = || unsafe { sys::gpio_get_level(PIN_BOOT as _) } == 0;
    let mut was_down = true;
    loop {
        if EJECTED.load(Ordering::Relaxed) {
            info!("Ejected: restarting into the game");
            if let Some(drive) = status::lock(status).usb_drive.as_mut() {
                drive.ejected = true;
            }
            // Time for the PC to hear that the eject went through, and for
            // the screen to say so (it is redrawn every second).
            std::thread::sleep(Duration::from_millis(1500));
            break;
        }
        let now_down = down();
        if failed && now_down && !was_down {
            info!("BOOT pressed: restarting");
            break;
        }
        was_down = now_down;
        std::thread::sleep(Duration::from_millis(20));
    }
    // Give the USB pins back to the serial port (USB-Serial-JTAG) before
    // restarting. TinyUSB switched them to the USB controller with a bit
    // in the RTC domain, which a restart doesn't reset, so otherwise the
    // serial port (the log, and flashing without holding BOOT) wouldn't
    // come back until the power is cut.
    if !failed {
        if let Err(e) = sys::esp!(unsafe { sys::tinyusb_driver_uninstall() }) {
            log::warn!("USB drive: stopping TinyUSB: {e}");
        }
    }
    give_usb_to_serial();
    // The PC sees the drive go, and the serial port arrive after the restart.
    std::thread::sleep(Duration::from_millis(100));
    unsafe { sys::esp_restart() }
}

/// Hand the chip's USB pins back to the USB-Serial-JTAG controller, as
/// ESP-IDF's usb_serial_jtag_ll_phy_enable_external(false) does for the
/// ESP32-S3: in RTC_CNTL_USB_CONF_REG, SW_HW_USB_PHY_SEL (bit 20) set, so
/// software chooses, and SW_USB_PHY_SEL (bit 19) clear, for the serial
/// port. The register is in the RTC domain, which a restart (or flashing
/// over USB) doesn't reset; only cutting the power does.
fn give_usb_to_serial() {
    const RTC_CNTL_USB_CONF_REG: *mut u32 = (0x6000_8000 + 0x120) as *mut u32;
    const SW_HW_USB_PHY_SEL: u32 = 1 << 20;
    const SW_USB_PHY_SEL: u32 = 1 << 19;
    unsafe {
        let v = RTC_CNTL_USB_CONF_REG.read_volatile();
        RTC_CNTL_USB_CONF_REG.write_volatile((v | SW_HW_USB_PHY_SEL) & !SW_USB_PHY_SEL);
    }
}

/// Run the board's own hotspot, `net`, at HOTSPOT_IP, handing out
/// addresses on its network to whoever joins, and tell the screen.
fn start_hotspot(
    driver: WifiDriver<'static>,
    sysloop: EspSystemEventLoop,
    net: &wifi_config::Network,
    status: &status::SharedStatus,
) -> anyhow::Result<BlockingWifi<EspWifi<'static>>> {
    use esp_idf_svc::ipv4::{self, Mask, RouterConfiguration, Subnet};
    use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
    use esp_idf_svc::wifi::AuthMethod;
    // esp-idf-svc's own default is 192.168.71.1, so the address is set here.
    let ap_netif = EspNetif::new_with_conf(&NetifConfiguration {
        ip_configuration: Some(ipv4::Configuration::Router(RouterConfiguration {
            subnet: Subnet { gateway: HOTSPOT_IP, mask: Mask(24) },
            dhcp_enabled: true,
            // No DNS: there is no internet here, and nothing to look up.
            dns: None,
            secondary_dns: None,
        })),
        ..NetifConfiguration::wifi_default_router()
    })?;
    let mut wifi = BlockingWifi::wrap(EspWifi::wrap_all(driver, EspNetif::new(NetifStack::Sta)?, ap_netif)?, sysloop)?;
    let (Ok(ssid), Ok(password)) = (net.ssid.as_str().try_into(), net.password.as_str().try_into()) else {
        anyhow::bail!("hotspot name or password too long");
    };
    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid,
        password,
        auth_method: if net.password.is_empty() { AuthMethod::None } else { AuthMethod::WPA2Personal },
        channel: HOTSPOT_CHANNEL,
        max_connections: HOTSPOT_MAX_DEVICES,
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.wait_netif_up()?;
    let ip = wifi.wifi().ap_netif().get_ip_info()?.ip;
    info!("Hotspot \"{}\" up, server at {ip}:{PORT}", net.ssid);
    let qr = wifi_config::join_qr(net);
    if qr.is_none() {
        log::warn!("The hotspot's details don't fit in a QR code");
    }
    let mut st = status::lock(status);
    st.address = format!("{ip}:{PORT}");
    st.event(format!("Hotspot {} up", net.ssid));
    st.hotspot = Some(std::sync::Arc::new(status::Hotspot {
        ssid: net.ssid.clone(),
        password: net.password.clone(),
        ip: ip.to_string(),
        qr,
    }));
    st.boot_window = false;
    st.boot_hold = None;
    Ok(wifi)
}

/// Join one of `networks`, trying those in range in the file's order, and
/// keep trying until one works. Returns its name.
fn join_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>, networks: &[wifi_config::Network], status: &status::SharedStatus) -> String {
    use esp_idf_svc::wifi::AuthMethod;
    let say = |text: String| {
        info!("{text}");
        status::lock(status).event(text);
    };
    if networks.is_empty() {
        say(format!("No Wi-Fi networks: add one to {WIFI_FILE}"));
    }
    loop {
        let seen = match wifi.scan() {
            Ok(aps) => aps,
            Err(e) => {
                log::warn!("Wi-Fi scan failed: {e}");
                Vec::new()
            }
        };
        // Known networks in range first (in the file's order), then the
        // rest, in case one is hidden (not broadcasting its name).
        let mut order: Vec<(&wifi_config::Network, Option<&esp_idf_svc::wifi::AccessPointInfo>)> = Vec::new();
        for net in networks {
            let ap = seen
                .iter()
                .filter(|ap| ap.ssid.as_str() == net.ssid)
                .max_by_key(|ap| ap.signal_strength);
            if ap.is_some() {
                order.push((net, ap));
            }
        }
        if order.is_empty() && !networks.is_empty() {
            say(format!("None of the {} known Wi-Fi networks is in range", networks.len()));
        }
        for net in networks {
            if !order.iter().any(|(n, _)| n.ssid == net.ssid) {
                order.push((net, None));
            }
        }
        for (net, ap) in order {
            let (Ok(ssid), Ok(password)) = (net.ssid.as_str().try_into(), net.password.as_str().try_into()) else {
                continue;
            };
            let auth_method = match ap.and_then(|ap| ap.auth_method) {
                Some(m) => m,
                None if net.password.is_empty() => AuthMethod::None,
                None => AuthMethod::WPA2Personal,
            };
            let config = Configuration::Client(ClientConfiguration {
                ssid,
                password,
                auth_method,
                channel: ap.map(|ap| ap.channel),
                ..Default::default()
            });
            say(format!("Joining Wi-Fi {}...", net.ssid));
            let joined = wifi
                .set_configuration(&config)
                .and_then(|_| wifi.connect())
                .and_then(|_| wifi.wait_netif_up());
            match joined {
                Ok(()) => return net.ssid.clone(),
                Err(e) => {
                    log::warn!("Joining {} failed: {e}", net.ssid);
                    let _ = wifi.disconnect();
                }
            }
        }
        std::thread::sleep(WIFI_RETRY);
    }
}

/// Keep the Wi-Fi connection: check it every WIFI_CHECK, and when it has
/// gone, try the same network again (most drop-outs are short), then the
/// known networks as at startup, until one works.
fn watch_wifi(mut wifi: BlockingWifi<EspWifi<'static>>, networks: &[wifi_config::Network], status: &status::SharedStatus, mut joined: String) {
    loop {
        std::thread::sleep(WIFI_CHECK);
        if wifi.is_up().unwrap_or(false) {
            continue;
        }
        let text = format!("Lost Wi-Fi {joined}");
        log::warn!("{text}");
        {
            // The screen's header and the status light show it's offline.
            let mut st = status::lock(status);
            st.event(text);
            st.address = "Wi-Fi...".to_string();
        }
        info!("Rejoining {joined}...");
        let again = wifi.connect().and_then(|_| wifi.wait_netif_up());
        if let Err(e) = again {
            log::warn!("Rejoining {joined} failed: {e}");
            let _ = wifi.disconnect();
            joined = join_wifi(&mut wifi, networks, status);
        }
        match wifi.wifi().sta_netif().get_ip_info() {
            Ok(info) => {
                info!("Back on Wi-Fi {joined}, IP address {}", info.ip);
                let mut st = status::lock(status);
                st.address = format!("{}:{PORT}", info.ip);
                st.event(format!("Rejoined Wi-Fi {joined}"));
            }
            Err(e) => log::warn!("Rejoined {joined}, but no IP address: {e}"),
        }
    }
}

/// This board's id, for its default hotspot's name (see
/// wifi_config::board_id): from its hotspot MAC address.
fn board_id() -> String {
    let mut mac = [0u8; 6];
    let result = unsafe {
        esp_idf_svc::sys::esp_read_mac(mac.as_mut_ptr(), esp_idf_svc::sys::esp_mac_type_t_ESP_MAC_WIFI_SOFTAP)
    };
    if result != 0 {
        log::warn!("Reading the board's MAC address: error {result}");
    }
    wifi_config::board_id(mac)
}

/// The flash partition that holds the game when there is no SD card (see
/// partitions.csv).
const FLASH_PARTITION: &str = "storage";

/// Mount the board's own flash at MOUNT: a FAT file system, formatted the
/// first time, over ESP-IDF's wear levelling, which spreads the writes
/// across the partition so no part of the flash wears out first.
fn mount_flash() -> anyhow::Result<esp_idf_svc::sys::wl_handle_t> {
    use esp_idf_svc::sys;
    let base = std::ffi::CString::new(MOUNT)?;
    let label = std::ffi::CString::new(FLASH_PARTITION)?;
    let config = sys::esp_vfs_fat_mount_config_t {
        format_if_mount_failed: true,
        max_files: 4,
        allocation_unit_size: 4096,
        disk_status_check_enable: false,
        use_one_fat: false,
    };
    // The handle is only needed to unmount, for the USB drive.
    let mut handle: sys::wl_handle_t = 0;
    let result = unsafe { sys::esp_vfs_fat_spiflash_mount_rw_wl(base.as_ptr(), label.as_ptr(), &config, &mut handle) };
    if result == sys::ESP_ERR_NOT_FOUND as sys::esp_err_t {
        anyhow::bail!(
            "no SD card, and no \"{FLASH_PARTITION}\" partition in flash to use instead: \
             flash with partitions.csv (cargo run does, as set in .cargo/config.toml)"
        );
    }
    sys::esp!(result)?;
    info!("Board's flash mounted at {MOUNT}");
    Ok(handle)
}

/// Game data on the SD card that a reset deletes. Extra media (skins and
/// textures in /sd/media) are kept.
const RESET_DIRS: [&str; 3] = ["auth", "world", "players"];

/// If this firmware was built with RESET_SD=1 (see build.rs), delete the
/// game data on the SD card, once: the build's token is written to the card
/// afterwards, and a card already reset by this build is left alone.
fn reset_sd_if_asked() {
    let Some(token) = option_env!("RESET_SD_TOKEN") else {
        return;
    };
    let marker = format!("{MOUNT}/reset-done");
    if fs::read_to_string(&marker).is_ok_and(|t| t.trim() == token) {
        info!("SD card reset: already done by this firmware, so the card is left as it is");
        return;
    }
    log::warn!("SD card reset: deleting {}", RESET_DIRS.join(", "));
    for dir in RESET_DIRS {
        let path = format!("{MOUNT}/{dir}");
        match remove_tree(std::path::Path::new(&path)) {
            Ok(n) => info!("  {path}: {n} files deleted"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("  {path}: {e}"),
        }
    }
    if let Err(e) = fs::write(&marker, token) {
        log::warn!("SD card reset: couldn't record it ({e}); it will happen again at the next start");
    }
}

/// Delete a directory and everything in it, returning how many files went.
/// (Done by hand: FAT on ESP-IDF supports only the basic file operations.)
fn remove_tree(dir: &std::path::Path) -> std::io::Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            count += remove_tree(&path)?;
        } else {
            fs::remove_file(&path)?;
            count += 1;
        }
    }
    fs::remove_dir(dir)?;
    Ok(count)
}

fn log_heap(label: &str) {
    use esp_idf_svc::sys::*;
    unsafe {
        info!(
            "[{label}] internal free {} KB (largest block {} KB), PSRAM free {} KB",
            heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) / 1024,
            heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) / 1024,
            heap_caps_get_free_size(MALLOC_CAP_SPIRAM) / 1024,
        );
    }
}
