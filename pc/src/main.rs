//! The esp32-s3-minetest server as an ordinary program, for a PC or a VPS:
//! the firmware's game code (../src), started from the command line, with
//! its world in a folder laid out as the board's SD card is.
//!
//!     minetest-server [--port 30000] [--bind 0.0.0.0] [--data minetest-data]
//!                     [--max-players 15] [--log info]
//!
//! Like the board, it saves as it goes (world changes every second,
//! inventories within 5 s), so it can be stopped at any time with Ctrl+C
//! or `systemctl stop`.

use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../src/auth.rs"]
mod auth;
#[path = "../../src/beds.rs"]
mod beds;
#[path = "../../src/binoculars.rs"]
mod binoculars;
#[allow(dead_code)] // the board's pins
#[path = "../../src/board.rs"]
mod board;
#[path = "../../src/bones.rs"]
mod bones;
#[path = "../../src/books.rs"]
mod books;
#[path = "../../src/chest.rs"]
mod chest;
#[path = "../../src/clock.rs"]
mod clock;
#[path = "../../src/colours.rs"]
mod colours;
#[path = "../../src/content.rs"]
mod content;
#[path = "../../src/crafting.rs"]
mod crafting;
#[path = "../../src/craftguide.rs"]
mod craftguide;
#[path = "../../src/doors.rs"]
mod doors;
#[path = "../../src/dungeons.rs"]
mod dungeons;
#[path = "../../src/env_sounds.rs"]
mod env_sounds;
#[path = "../../src/falling.rs"]
mod falling;
#[path = "../../src/fire.rs"]
mod fire;
#[path = "../../src/farming.rs"]
mod farming;
#[path = "../../src/fasthash.rs"]
mod fasthash;
#[path = "../../src/fences.rs"]
mod fences;
#[path = "../../src/furnace.rs"]
mod furnace;
#[path = "../../src/inventory.rs"]
mod inventory;
#[path = "../../src/items.rs"]
mod items;
#[path = "../../src/jsonstring.rs"]
mod jsonstring;
#[path = "../../src/keys.rs"]
mod keys;
#[path = "../../src/liquids.rs"]
mod liquids;
#[path = "../../src/messages.rs"]
mod messages;
#[path = "../../src/meta.rs"]
mod meta;
#[path = "../../src/mobs.rs"]
mod mobs;
#[path = "../../src/nature.rs"]
mod nature;
#[path = "../../src/panes.rs"]
mod panes;
#[path = "../../src/plants.rs"]
mod plants;
#[path = "../../src/player.rs"]
mod player;
#[path = "../../src/privs.rs"]
mod privs;
#[path = "../../src/protocol.rs"]
mod protocol;
#[path = "../../src/screwdriver.rs"]
mod screwdriver;
#[path = "../../src/server.rs"]
mod server;
#[path = "../../src/settings.rs"]
mod settings;
#[path = "../../src/sha1.rs"]
mod sha1;
#[path = "../../src/sha256.rs"]
mod sha256;
#[path = "../../src/skins.rs"]
mod skins;
#[path = "../../src/srp.rs"]
mod srp;
#[path = "../../src/stairs.rs"]
mod stairs;
#[path = "../../src/status.rs"]
mod status;
#[path = "../../src/storage.rs"]
mod storage;
#[path = "../../src/terrain.rs"]
mod terrain;
#[path = "../../src/weather.rs"]
mod weather;
#[path = "../../src/tnt.rs"]
mod tnt;
#[path = "../../src/world.rs"]
mod world;
#[path = "../../src/zstd.rs"]
mod zstd;

// The board's screen, status light and Wi-Fi set-up: the server refers to
// them, but nothing here shows them.
#[allow(dead_code)]
#[path = "../../src/font.rs"]
mod font;
#[allow(dead_code)]
#[path = "../../src/indicator.rs"]
mod indicator;
#[allow(dead_code)]
#[path = "../../src/screen.rs"]
mod screen;
#[allow(dead_code)]
#[path = "../../src/wifi_config.rs"]
mod wifi_config;

const USAGE: &str = "\
Usage: minetest-server [options]

  --port <n>          UDP port to listen on (default 30000)
  --bind <address>    address to listen on (default 0.0.0.0: all)
  --data <folder>     where the world, accounts and players are kept, and
                      the optional media/ folder and motd.txt
                      (default: minetest-data)
  --max-players <n>   players in the game at once (default 15)
  --log <level>       error, warn, info or debug (default info)
  --help              this
";

struct Options {
    port: u16,
    bind: String,
    data: PathBuf,
    max_players: usize,
    log: log::LevelFilter,
}

fn options() -> Result<Options, String> {
    let mut o = Options { port: 30000, bind: "0.0.0.0".into(), data: "minetest-data".into(), max_players: 15, log: log::LevelFilter::Info };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--port" => o.port = value()?.parse().map_err(|_| "--port needs a number from 1 to 65535")?,
            "--bind" => o.bind = value()?,
            "--data" => o.data = value()?.into(),
            "--max-players" => o.max_players = value()?.parse().map_err(|_| "--max-players needs a number")?,
            "--log" => o.log = value()?.parse().map_err(|_| "--log needs error, warn, info or debug")?,
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    if o.max_players == 0 {
        return Err("--max-players needs at least 1".into());
    }
    Ok(o)
}

fn main() -> ExitCode {
    let o = match options() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("minetest-server: {e}\n");
            }
            eprint!("{USAGE}");
            return if e.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(2) };
        }
    };
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(o.log);
    match run(&o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(o: &Options) -> std::io::Result<()> {
    let data = &o.data;
    std::fs::create_dir_all(data)?;
    let d = |name: &str| data.join(name).to_string_lossy().into_owned();
    log::info!("Data folder {}", std::fs::canonicalize(data).unwrap_or(data.clone()).display());
    let address = format!("{}:{}", o.bind, o.port);
    let socket = UdpSocket::bind(&address).map_err(|e| std::io::Error::new(e.kind(), format!("listening on {address}: {e}")))?;
    let motd = std::fs::read_to_string(d("motd.txt")).ok();
    if motd.is_some() {
        log::info!("Welcome message from {}", d("motd.txt"));
    }
    let setup = server::Setup {
        socket,
        motd,
        auth: auth::AuthStore::open(d("auth"))?,
        world: world::World::open(d("world"))?,
        inventories: inventory::InventoryStore::open(d("players"))?,
        players: player::PlayerStore::open(d("players"))?,
        furnaces: furnace::Furnaces::open(d("world/furnaces.txt")),
        chests: chest::Chests::open(d("world/chests.txt")),
        items: items::Items::open(d("world/items.txt")),
        clock: clock::Clock::open(d("world/env_meta.txt")),
        door_owners: doors::Owners::open(d("world/doors.txt")),
        bed_spawns: beds::Spawns::open(d("world/beds_spawns")),
        settings: settings::Settings::open(d("world/settings.txt")),
        media: content::Media::load(&d("media")),
        status: status::Status::new(address.clone()),
        max_players: o.max_players,
    };
    let mut server = server::Server::new(setup);
    // (Board-only: the screen's view of the server, and SD card or flash.)
    let _ = (server.status_data_addr(), status::problems, status::set_on_flash);
    log::info!("Luanti server listening on {address} (UDP), up to {} players (the board takes {})", o.max_players, server::MAX_PLAYERS);
    server.run()
}

/// Log lines on stderr: UTC time, level, message (systemd's journal keeps
/// them, with its own times as well).
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        eprintln!("{} {:<5} {}", utc_now(), r.level(), r.args());
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

/// "2026-10-24 04:05:06", UTC.
fn utc_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
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
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::civil_from_days(0), (1970, 1, 1));
        assert_eq!(super::civil_from_days(20_385), (2025, 10, 24));
        assert_eq!(super::civil_from_days(-1), (1969, 12, 31));
        assert_eq!(super::civil_from_days(11_016), (2000, 2, 29));
    }
}
