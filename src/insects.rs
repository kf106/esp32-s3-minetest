// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates
// Based on minetest_game's butterflies and fireflies mods
// (mods/butterflies/init.lua, mods/fireflies/init.lua):
//   Copyright (C) 2018 Shara RedCat
// Licensed under the MIT License: permission is hereby granted, free of
// charge, to any person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the Software without
// restriction, including without limitation the rights to use, copy,
// modify, merge, publish, distribute, sublicense, and/or sell copies of the
// Software, and to permit persons to whom the Software is furnished to do
// so, subject to the following conditions: the above copyright notice and
// this permission notice shall be included in all copies or substantial
// portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT
// WARRANTY OF ANY KIND, EXPRESS OR IMPLIED.

//! Butterflies and fireflies, as minetest_game's butterflies and fireflies
//! mods have them: not creatures but nodes, small animated pictures that
//! sway in the air.
//!
//! - Butterflies (white, red and violet) hang about flowers on the
//!   grassland, three nodes above the ground. Where the light is under 11
//!   (at night, or in the shade) they hide: they become an invisible node,
//!   which shows the butterfly again once the light is back.
//! - Fireflies hang about in the woods (deciduous, coniferous and
//!   rainforest), three or four nodes up, glowing (light 6). They are the
//!   other way round: hidden where the light is over 11 (by day), shown
//!   in the dark.
//! - Each looks at the light every 30 s (a node timer in minetest_game);
//!   here the server looks at those near players every INSECT_EVERY.
//! - Only a bug net (string and a stick) can point at them, and catches
//!   them (they are dug at once, into the inventory); placed again, they
//!   fly there.
//! - A firefly over a glass bottle makes a firefly in a bottle: a light
//!   (9). Right-clicked, it lets the firefly out above it, and is a glass
//!   bottle again.
//!
//! The pure parts are here; the world generates them (terrain.rs) and the
//! server shows, hides, catches and releases them.

use std::time::Duration;

/// The butterflies mod's nodes (shown, then hidden, white, red and violet),
/// and the fireflies mod's (shown, hidden, and in a bottle).
pub const BUTTERFLY_WHITE: u16 = crate::tnt::TNT_BURNING + 1;
pub const BUTTERFLY_RED: u16 = BUTTERFLY_WHITE + 1;
pub const BUTTERFLY_VIOLET: u16 = BUTTERFLY_RED + 1;
pub const HIDDEN_BUTTERFLY_WHITE: u16 = BUTTERFLY_VIOLET + 1;
pub const HIDDEN_BUTTERFLY_RED: u16 = HIDDEN_BUTTERFLY_WHITE + 1;
pub const HIDDEN_BUTTERFLY_VIOLET: u16 = HIDDEN_BUTTERFLY_RED + 1;
pub const FIREFLY: u16 = HIDDEN_BUTTERFLY_VIOLET + 1;
pub const HIDDEN_FIREFLY: u16 = FIREFLY + 1;
pub const FIREFLY_BOTTLE: u16 = HIDDEN_FIREFLY + 1;

pub const BUTTERFLIES: [u16; 3] = [BUTTERFLY_WHITE, BUTTERFLY_RED, BUTTERFLY_VIOLET];

/// The light a firefly gives (light_source 6), and one in a bottle (9).
pub const FIREFLY_LIGHT: u8 = 6;
pub const BOTTLE_LIGHT: u8 = 9;
/// A butterfly hides below this light, a firefly above it.
pub const LIGHT_LIMIT: u8 = 11;
/// The bug net lasts this many catches (uses 256).
pub const NET_USES: u32 = 256;

/// How often the server looks at the insects near players (their node
/// timers: 30 s, here a little more often, or every second with
/// LUANTI_INSECTS_FAST for the tests).
pub fn every() -> Duration {
    if std::env::var_os("LUANTI_INSECTS_FAST").is_some() { Duration::from_secs(1) } else { Duration::from_secs(10) }
}

pub fn is_insect(id: u16) -> bool {
    (BUTTERFLY_WHITE..=HIDDEN_FIREFLY).contains(&id)
}

/// Whether a node is an insect that can be seen (and caught).
pub fn is_shown(id: u16) -> bool {
    BUTTERFLIES.contains(&id) || id == FIREFLY
}

/// The other state of an insect node: hidden for shown, and the other way.
pub fn twin(id: u16) -> Option<u16> {
    match id {
        BUTTERFLY_WHITE..=BUTTERFLY_VIOLET => Some(id + 3),
        HIDDEN_BUTTERFLY_WHITE..=HIDDEN_BUTTERFLY_VIOLET => Some(id - 3),
        FIREFLY => Some(HIDDEN_FIREFLY),
        HIDDEN_FIREFLY => Some(FIREFLY),
        _ => None,
    }
}

/// What an insect node becomes, now that the light at it is `light`: a
/// butterfly hides under LIGHT_LIMIT and comes out at it or over; a
/// firefly hides over it and comes out at it or under. None: it stays.
pub fn turns_into(id: u16, light: u8) -> Option<u16> {
    let butterfly = id < FIREFLY;
    let wants_shown = if butterfly { light >= LIGHT_LIMIT } else { light <= LIGHT_LIMIT };
    if is_shown(id) == wants_shown { None } else { twin(id) }
}

/// The picture of an insect (its inventory and wield image), and its
/// animated tile.
pub fn image(id: u16) -> &'static str {
    match id {
        BUTTERFLY_WHITE | HIDDEN_BUTTERFLY_WHITE => "butterflies_butterfly_white.png",
        BUTTERFLY_RED | HIDDEN_BUTTERFLY_RED => "butterflies_butterfly_red.png",
        BUTTERFLY_VIOLET | HIDDEN_BUTTERFLY_VIOLET => "butterflies_butterfly_violet.png",
        FIREFLY | HIDDEN_FIREFLY => "fireflies_firefly.png",
        _ => "fireflies_bottle.png",
    }
}

/// How long an animated insect tile takes to go round (its animation's
/// length): butterflies 3 s, fireflies and the bottle 1.5 s.
pub fn tile_length(tile: &str) -> Option<f32> {
    match tile {
        "butterflies_butterfly_white_animated.png" | "butterflies_butterfly_red_animated.png" | "butterflies_butterfly_violet_animated.png" => Some(3.0),
        "fireflies_firefly_animated.png" | "fireflies_bottle_animated.png" => Some(1.5),
        _ => None,
    }
}

/// Insects' selection box: a small cube (shown only to a bug net).
pub const BOX: [f32; 6] = [-0.1, -0.1, -0.1, 0.1, 0.1, 0.1];

/// Where a firefly let out of a bottle at `pos` goes (its on_rightclick):
/// two nodes up if both are free, else one up if that is; None if the node
/// above isn't air.
pub fn release_to(pos: (i32, i32, i32), node: &dyn Fn((i32, i32, i32)) -> u16) -> Option<(i32, i32, i32)> {
    let lower = (pos.0, pos.1 + 1, pos.2);
    let upper = (pos.0, pos.1 + 2, pos.2);
    let air = crate::world::CONTENT_AIR;
    if node(lower) != air {
        return None;
    }
    Some(if node(upper) == air { upper } else { lower })
}

/// Luanti's time_to_daynight_ratio (smooth): how much of the daylight
/// there is at a time of day (0 to 24000), out of 1000.
pub fn daynight_ratio(time_of_day: u16) -> u32 {
    let mut t = time_of_day as f32 % 24000.0;
    if t > 12000.0 {
        t = 24000.0 - t;
    }
    const VALUES: [(f32, f32); 9] = [
        (4375.0, 175.0),
        (4625.0, 175.0),
        (4875.0, 250.0),
        (5125.0, 350.0),
        (5375.0, 500.0),
        (5625.0, 675.0),
        (5875.0, 875.0),
        (6125.0, 1000.0),
        (6375.0, 1000.0),
    ];
    if t <= 4625.0 {
        return 175;
    }
    if t >= 6125.0 {
        return 1000;
    }
    for i in 1..9 {
        if VALUES[i].0 <= t {
            continue;
        }
        let (t0, v0) = VALUES[i - 1];
        let (t1, v1) = VALUES[i];
        let f = (t - t0) / (t1 - t0);
        return (f * v1 + (1.0 - f) * v0) as u32;
    }
    1000
}

/// The light at a node (Luanti's getLightBlend): its day light (sunlight,
/// or what lights it at night if more) and its night light (torches),
/// blended by the time of day.
pub fn light(sun: u8, torch: u8, time_of_day: u16) -> u8 {
    let ratio = daynight_ratio(time_of_day);
    let day = sun.max(torch) as u32;
    (((ratio * day + (1000 - ratio) * torch as u32) / 1000).min(15)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        // Butterflies: out in daylight, hidden below 11.
        assert_eq!(turns_into(BUTTERFLY_RED, 10), Some(HIDDEN_BUTTERFLY_RED));
        assert_eq!(turns_into(BUTTERFLY_RED, 11), None);
        assert_eq!(turns_into(HIDDEN_BUTTERFLY_VIOLET, 14), Some(BUTTERFLY_VIOLET));
        // Fireflies: out at 11 and under, hidden over.
        assert_eq!(turns_into(FIREFLY, 12), Some(HIDDEN_FIREFLY));
        assert_eq!(turns_into(HIDDEN_FIREFLY, 11), Some(FIREFLY));
        assert_eq!(turns_into(HIDDEN_FIREFLY, 15), None);
        assert!(is_insect(HIDDEN_FIREFLY) && !is_insect(FIREFLY_BOTTLE) && !is_shown(HIDDEN_BUTTERFLY_WHITE));
        // Light: full sun at noon, the moon's little at midnight, torches
        // either way.
        assert_eq!(light(15, 0, 12000), 15);
        assert_eq!(light(15, 0, 0), 2);
        assert_eq!(light(15, 13, 0), 13);
        assert_eq!(light(0, 7, 12000), 7);
        assert!(light(15, 0, 6000) >= LIGHT_LIMIT && light(15, 0, 5000) < LIGHT_LIMIT);
        // Let out of a bottle: two up, or one if that's all there is.
        let air = crate::world::CONTENT_AIR;
        assert_eq!(release_to((0, 0, 0), &|_| air), Some((0, 2, 0)));
        assert_eq!(release_to((0, 0, 0), &|p| if p.1 == 2 { 1 } else { air }), Some((0, 1, 0)));
        assert_eq!(release_to((0, 0, 0), &|p| if p.1 == 1 { 1 } else { air }), None);
    }
}
