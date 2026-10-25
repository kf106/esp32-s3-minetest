// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates
// Based on minetest_game's tnt mod (mods/tnt/init.lua):
//   Copyright (C) 2014-2016 PilzAdam, ShadowNinja, sofar and Various
//   Minetest Game developers and contributors
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

//! TNT and gunpowder, as minetest_game's tnt mod has them.
//!
//! - Coal and gravel make gunpowder, which lies on the ground in a trail.
//!   Lit (punched with a torch, struck with flint and steel, caught by
//!   fire or lava, or in a blast), it burns for a second, lights what is
//!   next to it along the trail (and a node up or down), and is gone.
//! - Gunpowder and paper make TNT sticks, nine of which make TNT. Lit the
//!   same ways, TNT burns for 4 s, then explodes: everything within 3
//!   nodes (a little more or less in places) is destroyed and thrown out
//!   as items, flammable things catch fire, other TNT in the blast goes
//!   off a moment later, and players near it are hurt and thrown back.
//!   TNT within 2 nodes of each other goes off together, in a bigger blast
//!   (on the board, no bigger than 27 together make: radius 9).
//! - Chests, furnaces and shelves in a blast spill what they hold; locked
//!   chests, doors and trapdoors, open chests and bones are left alone.
//!
//! While TNT is off (`/tnt off`, see settings.rs), neither TNT nor
//! gunpowder can be lit, and fire doesn't burn them, so a moderator can
//! stop a griefer. (minetest_game has no TNT at all when it is off.)
//!
//! Unlike minetest_game's, lit TNT doesn't fall.
//!
//! The pure parts are here; the server applies them.

use std::time::Duration;

use crate::content;
use crate::nature::Rng;

/// The tnt mod's tnt:gunpowder and tnt:gunpowder_burning, tnt:tnt and
/// tnt:tnt_burning.
pub const GUNPOWDER: u16 = crate::fire::PERMANENT_FLAME + 1;
pub const GUNPOWDER_BURNING: u16 = GUNPOWDER + 1;
pub const TNT: u16 = GUNPOWDER_BURNING + 1;
pub const TNT_BURNING: u16 = TNT + 1;

/// Gunpowder joins gunpowder (minetest.raillike_group("gunpowder")).
pub const RAILLIKE_GROUP: i16 = 1;
pub const POWDER_IMAGE: &str = "tnt_gunpowder_inventory.png";
/// A trail's selection box: a sixteenth of a node thick.
pub const POWDER_BOX: [f32; 6] = [-0.5, -0.5, -0.5, 0.5, -0.5 + 1.0 / 16.0, 0.5];
/// Lit gunpowder and TNT glow (light_source 5; only the client draws it).
pub const BURNING_GLOW: u8 = 5;

/// The blast's radius for one TNT (tnt_radius), and how far it hurts and
/// throws back (damage_radius, twice that).
pub const RADIUS: i32 = 3;
const DAMAGE_RADIUS: f32 = 6.0;
/// Lit TNT goes off after this; lit gunpowder burns this long; TNT caught
/// in a blast goes off this much later (minetest.after(0.1)).
pub const FUSE: Duration = Duration::from_secs(4);
pub const POWDER_BURNS: Duration = Duration::from_secs(1);
pub const CHAINED: Duration = Duration::from_millis(100);
/// The "TNT ignition" ABM: every 4 s, TNT and gunpowder beside a flame or
/// lava are lit.
pub const IGNITION_EVERY: Duration = Duration::from_secs(4);

pub fn is_powder(id: u16) -> bool {
    id == GUNPOWDER || id == GUNPOWDER_BURNING
}

pub fn is_lit(id: u16) -> bool {
    id == GUNPOWDER_BURNING || id == TNT_BURNING
}

/// Whether a node is unlit TNT or gunpowder: what lighting lights.
pub fn lights(id: u16) -> bool {
    id == GUNPOWDER || id == TNT
}

/// The tnt mod's animated textures (lit TNT's top, burning gunpowder).
pub fn is_animated(tile: &str) -> bool {
    tile.starts_with("tnt_") && tile.ends_with("_animated.png")
}

/// What lighting turns a node into: lit gunpowder or TNT.
pub fn lit(id: u16) -> Option<u16> {
    match id {
        GUNPOWDER => Some(GUNPOWDER_BURNING),
        TNT => Some(TNT_BURNING),
        _ => None,
    }
}

/// Where burning gunpowder lights next (its on_timer): the four nodes
/// beside it, and those a node up and down from them.
pub fn powder_reach(p: (i32, i32, i32)) -> impl Iterator<Item = (i32, i32, i32)> {
    [(1, 0), (-1, 0), (0, 1), (0, -1)].into_iter().flat_map(move |(dx, dz)| (-1..=1).map(move |dy| (p.0 + dx, p.1 + dy, p.2 + dz)))
}

/// The most TNT that count towards one blast on the board (27: radius
/// 9). More go off with it, but make it no bigger; a bigger blast would
/// hold the board up too long. The PC build takes all there can be (a
/// 5x5x5 block of 125: radius 15), as minetest_game does.
pub const MAX_TOGETHER: usize = if cfg!(pc) { 125 } else { 27 };

/// A blast's radius when `count` TNT go off together (radius times the
/// cube root of how many, up to MAX_TOGETHER).
pub fn blast_radius(count: usize) -> i32 {
    (RADIUS as f64 * (count.clamp(1, MAX_TOGETHER) as f64).cbrt()).floor() as i32
}

/// How far a blast of `radius` hurts and throws things.
pub fn damage_radius(radius: i32) -> f32 {
    radius as f32 / RADIUS as f32 * DAMAGE_RADIUS
}

/// Whether the blast reaches a node `d` from its middle: within the
/// radius, give or take a fifth, at random (radius^2 / r^2 >= 0.8 to
/// 1.25).
pub fn reaches(d: (i32, i32, i32), radius: i32, rng: &mut Rng) -> bool {
    let r2 = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2) as f32;
    let roll = rng.range(80, 125) as f32 / 100.0;
    r2 == 0.0 || (radius * radius) as f32 / r2 >= roll
}

/// What a blast does to a node (the tnt mod's destroy, and the nodes'
/// on_blast).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blasted {
    /// Left as it is: air, and what is unaffected by explosions.
    Spared,
    /// TNT: it goes off itself a moment later.
    Chain,
    /// Gunpowder: lit.
    Light,
    /// A chest, furnace or shelf: it and what it holds are thrown out.
    Emptied,
    /// Flammable: it catches fire.
    Flame,
    /// Gone, its drops thrown out.
    Destroyed,
}

pub fn blasted(id: u16) -> Blasted {
    use Blasted::*;
    match id {
        crate::world::CONTENT_AIR | GUNPOWDER_BURNING | TNT_BURNING => Spared,
        TNT => Chain,
        GUNPOWDER => Light,
        // On_blast doing nothing: locked chests, doors and trapdoors, an
        // open chest, bones, a door's hidden top; and our cloud, which
        // nothing digs.
        id if content::is_locked_chest(id) || crate::doors::is_locked(id) => Spared,
        content::CONTENT_CHEST_OPEN | content::CONTENT_CLOUD | crate::bones::BONES | crate::doors::HIDDEN => Spared,
        id if content::is_chest(id) || content::is_furnace(id) || content::shelf_of(id).is_some() => Emptied,
        // Unlocked doors and trapdoors are thrown out whole, though wooden.
        id if crate::doors::is_door(id) || crate::doors::is_trapdoor(id) => Destroyed,
        id if content::group_of(id, "flammable") > 0 => Flame,
        _ => Destroyed,
    }
}

/// The HP a blast of `radius` takes from a player `dist` nodes from it.
pub fn damage(dist: f32, radius: i32) -> f32 {
    4.0 / dist.max(1.0) * radius as f32
}

/// How much faster a blast throws a player `dist` from it, away from it
/// (`dir`, a unit vector), in nodes per second.
pub fn knockback(dir: [f32; 3], dist: f32, radius: i32) -> [f32; 3] {
    let s = 2.0 / dist.max(1.0) * radius as f32;
    dir.map(|v| v * s)
}

/// How a stack of `count` drops is thrown out: in handfuls of up to the
/// radius squared (and a stack's worth).
pub fn handfuls(count: u16, radius: i32, stack_max: u16) -> Vec<u16> {
    let most = ((radius * radius).max(1) as u16).min(stack_max).max(1);
    let mut left = count;
    let mut out = Vec::new();
    while left > 0 {
        let take = left.min(most);
        out.push(take);
        left -= take;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius() {
        assert_eq!(blast_radius(1), 3);
        assert_eq!(blast_radius(8), 6);
        assert_eq!(blast_radius(27), 9);
        assert_eq!(blast_radius(125), if cfg!(pc) { 15 } else { 9 });
        assert_eq!(blast_radius(64), if cfg!(pc) { 12 } else { 9 });
        assert_eq!(damage_radius(3), 6.0);
        // The middle and the near nodes always; beyond 1.25 times the
        // radius never.
        let mut rng = Rng::new();
        for _ in 0..100 {
            assert!(reaches((0, 0, 0), 3, &mut rng) && reaches((2, 0, 0), 3, &mut rng));
            assert!(!reaches((4, 0, 0), 3, &mut rng));
        }
        let some = (0..1000).filter(|_| reaches((3, 1, 0), 3, &mut rng)).count();
        assert!(some > 100 && some < 900, "{some}");
    }

    #[test]
    fn what_blasts_do() {
        use Blasted::*;
        assert_eq!(blasted(content::CONTENT_STONE), Destroyed);
        assert_eq!(blasted(content::CONTENT_APPLE_WOOD), Flame);
        assert_eq!(blasted(TNT), Chain);
        assert_eq!(blasted(GUNPOWDER), Light);
        assert_eq!(blasted(TNT_BURNING), Spared);
        assert_eq!(blasted(content::CONTENT_CHEST), Emptied);
        assert_eq!(blasted(content::CONTENT_CHEST_LOCKED), Spared);
        assert_eq!(blasted(content::CONTENT_FURNACE), Emptied);
        assert_eq!(blasted(content::CONTENT_BOOKSHELF), Emptied);
        assert_eq!(blasted(crate::bones::BONES), Spared);
        assert_eq!(blasted(crate::world::CONTENT_AIR), Spared);
        assert_eq!(blasted(content::CONTENT_CLOUD), Spared);
    }

    #[test]
    fn rules() {
        assert_eq!(lit(GUNPOWDER), Some(GUNPOWDER_BURNING));
        assert_eq!(lit(TNT_BURNING), None);
        assert_eq!(powder_reach((0, 0, 0)).count(), 12);
        assert_eq!(damage(0.5, 3), 12.0);
        assert_eq!(damage(4.0, 3), 3.0);
        assert_eq!(knockback([1.0, 0.0, 0.0], 2.0, 3), [3.0, 0.0, 0.0]);
        assert_eq!(handfuls(20, 3, 99), vec![9, 9, 2]);
        assert_eq!(handfuls(1, 15, 1), vec![1]);
        assert!(is_animated("tnt_top_burning_animated.png") && !is_animated("tnt_top.png"));
        assert!(content::stands_on_floor(GUNPOWDER) && !content::walkable(GUNPOWDER));
        assert_eq!(content::drop_of(TNT_BURNING), None);
    }
}
