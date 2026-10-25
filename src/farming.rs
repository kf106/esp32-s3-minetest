// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates
// Based on minetest_game's farming mod (mods/farming, MIT licence):
//   Copyright (C) 2012-2016 PilzAdam
//   Copyright (C) 2014-2016 webdesigner97
//   Copyright (C) 2012-2016 Various Minetest Game developers and contributors

//! Farming, as minetest_game's farming mod (mods/farming): hoes turn dirt
//! into soil, which is wet with water within 3 nodes and dries out (and
//! turns back to dirt if nothing is planted) without; seeds planted on soil
//! sprout and grow through 8 stages on wet soil in daylight; ripe plants
//! give more. Wheat seeds come from tall grass, cotton seeds from wild
//! cotton.
//!
//! The pure parts are here; the server applies the changes, with the
//! timers of nature.rs.

use std::time::Duration;

use crate::content::*;
use crate::nature::{quick, Rng};
use crate::world::CONTENT_AIR;

// Node ids, after water. Luanti keeps 125, 126 and 127 for itself (unknown,
// air, ignore), so the list jumps from 124 to 128.
pub const GRASS: [u16; 5] = [100, 101, 102, 103, 104];
pub const COTTON_WILD: u16 = 105;
pub const SOIL: u16 = 106;
pub const SOIL_WET: u16 = 107;
/// Savanna soil and desert sand soil, dry and wet.
pub const DRY_SOIL: u16 = crate::content::EMERGENT_JUNGLE_SAPLING + 1;
pub const DRY_SOIL_WET: u16 = DRY_SOIL + 1;
pub const DESERT_SAND_SOIL: u16 = DRY_SOIL_WET + 1;
pub const DESERT_SAND_SOIL_WET: u16 = DESERT_SAND_SOIL + 1;
pub const SEED_WHEAT: u16 = 108;
pub const SEED_COTTON: u16 = 109;
pub const WHEAT_STAGES: [u16; 8] = [110, 111, 112, 113, 114, 115, 116, 117];
pub const STRAW: u16 = 118;
pub const WOOL_WHITE: u16 = 119;
pub const COTTON_STAGES: [u16; 8] = [120, 121, 122, 123, 124, 128, 129, 130];
/// The savanna's dry grass (five heights) and the rainforest's jungle
/// grass, after the rainforest ground.
pub const DRY_GRASS: [u16; 5] = [161, 162, 163, 164, 165];
pub const JUNGLEGRASS: u16 = 166;

/// minetest_game's register_plant, for what we need of it.
pub struct Crop {
    pub seed: u16,
    pub harvest: u16,
    pub stages: [u16; 8],
    /// Wheat grows as a "#" of four pictures (meshoptions param2 3);
    /// cotton as the usual "X".
    pub param2: u8,
}

pub const CROPS: [Crop; 2] = [
    Crop { seed: SEED_WHEAT, harvest: ITEM_WHEAT, stages: WHEAT_STAGES, param2: 3 },
    Crop { seed: SEED_COTTON, harvest: ITEM_COTTON, stages: COTTON_STAGES, param2: 0 },
];

/// The crop a seed or plant belongs to, and its stage (0 for the seed, 1 to
/// 8 for the plant).
pub fn crop_of(id: u16) -> Option<(&'static Crop, usize)> {
    CROPS.iter().find_map(|c| {
        if c.seed == id {
            Some((c, 0))
        } else {
            c.stages.iter().position(|&s| s == id).map(|i| (c, i + 1))
        }
    })
}

pub fn is_seed(id: u16) -> bool {
    matches!(crop_of(id), Some((_, 0)))
}

/// Seeds and unripe plants: what has a growth timer.
pub fn is_growing(id: u16) -> bool {
    matches!(crop_of(id), Some((_, stage)) if stage < 8)
}

pub fn is_grass(id: u16) -> bool {
    GRASS.contains(&id)
}

pub fn is_dry_grass(id: u16) -> bool {
    DRY_GRASS.contains(&id)
}

/// Plants standing on the node below, which fall off (and drop) when it is
/// dug (minetest_game's attached_node).
pub fn is_attached_plant(id: u16) -> bool {
    crop_of(id).is_some() || is_grass(id) || is_dry_grass(id) || id == COTTON_WILD || id == JUNGLEGRASS || crate::plants::is_small_plant(id)
        || id == crate::content::DRY_SHRUB
        || crate::colours::is_flower(id)
        || crate::colours::is_mushroom(id)
}

/// farming's soils, each with its "soil" definition (farming/nodes.lua):
/// what it goes back to, its dry form and its wet form.
const SOILS: [(u16, u16, u16); 3] = [
    (CONTENT_DIRT, SOIL, SOIL_WET),
    (CONTENT_DRY_DIRT, DRY_SOIL, DRY_SOIL_WET),
    (CONTENT_DESERT_SAND, DESERT_SAND_SOIL, DESERT_SAND_SOIL_WET),
];

/// A soil's (base, dry, wet), if it is one.
fn soil_of(id: u16) -> Option<(u16, u16, u16)> {
    SOILS.iter().copied().find(|&(_, dry, wet)| id == dry || id == wet)
}

/// Whether a node is farming soil (group:field), wet or dry.
pub fn is_soil(id: u16) -> bool {
    soil_of(id).is_some()
}

/// Whether a node is wet soil (soil 3): what plants grow on.
pub fn is_wet_soil(id: u16) -> bool {
    soil_of(id).is_some_and(|(_, _, wet)| id == wet)
}

/// What a hoe makes of a node, by its "soil" definition: dirt, grass
/// and forest litter become soil, savanna dirt (and its grass) savanna
/// soil, and desert sand desert sand soil.
pub fn hoed(id: u16) -> Option<u16> {
    match id {
        CONTENT_DIRT | CONTENT_GRASS | CONTENT_LITTER | CONTENT_CONIFEROUS_LITTER | CONTENT_DIRT_DRY_GRASS => Some(SOIL),
        CONTENT_DRY_DIRT | CONTENT_DRY_GRASS => Some(DRY_SOIL),
        CONTENT_DESERT_SAND => Some(DESERT_SAND_SOIL),
        _ => None,
    }
}

/// How many uses a hoe has (farming/hoes.lua's max_uses).
pub fn hoe_uses(tool: u16) -> Option<u32> {
    match tool {
        TOOL_HOE_WOOD => Some(30),
        TOOL_HOE_STONE => Some(90),
        TOOL_HOE_STEEL => Some(500),
        _ => None,
    }
}

/// What happens to soil now (the "Farming soil" ABM), given the node above
/// it and whether there is water within 3 nodes. None: it stays as it is.
pub fn soil_update(soil: u16, above: u16, water_near: bool) -> Option<u16> {
    let (base, dry, wet) = soil_of(soil)?;
    // Something solid on top: back to what it was.
    if walkable(above) && !is_attached_plant(above) {
        return Some(base);
    }
    let planted = crop_of(above).is_some();
    match (soil == wet, water_near) {
        (false, true) => Some(wet),
        (false, false) if !planted => Some(base),
        (true, false) => Some(dry),
        _ => None,
    }
}

/// How often soil is looked at, and the chance each time (the ABM's
/// interval 15 and chance 4).
const SOIL_INTERVAL: Duration = Duration::from_secs(15);
const SOIL_CHANCE: u32 = 4;

/// The soil check's interval and chance. For testing on a PC,
/// LUANTI_SOIL_SECS=n looks at every soil node every n seconds.
pub fn soil_check() -> (Duration, u32) {
    #[cfg(not(target_os = "espidf"))]
    if let Some(secs) = std::env::var("LUANTI_SOIL_SECS").ok().and_then(|v| v.parse().ok()) {
        return (Duration::from_secs(secs), 1);
    }
    (SOIL_INTERVAL, SOIL_CHANCE)
}
/// How far water keeps soil wet.
pub const WATER_REACH: i32 = 3;

/// The next step for a seed or plant (farming.grow_plant): the node and
/// param2 it becomes, or None if it can't grow now. A seed sprouts on any
/// soil; a plant needs wet soil and light (daylight under the open sky:
/// minetest_game's minlight 13).
pub fn grow(id: u16, soil_below: u16, lit: bool) -> Option<(u16, u8)> {
    let (crop, stage) = crop_of(id)?;
    if stage >= 8 {
        return None;
    }
    let ok = if stage == 0 { is_soil(soil_below) } else { is_wet_soil(soil_below) && lit };
    ok.then(|| (crop.stages[stage], crop.param2))
}

/// The time to the next growth step (farming's tick: 166 to 286 s), and
/// after a failed try (tick_again: 40 to 80 s).
pub fn grow_delay(rng: &mut Rng) -> Duration {
    test_delay().unwrap_or_else(|| quick(Duration::from_secs(rng.range(166, 286) as u64)))
}

pub fn grow_retry(rng: &mut Rng) -> Duration {
    test_delay().unwrap_or_else(|| quick(Duration::from_secs(rng.range(40, 80) as u64)))
}

/// For testing on a PC: LUANTI_GROW_SECS=n makes every step take n s.
fn test_delay() -> Option<Duration> {
    #[cfg(not(target_os = "espidf"))]
    if let Some(secs) = std::env::var("LUANTI_GROW_SECS").ok().and_then(|v| v.parse().ok()) {
        return Some(Duration::from_secs(secs));
    }
    None
}

/// What digging a farming node gives: a plant at stage i rolls for its
/// harvest one time in r and again one time in 2r, and the same for seeds,
/// with r = 9 - i (register_plant's base_rarity), so a ripe plant always
/// gives one of each and often two. Tall grass gives a wheat seed one time
/// in 5, otherwise itself; wild cotton gives a cotton seed.
pub fn drops(id: u16, rng: &mut Rng) -> Vec<(u16, u16)> {
    if let Some((crop, stage)) = crop_of(id) {
        if stage == 0 {
            return vec![(id, 1)];
        }
        let r = 9 - stage as u32;
        let mut out = Vec::new();
        for (item, rarity) in [(crop.harvest, r), (crop.harvest, 2 * r), (crop.seed, r), (crop.seed, 2 * r)] {
            if rng.one_in(rarity) {
                match out.iter_mut().find(|(i, _): &&mut (u16, u16)| *i == item) {
                    Some((_, n)) => *n += 1,
                    None => out.push((item, 1)),
                }
            }
        }
        return out;
    }
    if is_grass(id) {
        return vec![(if rng.one_in(5) { SEED_WHEAT } else { GRASS[0] }, 1)];
    }
    // Savanna grass gives itself; jungle grass a cotton seed one time in 8
    // (farming/init.lua's override), otherwise itself.
    if is_dry_grass(id) {
        return vec![(DRY_GRASS[0], 1)];
    }
    // So do ferns and marram grass; flowers and mushrooms give themselves.
    if let Some(item) = crate::plants::small_plant_drop(id) {
        return vec![(item, 1)];
    }
    if crate::colours::is_flower(id) || crate::colours::is_mushroom(id) {
        return vec![(id, 1)];
    }
    match id {
        JUNGLEGRASS => vec![(if rng.one_in(8) { SEED_COTTON } else { JUNGLEGRASS }, 1)],
        COTTON_WILD => vec![(SEED_COTTON, 1)],
        // Soil gives back its dirt or sand.
        id if is_soil(id) => drop_of(id).map(|(item, _)| vec![(item, 1)]).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Tall grass placed by a player is any of the five heights
/// (default:grass_1's on_place).
pub fn placed_grass(id: u16, rng: &mut Rng) -> u16 {
    match id {
        _ if id == GRASS[0] => GRASS[rng.range(0, 4) as usize],
        _ if id == DRY_GRASS[0] => DRY_GRASS[rng.range(0, 4) as usize],
        _ => id,
    }
}

/// Whether a node is water, for keeping soil wet (group:water).
pub fn is_water(id: u16) -> bool {
    matches!(id, CONTENT_WATER | CONTENT_WATER_FLOWING)
}

/// Whether nothing stands on the soil (for hoeing: the node above must be
/// air).
pub fn clear_above(id: u16) -> bool {
    id == CONTENT_AIR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_and_soil() {
        assert!(matches!(crop_of(SEED_COTTON), Some((c, 0)) if c.harvest == ITEM_COTTON));
        assert!(matches!(crop_of(128), Some((c, 6)) if c.seed == SEED_COTTON));
        assert!(is_growing(WHEAT_STAGES[6]) && !is_growing(WHEAT_STAGES[7]));
        // A seed sprouts on dry soil; a plant needs wet soil and light.
        assert_eq!(grow(SEED_WHEAT, SOIL, false), Some((WHEAT_STAGES[0], 3)));
        assert_eq!(grow(WHEAT_STAGES[0], SOIL, true), None);
        assert_eq!(grow(WHEAT_STAGES[0], SOIL_WET, false), None);
        assert_eq!(grow(COTTON_STAGES[4], SOIL_WET, true), Some((128, 0)));
        assert_eq!(grow(WHEAT_STAGES[7], SOIL_WET, true), None);
        // Soil: wet by water, dries without, back to dirt when bare and dry
        // or covered.
        assert_eq!(soil_update(SOIL, CONTENT_AIR, true), Some(SOIL_WET));
        assert_eq!(soil_update(SOIL_WET, CONTENT_AIR, false), Some(SOIL));
        assert_eq!(soil_update(SOIL, CONTENT_AIR, false), Some(CONTENT_DIRT));
        assert_eq!(soil_update(SOIL, SEED_WHEAT, false), None);
        assert_eq!(soil_update(SOIL_WET, CONTENT_STONE, true), Some(CONTENT_DIRT));
        assert_eq!(soil_update(SOIL_WET, WHEAT_STAGES[3], true), None);
        assert_eq!(hoed(CONTENT_GRASS), Some(SOIL));
        assert_eq!(hoed(CONTENT_SAND), None);
        // Savanna soil and desert sand soil: the same, each back to its
        // own dirt or sand.
        assert_eq!(hoed(CONTENT_DRY_GRASS), Some(DRY_SOIL));
        assert_eq!(hoed(CONTENT_DRY_DIRT), Some(DRY_SOIL));
        assert_eq!(hoed(CONTENT_DESERT_SAND), Some(DESERT_SAND_SOIL));
        assert_eq!(soil_update(DRY_SOIL, CONTENT_AIR, true), Some(DRY_SOIL_WET));
        assert_eq!(soil_update(DRY_SOIL_WET, CONTENT_AIR, false), Some(DRY_SOIL));
        assert_eq!(soil_update(DRY_SOIL, CONTENT_AIR, false), Some(CONTENT_DRY_DIRT));
        assert_eq!(soil_update(DESERT_SAND_SOIL_WET, CONTENT_STONE, true), Some(CONTENT_DESERT_SAND));
        assert_eq!(soil_update(DESERT_SAND_SOIL, SEED_COTTON, false), None);
        assert_eq!(grow(WHEAT_STAGES[0], DESERT_SAND_SOIL_WET, true), Some((WHEAT_STAGES[1], 3)));
        assert_eq!(grow(WHEAT_STAGES[0], DRY_SOIL, true), None);
        let mut rng = Rng::new();
        assert_eq!(drops(DRY_SOIL_WET, &mut rng), vec![(CONTENT_DRY_DIRT, 1)]);
        assert_eq!(drops(DESERT_SAND_SOIL, &mut rng), vec![(CONTENT_DESERT_SAND, 1)]);
        assert_eq!(drops(SOIL, &mut rng), vec![(CONTENT_DIRT, 1)]);
    }

    #[test]
    fn harvests() {
        let mut rng = Rng::new();
        let (mut wheat, mut seeds, mut unripe) = (0, 0, 0);
        for _ in 0..2000 {
            for (item, n) in drops(WHEAT_STAGES[7], &mut rng) {
                if item == ITEM_WHEAT { wheat += n } else { seeds += n }
            }
            unripe += drops(WHEAT_STAGES[0], &mut rng).len();
        }
        // Ripe: one of each always, a second half the time.
        assert!((2900..3100).contains(&wheat) && (2900..3100).contains(&seeds), "{wheat} {seeds}");
        // Stage 1: rarely anything.
        assert!(unripe < 1000, "{unripe}");
        let grass_seeds = (0..5000).filter(|_| drops(GRASS[2], &mut rng)[0].0 == SEED_WHEAT).count();
        assert!((800..1200).contains(&grass_seeds), "{grass_seeds}");
    }
}
