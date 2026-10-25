// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2026 Keir Finlow-Bates
// Translated from minetest_game's leaf decay and saplings (mods/default/functions.lua,
// trees.lua, nodes.lua):
//   Copyright (C) 2011-2018 celeron55, Perttu Ahola <celeron55@gmail.com>
//   Copyright (C) 2011-2018 Various Minetest Game developers and contributors
// Licensed under the GNU Lesser General Public License, version 2.1 or (at
// your option) any later version: see COPYING.LESSER.

//! Leaf decay and saplings, as minetest_game has them (functions.lua,
//! trees.lua, nodes.lua):
//!
//! - When a trunk is dug, the leaves of its kind within LEAF_DECAY_RADIUS
//!   get a timer of 2 to 12 seconds. When it runs out, a leaf with no trunk
//!   of its tree within that radius disappears, and may drop a sapling.
//!   Leaves placed by players (param2 1) never decay.
//! - Leaves give a sapling one time in SAPLING_RARITY, dug or decayed.
//! - A planted sapling tries to grow after 5 to 25 minutes: on soil, in
//!   daylight. If it can't, it tries again 5 minutes later. It grows into
//!   a tree like the generated ones, filling only air (and leaves, for the
//!   trunk).
//!
//! - Apples hang among apple tree leaves. A picked apple leaves an invisible
//!   mark where an apple grows again after 5 to 25 minutes, if apple leaves
//!   are still next to it (minetest_game's apple_mark). Apples placed by
//!   players (param2 1) don't regrow, and like leaves never decay.
//!
//! The pure parts are here; the server applies the changes.

use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, Instant};

use crate::content::{self, *};
use crate::plants;
use crate::terrain::{Tree, TreeKind, TREE_REACH};
use crate::world::{NodePos, CONTENT_AIR};

/// param2 of leaves placed by a player: they never decay.
pub const PLACED_LEAVES: u8 = 1;

/// What a timer is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Job {
    /// See whether a leaf still has a trunk near it.
    Decay,
    /// Try to grow a sapling.
    Grow,
    /// Try to grow an apple again where one was picked.
    Regrow,
    /// Try to grow a seed or crop a stage (farming.rs).
    Crop,
    /// Try to grow blueberries again on a picked bush (plants.rs).
    Berries,
    /// See whether a flame still has something to burn (fire.rs).
    Flame,
    /// Lit TNT goes off (tnt.rs).
    Fuse,
    /// Lit gunpowder burns out, lighting what is next to it.
    Powder,
    /// TNT caught in a blast goes off.
    Chain,
}

/// Node timers (minetest_game's are per node; here they are kept in one
/// queue, soonest first). A node has at most one timer of each kind.
pub struct Timers {
    queue: BTreeSet<(Instant, NodePos, Job)>,
    set: HashSet<(NodePos, Job)>,
}

impl Timers {
    pub fn new() -> Self {
        Self { queue: BTreeSet::new(), set: HashSet::new() }
    }

    /// Start a timer unless that node already has one of that kind
    /// (minetest_game: "not timer:is_started()").
    pub fn start(&mut self, pos: NodePos, job: Job, after: Duration) {
        if self.set.insert((pos, job)) {
            self.queue.insert((Instant::now() + after, pos, job));
        }
    }

    /// The next timer that has run out, if any.
    pub fn pop_due(&mut self, now: Instant) -> Option<(NodePos, Job)> {
        let &(when, pos, job) = self.queue.first()?;
        if when > now {
            return None;
        }
        self.queue.pop_first();
        self.set.remove(&(pos, job));
        Some((pos, job))
    }
}

/// A small fast random number generator (xorshift32) for game chances.
pub struct Rng(u32);

impl Rng {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x1234_5678, |d| d.subsec_nanos() ^ d.as_secs() as u32);
        Self(seed | 1)
    }

    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A whole number from `lo` to `hi` inclusive (Lua's math.random(lo, hi)).
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.next() % (hi - lo + 1)
    }

    /// True one time in `n`.
    pub fn one_in(&mut self, n: u32) -> bool {
        self.next() % n == 0
    }
}

/// The leaf decay timer: math.random(20, 120) / 10 seconds.
pub fn decay_delay(rng: &mut Rng) -> Duration {
    Duration::from_millis(rng.range(20, 120) as u64 * 100)
}

/// A sapling's first try at growing: math.random(300, 1500) seconds.
pub fn grow_delay(rng: &mut Rng) -> Duration {
    // For testing on a PC: LUANTI_GROW_SECS=n makes saplings try after n s.
    #[cfg(not(target_os = "espidf"))]
    if let Some(secs) = std::env::var("LUANTI_GROW_SECS").ok().and_then(|v| v.parse().ok()) {
        return Duration::from_secs(secs);
    }
    quick(Duration::from_secs(rng.range(300, 1500) as u64))
}

/// Whether this firmware was built for testing (TEST=..., see build.rs).
pub const TEST_BUILD: bool = option_env!("TEST_KIT_TOKEN").is_some();
/// How much faster things grow in a test build.
const TEST_SPEED: u32 = 10;

/// A growing time, ten times shorter in a test build, so saplings, apples
/// and crops can be watched growing (normal builds keep minetest_game's
/// times).
pub fn quick(d: Duration) -> Duration {
    if TEST_BUILD { d / TEST_SPEED } else { d }
}

/// minetest_game's on_grow_failed: try again in 300 seconds.
pub const GROW_RETRY: Duration = Duration::from_secs(300);
/// An apple mark in the dark tries again after 200 seconds.
pub const REGROW_RETRY: Duration = Duration::from_secs(200);

/// What a leaf gives when dug or decayed: a sapling now and then (one
/// time in SAPLING_RARITY for trees, 5 for bushes), otherwise the leaf
/// (minetest_game's drop table, which gives one item).
pub fn leaf_drop(leaf: u16, rng: &mut Rng) -> u16 {
    match content::sapling_of_leaves(leaf) {
        Some((sapling, rarity)) if rng.one_in(rarity) => sapling,
        _ => leaf,
    }
}

/// What a node drops when it decays (leafdecay_on_timer drops everything
/// the node would drop except the leaves themselves, and apples, which
/// are leafdecay_drop): a sapling now and then, or the apple.
pub fn decay_drop(id: u16, rng: &mut Rng) -> Option<u16> {
    if content::is_apple(id) {
        return Some(id);
    }
    let drop = leaf_drop(id, rng);
    (drop != id).then_some(drop)
}

/// minetest_game's apple_mark timer: with no apple leaves next to it the
/// mark just goes; in the dark it waits; otherwise the apple is back.
#[derive(Debug, PartialEq)]
pub enum Regrow {
    Gone,
    Wait,
    Apple,
}

pub fn regrow(pos: NodePos, node: &dyn Fn(NodePos) -> u16, daylight: bool) -> Regrow {
    if !around(pos, 1).any(|p| node(p) == CONTENT_APPLE_LEAVES) {
        Regrow::Gone
    } else if !daylight {
        Regrow::Wait
    } else {
        Regrow::Apple
    }
}

fn around(pos: NodePos, r: i32) -> impl Iterator<Item = NodePos> {
    (-r..=r).flat_map(move |dy| (-r..=r).flat_map(move |dz| (-r..=r).map(move |dx| (pos.0 + dx, pos.1 + dy, pos.2 + dz))))
}

/// After a trunk was dug at `pos`: the leaves of its tree near it that may
/// now decay (leafdecay_after_destruct). `node` gives (content, param2).
pub fn leaves_to_check(pos: NodePos, trunk: u16, node: &dyn Fn(NodePos) -> (u16, u8)) -> Vec<NodePos> {
    if !content::is_trunk(trunk) {
        return Vec::new();
    }
    around(pos, LEAF_DECAY_RADIUS)
        .filter(|&p| {
            let (id, param2) = node(p);
            content::decay_trunk(id) == Some(trunk) && param2 != PLACED_LEAVES
        })
        .collect()
}

/// Whether a leaf at `pos` should go: it is still that leaf, not placed
/// by a player, and no trunk of its tree is within reach
/// (leafdecay_on_timer).
pub fn should_decay(pos: NodePos, node: &dyn Fn(NodePos) -> (u16, u8)) -> bool {
    let (id, param2) = node(pos);
    let Some(trunk) = content::decay_trunk(id) else {
        return false;
    };
    param2 != PLACED_LEAVES && !around(pos, LEAF_DECAY_RADIUS).any(|p| node(p).0 == trunk)
}

/// minetest_game's can_grow: soil under the sapling (sand under a large
/// cactus seedling), and light 13 or more on it, which (torches giving
/// 12) means daylight reaching it.
pub fn can_grow(pos: NodePos, node: &dyn Fn(NodePos) -> u16, sky_above: bool, daytime: bool) -> bool {
    let sapling = node(pos);
    let ground = if sapling == plants::LARGE_CACTUS_SEEDLING { "sand" } else { "soil" };
    content::is_sapling(sapling) && content::group_of(node((pos.0, pos.1 - 1, pos.2)), ground) > 0 && sky_above && daytime
}

/// The nodes of the tree (or bush, or large cactus) a sapling at `pos`
/// grows into: what grows at `pos` replaces the sapling, the trunk also
/// replaces leaves, and the rest fills only air (minetest_game's
/// tree schematics, which aren't force-placed). The tree is like the ones
/// the map generator makes; `height` is its trunk length.
pub fn grow(pos: NodePos, sapling: u16, height: i32, node: &dyn Fn(NodePos) -> u16) -> Vec<(NodePos, u16)> {
    let kind = match sapling {
        plants::BUSH_SAPLING => TreeKind::Bush,
        plants::BLUEBERRY_SAPLING => TreeKind::BlueberryBush,
        plants::ACACIA_BUSH_SAPLING => TreeKind::AcaciaBush,
        plants::FIR_BUSH_SAPLING => TreeKind::FirBush,
        plants::LARGE_CACTUS_SEEDLING => TreeKind::LargeCactus,
        CONTENT_FIR_SAPLING => TreeKind::Fir,
        CONTENT_APPLE_SAPLING => TreeKind::Apple,
        CONTENT_JUNGLE_SAPLING => TreeKind::Jungle,
        content::EMERGENT_JUNGLE_SAPLING => TreeKind::Emergent,
        CONTENT_ACACIA_SAPLING => TreeKind::Acacia,
        _ => TreeKind::Birch,
    };
    let tree = Tree { x: pos.0, z: pos.2, ground: pos.1 - 1, kind, height };
    let mut out = Vec::new();
    for y in pos.1..=tree.top() {
        for z in pos.2 - TREE_REACH..=pos.2 + TREE_REACH {
            for x in pos.0 - TREE_REACH..=pos.0 + TREE_REACH {
                let Some(id) = tree.node_at(x, y, z) else {
                    continue;
                };
                let here = node((x, y, z));
                let trunk = content::is_trunk(id) || id == plants::CACTUS;
                // The sapling itself always goes (a blueberry bush has no
                // stem to replace it with: its middle leaves do).
                let fits = here == CONTENT_AIR || (x, y, z) == pos || (trunk && content::is_leaves(here));
                if fits && here != id {
                    out.push(((x, y, z), id));
                }
            }
        }
    }
    out
}

/// A height for a grown tree, in the generator's range.
pub fn tree_height(sapling: u16, rng: &mut Rng) -> i32 {
    match sapling {
        CONTENT_FIR_SAPLING => rng.range(7, 10) as i32,
        CONTENT_APPLE_SAPLING => rng.range(4, 5) as i32,
        CONTENT_JUNGLE_SAPLING => rng.range(8, 12) as i32,
        CONTENT_ACACIA_SAPLING => rng.range(5, 6) as i32,
        // Bushes are a node tall (their stem); a large cactus 5.
        plants::LARGE_CACTUS_SEEDLING => 5,
        id if content::is_sapling(id) && id >= plants::CACTUS => 1,
        _ => rng.range(5, 7) as i32,
    }
}

#[cfg(test)]
mod grass_tests {
    use super::*;

    #[test]
    fn spreading_and_covering() {
        assert!(spreads(CONTENT_GRASS) && spreads(CONTENT_LITTER) && spreads(CONTENT_CONIFEROUS_LITTER) && spreads(CONTENT_SNOW));
        assert!(!spreads(CONTENT_DIRT) && !spreads(CONTENT_DRY_GRASS));
        assert_eq!(spread_onto(CONTENT_AIR, Some(CONTENT_LITTER)), Some(CONTENT_LITTER));
        assert_eq!(spread_onto(CONTENT_SNOW_LAYER, None), Some(CONTENT_SNOW));
        assert_eq!(spread_onto(crate::farming::GRASS[2], None), Some(CONTENT_GRASS));
        // Savanna grass on dirt: dirt with savanna grass, which spreads
        // in turn, and goes back to dirt when covered.
        assert_eq!(spread_onto(crate::farming::DRY_GRASS[0], None), Some(content::CONTENT_DIRT_DRY_GRASS));
        assert!(spreads(content::CONTENT_DIRT_DRY_GRASS));
        assert_eq!(spread_onto(CONTENT_AIR, Some(content::CONTENT_DIRT_DRY_GRASS)), Some(content::CONTENT_DIRT_DRY_GRASS));
        assert_eq!(covered(content::CONTENT_DIRT_DRY_GRASS, CONTENT_STONE), Some(CONTENT_DIRT));
        assert_eq!(spread_onto(CONTENT_AIR, None), None);
        assert!(lets_grass_in(CONTENT_AIR) && !lets_grass_in(CONTENT_STONE));
        // Covered by stone or water: dirt. Under glass, a torch, a slab or
        // tall grass: no.
        assert_eq!(covered(CONTENT_GRASS, CONTENT_STONE), Some(CONTENT_DIRT));
        assert_eq!(covered(CONTENT_SNOW, CONTENT_WATER), Some(CONTENT_DIRT));
        assert_eq!(covered(CONTENT_DRY_GRASS, CONTENT_COBBLE), Some(CONTENT_DRY_DIRT));
        for above in [CONTENT_AIR, CONTENT_GLASS, CONTENT_TORCH, crate::farming::GRASS[0], CONTENT_SNOW_LAYER] {
            assert_eq!(covered(CONTENT_GRASS, above), None, "{}", content::name_of(above));
        }
        assert_eq!(covered(CONTENT_DIRT, CONTENT_STONE), None);
        // The 26 neighbours, each once, faces first.
        let mut all = AROUND.to_vec();
        assert_eq!(&all[..6], &[(0, 0, -1), (0, -1, 0), (-1, 0, 0), (1, 0, 0), (0, 1, 0), (0, 0, 1)]);
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 26);
        assert!(!all.contains(&(0, 0, 0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn world(nodes: &[(NodePos, u16, u8)]) -> HashMap<NodePos, (u16, u8)> {
        nodes.iter().map(|&(p, id, p2)| (p, (id, p2))).collect()
    }

    /// An emergent jungle sapling grows the emergent tree, into air only
    /// (its schematic isn't force-placed), from where the sapling stood.
    #[test]
    fn emergent_from_sapling() {
        let node = |p: NodePos| match p.1 {
            y if y < 1 => CONTENT_DIRT,
            1 if p.0 == 0 && p.2 == 0 => EMERGENT_JUNGLE_SAPLING,
            _ => crate::world::CONTENT_AIR,
        };
        let tree = grow((0, 1, 0), EMERGENT_JUNGLE_SAPLING, 1, &node);
        assert!(tree.contains(&((0, 1, 0), CONTENT_JUNGLE_TREE)));
        assert!(tree.iter().all(|&(p, _)| p.1 >= 1), "nothing into the ground");
        let top = tree.iter().map(|&(p, _)| p.1).max().unwrap();
        assert!((21..=33).contains(&top), "top {top}");
        assert!(tree.iter().filter(|&&(_, id)| id == CONTENT_JUNGLE_LEAVES).count() > 60);
    }

    #[test]
    fn leaves_decay_without_their_trunk() {
        let mut w = world(&[
            ((0, 5, 0), CONTENT_BIRCH_TREE, 0),
            ((1, 6, 0), CONTENT_BIRCH_LEAVES, 0),
            ((0, 7, 1), CONTENT_BIRCH_LEAVES, PLACED_LEAVES),
            ((2, 6, 2), CONTENT_FIR_NEEDLES, 0),
        ]);
        fn node(w: &HashMap<NodePos, (u16, u8)>) -> impl Fn(NodePos) -> (u16, u8) + '_ {
            move |p| w.get(&p).copied().unwrap_or((CONTENT_AIR, 0))
        }
        assert!(!should_decay((1, 6, 0), &node(&w)), "trunk still there");
        w.remove(&(0, 5, 0));
        let check = leaves_to_check((0, 5, 0), CONTENT_BIRCH_TREE, &node(&w));
        assert_eq!(check, vec![(1, 6, 0)], "only unplaced birch leaves");
        assert!(should_decay((1, 6, 0), &node(&w)));
        assert!(!should_decay((0, 7, 1), &node(&w)), "placed leaves stay");
        // A fir trunk doesn't keep birch leaves.
        w.insert((1, 7, 0), (CONTENT_FIR_TREE, 0));
        assert!(should_decay((1, 6, 0), &node(&w)));
    }

    #[test]
    fn saplings_grow_into_trees() {
        let node = |p: NodePos| match p.1 {
            0 => CONTENT_GRASS,
            1 if p.0 == 0 && p.2 == 0 => CONTENT_FIR_SAPLING,
            _ => CONTENT_AIR,
        };
        assert!(can_grow((0, 1, 0), &node, true, true));
        assert!(!can_grow((0, 1, 0), &node, true, false), "not at night");
        assert!(!can_grow((0, 1, 0), &node, false, true), "not in shade");
        let tree = grow((0, 1, 0), CONTENT_FIR_SAPLING, 8, &node);
        assert!(tree.contains(&((0, 1, 0), CONTENT_FIR_TREE)), "trunk replaces the sapling");
        assert!(tree.contains(&((0, 8, 0), CONTENT_FIR_TREE)));
        assert!(tree.iter().filter(|(_, id)| *id == CONTENT_FIR_NEEDLES).count() > 30);
    }

    #[test]
    fn jungle_and_acacia_saplings() {
        for (sapling, trunk, leaves, height) in
            [(CONTENT_JUNGLE_SAPLING, CONTENT_JUNGLE_TREE, CONTENT_JUNGLE_LEAVES, 10), (CONTENT_ACACIA_SAPLING, CONTENT_ACACIA_TREE, CONTENT_ACACIA_LEAVES, 5)]
        {
            let node = |p: NodePos| match p.1 {
                0 => CONTENT_LITTER,
                1 if p.0 == 0 && p.2 == 0 => sapling,
                _ => CONTENT_AIR,
            };
            assert!(can_grow((0, 1, 0), &node, true, true));
            let tree = grow((0, 1, 0), sapling, height, &node);
            assert!(tree.contains(&((0, 1, 0), trunk)), "trunk replaces the sapling");
            assert!(tree.iter().filter(|(_, id)| *id == leaves).count() > 20);
            assert!(tree.iter().all(|(_, id)| *id == trunk || *id == leaves));
        }
    }

    #[test]
    fn bushes_and_large_cactus() {
        let ground = |g: u16, sapling: u16| {
            move |p: NodePos| match p.1 {
                0 => g,
                1 if p.0 == 0 && p.2 == 0 => sapling,
                _ => CONTENT_AIR,
            }
        };
        let node = ground(CONTENT_GRASS, plants::BUSH_SAPLING);
        let bush = grow((0, 1, 0), plants::BUSH_SAPLING, 1, &node);
        assert!(bush.contains(&((0, 1, 0), plants::BUSH_STEM)) && bush.contains(&((0, 2, 0), plants::BUSH_LEAVES)));
        assert!(bush.iter().all(|((x, _, z), _)| x.abs() <= 1 && z.abs() <= 1));
        let node = ground(CONTENT_GRASS, plants::BLUEBERRY_SAPLING);
        let berries = grow((0, 1, 0), plants::BLUEBERRY_SAPLING, 1, &node);
        assert!(berries.contains(&((0, 1, 0), plants::BLUEBERRY_LEAVES_WITH_BERRIES)));
        // A large cactus seedling needs sand.
        let node = ground(CONTENT_GRASS, plants::LARGE_CACTUS_SEEDLING);
        assert!(!can_grow((0, 1, 0), &node, true, true));
        let node = ground(CONTENT_SAND, plants::LARGE_CACTUS_SEEDLING);
        assert!(can_grow((0, 1, 0), &node, true, true));
        let cactus = grow((0, 1, 0), plants::LARGE_CACTUS_SEEDLING, 6, &node);
        assert!(cactus.contains(&((0, 1, 0), plants::CACTUS)) && cactus.contains(&((0, 6, 0), plants::CACTUS)));
        assert!(cactus.len() >= 12, "trunk and arms: {}", cactus.len());
    }

    #[test]
    fn apples() {
        let mut rng = Rng::new();
        // Apples decay with their tree's leaves, and drop as apples.
        let w = world(&[((0, 6, 1), CONTENT_APPLE, 0), ((0, 5, 0), CONTENT_APPLE_TREE, 0)]);
        fn node(w: &HashMap<NodePos, (u16, u8)>) -> impl Fn(NodePos) -> (u16, u8) + '_ {
            move |p| w.get(&p).copied().unwrap_or((CONTENT_AIR, 0))
        }
        assert!(!should_decay((0, 6, 1), &node(&w)));
        assert_eq!(leaves_to_check((0, 5, 0), CONTENT_APPLE_TREE, &node(&w)), vec![(0, 6, 1)]);
        assert_eq!(decay_drop(CONTENT_APPLE, &mut rng), Some(CONTENT_APPLE));
        // A mark regrows next to apple leaves, in daylight.
        let leaves = |p: NodePos| if p == (1, 6, 1) { CONTENT_APPLE_LEAVES } else { CONTENT_AIR };
        assert_eq!(regrow((0, 6, 1), &leaves, true), Regrow::Apple);
        assert_eq!(regrow((0, 6, 1), &leaves, false), Regrow::Wait);
        assert_eq!(regrow((5, 6, 1), &leaves, true), Regrow::Gone);
        // An apple sapling grows an apple tree with apples on it.
        let ground = |p: NodePos| match p.1 {
            0 => CONTENT_GRASS,
            1 if p.0 == 0 && p.2 == 0 => CONTENT_APPLE_SAPLING,
            _ => CONTENT_AIR,
        };
        let tree = grow((0, 1, 0), CONTENT_APPLE_SAPLING, 5, &ground);
        assert!(tree.contains(&((0, 5, 0), CONTENT_APPLE_TREE)));
        assert!(tree.iter().filter(|(_, id)| *id == CONTENT_APPLE_LEAVES).count() > 20);
    }

    #[test]
    fn timers_run_in_order_once() {
        let mut t = Timers::new();
        t.start((1, 0, 0), Job::Decay, Duration::ZERO);
        t.start((1, 0, 0), Job::Decay, Duration::ZERO);
        t.start((2, 0, 0), Job::Grow, Duration::from_secs(100));
        assert_eq!(t.pop_due(Instant::now()), Some(((1, 0, 0), Job::Decay)));
        assert_eq!(t.pop_due(Instant::now()), None);
    }
}

// ---------------------------------------------------------------------------
// Grass
// ---------------------------------------------------------------------------

/// minetest_game's "Grass spread" ABM (every 6 s, one time in 50): dirt
/// in daylight next to grass (or forest litter, or snowy dirt) becomes
/// the same; with tall grass or snow on top and none beside, grass or
/// snowy dirt.
pub const GRASS_SPREAD_INTERVAL: Duration = Duration::from_secs(6);
/// "Grass covered" (every 8 s, one time in 50): under something that
/// keeps out the light, or under water, it goes back to dirt.
pub const GRASS_COVERED_INTERVAL: Duration = Duration::from_secs(8);
pub const GRASS_CHANCE: u32 = 50;

/// How often grass spreads or dies (`normal`), or every second with
/// LUANTI_GRASS_FAST set (for the tests).
pub fn grass_interval(normal: Duration) -> Duration {
    if std::env::var_os("LUANTI_GRASS_FAST").is_some() { Duration::from_secs(1) } else { normal }
}

/// One time in how many: GRASS_CHANCE, or every time with LUANTI_GRASS_FAST.
pub fn grass_chance() -> u32 {
    if std::env::var_os("LUANTI_GRASS_FAST").is_some() { 1 } else { GRASS_CHANCE }
}

/// group:spreading_dirt_type: grass, forest litter and snowy dirt.
pub fn spreads(id: u16) -> bool {
    content::group_of(id, "spreading_dirt_type") > 0
}

/// The grass ABM's neighbours a dirt node must have: air, tall grass or
/// savanna grass, or snow.
pub fn lets_grass_in(id: u16) -> bool {
    id == CONTENT_AIR || content::group_of(id, "grass") > 0 || content::group_of(id, "dry_grass") > 0 || id == CONTENT_SNOW_LAYER
}

/// What dirt becomes when grass spreads onto it: the first spreading
/// neighbour's kind; else, by what is on top, snowy dirt under snow or
/// dirt with savanna grass under savanna grass, or grass under tall grass.
pub fn spread_onto(above: u16, beside: Option<u16>) -> Option<u16> {
    if let Some(id) = beside {
        return Some(id);
    }
    if above == CONTENT_SNOW_LAYER {
        Some(CONTENT_SNOW)
    } else if content::group_of(above, "dry_grass") > 0 {
        Some(content::CONTENT_DIRT_DRY_GRASS)
    } else if content::group_of(above, "grass") > 0 {
        Some(CONTENT_GRASS)
    } else {
        None
    }
}

/// What grass (and savanna grass) with `above` on top becomes: dirt, if
/// that keeps out the light or is a liquid ("Grass covered").
pub fn covered(id: u16, above: u16) -> Option<u16> {
    if !(spreads(id) || id == CONTENT_DRY_GRASS) {
        return None;
    }
    if (content::passes_light(above) || content::passes_sunlight(above)) && !content::is_liquid(above) {
        return None;
    }
    Some(if id == CONTENT_DRY_GRASS { CONTENT_DRY_DIRT } else { CONTENT_DIRT })
}

/// The 26 nodes round one, faces first (find_node_near's order).
pub const AROUND: [(i32, i32, i32); 26] = {
    let mut out = [(0, 0, 0); 26];
    let mut n = 0;
    let mut shell = 1;
    while shell <= 3 {
        let mut i = 0;
        while i < 27 {
            let d = (i % 3 - 1, i / 3 % 3 - 1, i / 9 - 1);
            let k = (d.0 != 0) as i32 + (d.1 != 0) as i32 + (d.2 != 0) as i32;
            if k == shell {
                out[n] = d;
                n += 1;
            }
            i += 1;
        }
        shell += 1;
    }
    out
};

// ---------------------------------------------------------------------------
// Moss
// ---------------------------------------------------------------------------

/// minetest_game's "Moss growth" ABM: every MOSS_INTERVAL, cobblestone
/// (and its slab, stairs and wall) with water beside it turns mossy, one
/// time in MOSS_CHANCE.
pub const MOSS_INTERVAL: Duration = Duration::from_secs(16);
pub const MOSS_CHANCE: u32 = 200;

/// How often moss is looked for: MOSS_INTERVAL, or every second with
/// LUANTI_MOSS_FAST set (for the tests).
pub fn moss_interval() -> Duration {
    if std::env::var_os("LUANTI_MOSS_FAST").is_some() { Duration::from_secs(1) } else { MOSS_INTERVAL }
}

/// One time in how many it grows: MOSS_CHANCE, ten times as often in a
/// test build (as saplings and crops grow), every time with
/// LUANTI_MOSS_FAST.
pub fn moss_chance() -> u32 {
    if std::env::var_os("LUANTI_MOSS_FAST").is_some() {
        1
    } else if TEST_BUILD {
        MOSS_CHANCE / 10
    } else {
        MOSS_CHANCE
    }
}

/// What a node turns into when moss grows on it (moss_correspondences).
pub fn mossy_of(id: u16) -> Option<u16> {
    use crate::stairs::{EARLY, MOSSY};
    // Cobblestone is the fifth of the first stair set's materials.
    const COBBLE: u16 = 4;
    Some(match id {
        crate::content::CONTENT_COBBLE => crate::dungeons::MOSSYCOBBLE,
        crate::fences::WALL_COBBLE => crate::dungeons::WALL_MOSSYCOBBLE,
        id if id == EARLY.first_stair() + COBBLE => MOSSY.first_stair(),
        id if id == EARLY.first_slab() + COBBLE => MOSSY.first_slab(),
        id if id == EARLY.first_inner() + COBBLE => MOSSY.first_inner(),
        id if id == EARLY.first_outer() + COBBLE => MOSSY.first_outer(),
        _ => return None,
    })
}

#[cfg(test)]
mod moss_tests {
    use super::*;
    use crate::stairs::{shape_of, Shape};

    #[test]
    fn what_moss_grows_on() {
        assert_eq!(mossy_of(CONTENT_COBBLE), Some(crate::dungeons::MOSSYCOBBLE));
        assert_eq!(mossy_of(crate::fences::WALL_COBBLE), Some(crate::dungeons::WALL_MOSSYCOBBLE));
        for shape in [Shape::Stair, Shape::Slab, Shape::Inner, Shape::Outer] {
            let cobble = crate::stairs::shaped(shape, CONTENT_COBBLE).unwrap();
            let mossy = mossy_of(cobble).unwrap();
            let (s, m) = shape_of(mossy).unwrap();
            assert_eq!((s, m.base), (shape, crate::dungeons::MOSSYCOBBLE));
        }
        assert_eq!(mossy_of(CONTENT_STONE), None);
        assert_eq!(mossy_of(crate::dungeons::MOSSYCOBBLE), None);
    }
}
