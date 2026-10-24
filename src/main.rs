mod auth;
mod beds;
mod binoculars;
mod board;
mod books;
mod bones;
mod chest;
mod clock;
mod colours;
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
/// How long to wait before looking for known networks again.
const WIFI_RETRY: std::time::Duration = std::time::Duration::from_secs(10);
/// How often to check that the Wi-Fi connection is still there.
const WIFI_CHECK: std::time::Duration = std::time::Duration::from_secs(2);
const PORT: u16 = 30000;
/// The BOOT button: GPIO 0, pulled up, low while pressed.
const PIN_BOOT: i32 = 0;
/// How long after start-up the BOOT button is watched, and how long it
/// must be held, to run the board's own hotspot. (Holding it while the
/// board powers up doesn't work: the chip then waits to be flashed.)
const HOTSPOT_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);
const HOTSPOT_HOLD: std::time::Duration = std::time::Duration::from_secs(1);
/// The hotspot's address (the gateway of its own 192.168.4.x network) and
/// radio channel, and how many devices may join it.
const HOTSPOT_IP: std::net::Ipv4Addr = std::net::Ipv4Addr::new(192, 168, 4, 1);
const HOTSPOT_CHANNEL: u8 = 1;
const HOTSPOT_MAX_DEVICES: u16 = 8;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
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
    let sd = (|| -> anyhow::Result<_> {
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
        let sd = SdCardDriver::new_mmc(host, &SdCardConfiguration::new())?;
        let fatfs = Fatfs::new_sdcard(0, sd)?;
        Ok(MountedFatfs::mount(fatfs, MOUNT, 4)?)
    })();
    // Kept until the server stops: dropping it would unmount the card.
    let _sd = match sd {
        Ok(mounted) => {
            info!("SD card mounted at {MOUNT}");
            Some(mounted)
        }
        Err(e) => {
            log::warn!("No SD card ({e}): keeping the game in the board's own flash");
            mount_flash()?;
            status::set_on_flash();
            None
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
    let ip = if hotspot_asked(&status) {
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

/// Whether to run the board's own hotspot: the BOOT button held for
/// HOTSPOT_HOLD within HOTSPOT_WINDOW of start-up (or a hold that began in
/// the window). The screen asks meanwhile.
fn hotspot_asked(status: &status::SharedStatus) -> bool {
    use esp_idf_svc::sys::*;
    use std::time::{Duration, Instant};
    unsafe {
        gpio_set_direction(PIN_BOOT as _, gpio_mode_t_GPIO_MODE_INPUT);
        gpio_set_pull_mode(PIN_BOOT as _, gpio_pull_mode_t_GPIO_PULLUP_ONLY);
    }
    status::lock(status).event("Hold BOOT now for a hotspot".to_string());
    info!("Hold BOOT for {}s now to run a hotspot", HOTSPOT_HOLD.as_secs());
    let start = Instant::now();
    let mut held_since: Option<Instant> = None;
    while start.elapsed() < HOTSPOT_WINDOW || held_since.is_some() {
        let down = unsafe { gpio_get_level(PIN_BOOT as _) } == 0;
        match (down, held_since) {
            (true, None) => held_since = Some(Instant::now()),
            (false, _) => held_since = None,
            _ => {}
        }
        if held_since.is_some_and(|t| t.elapsed() >= HOTSPOT_HOLD) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
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
fn mount_flash() -> anyhow::Result<()> {
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
    // The handle is only needed to unmount, which never happens.
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
    Ok(())
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
