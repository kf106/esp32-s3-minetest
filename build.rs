//! Build script: ESP-IDF set-up, plus the optional SD card reset.
//!
//! `RESET_SD=1 cargo run --release` builds firmware that, the first time it
//! starts, deletes the game data on the SD card (accounts, world, players)
//! so everything is generated afresh. Each such build gets its own token,
//! which the firmware records on the card once it has reset it, so a
//! reboot doesn't wipe the card again. Build normally afterwards.
//!
//! `TEST=1 cargo run --release` builds firmware for testing that gives every
//! player, once, a test kit: 50 of each crafting material (see
//! inventory.rs), and grows saplings, apples and crops ten times faster
//! (nature.rs). The value is the kit's token: rebuilding with the same one
//! gives nothing more, and a new value (TEST=2, and so on) hands out a fresh
//! kit. Build normally for play.
//!
//! `PERF=1 cargo run --release` adds performance logs for load testing:
//! every 10 s, the load on each core, the busiest tasks and the map blocks
//! built and sent ("Load over 10 s"), where the main loop's time went
//! ("Main loop over 10 s"), and every minute PSRAM use and the dungeons
//! kept. Without it, none of this is measured.
//!
//! `BOARD=es3c28p cargo run --release` builds for LCDwiki's ES3C28P or
//! ES3N28P instead of the Waveshare ESP32-S3-LCD-1.47 (see src/board.rs).
//! Changing the board needs no `cargo clean`.

use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    embuild::espidf::sysenv::output();

    // Which board (src/board.rs), as `cfg(board = "...")`.
    println!("cargo:rerun-if-env-changed=BOARD");
    println!("cargo:rustc-check-cfg=cfg(board, values(\"waveshare\", \"es3c28p\"))");
    // cfg(pc): set by pc/build.rs for the PC build (bigger caches).
    println!("cargo:rustc-check-cfg=cfg(pc)");
    let board = match std::env::var("BOARD").unwrap_or_default().to_ascii_lowercase().as_str() {
        "" | "waveshare" | "waveshare-1.47" | "esp32-s3-lcd-1.47" => "waveshare",
        "es3c28p" | "es3n28p" => "es3c28p",
        other => panic!("Unknown BOARD \"{other}\": use waveshare (the default) or es3c28p (for the ES3C28P and ES3N28P)"),
    };
    println!("cargo:rustc-cfg=board=\"{board}\"");
    if board != "waveshare" {
        println!("cargo:warning=Building for the {board} board");
    }

    println!("cargo:rerun-if-env-changed=RESET_SD");
    if std::env::var("RESET_SD").is_ok_and(|v| v == "1") {
        let token = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        println!("cargo:rustc-env=RESET_SD_TOKEN={token}");
        // A path that never exists makes Cargo run this script on every
        // build, so each reset build gets a fresh token.
        println!("cargo:rerun-if-changed=.reset-sd-every-build");
        println!("cargo:warning=This firmware will delete the accounts, world and players on the SD card when it starts");
    }

    // PERF=1: performance logs (cfg(perf)), see the comment at the top.
    println!("cargo:rustc-check-cfg=cfg(perf)");
    println!("cargo:rerun-if-env-changed=PERF");
    if std::env::var("PERF").is_ok_and(|v| v == "1") {
        println!("cargo:rustc-cfg=perf");
        println!("cargo:warning=This firmware writes performance logs every 10 s (PERF=1)");
    }

    println!("cargo:rerun-if-env-changed=TEST");
    let test = std::env::var("TEST").unwrap_or_default();
    let test = test.trim();
    if !test.is_empty() && test != "0" && test.chars().all(|c| c.is_ascii_alphanumeric()) {
        println!("cargo:rustc-env=TEST_KIT_TOKEN={test}");
        println!("cargo:warning=This firmware gives every player test kit {test} of crafting materials, once, and grows plants ten times faster");
    }
}
