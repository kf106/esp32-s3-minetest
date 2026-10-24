//! The world: generated terrain (see terrain.rs) plus the changes players
//! have made, and map block serialisation.
//!
//! A map block is 16x16x16 nodes. Each node has param0 (content id, u16),
//! param1 (light: day level in the low nibble, night level in the high
//! nibble) and param2 (unused here). Format: serialisation version 29, as
//! written by MapBlock::serialize in Luanti's src/mapblock.cpp for the
//! network (no name-id mapping, no timestamps, no static objects).
//!
//! Player changes are kept in memory and appended to a log file on the SD
//! card, one record per change, so they survive a restart:
//!   i32 x, i32 y, i32 z, u8 length, node name [, 0, param2]
//! Names rather than content ids, so the log stays valid if ids change.
//! param2 (which way a torch faces) follows a zero byte when non-zero;
//! records written before torches simply don't have it.
//!
//! Torches light the blocks we send: each has a precomputed light field,
//! spread outward through nodes that let light through, losing one level
//! per node, as Luanti does.
//! Later records override earlier ones. On startup the log is read and, if
//! it has many superseded records, rewritten.
//!
//! The changes only make sense on top of the terrain they were made in, so
//! the generator version is stored alongside (`generator`). A log from a
//! different generator is renamed out of the way and the world starts
//! fresh.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use crate::fasthash::{FastMap, FastSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use log::{info, warn};

use crate::content::{self, TORCH_LIGHT};
use crate::dungeons;
use crate::farming;
use crate::liquids;
use crate::meta;
use crate::messages::Writer;
use crate::storage;
use crate::nature;
use crate::terrain::{self, Column, GENERATOR_VERSION};
use crate::zstd;

pub const BLOCK_SIZE: i32 = 16;
const NODE_COUNT: usize = 16 * 16 * 16;
pub const CONTENT_AIR: u16 = 126;
const LIGHT_SUN: u8 = 15;

/// Terrain cache limits, in entries. Players exploring in different
/// directions fill them, so together they must fit in PSRAM with room to
/// spare: once five bots walking outwards ran it out.
///
/// Block columns are about 4 KB each (256 columns of 12 bytes, and tree
/// tops). The block builder makes the terrain, so it keeps more; the
/// server's own world only looks up nodes near players. When full, the
/// older half goes.
///
/// The PC build (cfg(pc)) has memory to spare, and keeps far more (with
/// the built blocks and dungeons, up to about 60 MB in all), so a player
/// going back and forth finds their surroundings already worked out.
const BUILDER_COLUMNS: usize = if cfg!(pc) { 4096 } else { 384 }; // about 16 MB, or 1.6 MB
const SERVER_COLUMNS: usize = if cfg!(pc) { 1024 } else { 128 }; // about 4 MB, or 0.5 MB
/// Cave noise samples, 20 bytes each. A hash table doubles once it is 7/8
/// full, so this stays under 7/8 of 32768: the table is 0.7 MB, and never
/// grows to the next size (2.75 MB, needed in one piece). On the PC, under
/// 7/8 of 524288 (about 11 MB).
const CAVE_CACHE_MAX: usize = if cfg!(pc) { 458_000 } else { 28_000 };
/// Height ranges of block columns, for deciding which blocks players need
/// (16 bytes each; under 7/8 of 4096, or of 65536 on the PC, for the same
/// reason).
const HEIGHT_CACHE_MAX: usize = if cfg!(pc) { 57_000 } else { 3_500 };
/// Map blocks whose dungeon nodes are kept (most have none).
const DUNGEON_CACHE_MAX: usize = if cfg!(pc) { 65_536 } else { 4_096 };

/// How far torch light reaches: it starts at TORCH_LIGHT and fades by one
/// per node, so nodes further than this get none.
const TORCH_REACH: i32 = TORCH_LIGHT as i32 - 1;
const FIELD_SIDE: i32 = 2 * TORCH_REACH + 1;

pub type BlockPos = (i16, i16, i16);
pub type NodePos = (i32, i32, i32);

/// What grass does to a node (see World::turf): spreads onto dirt, making
/// it this, or is covered, going back to this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turf {
    Spread(u16),
    Covered(u16),
}

/// Block coordinate containing node coordinate `n`.
pub fn block_of(n: i32) -> i16 {
    n.div_euclid(BLOCK_SIZE) as i16
}

pub fn block_of_node(p: NodePos) -> BlockPos {
    (block_of(p.0), block_of(p.1), block_of(p.2))
}

/// The edge of the world, as in Luanti (MAX_MAP_GENERATION_LIMIT): only map
/// blocks within this many nodes of the origin exist. Beyond them the
/// client has nothing, and it treats that as solid: an invisible wall.
pub const MAP_LIMIT: i32 = 31007;

/// Whether a map block is inside the world (blockpos_over_max_limit in
/// Luanti's mapblock.h, which makes the edge at -30992 and +31007).
pub fn block_in_world(p: BlockPos) -> bool {
    let limit = (MAP_LIMIT / BLOCK_SIZE) as i16;
    [p.0, p.1, p.2].iter().all(|v| (-limit..=limit).contains(v))
}

/// The 16x16 columns of a block column, their height range, and the trees
/// growing in or into it.
struct BlockColumn {
    columns: Vec<Column>, // index z * 16 + x
    min_height: i32,
    max_height: i32,
    trees: Vec<terrain::Tree>,
    /// Per column, the highest y a tree could reach there (i32::MIN for
    /// none): above it, and above the surface, there is only air.
    tree_top: Vec<i32>,
    /// The highest of those.
    max_tree_top: i32,
    /// Whether any of the trees is an emergent jungle tree, whose trunk
    /// and roots go into the ground.
    has_emergent: bool,
}

impl BlockColumn {
    /// An emergent jungle tree's trunk or root at or below the surface:
    /// it replaces the ground (minetest_game force-places it).
    fn forced_trunk(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        if !self.has_emergent {
            return None;
        }
        terrain::forced_trunk_in(&self.trees, x, y, z)
    }

    /// The generated node above the surface: a tree's, or air.
    /// A node above the surface (the caller checks that it is): part of a
    /// tree, lake water up to the water level, or air.
    fn above_ground(&self, x: i32, y: i32, z: i32) -> u16 {
        let i = (z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize;
        let open = || {
            let col = &self.columns[i];
            if y <= terrain::WATER_LEVEL {
                terrain::lake_node(y, col)
            } else if y == terrain::WATER_LEVEL + 1 && col.height < terrain::WATER_LEVEL {
                terrain::on_lake(x, z, col).unwrap_or(CONTENT_AIR)
            } else if y == col.height + 1 {
                terrain::plant_on(x, z, col).unwrap_or(CONTENT_AIR)
            } else {
                CONTENT_AIR
            }
        };
        if y > self.tree_top[i] {
            return open();
        }
        terrain::tree_node_in(&self.trees, x, y, z).unwrap_or_else(open)
    }

    /// How high anything generated goes in this column (surface, lake or
    /// tree).
    fn generated_top(&self, x: i32, z: i32) -> i32 {
        let i = (z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize;
        surface_or_lake(&self.columns[i]).max(self.tree_top[i])
    }
}

/// The top of a column's ground, or of the lake over it (and the snow
/// block on the icesheet's frozen lakes).
fn surface_or_lake(c: &Column) -> i32 {
    match c.height.max(terrain::WATER_LEVEL) {
        top if c.biome == terrain::Biome::Icesheet && c.height < terrain::WATER_LEVEL => top + 1,
        top => top,
    }
}

/// How many recent changes are remembered for `changed_near_since`.
const RECENT_CHANGES: usize = 512;

/// What players have done to the world: everything that isn't generated.
/// Shared (copy on write) with the thread that builds map blocks, which
/// works from a snapshot while the server carries on changing the world.
#[derive(Clone, Default)]
pub struct State {
    changes: FastMap<NodePos, u16>,
    /// param2 of changed nodes, where non-zero.
    param2s: FastMap<NodePos, u8>,
    /// Light fields of torches, FIELD_SIDE^3 levels around each.
    torch_light: FastMap<NodePos, Arc<Vec<u8>>>,
    /// Highest y of any solid node players have placed in each column, so
    /// sunlight is blocked by towers and roofs.
    column_top: FastMap<(i32, i32), i32>,
    /// Map blocks containing any changes, so unchanged blocks skip the
    /// per-node lookups.
    changed_blocks: FastSet<BlockPos>,
    /// Node metadata (meta.rs): a sign's text.
    metas: FastMap<NodePos, Arc<meta::Fields>>,
    /// Counts changes; a block built from a snapshot is tagged with it.
    version: u64,
    /// The latest changes and their versions, oldest first.
    recent: VecDeque<(u64, NodePos)>,
}

impl State {
    pub fn version(&self) -> u64 {
        self.version
    }
}

pub struct World {
    state: Arc<State>,
    /// The change log on the SD card; None for a copy that only builds
    /// map blocks.
    log_path: Option<PathBuf>,
    /// Where node metadata is saved (meta.rs), beside the log.
    meta_path: Option<PathBuf>,
    /// Terrain caches (filled on demand; each World is used by one thread).
    /// Block columns carry the order they were made in, so the oldest can go.
    block_columns: RefCell<FastMap<(i16, i16), (u32, Arc<BlockColumn>)>>,
    columns_made: Cell<u32>,
    column_limit: usize,
    heights: RefCell<FastMap<(i16, i16), (i32, i32)>>,
    cave_samples: RefCell<FastMap<(i32, i32, i32), (f32, f32)>>,
    /// The nodes dungeons change in each map block (None: none), as
    /// dungeons::block gives them.
    dungeon_blocks: RefCell<FastMap<BlockPos, Option<Arc<[dungeons::Placed]>>>>,
    /// Places where liquid may flow (liquids.rs).
    liquid_queue: liquids::Queue,
    /// Lava that touches water, and may cool (see lava_touching_water).
    cooling: FastSet<NodePos>,
    /// Cobblestone with water beside it, where moss may grow
    /// (moss_beside_water).
    moss: FastSet<NodePos>,
    /// Dirt grass may spread onto, and grass that may be covered (see
    /// turf).
    turf: FastSet<NodePos>,
    /// Change records not yet appended to the log (see flush_log).
    pending_log: Vec<u8>,
}

/// Change records are appended to the log when this much is waiting, or
/// when the server calls flush_log (once a second).
const LOG_FLUSH_BYTES: usize = 8192;

/// The most queued places the liquids look at each second (Luanti's
/// liquid_loop_max, which is far higher on a PC).
const LIQUID_LOOP_MAX: usize = 1000;

impl World {
    /// Load the world's changes from `dir` (created if missing).
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let log_path = dir.join("nodes.log");
        Self::check_generator(&dir, &log_path)?;
        let mut world = Self {
            state: Arc::new(State::default()),
            log_path: Some(log_path.clone()),
            meta_path: Some(dir.join("meta.txt")),
            block_columns: RefCell::new(FastMap::default()),
            columns_made: Cell::new(0),
            column_limit: SERVER_COLUMNS,
            heights: RefCell::new(FastMap::default()),
            cave_samples: RefCell::new(FastMap::default()),
            dungeon_blocks: RefCell::new(FastMap::default()),
            liquid_queue: liquids::Queue::default(),
            cooling: FastSet::default(),
            moss: FastSet::default(),
            turf: FastSet::default(),
            pending_log: Vec::new(),
        };

        let data = match storage::read(&log_path) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let (records, unknown) = world.load_log(&data);
        info!(
            "World: {} changed nodes from {} log records{}",
            world.state.changes.len(),
            records,
            if unknown > 0 { format!(" ({unknown} with unknown node names skipped)") } else { String::new() }
        );
        // Light up the torches now that the world is complete.
        let torches: Vec<NodePos> = world.state.changes.iter().filter(|(_, &id)| content::light_source(id) > 0).map(|(&p, _)| p).collect();
        for p in torches {
            let field = Arc::new(world.torch_field(p));
            Arc::make_mut(&mut world.state).torch_light.insert(p, field);
        }
        if !world.state.torch_light.is_empty() {
            info!("World: {} torches lit", world.state.torch_light.len());
        }
        // Cobblestone players left beside water, for moss to grow on.
        let cobble: Vec<NodePos> = world.state.changes.iter().filter(|(_, &id)| crate::nature::mossy_of(id).is_some()).map(|(&p, _)| p).collect();
        for p in cobble {
            world.watch_moss(p);
        }
        // Where players changed the ground, for grass to spread or die.
        let changed: Vec<NodePos> = world.state.changes.keys().copied().collect();
        for p in changed {
            world.watch_turf(p);
        }
        world.load_metas();
        if records > 2 * world.state.changes.len() + 256 {
            if let Err(e) = world.compact(&log_path) {
                warn!("Compacting {}: {e}", log_path.display());
            }
        }
        Ok(world)
    }

    /// A world for building map blocks on another thread: no change log,
    /// its own terrain caches, and the changes from `snapshot` (replaced
    /// with `use_snapshot` before each block).
    pub fn builder() -> Self {
        Self {
            state: Arc::new(State::default()),
            log_path: None,
            meta_path: None,
            block_columns: RefCell::new(FastMap::default()),
            columns_made: Cell::new(0),
            column_limit: BUILDER_COLUMNS,
            heights: RefCell::new(FastMap::default()),
            cave_samples: RefCell::new(FastMap::default()),
            dungeon_blocks: RefCell::new(FastMap::default()),
            liquid_queue: liquids::Queue::default(),
            cooling: FastSet::default(),
            moss: FastSet::default(),
            turf: FastSet::default(),
            pending_log: Vec::new(),
        }
    }

    /// The current changes, cheaply shared.
    pub fn snapshot(&self) -> Arc<State> {
        self.state.clone()
    }

    pub fn use_snapshot(&mut self, state: Arc<State>) {
        self.state = state;
    }

    pub fn version(&self) -> u64 {
        self.state.version
    }

    /// Whether a change since `version` could make a copy of block `pos`
    /// built then out of date: a change in or near it (torch light reaches
    /// TORCH_REACH nodes) or above it (shade from a new roof). True if
    /// `version` is too old to tell.
    pub fn changed_near_since(&self, pos: BlockPos, version: u64) -> bool {
        if version == self.state.version {
            return false;
        }
        let oldest = self.state.recent.front().map_or(u64::MAX, |r| r.0);
        if version + 1 < oldest {
            return true; // the changes since then are no longer listed
        }
        let (bx, by, bz) = (pos.0 as i32 * 16, pos.1 as i32 * 16, pos.2 as i32 * 16);
        let near = |v: i32, b: i32| v >= b - TORCH_REACH && v < b + BLOCK_SIZE + TORCH_REACH;
        self.state
            .recent
            .iter()
            .rev()
            .take_while(|(v, _)| *v > version)
            .any(|(_, p)| near(p.0, bx) && near(p.2, bz) && p.1 >= by - TORCH_REACH)
    }

    /// The files beside the change log that describe nodes of the world.
    const WORLD_FILES: [&str; 6] = ["chests.txt", "furnaces.txt", "doors.txt", "beds_spawns", "meta.txt", "items.txt"];

    /// If the change log was made with another terrain generator, rename it
    /// (it would make no sense on new terrain) and record the current one.
    fn check_generator(dir: &std::path::Path, log_path: &std::path::Path) -> io::Result<()> {
        let version_path = dir.join("generator");
        let stored = fs::read_to_string(&version_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(1); // worlds from before versioning
        if stored == GENERATOR_VERSION {
            return Ok(());
        }
        if log_path.exists() {
            let old = dir.join(format!("nodes-v{stored}.log"));
            warn!(
                "World: the terrain generator changed (v{stored} to v{GENERATOR_VERSION}); \
                 keeping the old changes as {} and starting afresh",
                old.display()
            );
            let _ = fs::remove_file(&old);
            fs::rename(log_path, &old)?;
            // What was kept about nodes of the old terrain goes with it:
            // chests' and furnaces' contents, door owners, bed spawns,
            // node metadata and dropped items would turn up in odd places
            // (a new chest would hold an old one's things). Players keep
            // their inventories; one inside the new ground is moved up.
            for name in Self::WORLD_FILES {
                let path = dir.join(name);
                if path.exists() {
                    let kept = dir.join(format!("{name}.v{stored}"));
                    let _ = fs::remove_file(&kept);
                    fs::rename(&path, &kept)?;
                }
            }
        }
        storage::write(&version_path, GENERATOR_VERSION.to_string().as_bytes())
    }

    fn load_log(&mut self, mut data: &[u8]) -> (usize, usize) {
        // Loading isn't "changing": no versions or recent list.
        let (mut records, mut unknown) = (0, 0);
        while data.len() >= 13 {
            let i = |o: usize| i32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
            let pos = (i(0), i(4), i(8));
            let len = data[12] as usize;
            let Some(field) = data.get(13..13 + len) else {
                break; // truncated by a power cut mid-write
            };
            let (name, param2) = match field.iter().position(|&b| b == 0) {
                Some(i) => (&field[..i], field.get(i + 1).copied().unwrap_or(0)),
                None => (field, 0),
            };
            match content::node_id_of(&String::from_utf8_lossy(name)) {
                // Torches saved before there were wall and ceiling torch
                // nodes get the right one for the way they face.
                Some(id) => self.apply(pos, content::torch_variant(id, param2), param2),
                None => unknown += 1,
            }
            records += 1;
            data = &data[13 + len..];
        }
        if !data.is_empty() {
            warn!("World log ends with {} bytes of a partial record; ignoring them", data.len());
        }
        (records, unknown)
    }

    /// Rewrite the log with one record per changed node.
    fn compact(&self, log_path: &std::path::Path) -> io::Result<()> {
        let mut out = Vec::new();
        for (&pos, &id) in &self.state.changes {
            out.extend_from_slice(&record(pos, id, self.param2(pos)));
        }
        storage::write(log_path, &out)?;
        info!("World log compacted to {} records", self.state.changes.len());
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Terrain with caches
    // -----------------------------------------------------------------------

    fn block_column(&self, bx: i16, bz: i16) -> Arc<BlockColumn> {
        if let Some((_, c)) = self.block_columns.borrow().get(&(bx, bz)) {
            return c.clone();
        }
        let mut columns = Vec::with_capacity(256);
        let (x0, z0) = (bx as i32 * 16, bz as i32 * 16);
        for z in 0..16 {
            for x in 0..16 {
                columns.push(terrain::column(x0 + x, z0 + z));
            }
        }
        let min_height = columns.iter().map(|c| c.height).min().unwrap_or(0);
        let max_height = columns.iter().map(surface_or_lake).max().unwrap_or(0);
        let trees = terrain::trees_in(x0, z0, x0 + 15, z0 + 15);
        let mut tree_top = vec![i32::MIN; 256];
        for t in &trees {
            for z in (t.z - terrain::TREE_REACH).max(z0)..=(t.z + terrain::TREE_REACH).min(z0 + 15) {
                for x in (t.x - terrain::TREE_REACH).max(x0)..=(t.x + terrain::TREE_REACH).min(x0 + 15) {
                    let top = &mut tree_top[((z - z0) * 16 + x - x0) as usize];
                    *top = (*top).max(t.top());
                }
            }
        }
        let max_tree_top = tree_top.iter().copied().max().unwrap_or(i32::MIN);
        let has_emergent = trees.iter().any(|t| t.kind == terrain::TreeKind::Emergent);
        let bc = Arc::new(BlockColumn { columns, min_height, max_height, trees, tree_top, max_tree_top, has_emergent });
        let mut cache = self.block_columns.borrow_mut();
        let made = self.columns_made.get().wrapping_add(1);
        self.columns_made.set(made);
        if cache.len() >= self.column_limit {
            // Keep the newer half: nearby columns are wanted again soon.
            let keep = (self.column_limit / 2) as u32;
            cache.retain(|_, (when, _)| made.wrapping_sub(*when) <= keep);
        }
        cache.insert((bx, bz), (made, bc.clone()));
        bc
    }

    fn column(&self, x: i32, z: i32) -> Column {
        let bc = self.block_column(block_of(x), block_of(z));
        bc.columns[(z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize]
    }

    /// Lowest generated surface and highest generated node (surface or
    /// tree) in a block column, for deciding which blocks to send.
    pub fn height_range(&self, bx: i16, bz: i16) -> (i32, i32) {
        if let Some(&h) = self.heights.borrow().get(&(bx, bz)) {
            return h;
        }
        let bc = self.block_column(bx, bz);
        let h = (bc.min_height, bc.max_height.max(bc.max_tree_top));
        let mut heights = self.heights.borrow_mut();
        if heights.len() >= HEIGHT_CACHE_MAX {
            heights.clear();
        }
        heights.insert((bx, bz), h);
        h
    }

    /// The generated node: the dungeons', or the terrain's.
    fn generated(&self, x: i32, y: i32, z: i32, col: &Column) -> u16 {
        match self.dungeon_node((x, y, z)) {
            Some(d) => d.id,
            None => self.terrain_node(x, y, z, col),
        }
    }

    /// The param2 of a generated node: a dungeon's stair or chest is
    /// turned; kelp has its height.
    fn generated_param2(&self, pos: NodePos, id: u16) -> u8 {
        match self.dungeon_node(pos) {
            Some(d) => d.param2,
            None => terrain::generated_param2(pos.0, pos.1, pos.2, id),
        }
    }

    /// The nodes dungeons change in a map block, sorted (dungeons::block).
    fn dungeon_block(&self, b: BlockPos) -> Option<Arc<[dungeons::Placed]>> {
        if let Some(found) = self.dungeon_blocks.borrow().get(&b) {
            return found.clone();
        }
        // The dungeons are dug into the terrain without them, and without
        // trees (which they leave alone, as anything above the ground).
        let mut terrain = |p: NodePos| {
            let col = self.column(p.0, p.2);
            if p.1 > col.height {
                if p.1 <= terrain::WATER_LEVEL {
                    terrain::lake_node(p.1, &col)
                } else {
                    CONTENT_AIR
                }
            } else {
                self.terrain_node(p.0, p.1, p.2, &col)
            }
        };
        let placed = dungeons::block(b, &mut terrain);
        let found: Option<Arc<[dungeons::Placed]>> = (!placed.is_empty()).then(|| placed.into());
        let mut cache = self.dungeon_blocks.borrow_mut();
        if cache.len() >= DUNGEON_CACHE_MAX {
            cache.clear();
        }
        cache.insert(b, found.clone());
        found
    }

    fn dungeon_node(&self, p: NodePos) -> Option<dungeons::Placed> {
        dungeons::find(&self.dungeon_block(block_of_node(p))?, p)
    }

    /// The terrain's node, without dungeons.
    fn terrain_node(&self, x: i32, y: i32, z: i32, col: &Column) -> u16 {
        // Above the surface it's air or trees: no need to look at caves.
        if y > col.height {
            let id = self.block_column(block_of(x), block_of(z)).above_ground(x, y, z);
            return self.unless_unsupported(id, x, y, z, col);
        }
        if let Some(trunk) = self.block_column(block_of(x), block_of(z)).forced_trunk(x, y, z) {
            return trunk;
        }
        let mut cache = self.cave_samples.borrow_mut();
        if cache.len() >= CAVE_CACHE_MAX {
            cache.clear();
        }
        terrain::node(x, y, z, col, &mut |gx, gy, gz| {
            *cache.entry((gx, gy, gz)).or_insert_with(|| terrain::cave_sample(gx, gy, gz))
        })
    }

    // -----------------------------------------------------------------------
    // Nodes
    // -----------------------------------------------------------------------

    fn apply(&mut self, pos: NodePos, id: u16, param2: u8) {
        let col = self.column(pos.0, pos.2);
        let generated = self.generated(pos.0, pos.1, pos.2, &col);
        let generated_param2 = self.generated_param2(pos, generated);
        let st = Arc::make_mut(&mut self.state);
        // Kept even if the change is undone; it only costs some lookups.
        st.changed_blocks.insert(block_of_node(pos));
        if id == generated && param2 == generated_param2 {
            st.changes.remove(&pos);
        } else {
            st.changes.insert(pos, id);
        }
        if param2 == 0 {
            st.param2s.remove(&pos);
        } else {
            st.param2s.insert(pos, param2);
        }
        if !content::passes_sunlight(id) {
            let top = st.column_top.entry((pos.0, pos.2)).or_insert(pos.1);
            *top = (*top).max(pos.1);
        }
    }

    /// A node's param2: as a player left it, or as generated (only kelp
    /// has one, its height).
    pub fn param2(&self, pos: NodePos) -> u8 {
        if let Some(&p) = self.state.param2s.get(&pos) {
            return p;
        }
        if self.state.changes.contains_key(&pos) {
            return 0;
        }
        if let Some(d) = self.dungeon_node(pos) {
            return d.param2;
        }
        let col = self.column(pos.0, pos.2);
        // A dry shrub on the ground has its own way of being drawn.
        if pos.1 == col.height + 1 {
            let id = self.terrain_node(pos.0, pos.1, pos.2, &col);
            return terrain::generated_param2(pos.0, pos.1, pos.2, id);
        }
        // Kelp grows only on lake beds, deep down: most positions can't be
        // kelp, and are answered without generating anything.
        if pos.1 > terrain::KELP_TOP || pos.1 != col.height {
            return 0;
        }
        terrain::generated_param2(pos.0, pos.1, pos.2, self.terrain_node(pos.0, pos.1, pos.2, &col))
    }

    /// What a player put (or what flowed) at a node, if anything has
    /// changed there: a quick look that generates nothing.
    pub fn changed_node(&self, pos: NodePos) -> Option<u16> {
        self.state.changes.get(&pos).copied()
    }

    pub fn node(&self, pos: NodePos) -> u16 {
        let col = self.column(pos.0, pos.2);
        self.node_with(pos.0, pos.1, pos.2, &col)
    }

    fn node_with(&self, x: i32, y: i32, z: i32, col: &Column) -> u16 {
        if let Some(&id) = self.state.changes.get(&(x, y, z)) {
            return id;
        }
        self.generated(x, y, z, col)
    }

    // -----------------------------------------------------------------------
    // Node metadata
    // -----------------------------------------------------------------------

    /// A node's metadata, if it has any.
    pub fn meta(&self, pos: NodePos) -> Option<Arc<meta::Fields>> {
        self.state.metas.get(&pos).cloned()
    }

    /// Set a node's metadata (None or no values: remove it), and save it.
    /// Blocks built from here on carry it.
    pub fn set_meta(&mut self, pos: NodePos, fields: Option<meta::Fields>) {
        let st = Arc::make_mut(&mut self.state);
        match fields.filter(|f| !f.is_empty()) {
            Some(f) => st.metas.insert(pos, Arc::new(f)),
            None => st.metas.remove(&pos),
        };
        st.version += 1;
        let version = st.version;
        st.recent.push_back((version, pos));
        if st.recent.len() > RECENT_CHANGES {
            st.recent.pop_front();
        }
        self.save_metas();
    }

    fn save_metas(&self) {
        let Some(path) = &self.meta_path else {
            return;
        };
        let mut all: Vec<(NodePos, &meta::Fields)> = self.state.metas.iter().map(|(p, f)| (*p, &**f)).collect();
        all.sort_by_key(|(p, _)| *p);
        if let Err(e) = storage::write(path, meta::format(&all).as_bytes()) {
            warn!("Saving {}: {e}", path.display());
        }
    }

    /// Read the saved metadata, keeping only that of nodes that still take
    /// it (a world started afresh has lost its signs).
    fn load_metas(&mut self) {
        let Some(path) = self.meta_path.clone() else {
            return;
        };
        let Ok(data) = storage::read(&path) else {
            return;
        };
        let mut kept = 0;
        for (pos, fields) in meta::parse(&String::from_utf8_lossy(&data)) {
            if content::takes_meta(self.node(pos)) {
                Arc::make_mut(&mut self.state).metas.insert(pos, Arc::new(fields));
                kept += 1;
            }
        }
        if kept > 0 {
            info!("World: metadata for {kept} nodes");
        }
    }

    /// The metadata in a map block.
    fn metas_in_block(&self, b: BlockPos) -> Vec<(NodePos, &meta::Fields)> {
        let mut found: Vec<(NodePos, &meta::Fields)> =
            self.state.metas.iter().filter(|(p, _)| block_of_node(**p) == b).map(|(p, f)| (*p, &**f)).collect();
        found.sort_by_key(|(p, _)| *p);
        found
    }

    /// Whether a dungeon's loot chest stands at `pos`, as generated.
    pub fn is_dungeon_chest(&self, pos: NodePos) -> bool {
        !self.state.changes.contains_key(&pos) && self.dungeon_node(pos).is_some_and(|d| d.id == content::CONTENT_CHEST)
    }

    /// Nodes players have put in the world that match `wanted` (saplings,
    /// say, which need their timers again after a restart).
    pub fn changed_nodes(&self, wanted: impl Fn(u16) -> bool) -> Vec<NodePos> {
        self.state.changes.iter().filter(|(_, &id)| wanted(id)).map(|(&p, _)| p).collect()
    }

    /// Change a node and record it on the SD card. Liquid beside it may
    /// now flow (or stop).
    pub fn set_node(&mut self, pos: NodePos, id: u16, param2: u8) {
        self.change(pos, id, param2);
        self.append_log(&record(pos, id, param2));
        let mut queue = std::mem::take(&mut self.liquid_queue);
        liquids::after_change(pos, |p| self.node(p), &mut queue);
        self.liquid_queue = queue;
        self.watch_cooling(pos);
        self.watch_moss(pos);
        self.watch_turf(pos);
    }

    /// After a change at `pos`: the dirt there or under it that grass may
    /// now spread onto, and grass there or under it that may now be
    /// covered, are noted (minetest_game's "Grass spread" and "Grass
    /// covered" ABMs, which look at all dirt and grass near players; here
    /// only where something changed). Grass that appeared notes the dirt
    /// round it.
    fn watch_turf(&mut self, pos: NodePos) {
        let below = (pos.0, pos.1 - 1, pos.2);
        for p in [pos, below] {
            if self.turf_change(p).is_some() {
                self.turf.insert(p);
            }
        }
        if nature::spreads(self.node(pos)) {
            for d in nature::AROUND {
                let p = (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2);
                if self.turf_change(p).is_some() {
                    self.turf.insert(p);
                }
            }
        }
    }

    /// What a node would become by grass spreading onto it or being
    /// covered, light aside: Spread for dirt with a way in for grass
    /// (one of the ABM's neighbours) and grass beside or tall grass or
    /// snow on top, Covered for grass under something.
    fn turf_change(&self, p: NodePos) -> Option<Turf> {
        let id = self.node(p);
        let above = self.node((p.0, p.1 + 1, p.2));
        if id == content::CONTENT_DIRT {
            let around: Vec<u16> = nature::AROUND.iter().map(|d| self.node((p.0 + d.0, p.1 + d.1, p.2 + d.2))).collect();
            if !around.iter().any(|&n| nature::lets_grass_in(n)) {
                return None;
            }
            let beside = around.iter().copied().find(|&n| nature::spreads(n));
            return nature::spread_onto(above, beside).map(Turf::Spread);
        }
        nature::covered(id, above).map(Turf::Covered)
    }

    /// The noted dirt and grass that would still change, and how; the
    /// rest is forgotten.
    pub fn turf(&mut self) -> Vec<(NodePos, Turf)> {
        let noted: Vec<NodePos> = self.turf.iter().copied().collect();
        let mut out = Vec::new();
        for p in noted {
            match self.turf_change(p) {
                Some(t) => out.push((p, t)),
                None => {
                    self.turf.remove(&p);
                }
            }
        }
        out
    }

    /// After a change at `pos`: lava there or beside it that now touches
    /// water is noted, to cool (minetest_game's lava cooling ABM, which
    /// looks for lava with water next to it).
    fn watch_cooling(&mut self, pos: NodePos) {
        for d in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let p = (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2);
            if content::is_lava(self.node(p)) && self.touches_water(p) {
                self.cooling.insert(p);
            }
        }
    }

    /// After a change at `pos`: cobblestone there or beside it that now
    /// has water beside it is noted, for moss to grow on (minetest_game's
    /// moss growth ABM, on cobblestone with group:water as a neighbour).
    fn watch_moss(&mut self, pos: NodePos) {
        for d in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let p = (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2);
            if crate::nature::mossy_of(self.node(p)).is_some() && self.beside_water(p) {
                self.moss.insert(p);
            }
        }
    }

    fn beside_water(&self, p: NodePos) -> bool {
        [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
            .iter()
            .any(|d| content::group_of(self.node((p.0 + d.0, p.1 + d.1, p.2 + d.2)), "water") > 0)
    }

    /// Cobblestone that had water beside it when noted and still does. The
    /// rest is forgotten.
    pub fn moss_beside_water(&mut self) -> Vec<NodePos> {
        let noted: Vec<NodePos> = self.moss.iter().copied().collect();
        for p in noted {
            if !(crate::nature::mossy_of(self.node(p)).is_some() && self.beside_water(p)) {
                self.moss.remove(&p);
            }
        }
        self.moss.iter().copied().collect()
    }

    fn touches_water(&self, p: NodePos) -> bool {
        [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
            .iter()
            .any(|d| content::group_of(self.node((p.0 + d.0, p.1 + d.1, p.2 + d.2)), "cools_lava") > 0)
    }

    /// Lava that touched water when it was noted and still does. The
    /// rest is forgotten.
    pub fn lava_touching_water(&mut self) -> Vec<NodePos> {
        let noted: Vec<NodePos> = self.cooling.iter().copied().collect();
        for p in noted {
            if !(content::is_lava(self.node(p)) && self.touches_water(p)) {
                self.cooling.remove(&p);
            }
        }
        self.cooling.iter().copied().collect()
    }

    /// Change a node in memory: light, and the version clients' blocks are
    /// checked against.
    fn change(&mut self, pos: NodePos, id: u16, param2: u8) {
        let was = self.node(pos);
        let was_torch = content::light_source(was) > 0;
        // A different node loses the old one's metadata (Luanti's
        // set_node; swapping a node for itself keeps it).
        if id != was && self.state.metas.contains_key(&pos) {
            Arc::make_mut(&mut self.state).metas.remove(&pos);
            self.save_metas();
        }
        self.apply(pos, id, param2);
        self.relight(pos, was_torch, content::light_source(id) > 0);
        let st = Arc::make_mut(&mut self.state);
        st.version += 1;
        let version = st.version;
        st.recent.push_back((version, pos));
        if st.recent.len() > RECENT_CHANGES {
            st.recent.pop_front();
        }
    }

    /// Keep change records to be appended to the log. Opening and
    /// appending to a file on the SD card takes long enough that doing it
    /// for every dig held the server up; now it is done at most once a
    /// second (or a power cut loses at most the last second of changes).
    fn append_log(&mut self, records: &[u8]) {
        if self.log_path.is_none() {
            return;
        }
        self.pending_log.extend_from_slice(records);
        if self.pending_log.len() >= LOG_FLUSH_BYTES {
            self.flush_log();
        }
    }

    /// Append the waiting change records to the log on the SD card.
    pub fn flush_log(&mut self) {
        if self.pending_log.is_empty() {
            return;
        }
        let records = std::mem::take(&mut self.pending_log);
        if let Some(log_path) = &self.log_path {
            let result = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
                .and_then(|mut f: File| f.write_all(&records));
            if let Err(e) = result {
                warn!("Saving node changes to {}: {e}", log_path.display());
                crate::status::problem();
            }
        }
    }

    /// Let the liquids flow a step: look at what is queued now, up to
    /// LIQUID_LOOP_MAX places, and record what changed in one write.
    pub fn flow_liquids(&mut self) -> Vec<liquids::Change> {
        let max = self.liquid_queue.len().min(LIQUID_LOOP_MAX);
        if max == 0 {
            return Vec::new();
        }
        let mut queue = std::mem::take(&mut self.liquid_queue);
        let changes = liquids::transform(&mut LiquidView(self), &mut queue, max);
        self.liquid_queue = queue;
        let mut log = Vec::new();
        for c in &changes {
            log.extend_from_slice(&record(c.pos, c.id, c.param2));
            if [c.id, c.was].iter().any(|&id| content::is_lava(id) || farming::is_water(id)) {
                self.watch_cooling(c.pos);
            }
            if farming::is_water(c.id) {
                self.watch_moss(c.pos);
            }
            // Grass under water dies.
            if content::is_liquid(c.id) {
                let below = (c.pos.0, c.pos.1 - 1, c.pos.2);
                if matches!(self.turf_change(below), Some(Turf::Covered(_))) {
                    self.turf.insert(below);
                }
            }
        }
        self.append_log(&log);
        changes
    }

    /// How many places are waiting for the liquids.
    pub fn liquid_queue_len(&self) -> usize {
        self.liquid_queue.len()
    }

    /// Keep torch light up to date after a node changed: the torch there
    /// (if any) and any torch whose light could reach it.
    fn relight(&mut self, pos: NodePos, was_torch: bool, is_torch: bool) {
        if was_torch && !is_torch {
            Arc::make_mut(&mut self.state).torch_light.remove(&pos);
        }
        let near = |t: &NodePos| {
            (t.0 - pos.0).abs() <= TORCH_REACH && (t.1 - pos.1).abs() <= TORCH_REACH && (t.2 - pos.2).abs() <= TORCH_REACH
        };
        let mut affected: Vec<NodePos> = self.state.torch_light.keys().filter(|t| near(t)).copied().collect();
        if is_torch && !affected.contains(&pos) {
            affected.push(pos);
        }
        for t in affected {
            let field = Arc::new(self.torch_field(t));
            Arc::make_mut(&mut self.state).torch_light.insert(t, field);
        }
    }

    /// Light levels around a torch: TORCH_LIGHT at the torch, one less per
    /// node outward through nodes that let light through.
    fn torch_field(&self, t: NodePos) -> Vec<u8> {
        let side = FIELD_SIDE as usize;
        let mut field = vec![0u8; side * side * side];
        let index = |d: (i32, i32, i32)| {
            ((d.2 + TORCH_REACH) as usize * side + (d.1 + TORCH_REACH) as usize) * side + (d.0 + TORCH_REACH) as usize
        };
        // Torches, and anything else that gives light (a burning furnace).
        let source = content::light_source(self.node(t)).min(TORCH_LIGHT);
        field[index((0, 0, 0))] = source;
        let mut queue: VecDeque<((i32, i32, i32), u8)> = VecDeque::from([((0, 0, 0), source)]);
        while let Some((d, l)) = queue.pop_front() {
            if l <= 1 {
                continue;
            }
            for (sx, sy, sz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let n = (d.0 + sx, d.1 + sy, d.2 + sz);
                if n.0.abs() > TORCH_REACH || n.1.abs() > TORCH_REACH || n.2.abs() > TORCH_REACH {
                    continue;
                }
                let i = index(n);
                if field[i] >= l - 1 {
                    continue;
                }
                if !content::passes_light(self.node((t.0 + n.0, t.1 + n.1, t.2 + n.2))) {
                    continue;
                }
                field[i] = l - 1;
                queue.push_back((n, l - 1));
            }
        }
        field
    }

    /// A generated plant needs the ground it grows on: where a player dug
    /// that out (in a world from before plants, or just now), there is
    /// only air.
    fn unless_unsupported(&self, id: u16, x: i32, y: i32, z: i32, col: &Column) -> u16 {
        let dug_out = || self.state.changes.get(&(x, col.height, z)).is_some_and(|&g| !content::walkable(g));
        if y == col.height + 1 && farming::is_attached_plant(id) && dug_out() {
            CONTENT_AIR
        } else {
            id
        }
    }

    /// The light torches (and other lights) give a node: what lights it at
    /// night, when there is no sunlight.
    pub fn torch_light_at(&self, p: NodePos) -> u8 {
        let side = FIELD_SIDE;
        self.state
            .torch_light
            .iter()
            .filter_map(|(t, field)| {
                let d = (p.0 - t.0, p.1 - t.1, p.2 - t.2);
                if d.0.abs() > TORCH_REACH || d.1.abs() > TORCH_REACH || d.2.abs() > TORCH_REACH {
                    return None;
                }
                Some(field[(((d.2 + TORCH_REACH) * side + d.1 + TORCH_REACH) * side + d.0 + TORCH_REACH) as usize])
            })
            .max()
            .unwrap_or(0)
    }

    /// The y of the highest solid node in a column (leaves included): below
    /// it is out of direct sunlight, and on it is where a player can stand.
    pub fn highest_solid(&self, x: i32, z: i32) -> i32 {
        let col = self.column(x, z);
        self.highest_solid_with(x, z, &col)
    }

    fn highest_solid_with(&self, x: i32, z: i32, col: &Column) -> i32 {
        let generated = self.block_column(block_of(x), block_of(z)).generated_top(x, z);
        let start = generated.max(self.state.column_top.get(&(x, z)).copied().unwrap_or(i32::MIN));
        let mut y = start;
        // Scan down past dug-out nodes and cave mouths. A very deep shaft is
        // rare; stop after a while rather than scan forever.
        while content::passes_sunlight(self.node_with(x, y, z, col)) && y > start - 256 {
            y -= 1;
        }
        y
    }

    /// The uncompressed version 29 block data.
    pub fn block_raw(&self, pos: BlockPos) -> Vec<u8> {
        let (bx, by, bz) = (pos.0 as i32 * 16, pos.1 as i32 * 16, pos.2 as i32 * 16);
        let idx = |x: i32, y: i32, z: i32| (z * 256 + y * 16 + x) as usize;
        let mut param0 = vec![0u16; NODE_COUNT];
        let mut light = vec![0u8; NODE_COUNT];
        let mut param2 = vec![0u8; NODE_COUNT];
        let mut any_air = false;
        let mut any_solid = false;
        let mut lit = VecDeque::new();
        let mut any_shaded = false;
        let bc = self.block_column(pos.0, pos.2);
        let changed = self.state.changed_blocks.contains(&pos);
        let dungeon = self.dungeon_block(pos);
        // Cave samples for the 5x5x5 grid points around this block, filled
        // on first use. Same values as the shared cache, just quicker to
        // reach than through a hash map.
        let (g0x, g0y, g0z) = (pos.0 as i32 * 4, pos.1 as i32 * 4, pos.2 as i32 * 4);
        let mut grid = [(0.0f32, 0.0f32); 125];
        let mut grid_done = [false; 125];
        let mut local = |gx: i32, gy: i32, gz: i32| {
            let i = ((gz - g0z) * 25 + (gy - g0y) * 5 + (gx - g0x)) as usize;
            if !grid_done[i] {
                grid[i] = terrain::cave_sample(gx, gy, gz);
                grid_done[i] = true;
            }
            grid[i]
        };
        // Corner samples per 4x4x4 cell of the block, gathered once.
        let mut cells: [Option<[(f32, f32); 8]>; 64] = [None; 64];

        for z in 0..BLOCK_SIZE {
            for x in 0..BLOCK_SIZE {
                let col = bc.columns[(z * 16 + x) as usize];
                // Where sunlight stops. A block wholly above every possible
                // solid node needs no scan.
                let top = bc
                    .generated_top(bx + x, bz + z)
                    .max(self.state.column_top.get(&(bx + x, bz + z)).copied().unwrap_or(i32::MIN));
                let floor = if top < by { top } else { self.highest_solid_with(bx + x, bz + z, &col) };
                for y in 0..BLOCK_SIZE {
                    let (wx, wy, wz) = (bx + x, by + y, bz + z);
                    // Look for a player's change only in blocks that have any.
                    let change = if changed { self.state.changes.get(&(wx, wy, wz)) } else { None };
                    let dug = match (&dungeon, change) {
                        (Some(list), None) => dungeons::find(list, (wx, wy, wz)),
                        _ => None,
                    };
                    let id = match (change, dug) {
                        (Some(&id), _) => id,
                        (_, Some(d)) => d.id,
                        _ if wy > col.height => self.unless_unsupported(bc.above_ground(wx, wy, wz), wx, wy, wz, &col),
                        _ if bc.has_emergent && bc.forced_trunk(wx, wy, wz).is_some() => bc.forced_trunk(wx, wy, wz).unwrap_or(CONTENT_AIR),
                        _ => {
                            let cell = ((z / 4) * 16 + (y / 4) * 4 + x / 4) as usize;
                            let corners = *cells[cell]
                                .get_or_insert_with(|| terrain::corners_at(wx, wy, wz, &mut local));
                            terrain::finish(wx, wy, wz, &col, || {
                                terrain::cave_from_corners(&corners, wx, wy, wz)
                            })
                        }
                    };
                    let i = idx(x, y, z);
                    param0[i] = id;
                    param2[i] = match (change, dug) {
                        (Some(_), _) => self.param2((wx, wy, wz)),
                        (_, Some(d)) => d.param2,
                        _ => terrain::generated_param2(wx, wy, wz, id),
                    };
                    if content::passes_light(id) {
                        any_air |= id == CONTENT_AIR;
                        any_solid |= id != CONTENT_AIR; // flag 0x02 means "not all air"
                        if wy > floor {
                            light[i] = LIGHT_SUN;
                            lit.push_back((x, y, z));
                        } else {
                            any_shaded = true;
                        }
                    } else {
                        any_solid = true;
                    }
                }
            }
        }

        // Spread light sideways and down into shaded air, one level dimmer
        // per node, as Luanti does. Only within this block: light from
        // neighbouring blocks isn't considered.
        if !any_shaded {
            lit.clear(); // nothing in the shade, so nothing to spread into
        }
        while let Some((x, y, z)) = lit.pop_front() {
            let l = light[idx(x, y, z)];
            if l <= 1 {
                continue;
            }
            for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                if !(0..16).contains(&nx) || !(0..16).contains(&ny) || !(0..16).contains(&nz) {
                    continue;
                }
                let j = idx(nx, ny, nz);
                if content::passes_light(param0[j]) && light[j] < l - 1 {
                    light[j] = l - 1;
                    lit.push_back((nx, ny, nz));
                }
            }
        }

        // Torch light: in the day bank where it beats sunlight, and alone in
        // the night bank (sunlight has none at night).
        let mut night = vec![0u8; NODE_COUNT];
        let in_block = |t: &&NodePos| {
            let reach = |t: i32, b: i32| t + TORCH_REACH >= b && t - TORCH_REACH < b + BLOCK_SIZE;
            reach(t.0, bx) && reach(t.1, by) && reach(t.2, bz)
        };
        for (t, field) in self.state.torch_light.iter().filter(|(t, _)| in_block(t)) {
            let side = FIELD_SIDE;
            for z in 0..BLOCK_SIZE {
                let dz = bz + z - t.2;
                if dz.abs() > TORCH_REACH {
                    continue;
                }
                for y in 0..BLOCK_SIZE {
                    let dy = by + y - t.1;
                    if dy.abs() > TORCH_REACH {
                        continue;
                    }
                    for x in 0..BLOCK_SIZE {
                        let dx = bx + x - t.0;
                        if dx.abs() > TORCH_REACH {
                            continue;
                        }
                        let l = field[(((dz + TORCH_REACH) * side + dy + TORCH_REACH) * side + dx + TORCH_REACH) as usize];
                        let i = idx(x, y, z);
                        light[i] = light[i].max(l);
                        night[i] = night[i].max(l);
                    }
                }
            }
        }
        for (l, n) in light.iter_mut().zip(&night) {
            *l |= n << 4;
        }

        let mut flags = 0u8;
        if !any_air {
            flags |= 0x01; // is_underground
        }
        if any_solid {
            flags |= 0x02; // not entirely air
        }
        // 0x08 would mean "not generated"; ours are.

        let mut w = Writer::with_capacity(5 + NODE_COUNT * 4 + 1);
        w.u8(flags)
            .u16(0xFFFF) // lighting complete in all directions, day and night
            .u8(2) // content width
            .u8(2); // params width
        for id in &param0 {
            w.u16(*id);
        }
        // param1: day light in the low nibble, night light in the high
        w.raw(&light);
        w.raw(&param2);
        let metas = if self.state.metas.is_empty() { Vec::new() } else { self.metas_in_block(pos) };
        meta::write_block_list(&mut w, &metas);
        w.into_inner()
    }

    /// The payload after the command id of TOCLIENT_BLOCKDATA: position,
    /// zstd-compressed block, then the network-specific part (version 2).
    pub fn block_payload(&self, pos: BlockPos) -> Vec<u8> {
        let compressed = zstd::compress(&self.block_raw(pos));
        let mut w = Writer::plain();
        w.i16(pos.0).i16(pos.1).i16(pos.2);
        w.raw(&compressed);
        w.u8(2);
        w.into_inner()
    }
}

/// Nothing waiting is lost when a world is closed.
impl Drop for World {
    fn drop(&mut self) {
        self.flush_log();
    }
}

/// The world as the liquids see and change it.
struct LiquidView<'a>(&'a mut World);

impl liquids::Nodes for LiquidView<'_> {
    fn get(&self, pos: NodePos) -> (u16, u8) {
        (self.0.node(pos), self.0.param2(pos))
    }
    fn set(&mut self, pos: NodePos, id: u16, param2: u8) {
        self.0.change(pos, id, param2);
    }
}

fn record(pos: NodePos, id: u16, param2: u8) -> Vec<u8> {
    let name = content::name_of(id);
    let mut field = name.as_bytes().to_vec();
    if param2 != 0 {
        field.extend_from_slice(&[0, param2]);
    }
    let mut r = Vec::with_capacity(13 + field.len());
    r.extend_from_slice(&pos.0.to_be_bytes());
    r.extend_from_slice(&pos.1.to_be_bytes());
    r.extend_from_slice(&pos.2.to_be_bytes());
    r.push(field.len() as u8);
    r.extend_from_slice(&field);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world made by an older terrain generator: its changes, and what
    /// was kept about its nodes, are set aside; players' files stay.
    #[test]
    fn new_generator_sets_the_old_world_aside() {
        let dir = std::env::temp_dir().join(format!("esp-gen-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("generator"), "4").unwrap();
        fs::write(dir.join("nodes.log"), b"").unwrap();
        fs::write(dir.join("chests.txt"), "chest 1 2 3\n").unwrap();
        fs::write(dir.join("doors.txt"), "1 2 3 Ann\n").unwrap();
        fs::write(dir.join("env_meta.txt"), "time_of_day = 6000\n").unwrap();
        let _world = World::open(&dir).unwrap();
        assert!(dir.join("nodes-v4.log").exists() && dir.join("chests.txt.v4").exists() && dir.join("doors.txt.v4").exists());
        assert!(!dir.join("chests.txt").exists() && !dir.join("doors.txt").exists());
        assert!(dir.join("env_meta.txt").exists(), "the time of day carries on");
        assert_eq!(fs::read_to_string(dir.join("generator")).unwrap().trim(), GENERATOR_VERSION.to_string());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Dirt beside grass is noted for grass to spread onto; grass under
    /// stone, to die; and both are forgotten once they can't.
    #[test]
    fn turf() {
        let mut w = World::builder();
        let (dirt, grass) = ((0, 400, 0), (1, 400, 0));
        w.set_node(dirt, content::CONTENT_DIRT, 0);
        assert!(w.turf().is_empty(), "bare dirt with no grass near");
        w.set_node(grass, content::CONTENT_LITTER, 0);
        assert_eq!(w.turf(), vec![(dirt, Turf::Spread(content::CONTENT_LITTER))]);
        w.set_node((1, 401, 0), content::CONTENT_STONE, 0);
        let mut t = w.turf();
        t.sort_by_key(|(p, _)| *p);
        assert_eq!(t, vec![(dirt, Turf::Spread(content::CONTENT_LITTER)), (grass, Turf::Covered(content::CONTENT_DIRT))]);
        // Walled in: no way for grass in.
        for d in nature::AROUND {
            let p = (dirt.0 + d.0, dirt.1 + d.1, dirt.2 + d.2);
            if p != grass {
                w.set_node(p, content::CONTENT_STONE, 0);
            }
        }
        assert_eq!(w.turf(), vec![(grass, Turf::Covered(content::CONTENT_DIRT))]);
        // Tall grass on dirt seeds it; water over grass kills it.
        w.set_node((5, 400, 0), content::CONTENT_DIRT, 0);
        w.set_node((5, 401, 0), crate::farming::GRASS[1], 0);
        w.set_node((8, 400, 0), content::CONTENT_GRASS, 0);
        w.set_node((8, 401, 0), content::CONTENT_WATER, 0);
        let t = w.turf();
        assert!(t.contains(&((5, 400, 0), Turf::Spread(content::CONTENT_GRASS))), "{t:?}");
        assert!(t.contains(&((8, 400, 0), Turf::Covered(content::CONTENT_DIRT))), "{t:?}");
    }
}
