// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates
// Based on minetest_game's binoculars and map mods (mods/binoculars and
// mods/map, MIT licence):
//   Copyright (C) 2017 paramat

//! Binoculars and the mapping kit, as minetest_game's binoculars and map
//! mods: carried anywhere in the main inventory, binoculars let the player
//! zoom (the "Zoom" key, to a 10 degree view), and a mapping kit lets them
//! show the minimap (the "Minimap" key; the radar view stays off). Without
//! them, there is neither: the server turns them off when the player
//! joins.
//!
//! As in the mods, the server looks again every few seconds (4.7 for the
//! binoculars, 5.3 for the map) and whenever the item is used
//! (left-clicked), and tells the client only of a change. (There is no
//! creative mode here, which would give a 15 degree zoom and the radar.)

use std::time::Duration;

use crate::content::{ITEM_BINOCULARS, ITEM_MAPPING_KIT};
use crate::inventory::Stack;

/// How often the binoculars and the mapping kit are looked for.
pub const BINOCULARS_EVERY: Duration = Duration::from_millis(4700);
pub const MAP_EVERY: Duration = Duration::from_millis(5300);

/// The zoomed view, in degrees, with binoculars (0: no zoom).
pub const ZOOM_FOV: f32 = 10.0;

/// Luanti's HUD flags for the minimap and its radar view.
pub const HUD_FLAG_MINIMAP_VISIBLE: u32 = 1 << 5;
pub const HUD_FLAG_MINIMAP_RADAR_VISIBLE: u32 = 1 << 6;

fn carries(main: &[Option<Stack>], item: u16) -> bool {
    main.iter().flatten().any(|s| s.id == item)
}

/// binoculars.update_player_property: the zoom a player gets.
pub fn zoom_fov(main: &[Option<Stack>]) -> f32 {
    if carries(main, ITEM_BINOCULARS) { ZOOM_FOV } else { 0.0 }
}

/// map.update_hud_flags: whether the player may show the minimap.
pub fn minimap(main: &[Option<Stack>]) -> bool {
    carries(main, ITEM_MAPPING_KIT)
}

/// The HUD flags for that, and the mask they go with: the radar too in
/// creative mode (map.update_hud_flags), else never.
pub fn minimap_flags(allowed: bool, radar: bool) -> (u32, u32) {
    let flags = if allowed { HUD_FLAG_MINIMAP_VISIBLE } else { 0 } | if radar { HUD_FLAG_MINIMAP_RADAR_VISIBLE } else { 0 };
    (flags, HUD_FLAG_MINIMAP_VISIBLE | HUD_FLAG_MINIMAP_RADAR_VISIBLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carried() {
        let mut main = vec![None; 32];
        assert_eq!(zoom_fov(&main), 0.0);
        assert!(!minimap(&main));
        main[31] = Some(Stack::new(ITEM_BINOCULARS, 1));
        assert_eq!(zoom_fov(&main), 10.0);
        assert!(!minimap(&main));
        main[9] = Some(Stack::new(ITEM_MAPPING_KIT, 1));
        assert!(minimap(&main));
        assert_eq!(minimap_flags(true, false), (1 << 5, (1 << 5) | (1 << 6)));
        assert_eq!(minimap_flags(false, false), (0, (1 << 5) | (1 << 6)));
        assert_eq!(minimap_flags(true, true), ((1 << 5) | (1 << 6), (1 << 5) | (1 << 6)));
    }
}
