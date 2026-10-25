//! The world's settings that admins and moderators change in the game, kept
//! in `world/settings.txt` as Luanti keeps settings ("name = value"), with
//! minetest_game's names: `enable_fire` (whether fire spreads and burns
//! things) and `enable_tnt` (whether TNT and gunpowder can be lit). Both
//! are on in a new world (minetest_game turns them off on servers unless
//! set; here `/fire` and `/tnt` turn them off). And ours, `enable_mobs`
//! (whether hostile mobs spawn: mobs.rs), off in a new world, since
//! minetest_game has none (`/mobs` turns them on). And Luanti's
//! `creative_mode`, off in a new world (`/mode creative` turns it on:
//! creative.rs).

use std::io;
use std::path::PathBuf;

use log::{info, warn};

use crate::storage;

pub struct Settings {
    path: PathBuf,
    fire: bool,
    tnt: bool,
    mobs: bool,
    creative: bool,
}

impl Settings {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let text = match storage::read(&path) {
            Ok(text) => String::from_utf8_lossy(&text).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                warn!("Reading {}: {e}", path.display());
                String::new()
            }
        };
        let fire = flag(&text, "enable_fire").unwrap_or(true);
        let tnt = flag(&text, "enable_tnt").unwrap_or(true);
        let mobs = flag(&text, "enable_mobs").unwrap_or(false);
        let creative = flag(&text, "creative_mode").unwrap_or(false);
        if creative {
            info!("Creative mode is on (creative_mode = true in {})", path.display());
        }
        if !fire {
            info!("Fire spreading is off (enable_fire = false in {})", path.display());
        }
        if !tnt {
            info!("TNT is off (enable_tnt = false in {})", path.display());
        }
        if mobs {
            info!("Mobs are on (enable_mobs = true in {})", path.display());
        }
        Self { path, fire, tnt, mobs, creative }
    }

    /// Whether fire spreads and burns things.
    pub fn fire(&self) -> bool {
        self.fire
    }

    /// Whether TNT and gunpowder can be lit.
    pub fn tnt(&self) -> bool {
        self.tnt
    }

    /// Whether hostile mobs spawn.
    pub fn mobs(&self) -> bool {
        self.mobs
    }

    /// Whether the whole server is in creative mode.
    pub fn creative(&self) -> bool {
        self.creative
    }

    pub fn set_creative(&mut self, on: bool) {
        self.creative = on;
        self.save();
    }

    pub fn set_mobs(&mut self, on: bool) {
        self.mobs = on;
        self.save();
    }

    pub fn set_fire(&mut self, on: bool) {
        self.fire = on;
        self.save();
    }

    pub fn set_tnt(&mut self, on: bool) {
        self.tnt = on;
        self.save();
    }

    fn save(&self) {
        let text = format!("enable_fire = {}\nenable_tnt = {}\nenable_mobs = {}\ncreative_mode = {}\n", self.fire, self.tnt, self.mobs, self.creative);
        if let Err(e) = storage::write(&self.path, text.as_bytes()) {
            warn!("Saving {}: {e}", self.path.display());
            crate::status::problem();
        }
    }
}

fn flag(text: &str, name: &str) -> Option<bool> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == name).then(|| matches!(v.trim(), "true" | "yes" | "1" | "on"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept() {
        assert_eq!(flag("enable_fire = false\n", "enable_fire"), Some(false));
        assert_eq!(flag("x = 1\nenable_tnt=true", "enable_tnt"), Some(true));
        assert_eq!(flag("", "enable_fire"), None);
        let dir = std::env::temp_dir().join(format!("esp-settings-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.txt");
        let _ = std::fs::remove_file(&path);
        let mut s = Settings::open(&path);
        assert!(s.fire() && s.tnt());
        s.set_fire(false);
        let mut s = Settings::open(&path);
        assert!(!s.fire() && s.tnt());
        s.set_tnt(false);
        let s = Settings::open(&path);
        assert!(!s.fire() && !s.tnt() && !s.mobs());
        let mut s = Settings::open(&path);
        s.set_mobs(true);
        let mut s = Settings::open(&path);
        assert!(!s.fire() && s.mobs() && !s.creative());
        s.set_creative(true);
        let s = Settings::open(&path);
        assert!(s.creative() && s.mobs());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
