// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2026 Keir Finlow-Bates
// Translated from minetest_game's cactus, papyrus, bushes, kelp and corals
// (mods/default/nodes.lua, functions.lua):
//   Copyright (C) 2011-2018 celeron55, Perttu Ahola <celeron55@gmail.com>
//   Copyright (C) 2011-2018 Various Minetest Game developers and contributors
// Licensed under the GNU Lesser General Public License, version 2.1 or (at
// your option) any later version: see COPYING.LESSER.

//! minetest_game's other plants (mods/default/nodes.lua, functions.lua):
//! cactus and papyrus, which grow upwards; the bushes, with blueberries
//! that grow back; ferns and marram grass; and under water, kelp and
//! corals.
//!
//! - Cactus grows on sand and papyrus on dirt near water, one node at a
//!   time up to 4 tall, in daylight (the "Grow cactus" and "Grow papyrus"
//!   ABMs). Here only those players planted grow; the generated ones are
//!   already grown.
//! - Digging papyrus takes the papyrus above it too (default.dig_up).
//! - Picking blueberries leaves the bush's leaves, which bear berries again
//!   after 5 to 25 minutes, if light enough.
//! - Kelp is a sand node with a plant growing out of it, drawn up to 6
//!   nodes tall through the water above (plantlike_rooted; param2 is its
//!   height in sixteenths). Digging it leaves the sand. Corals are the
//!   same on coral skeleton, a node tall.
//!
//! The pure parts are here; the server applies them, and terrain.rs
//! places the generated ones.

use std::time::Duration;

use crate::content::*;
use crate::world::{NodePos, CONTENT_AIR};

// Node ids, after the jungle and acacia stairs.
pub const CACTUS: u16 = 183;
pub const PAPYRUS: u16 = 184;
pub const BUSH_STEM: u16 = 185;
pub const BUSH_LEAVES: u16 = 186;
pub const BUSH_SAPLING: u16 = 187;
pub const BLUEBERRY_LEAVES_WITH_BERRIES: u16 = 188;
pub const BLUEBERRY_LEAVES: u16 = 189;
pub const BLUEBERRY_SAPLING: u16 = 190;
pub const ACACIA_BUSH_STEM: u16 = 191;
pub const ACACIA_BUSH_LEAVES: u16 = 192;
pub const ACACIA_BUSH_SAPLING: u16 = 193;
/// minetest_game's pine bush, for our fir.
pub const FIR_BUSH_STEM: u16 = 194;
pub const FIR_BUSH_NEEDLES: u16 = 195;
pub const FIR_BUSH_SAPLING: u16 = 196;
pub const FERN: [u16; 3] = [197, 198, 199];
pub const MARRAM: [u16; 3] = [200, 201, 202];
pub const KELP: u16 = 203;
pub const CORAL_GREEN: u16 = 204;
pub const CORAL_PINK: u16 = 205;
pub const CORAL_CYAN: u16 = 206;
pub const CORAL_BROWN: u16 = 207;
pub const CORAL_ORANGE: u16 = 208;
pub const CORAL_SKELETON: u16 = 209;
pub const LARGE_CACTUS_SEEDLING: u16 = 210;

/// Health blueberries give (minetest_game's item_eat(2)).
pub const BLUEBERRY_HP: u16 = 2;

/// The corals with a plant on them (plantlike_rooted).
pub fn is_rooted(id: u16) -> bool {
    matches!(id, KELP | CORAL_GREEN | CORAL_PINK | CORAL_CYAN)
}

/// Plants that stand on the node below and drop when it goes
/// (attached_node), and are built over (buildable_to).
pub fn is_small_plant(id: u16) -> bool {
    FERN.contains(&id) || MARRAM.contains(&id)
}

/// What a small plant drops: the first of its sizes (as grass does).
pub fn small_plant_drop(id: u16) -> Option<u16> {
    if FERN.contains(&id) {
        Some(FERN[0])
    } else if MARRAM.contains(&id) {
        Some(MARRAM[0])
    } else {
        None
    }
}

/// A fern or marram grass placed by a player is any of its sizes
/// (their on_place).
pub fn placed_size(id: u16, roll: u32) -> u16 {
    match id {
        _ if id == FERN[0] => FERN[(roll % 3) as usize],
        _ if id == MARRAM[0] => MARRAM[(roll % 3) as usize],
        _ => id,
    }
}

/// What is left where a node was dug (after_dig_node): the sand under
/// kelp, the skeleton of a coral, the leaves of picked blueberries.
pub fn after_dig(id: u16) -> Option<u16> {
    match id {
        KELP => Some(CONTENT_SAND),
        CORAL_GREEN | CORAL_PINK | CORAL_CYAN => Some(CORAL_SKELETON),
        BLUEBERRY_LEAVES_WITH_BERRIES => Some(BLUEBERRY_LEAVES),
        _ => None,
    }
}

/// Where a kelp item can go (its on_place): into sand, with water source
/// `height` nodes above it, where the plant's top reaches. `node` gives
/// the content at a position. Returns the param2 to set.
pub fn kelp_place(under: NodePos, height: i32, node: &dyn Fn(NodePos) -> u16) -> Option<u8> {
    let top = (under.0, under.1 + height, under.2);
    (node(under) == CONTENT_SAND && node(top) == CONTENT_WATER).then_some((height * 16) as u8)
}

/// Where a coral item can go (coral_on_place): into coral skeleton, with
/// water source above it.
pub fn coral_place(id: u16, under: NodePos, node: &dyn Fn(NodePos) -> u16) -> bool {
    matches!(id, CORAL_GREEN | CORAL_PINK | CORAL_CYAN)
        && node(under) == CORAL_SKELETON
        && node((under.0, under.1 + 1, under.2)) == CONTENT_WATER
}

/// The plants that grow upwards, and on what.
pub fn is_climber(id: u16) -> bool {
    matches!(id, CACTUS | PAPYRUS)
}

/// How often planted cactus and papyrus are looked at, and the chance
/// each time that one grows: the "Grow cactus" ABM's 12 s and 1 in 83
/// (the papyrus ABM's 14 s and 1 in 71 comes to about the same). Ten
/// times as likely in a test build.
pub const CLIMB_INTERVAL: Duration = Duration::from_secs(12);
pub const CLIMB_CHANCE: u32 = if crate::nature::TEST_BUILD { 8 } else { 83 };

/// How far water may be from the dirt under papyrus.
const PAPYRUS_WATER: i32 = 3;
/// The tallest a cactus or papyrus grows.
const CLIMB_MAX: i32 = 4;

/// default.grow_cactus and grow_papyrus, for the node at `pos`: where the
/// plant grows a node, if it can now. Only the bottom node grows, on sand
/// (cactus) or dirt with water near (papyrus), into air at the top of a
/// column shorter than 4, in daylight (`lit` says whether a position is
/// lit by the sun).
pub fn climb(pos: NodePos, node: &dyn Fn(NodePos) -> u16, lit: &dyn Fn(NodePos) -> bool) -> Option<NodePos> {
    let id = node(pos);
    let below = node((pos.0, pos.1 - 1, pos.2));
    let ground_ok = match id {
        CACTUS => group_of(below, "sand") > 0,
        PAPYRUS => {
            matches!(below, CONTENT_DIRT | CONTENT_GRASS | CONTENT_DRY_GRASS | CONTENT_LITTER | CONTENT_DRY_DIRT | CONTENT_DIRT_DRY_GRASS) && {
                let (x, y, z) = (pos.0, pos.1 - 1, pos.2);
                let r = PAPYRUS_WATER;
                (-r..=r).any(|dy| (-r..=r).any(|dz| (-r..=r).any(|dx| crate::farming::is_water(node((x + dx, y + dy, z + dz))))))
            }
        }
        _ => false,
    };
    if !ground_ok {
        return None;
    }
    let mut top = pos;
    let mut height = 1;
    while node((top.0, top.1 + 1, top.2)) == id && height < CLIMB_MAX {
        top.1 += 1;
        height += 1;
    }
    let next = (top.0, top.1 + 1, top.2);
    (height < CLIMB_MAX && node(next) == CONTENT_AIR && lit(next)).then_some(next)
}

/// default.dig_up: the papyrus above a dug one, which goes with it.
pub fn dug_up(pos: NodePos, id: u16, node: &dyn Fn(NodePos) -> u16) -> Vec<NodePos> {
    if id != PAPYRUS {
        return Vec::new();
    }
    (1..100).map(|dy| (pos.0, pos.1 + dy, pos.2)).take_while(|&p| node(p) == PAPYRUS).collect()
}

/// Blueberries grow back after math.random(300, 1500) s; in the dark
/// (light below 11) the bush waits 200 s more.
pub const BERRIES_RETRY: Duration = Duration::from_secs(200);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn climbing() {
        let mut w: HashMap<NodePos, u16> = HashMap::new();
        w.insert((0, 0, 0), CONTENT_SAND);
        w.insert((0, 1, 0), CACTUS);
        w.insert((5, 0, 0), CONTENT_DIRT);
        w.insert((5, 1, 0), PAPYRUS);
        let lit = |_: NodePos| true;
        let get = |w: &HashMap<NodePos, u16>, p: NodePos| w.get(&p).copied().unwrap_or(CONTENT_AIR);
        assert_eq!(climb((0, 1, 0), &|p| get(&w, p), &lit), Some((0, 2, 0)));
        // Papyrus needs water near its dirt.
        assert_eq!(climb((5, 1, 0), &|p| get(&w, p), &lit), None);
        w.insert((7, 0, 1), CONTENT_WATER);
        assert_eq!(climb((5, 1, 0), &|p| get(&w, p), &lit), Some((5, 2, 0)));
        // Four tall: no more.
        for y in 2..=4 {
            w.insert((0, y, 0), CACTUS);
        }
        assert_eq!(climb((0, 1, 0), &|p| get(&w, p), &lit), None);
        // Only the bottom node grows; not in the dark.
        assert_eq!(climb((0, 2, 0), &|p| get(&w, p), &lit), None);
        assert_eq!(climb((5, 1, 0), &|p| get(&w, p), &|_| false), None);
        w.insert((5, 2, 0), PAPYRUS);
        w.insert((5, 3, 0), PAPYRUS);
        assert_eq!(dug_up((5, 1, 0), PAPYRUS, &|p| get(&w, p)), vec![(5, 2, 0), (5, 3, 0)]);
    }

    #[test]
    fn under_water() {
        let node = |p: NodePos| if p.1 == 0 { CONTENT_SAND } else if p.1 <= 6 { CONTENT_WATER } else { CONTENT_AIR };
        assert_eq!(kelp_place((0, 0, 0), 5, &node), Some(80));
        assert_eq!(kelp_place((0, 0, 0), 7, &node), None, "its top would be out of the water");
        assert_eq!(after_dig(KELP), Some(CONTENT_SAND));
        let reef = |p: NodePos| if p.1 == 0 { CORAL_SKELETON } else { CONTENT_WATER };
        assert!(coral_place(CORAL_PINK, (0, 0, 0), &reef) && !coral_place(CORAL_BROWN, (0, 0, 0), &reef));
        assert_eq!(placed_size(FERN[0], 4), FERN[1]);
        assert_eq!(small_plant_drop(MARRAM[2]), Some(MARRAM[0]));
    }
}
