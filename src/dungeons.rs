// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2026 Keir Finlow-Bates
// Translated from Luanti's src/mapgen/dungeongen.cpp and MapgenBasic::generateDungeons
// in src/mapgen/mapgen.cpp, and minetest_game's dungeon materials
// (mods/default/mapgen.lua):
//   Copyright (C) 2010-2018 celeron55, Perttu Ahola <celeron55@gmail.com>
//   Copyright (C) 2013-2018 kwolekr, Ryan Kwolek <kwolekr@minetest.net>
//   Copyright (C) 2015-2018 paramat
//   Copyright (C) 2011-2018 Various Minetest Game developers and contributors
// The loot chests are translated from minetest_game's dungeon_loot mod
// (MIT licence): Copyright (C) 2017 sfan5.
// Licensed under the GNU Lesser General Public License, version 2.1 or (at
// your option) any later version: see COPYING.LESSER.

//! Dungeons, as Luanti's dungeon generator (src/mapgen/dungeongen.cpp,
//! and MapgenBasic::generateDungeons in mapgen.cpp), with minetest_game's
//! dungeon materials (mods/default/mapgen.lua) and loot chests
//! (mods/dungeon_loot).
//!
//! Luanti generates the map a mapchunk at a time: 5x5x5 map blocks, 80
//! nodes a side, starting 32 nodes below the origin. Each mapchunk with
//! stone in it may hold dungeons ("dungeons" noise): rooms joined by
//! corridors, some with stairs, walled in cobblestone that is mossy in
//! places (desert stone in the desert). They are only dug into the
//! ground: air, liquids and anything not part of the ground are left
//! alone, so they never stand in the open, though a room may break into a
//! cave or come up to the surface.
//!
//! Unlike Luanti, where they reach up to 16 nodes into the neighbouring
//! mapchunks, a mapchunk's dungeons are built within a box of 3x2x3 map
//! blocks (48x32x48 nodes) inside it, placed at random in the mapchunk
//! (from its position, so the same every time): a tighter warren of the
//! same rooms and corridors. On the board that matters: a map block needs
//! only its own mapchunk's dungeons, and only if it is in the box (18 of
//! the 125), so most blocks need no dungeon work at all; and a mapchunk's
//! dungeons, worked out once and kept, take less memory. (In Luanti a
//! block on a mapchunk's edge needed up to eight mapchunks' dungeons
//! worked out, at up to a second or more each on the board.)
//!
//! Here every node is a pure function of its position, so a mapchunk's
//! dungeons are worked out on their own the first time a block in its box
//! is wanted, against the terrain without dungeons, and kept (a few
//! dozen mapchunks' worth) as the nodes they change.
//!
//! dungeon_loot puts up to two chests in a mapchunk's rooms, against a
//! wall. Their contents depend on their position and depth, and are
//! worked out when the chest is first opened (or dug).

use crate::chest::SLOTS;
use crate::content::{self, CONTENT_CHEST, CONTENT_COBBLE, CONTENT_DESERT_STONE};
use crate::fasthash::{FastMap, FastSet};
use crate::inventory::{self, Stack};
use crate::terrain::{self, Biome, NoiseParams};
use crate::world::{BlockPos, NodePos, CONTENT_AIR};
use std::sync::{Condvar, Mutex};

/// minetest_game's mossy cobblestone, which the dungeons are partly built
/// of, and its wall; its stairs and slabs are stairs::MOSSY.
pub const MOSSYCOBBLE: u16 = crate::fences::WALL_DESERT_COBBLE + 1;
pub const WALL_MOSSYCOBBLE: u16 = MOSSYCOBBLE + 1;

/// Mapchunks: 80 nodes a side, starting at -32 (Luanti's chunksize 5 and
/// its offset of -2 blocks).
const CHUNK: i32 = 80;
const CHUNK_OFFSET: i32 = -32;
/// The box in a mapchunk that its dungeons are built in, in map blocks
/// and in nodes (see the top of this file).
const BOX_BLOCKS: (i32, i32, i32) = (3, 2, 3);
const BOX: (i32, i32, i32) = (BOX_BLOCKS.0 * 16, BOX_BLOCKS.1 * 16, BOX_BLOCKS.2 * 16);

const SEED: u32 = 0x0D06_E011;

/// Luanti's np_dungeons default: how many dungeons a mapchunk has
/// (rounded down; mostly none or one).
const NP_DUNGEONS: NoiseParams =
    NoiseParams { offset: 0.9, scale: 0.5, spread: 500.0, seed: 0, octaves: 2, persistence: 0.8, lacunarity: 2.0 };
/// Which walls are the alternative (mossy) material: where this is above
/// zero.
const NP_ALT_WALL: NoiseParams =
    NoiseParams { offset: -0.4, scale: 1.0, spread: 40.0, seed: 32474, octaves: 6, persistence: 1.1, lacunarity: 2.0 };

/// The mapchunk a node is in, along one axis.
fn chunk_of(v: i32) -> i32 {
    (v - CHUNK_OFFSET).div_euclid(CHUNK)
}

/// The first node of a mapchunk, along one axis.
fn chunk_start(c: i32) -> i32 {
    c * CHUNK + CHUNK_OFFSET
}

/// The lowest corner of the box mapchunk `c`'s dungeons are built in: at
/// a whole number of map blocks into the mapchunk, chosen from its
/// position, so that the boxes of neighbouring mapchunks fall in
/// different places and the dungeons don't line up.
fn box_min(c: (i32, i32, i32)) -> V {
    let h = terrain::hash(c.0, c.1, c.2, SEED ^ 0xB0C5);
    let spare = (5 - BOX_BLOCKS.0, 5 - BOX_BLOCKS.1, 5 - BOX_BLOCKS.2);
    let (ox, oy, oz) = (
        (h % (spare.0 as u32 + 1)) as i32,
        (h / 7 % (spare.1 as u32 + 1)) as i32,
        (h / 61 % (spare.2 as u32 + 1)) as i32,
    );
    (chunk_start(c.0) + ox * 16, chunk_start(c.1) + oy * 16, chunk_start(c.2) + oz * 16)
}

/// The mapchunk map block `b` is in, if it is in that mapchunk's dungeon
/// box: only then may it hold dungeon nodes.
fn chunk_with_box(b: BlockPos) -> Option<(i32, i32, i32)> {
    let p = (b.0 as i32 * 16, b.1 as i32 * 16, b.2 as i32 * 16);
    let c = (chunk_of(p.0), chunk_of(p.1), chunk_of(p.2));
    let m = box_min(c);
    let inside = (m.0..m.0 + BOX.0).contains(&p.0) && (m.1..m.1 + BOX.1).contains(&p.1) && (m.2..m.2 + BOX.2).contains(&p.2);
    inside.then_some(c)
}

// ---------------------------------------------------------------------------
// Luanti's PseudoRandom
// ---------------------------------------------------------------------------

struct PseudoRandom(i32);

impl PseudoRandom {
    fn next(&mut self) -> i32 {
        self.0 = (self.0 as u32).wrapping_mul(1_103_515_245).wrapping_add(12345) as i32;
        // Signed division, as Luanti's.
        ((self.0 / 65536) as u32 % 32768) as i32
    }

    fn range(&mut self, min: i32, max: i32) -> i32 {
        debug_assert!(max >= min);
        min + self.next() % (max - min + 1)
    }
}

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

type V = (i32, i32, i32);

fn add(a: V, b: V) -> V {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}

fn sub(a: V, b: V) -> V {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

/// Luanti's VoxelManipulator flags.
const INSIDE: u8 = 0x01;
const PRESERVE: u8 = 0x02;
const UNTOUCHABLE: u8 = INSIDE | PRESERVE;

/// Outside the area (Luanti's CONTENT_IGNORE).
const IGNORE: u16 = u16::MAX;

struct Params {
    wall: u16,
    /// None: no alternative walls.
    alt_wall: Option<u16>,
    stair: u16,
    num_dungeons: u32,
    num_rooms: u32,
    room_size_min: V,
    room_size_max: V,
    room_size_large_min: V,
    room_size_large_max: V,
    large_room_chance: i32,
    diagonal_dirs: bool,
    holesize: V,
    corridor_len_min: i32,
    corridor_len_max: i32,
}

/// A node the dungeons set: what it was, what it is now, its flags.
#[derive(Clone, Copy)]
struct Cell {
    base: u16,
    id: u16,
    param2: u8,
    flags: u8,
}

struct Gen<'a> {
    /// The area's lowest corner, and its size: the dungeon box.
    min: V,
    size: V,
    dp: Params,
    random: PseudoRandom,
    blockseed: u32,
    /// The terrain without dungeons.
    terrain: &'a mut dyn FnMut(NodePos) -> u16,
    cells: FastMap<V, Cell>,
    m_pos: V,
    m_dir: V,
    /// The rooms' centres (Luanti's gennotify "dungeon" events).
    rooms: Vec<V>,
}

/// Whether the dungeons leave a node of the terrain alone (Luanti's
/// only_in_ground): air, liquids, and what isn't ground (plants).
fn preserved(id: u16) -> bool {
    id == CONTENT_AIR || content::is_liquid(id) || !content::walkable(id) || id == crate::plants::KELP
}

impl Gen<'_> {
    fn contains(&self, p: V) -> bool {
        (0..self.size.0).contains(&(p.0 - self.min.0)) && (0..self.size.1).contains(&(p.1 - self.min.1)) && (0..self.size.2).contains(&(p.2 - self.min.2))
    }

    fn cell(&mut self, p: V) -> Cell {
        if let Some(&c) = self.cells.get(&p) {
            return c;
        }
        let base = (self.terrain)(p);
        Cell { base, id: base, param2: 0, flags: if preserved(base) { PRESERVE } else { 0 } }
    }

    /// The node at p (getNodeNoExNoEmerge): IGNORE outside the area.
    fn get(&mut self, p: V) -> u16 {
        if !self.contains(p) {
            return IGNORE;
        }
        self.cell(p).id
    }

    fn flags(&mut self, p: V) -> u8 {
        self.cell(p).flags
    }

    fn set(&mut self, p: V, id: u16, param2: u8, or_flags: u8) {
        let mut c = self.cell(p);
        c.id = id;
        c.param2 = param2;
        c.flags |= or_flags;
        self.cells.insert(p, c);
    }

    fn generate(&mut self) {
        // The terrain's flags are worked out as nodes are looked at
        // (`cell`), rather than for the whole area first.
        // (Luanti starts the first room clear of the margin it builds into
        // in the neighbours; here there is none.)
        for _ in 0..self.dp.num_dungeons {
            self.make_dungeon((0, 0, 0));
        }
        // Some of the walls in the alternative material.
        if let Some(alt) = self.dp.alt_wall {
            let wall = self.dp.wall;
            let seed = self.blockseed;
            for (p, c) in self.cells.iter_mut() {
                if c.id == wall && terrain::fractal3(*p, &NP_ALT_WALL, seed) > 0.0 {
                    c.id = alt;
                }
            }
        }
    }

    fn random_size(&mut self, min: V, max: V) -> V {
        let z = self.random.range(min.2, max.2);
        let y = self.random.range(min.1, max.1);
        let x = self.random.range(min.0, max.0);
        (x, y, z)
    }

    fn make_dungeon(&mut self, start_padding: V) {
        let mut roomsize = (0, 0, 0);
        let mut roomplace = (0, 0, 0);
        // Find a place for the first room.
        let mut fits = false;
        for _ in 0..100 {
            roomsize = if self.dp.large_room_chance >= 1 {
                self.random_size(self.dp.room_size_large_min, self.dp.room_size_large_max)
            } else {
                self.random_size(self.dp.room_size_min, self.dp.room_size_max)
            };
            // start_padding: not starting in a neighbouring mapchunk.
            let z = self.random.range(0, (self.size.2 - roomsize.2 - start_padding.2).max(0));
            let y = self.random.range(0, (self.size.1 - roomsize.1 - start_padding.1).max(0));
            let x = self.random.range(0, (self.size.0 - roomsize.0 - start_padding.0).max(0));
            roomplace = add(add(self.min, start_padding), (x, y, z));
            // All of it in the ground (it stops at the first node that
            // isn't, which Luanti's check carries on past to the same end).
            fits = true;
            'check: for z in 0..roomsize.2 {
                for y in 0..roomsize.1 {
                    for x in 0..roomsize.0 {
                        if self.flags(add(roomplace, (x, y, z))) & UNTOUCHABLE != 0 {
                            fits = false;
                            break 'check;
                        }
                    }
                }
            }
            if fits {
                break;
            }
        }
        if !fits {
            return;
        }
        // The centre of the last room made, so a corridor can start from
        // there instead of the new room.
        let mut last_room_center = add(roomplace, (roomsize.0 / 2, 1, roomsize.2 / 2));
        for i in 0..self.dp.num_rooms {
            self.make_room(roomsize, roomplace);
            let room_center = add(roomplace, (roomsize.0 / 2, 1, roomsize.2 / 2));
            self.rooms.push(room_center);
            if i + 1 == self.dp.num_rooms {
                break;
            }
            // Where the walker starts.
            let start_in_last_room = self.random.range(0, 2) != 0;
            let walker_start_place = if start_in_last_room {
                last_room_center
            } else {
                last_room_center = room_center;
                room_center
            };
            // A walker finds a place for a door.
            self.m_pos = walker_start_place;
            let Some((mut doorplace, doordir)) = self.find_place_for_door() else {
                return;
            };
            if self.random.range(0, 1) == 0 {
                self.make_door(doorplace);
            } else {
                doorplace = sub(doorplace, doordir);
            }
            // A random corridor from the door.
            let (corridor_end, corridor_end_dir) = self.make_corridor(doorplace, doordir);
            // A place for a random sized room.
            roomsize = if self.dp.large_room_chance > 1 && self.random.range(1, self.dp.large_room_chance) == 1 {
                self.random_size(self.dp.room_size_large_min, self.dp.room_size_large_max)
            } else {
                self.random_size(self.dp.room_size_min, self.dp.room_size_max)
            };
            self.m_pos = corridor_end;
            self.m_dir = corridor_end_dir;
            let Some((doorplace, doordir, place)) = self.find_place_for_room_door(roomsize) else {
                return;
            };
            roomplace = place;
            if self.random.range(0, 1) == 0 {
                self.make_door(doorplace);
            } else {
                roomplace = sub(roomplace, doordir);
            }
        }
    }

    /// One node of a room's wall, unless it's untouchable. False: outside
    /// the area (Luanti then skips the opposite wall's node too).
    fn wall_node(&mut self, p: V) -> bool {
        if !self.contains(p) {
            return false;
        }
        if self.flags(p) & UNTOUCHABLE == 0 {
            self.set(p, self.dp.wall, 0, 0);
        }
        true
    }

    fn make_room(&mut self, size: V, place: V) {
        // -X and +X walls.
        for z in 0..size.2 {
            for y in 0..size.1 {
                if self.wall_node(add(place, (0, y, z))) {
                    self.wall_node(add(place, (size.0 - 1, y, z)));
                }
            }
        }
        // -Z and +Z walls.
        for x in 0..size.0 {
            for y in 0..size.1 {
                if self.wall_node(add(place, (x, y, 0))) {
                    self.wall_node(add(place, (x, y, size.2 - 1)));
                }
            }
        }
        // Floor and ceiling.
        for z in 0..size.2 {
            for x in 0..size.0 {
                if self.wall_node(add(place, (x, 0, z))) {
                    self.wall_node(add(place, (x, size.1 - 1, z)));
                }
            }
        }
        // Fill with air, whatever was there.
        for z in 1..size.2 - 1 {
            for y in 1..size.1 - 1 {
                for x in 1..size.0 - 1 {
                    let p = add(place, (x, y, z));
                    if self.contains(p) {
                        self.set(p, CONTENT_AIR, 0, UNTOUCHABLE);
                    }
                }
            }
        }
    }

    fn make_fill(&mut self, place: V, size: V, avoid_flags: u8, id: u16, or_flags: u8) {
        for z in 0..size.2 {
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let p = add(place, (x, y, z));
                    if !self.contains(p) || self.flags(p) & avoid_flags != 0 {
                        continue;
                    }
                    self.set(p, id, 0, or_flags);
                }
            }
        }
    }

    fn make_hole(&mut self, place: V) {
        self.make_fill(place, self.dp.holesize, 0, CONTENT_AIR, INSIDE);
    }

    fn make_door(&mut self, doorplace: V) {
        self.make_hole(doorplace);
    }

    fn make_corridor(&mut self, doorplace: V, doordir: V) -> (V, V) {
        self.make_hole(doorplace);
        let mut p0 = doorplace;
        let mut dir = doordir;
        let (lmin, lmax) = (self.dp.corridor_len_min, self.dp.corridor_len_max);
        let length = self.random.range(lmin, lmax);
        let mut partlength = self.random.range(lmin, lmax);
        let mut partcount = 0;
        let mut make_stairs = 0;
        if self.random.next() % 2 == 0 && partlength >= 3 {
            make_stairs = if self.random.next() % 2 != 0 { 1 } else { -1 };
        }
        let hs = self.dp.holesize;
        for i in 0..length {
            let mut p = add(p0, dir);
            if partcount != 0 {
                p.1 += make_stairs;
            }
            // The smallest corridor's segment must be in the area.
            if self.contains(p) && self.contains(add(p, (0, 1, 0))) {
                if make_stairs != 0 {
                    self.make_fill(add(p, (-1, -1, -1)), add(hs, (2, 3, 2)), UNTOUCHABLE, self.dp.wall, 0);
                    self.make_fill(p, hs, UNTOUCHABLE, CONTENT_AIR, INSIDE);
                    self.make_fill(sub(p, dir), hs, UNTOUCHABLE, CONTENT_AIR, INSIDE);
                    // Stairs, but not on the bottom step, nor on diagonal
                    // steps.
                    if (dir.0 ^ dir.2) & 1 != 0 && ((make_stairs == 1 && i != 0) || (make_stairs == -1 && i != length - 1)) {
                        // Turned round when going down.
                        let facedir = dir_to_facedir((dir.0 * make_stairs, 0, dir.2 * make_stairs));
                        let stair_width = if dir.2 != 0 { hs.0 } else { hs.2 };
                        let swv = if dir.2 != 0 { (1, 0, 0) } else { (0, 0, 1) };
                        let mut ps = p;
                        for _ in 0..stair_width {
                            let at = if make_stairs == -1 { (ps.0 - dir.0, ps.1 - 1, ps.2 - dir.2) } else { (ps.0, ps.1 - 1, ps.2) };
                            if self.contains(at) && self.get(at) == self.dp.wall {
                                self.set(at, self.dp.stair, facedir, UNTOUCHABLE);
                            }
                            ps = add(ps, swv);
                        }
                    }
                } else {
                    self.make_fill(add(p, (-1, -1, -1)), add(hs, (2, 2, 2)), UNTOUCHABLE, self.dp.wall, 0);
                    self.make_hole(p);
                }
                p0 = p;
            } else {
                // Can't go here: turn away.
                dir = turn_xz(dir, self.random.range(0, 1));
                make_stairs = -make_stairs;
                partcount = 0;
                partlength = self.random.range(1, length);
                continue;
            }
            partcount += 1;
            if partcount >= partlength {
                partcount = 0;
                self.random_turn(&mut dir);
                partlength = self.random.range(1, length);
                make_stairs = 0;
                if self.random.next() % 2 == 0 && partlength >= 3 {
                    make_stairs = if self.random.next() % 2 != 0 { 1 } else { -1 };
                }
            }
        }
        (p0, dir)
    }

    fn randomize_dir(&mut self) {
        self.m_dir = self.rand_ortho_dir();
    }

    fn find_place_for_door(&mut self) -> Option<(V, V)> {
        let air = CONTENT_AIR;
        let wall = self.dp.wall;
        for i in 0..100 {
            let mut p = add(self.m_pos, self.m_dir);
            let p1 = add(p, (0, 1, 0));
            if !self.contains(p) || !self.contains(p1) || i % 4 == 0 {
                self.randomize_dir();
                continue;
            }
            if self.get(p) == wall && self.get(p1) == wall {
                // Found a wall: a good place.
                let found = (p, self.m_dir);
                self.randomize_dir();
                return Some(found);
            }
            // Up one if there's room there.
            if self.get(p) == wall && self.get(add(p, (0, 1, 0))) == air && self.get(add(p, (0, 2, 0))) == air {
                p = add(p, (0, 1, 0));
            }
            // Down one if there's room there.
            if self.get(add(p, (0, 1, 0))) == wall && self.get(p) == air && self.get(add(p, (0, -1, 0))) == air {
                p = add(p, (0, -1, 0));
            }
            if self.get(p) != air || self.get(add(p, (0, 1, 0))) != air {
                // Can't walk on here.
                self.randomize_dir();
                continue;
            }
            self.m_pos = p;
        }
        None
    }

    fn find_place_for_room_door(&mut self, roomsize: V) -> Option<(V, V, V)> {
        for _ in 0..30 {
            let Some((doorplace, doordir)) = self.find_place_for_door() else {
                continue;
            };
            // X east, Z north, Y up.
            let roomplace = match doordir {
                (1, 0, 0) => add(doorplace, (0, -1, self.random.range(-roomsize.2 + 2, -2))),
                (-1, 0, 0) => add(doorplace, (-roomsize.0 + 1, -1, self.random.range(-roomsize.2 + 2, -2))),
                (0, 0, 1) => add(doorplace, (self.random.range(-roomsize.0 + 2, -2), -1, 0)),
                (0, 0, -1) => add(doorplace, (self.random.range(-roomsize.0 + 2, -2), -1, -roomsize.2 + 1)),
                // A diagonal: Luanti's roomplace is left as it was made,
                // (0, 0, 0), which is never in the area.
                _ => continue,
            };
            let mut fits = true;
            'check: for z in 1..roomsize.2 - 1 {
                for y in 1..roomsize.1 - 1 {
                    for x in 1..roomsize.0 - 1 {
                        let p = add(roomplace, (x, y, z));
                        if !self.contains(p) || self.flags(p) & INSIDE != 0 {
                            fits = false;
                            break 'check;
                        }
                    }
                }
            }
            if fits {
                return Some((doorplace, doordir, roomplace));
            }
        }
        None
    }

    fn rand_ortho_dir(&mut self) -> V {
        // Diagonals somewhat rare.
        if self.dp.diagonal_dirs && self.random.next() % 4 == 0 {
            let mut dir = (0, 0, 0);
            let mut trycount = 0;
            loop {
                trycount += 1;
                dir.2 = self.random.next() % 3 - 1;
                dir.0 = self.random.next() % 3 - 1;
                if !((dir.0 == 0 || dir.2 == 0) && trycount < 10) {
                    break;
                }
            }
            return dir;
        }
        if self.random.next() % 2 == 0 {
            return if self.random.next() % 2 != 0 { (-1, 0, 0) } else { (1, 0, 0) };
        }
        if self.random.next() % 2 != 0 {
            (0, 0, -1)
        } else {
            (0, 0, 1)
        }
    }

    fn random_turn(&mut self, dir: &mut V) {
        match self.random.range(0, 2) {
            0 => {}
            1 => *dir = turn_xz(*dir, 0),
            _ => *dir = turn_xz(*dir, 1),
        }
    }
}

fn turn_xz(old: V, t: i32) -> V {
    if t == 0 {
        // Right.
        (old.2, old.1, -old.0)
    } else {
        // Left.
        (-old.2, old.1, old.0)
    }
}

fn dir_to_facedir(d: V) -> u8 {
    if d.0.abs() > d.2.abs() {
        if d.0 < 0 {
            3
        } else {
            1
        }
    } else if d.2 < 0 {
        2
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// A mapchunk's dungeons
// ---------------------------------------------------------------------------

/// A node the dungeons change: its index in its map block (z * 256 +
/// y * 16 + x), the node, and its param2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub index: u16,
    pub id: u16,
    pub param2: u8,
}

/// The nodes a mapchunk's dungeons change, by map block, each block's
/// sorted by index.
#[derive(Default)]
pub struct Chunk {
    blocks: FastMap<BlockPos, Box<[Placed]>>,
    /// Where the loot chests are (for tests and statistics).
    pub chests: Vec<NodePos>,
    /// The rooms' centres.
    pub rooms: Vec<NodePos>,
}

impl Chunk {
    pub fn nodes(&self) -> usize {
        self.blocks.values().map(|b| b.len()).sum()
    }
}

/// Whether a dungeon node is a wall (for the tests).
#[cfg(test)]
fn is_wall(id: u16) -> bool {
    matches!(id, CONTENT_COBBLE | MOSSYCOBBLE | CONTENT_DESERT_STONE | content::CONTENT_SANDSTONE_BRICK | content::CONTENT_ICE)
}

/// The dungeons of the mapchunk `c` (in mapchunks), against `terrain`, the
/// generated node without dungeons.
pub fn generate(c: (i32, i32, i32), terrain: &mut dyn FnMut(NodePos) -> u16) -> Chunk {
    let node_min = (chunk_start(c.0), chunk_start(c.1), chunk_start(c.2));
    // Only where there is stone: Luanti's max_stone_y, the highest stone
    // in the mapchunk, here from the ground's height at a few places.
    let mut max_stone_y = i32::MIN;
    for k in 0..81 {
        let col = terrain::column(node_min.0 + 5 + (k % 9) * 9, node_min.2 + 5 + (k / 9) * 9);
        max_stone_y = max_stone_y.max(col.height);
    }
    if node_min.1 > max_stone_y {
        return Chunk::default();
    }
    let num_dungeons = terrain::fractal3(node_min, &NP_DUNGEONS, SEED).floor().max(0.0) as u32;
    if num_dungeons == 0 {
        return Chunk::default();
    }
    let blockseed = terrain::hash(c.0, c.1, c.2, SEED);
    let mut ps = PseudoRandom(blockseed.wrapping_add(70033) as i32);
    let num_rooms = ps.range(2, 16) as u32;
    let large_room_chance = if ps.range(1, 4) == 1 { 8 } else { 0 };
    let diagonal_dirs = ps.range(1, 8) == 1;
    // Diagonal corridors need holes 2 wide to be passable.
    let holewidth = if diagonal_dirs { 2 } else { ps.range(1, 2) };
    // minetest_game's dungeon nodes for the biome in the middle of the
    // mapchunk: desert stone in the desert and sandstone brick (with
    // sandstone block stairs) in the sandstone desert (down to their
    // stone's bottom), ice on the icesheet (down to its bottom), else
    // cobblestone, mossy in places.
    let mid = (node_min.0 + 40, node_min.1 + 40, node_min.2 + 40);
    let biome = terrain::column(mid.0, mid.2).biome;
    let (wall, alt_wall, stair) = if biome == Biome::Desert && mid.1 > -256 {
        (CONTENT_DESERT_STONE, None, crate::stairs::BUILDING.first_stair())
    } else if biome == Biome::SandstoneDesert && mid.1 > -256 {
        let stair = crate::stairs::shaped(crate::stairs::Shape::Stair, content::CONTENT_SANDSTONE_BLOCK).expect("sandstone block stairs");
        (content::CONTENT_SANDSTONE_BRICK, None, stair)
    } else if biome == Biome::Icesheet && mid.1 >= terrain::ICESHEET_BOTTOM {
        (content::CONTENT_ICE, None, crate::stairs::BUILDING.first_stair() + 14)
    } else {
        (CONTENT_COBBLE, Some(MOSSYCOBBLE), crate::stairs::FIRST_STAIR + 4)
    };
    let dp = Params {
        wall,
        alt_wall,
        stair,
        num_dungeons,
        num_rooms,
        room_size_min: (5, 5, 5),
        room_size_max: (12, 6, 12),
        room_size_large_min: (12, 6, 12),
        room_size_large_max: (16, 16, 16),
        large_room_chance,
        diagonal_dirs,
        holesize: (holewidth, 3, holewidth),
        corridor_len_min: 1,
        corridor_len_max: 13,
    };
    let mut gen = Gen {
        min: box_min(c),
        size: BOX,
        dp,
        random: PseudoRandom(blockseed.wrapping_add(2) as i32),
        blockseed,
        terrain,
        cells: FastMap::default(),
        m_pos: (0, 0, 0),
        m_dir: (0, 0, 0),
        rooms: Vec::new(),
    };
    gen.generate();
    let chests = place_chests(&mut gen);
    let rooms = gen.rooms.clone();
    // What changed, by map block.
    let mut by_block: FastMap<BlockPos, Vec<Placed>> = FastMap::default();
    for (p, c) in &gen.cells {
        if c.id == c.base && c.param2 == 0 {
            continue;
        }
        let b = crate::world::block_of_node(*p);
        let (x, y, z) = (p.0.rem_euclid(16), p.1.rem_euclid(16), p.2.rem_euclid(16));
        by_block.entry(b).or_default().push(Placed { index: (z * 256 + y * 16 + x) as u16, id: c.id, param2: c.param2 });
    }
    let blocks = by_block
        .into_iter()
        .map(|(b, mut v)| {
            v.sort_unstable_by_key(|p| p.index);
            (b, v.into_boxed_slice())
        })
        .collect();
    Chunk { blocks, chests, rooms }
}

/// dungeon_loot's chests: up to two, each against a wall of a room at
/// least 4 nodes across with at least three walls round its centre.
fn place_chests(gen: &mut Gen) -> Vec<NodePos> {
    let Some(&first) = gen.rooms.first() else {
        return Vec::new();
    };
    let mut rand = Rng(terrain::hash(first.0, first.1, first.2, SEED ^ 10115));
    // The rooms (of the first 8) with walls to put a chest against.
    struct Room {
        walls: Vec<(V, V)>, // position, facing (into the room)
        size: (i32, i32),
    }
    let mut candidates = Vec::new();
    for c in gen.rooms.iter().take(8).copied().collect::<Vec<V>>() {
        let mut walls = Vec::new();
        let (mut mx, mut mz) = (0, 0);
        let min = |a: i32, b: i32| if a != 0 { a.min(b) } else { b };
        for dir in [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)] {
            for i in 1..=9 {
                let pos = add(c, (dir.0 * i, 0, dir.2 * i));
                if is_solid(gen, pos) {
                    let front_below = sub(pos, (dir.0, 1, dir.2));
                    let above = add(pos, (0, 1, 0));
                    if is_solid(gen, front_below) && is_solid(gen, above) {
                        walls.push((pos, (-dir.0, 0, -dir.2)));
                        if dir.2 == 0 {
                            mx = min(mx, i - 1);
                        } else {
                            mz = min(mz, i - 1);
                        }
                    }
                    break;
                }
            }
        }
        let size = (mx * 2, mz * 2);
        if size.0.min(size.1) >= 4 && walls.len() >= 3 {
            candidates.push(Room { walls, size });
        }
    }
    let num_chests = (rand.range(0, 2) as usize).min(candidates.len());
    let mut chests = Vec::new();
    for _ in 0..num_chests {
        let room = candidates.remove(rand.range(0, candidates.len() as i32 - 1) as usize);
        let (pos, facing) = room.walls[rand.range(0, room.walls.len() as i32 - 1) as usize];
        // Along the wall.
        let (v, half) = if facing.0 != 0 { ((0, 0, 1), room.size.1 / 2) } else { ((1, 0, 0), room.size.0 / 2) };
        let off = rand.range(-half + 1, half - 1);
        let chestpos = add(add(pos, facing), (v.0 * off, 0, v.2 * off));
        if gen.contains(chestpos) && gen.get(chestpos) == CONTENT_AIR {
            let facedir = dir_to_facedir((-facing.0, 0, -facing.2));
            gen.set(chestpos, CONTENT_CHEST, facedir, 0);
            chests.push(chestpos);
        }
    }
    chests
}

/// dungeon_loot's is_wall: anything but air (and ignore).
fn is_solid(gen: &mut Gen, p: V) -> bool {
    let id = gen.get(p);
    id != CONTENT_AIR && id != IGNORE
}

// ---------------------------------------------------------------------------
// Shared between the server and the block builder
// ---------------------------------------------------------------------------

/// How many mapchunks' dungeons are kept, and how many of their nodes in
/// all (6 bytes each). A map block can need up to eight mapchunks (those
/// whose dungeons reach into it), so one player's surroundings use a few
/// dozen, and several players exploring apart use well over a hundred;
/// keeping too few means working the same ones out again and again, at up
/// to a second or more each on the board. Many have no dungeons (above the
/// ground) and cost next to nothing, so the node budget is what limits
/// the board: 120,000 nodes is about 0.7 MB of PSRAM. The PC build keeps
/// far more.
const KEEP: usize = if cfg!(pc) { 4096 } else { 128 };
const KEEP_NODES: usize = if cfg!(pc) { 20_000_000 } else { 120_000 };

/// How many mapchunks are kept and their nodes, for the log.
#[cfg_attr(any(not(target_os = "espidf"), not(perf)), allow(dead_code))]
pub fn kept_summary() -> String {
    let guard = CHUNKS.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(kept) => format!("{} mapchunks, {} nodes", kept.map.len(), kept.nodes),
        None => "none".to_string(),
    }
}

/// The kept mapchunks, with when each was last used, and those being
/// worked out now.
#[derive(Default)]
struct Kept {
    uses: u32,
    map: FastMap<(i32, i32, i32), (u32, Chunk)>,
    /// The kept mapchunks' nodes, in all.
    nodes: usize,
    busy: FastSet<(i32, i32, i32)>,
}

static CHUNKS: Mutex<Option<Kept>> = Mutex::new(None);
/// Signalled when a mapchunk has been worked out.
static DONE: Condvar = Condvar::new();

/// Marks a mapchunk as no longer being worked out, even if working it
/// out panicked, and wakes whoever waits for it.
struct Busy((i32, i32, i32));

impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(kept) = CHUNKS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            kept.busy.remove(&self.0);
        }
        DONE.notify_all();
    }
}

/// Forget every kept mapchunk (memory is running low: see
/// world::trim_caches). They are worked out again as they are needed.
pub fn trim() {
    if let Some(kept) = CHUNKS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        kept.map = FastMap::default();
        kept.nodes = 0;
    }
}

/// Look at the dungeons of mapchunk `c` with `f`, working them out with
/// `terrain` if they aren't kept. Each is worked out once: a thread that
/// wants one the other thread is working out waits for it. `f` runs with
/// the cache locked, and gets the kept chunk itself, not a shared
/// reference to it: on the ESP32-S3 an atomic operation on PSRAM isn't
/// atomic between the two cores, so an Arc's count, changed from both,
/// gets lost, and what it holds is freed while still in use (that
/// crashed the board). What `f` needs is copied out instead.
pub fn with_chunk<R>(c: (i32, i32, i32), terrain: &mut dyn FnMut(NodePos) -> u16, f: impl FnOnce(&Chunk) -> R) -> R {
    let busy = {
        let mut guard = CHUNKS.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let kept = guard.get_or_insert_with(Default::default);
            kept.uses += 1;
            let uses = kept.uses;
            if let Some(found) = kept.map.get_mut(&c) {
                found.0 = uses;
                return f(&found.1);
            }
            if !kept.busy.contains(&c) {
                kept.busy.insert(c);
                break Busy(c);
            }
            guard = DONE.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
    };
    let started = std::time::Instant::now();
    let made = generate(c, terrain);
    if made.nodes() > 0 {
        log::info!(
            "Dungeons in mapchunk {c:?}: {} rooms, {} nodes, {} chests, worked out in {} ms",
            made.rooms.len(),
            made.nodes(),
            made.chests.len(),
            started.elapsed().as_millis()
        );
    }
    let result = {
        let mut guard = CHUNKS.lock().unwrap_or_else(|e| e.into_inner());
        let kept = guard.get_or_insert_with(Default::default);
        kept.uses += 1;
        // Forget those used longest ago until there is room.
        while !kept.map.is_empty() && (kept.map.len() >= KEEP || kept.nodes + made.nodes() > KEEP_NODES) {
            let Some(oldest) = kept.map.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| *k) else {
                break;
            };
            if let Some((_, gone)) = kept.map.remove(&oldest) {
                kept.nodes -= gone.nodes();
            }
        }
        let result = f(&made);
        let uses = kept.uses;
        kept.nodes += made.nodes();
        kept.map.insert(c, (uses, made));
        result
    };
    drop(busy);
    result
}

#[cfg(test)]
mod shared_tests {
    use super::*;

    /// Two threads wanting the same mapchunk at once: it is worked out
    /// once, and both get it.
    #[test]
    fn worked_out_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let c = (1000, -3, 1000);
        let run = move || {
            let mut stone = |_: NodePos| {
                CALLS.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_micros(1));
                content::CONTENT_STONE
            };
            with_chunk(c, &mut stone, |chunk| chunk.nodes())
        };
        let a = std::thread::spawn(run);
        let b = std::thread::spawn(run);
        let (a, b) = (a.join().unwrap(), b.join().unwrap());
        assert_eq!(a, b);
        let once = CALLS.load(Ordering::Relaxed);
        let mut stone = |_: NodePos| content::CONTENT_STONE;
        with_chunk(c, &mut stone, |_| ());
        // A third call finds it kept; the two together made it once (a
        // single generate reads the same nodes as this count).
        let mut count = 0;
        let mut counting = |_: NodePos| {
            count += 1;
            content::CONTENT_STONE
        };
        let _ = generate(c, &mut counting);
        assert_eq!(once, count);
    }
}

/// The nodes the dungeons change in map block `b`, sorted by index: none
/// unless the block is in its mapchunk's dungeon box (which costs nothing
/// to find out); otherwise from that mapchunk's dungeons, worked out the
/// first time.
pub fn block(b: BlockPos, terrain: &mut dyn FnMut(NodePos) -> u16) -> Vec<Placed> {
    let Some(c) = chunk_with_box(b) else {
        return Vec::new();
    };
    with_chunk(c, terrain, |chunk| chunk.blocks.get(&b).map(|p| p.to_vec())).unwrap_or_default()
}

/// The dungeon's node at a position of a block's list (from `block`).
pub fn find(placed: &[Placed], p: NodePos) -> Option<Placed> {
    let index = (p.2.rem_euclid(16) * 256 + p.1.rem_euclid(16) * 16 + p.0.rem_euclid(16)) as u16;
    placed.binary_search_by_key(&index, |q| q.index).ok().map(|i| placed[i])
}

// ---------------------------------------------------------------------------
// Loot
// ---------------------------------------------------------------------------

/// A small random number generator for the loot (dungeon_loot uses
/// Luanti's PcgRandom, seeded from the chest's position).
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        // xorshift32, never left at zero.
        let mut x = self.0 | 1;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// From min to max, both included.
    fn range(&mut self, min: i32, max: i32) -> i32 {
        min + (self.next() % (max - min + 1) as u32) as i32
    }
}

/// dungeon_loot's kinds of dungeon (its "types"; its "sandstone", for the
/// sandstone desert, isn't generated here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Normal,
    Desert,
    Ice,
    /// The sandstone desert's.
    Sandstone,
}

const NORMAL: u8 = 1 << Kind::Normal as u8;
const DESERT: u8 = 1 << Kind::Desert as u8;
const ICE: u8 = 1 << Kind::Ice as u8;
const SANDSTONE: u8 = 1 << Kind::Sandstone as u8;
const ALL: u8 = NORMAL | DESERT | ICE | SANDSTONE;

/// dungeon_loot's registered loot: item, chance, count range, depths, and
/// which kinds of dungeon (a bit each).
struct Loot {
    name: &'static str,
    chance: f32,
    count: (i32, i32),
    y: Option<(i32, i32)>,
    kinds: u8,
}

const fn loot(name: &'static str, chance: f32, count: (i32, i32)) -> Loot {
    Loot { name, chance, count, y: None, kinds: ALL }
}

const ABOVE_64: Option<(i32, i32)> = Some((-64, 32768));
const BELOW_512: Option<(i32, i32)> = Some((-32768, -512));

const LOOT: &[Loot] = &[
    loot("esp:stick", 0.6, (3, 6)),
    loot("esp:flint", 0.4, (1, 3)),
    loot("esp:apple", 0.4, (1, 4)),
    Loot { kinds: SANDSTONE | DESERT, ..loot("esp:cactus", 0.4, (1, 4)) },
    loot("esp:coal_lump", 0.9, (1, 12)),
    loot("esp:gold_ingot", 0.5, (1, 1)),
    loot("esp:steel_ingot", 0.4, (1, 6)),
    loot("esp:mese_crystal", 0.1, (2, 3)),
    loot("esp:sword_wood", 0.6, (1, 1)),
    loot("esp:pick_stone", 0.3, (1, 1)),
    loot("esp:axe_diamond", 0.05, (1, 1)),
    Loot { y: ABOVE_64, kinds: NORMAL, ..loot("esp:sand", 0.8, (4, 32)) },
    Loot { y: ABOVE_64, kinds: SANDSTONE, ..loot("esp:desert_sand", 0.8, (4, 32)) },
    Loot { kinds: DESERT, ..loot("esp:desert_cobble", 0.8, (4, 32)) },
    Loot { y: ABOVE_64, kinds: ICE, ..loot("esp:snow", 0.8, (8, 64)) },
    Loot { y: ABOVE_64, kinds: NORMAL | SANDSTONE | DESERT, ..loot("esp:dirt", 0.6, (2, 16)) },
    Loot { y: BELOW_512, ..loot("esp:obsidian", 0.25, (1, 3)) },
    Loot { y: BELOW_512, ..loot("esp:mese", 0.15, (1, 1)) },
    // Registered by the other mods: bucket's,
    loot("esp:bucket_empty", 0.55, (1, 1)),
    Loot { kinds: SANDSTONE | DESERT | ICE, ..loot("esp:bucket_water", 0.45, (1, 1)) },
    Loot { y: Some((0, 32768)), kinds: NORMAL, ..loot("esp:bucket_water", 0.45, (1, 1)) },
    Loot { y: Some((-32768, -1)), kinds: NORMAL, ..loot("esp:bucket_lava", 0.45, (1, 1)) },
    // carts',
    loot("esp:rail", 0.35, (1, 6)),
    // farming's,
    loot("esp:string", 0.5, (1, 8)),
    loot("esp:wheat", 0.5, (2, 5)),
    Loot { kinds: NORMAL, ..loot("esp:seed_cotton", 0.4, (1, 4)) },
    // and vessels'.
    loot("esp:glass_fragments", 0.35, (1, 4)),
];

/// The items dungeon_loot can put in a chest, all known items.
#[cfg(test)]
fn loot_names() -> impl Iterator<Item = &'static str> {
    LOOT.iter().map(|l| l.name)
}

/// What a dungeon chest at `pos` holds (dungeon_loot's populate_chest):
/// up to 8 kinds of item, from those found at its depth in its kind of
/// dungeon, each with its chance; tools worn 20% to 75%.
pub fn loot_for(pos: NodePos, kind: Kind) -> [Option<Stack>; SLOTS] {
    let mut rand = Rng(terrain::hash(pos.0, pos.1, pos.2, SEED ^ 0x7007));
    let mut list: Vec<&Loot> = LOOT
        .iter()
        .filter(|l| l.y.map_or(true, |(lo, hi)| (lo..=hi).contains(&pos.1)))
        .filter(|l| l.kinds & 1 << kind as u8 != 0)
        .collect();
    let n = list.len().min(8);
    let mut chosen = Vec::new();
    for _ in 0..n {
        chosen.push(list.remove(rand.range(0, list.len() as i32 - 1) as usize));
    }
    let mut items = Vec::new();
    for l in chosen {
        if rand.range(0, 1000) as f32 / 1000.0 > l.chance {
            continue;
        }
        let Some(id) = content::id_of(l.name) else {
            continue;
        };
        let amount = rand.range(l.count.0, l.count.1) as u16;
        if content::is_tool(id) {
            for _ in 0..amount {
                let wear = rand.range((0.20 * 65535.0) as i32, (0.75 * 65535.0) as i32) as u16;
                items.push(Stack { wear, ..Stack::new(id, 1) });
            }
        } else if content::stack_max(id) == 1 {
            items.extend((0..amount).map(|_| Stack::new(id, 1)));
        } else {
            items.push(Stack::new(id, amount));
        }
    }
    let mut slots = [None; SLOTS];
    for item in items {
        let i = rand.range(0, SLOTS as i32 - 1) as usize;
        if slots[i].is_none() {
            slots[i] = Some(item);
        } else {
            inventory::add_to(&mut slots, item);
        }
    }
    slots
}

/// The kind of dungeon a chest at `pos` is in, by its biome (dungeon_loot's
/// dungeontype).
pub fn kind_at(pos: NodePos) -> Kind {
    match terrain::column(pos.0, pos.2).biome {
        Biome::Desert => Kind::Desert,
        Biome::Icesheet => Kind::Ice,
        Biome::SandstoneDesert => Kind::Sandstone,
        _ => Kind::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudo_random_is_luantis() {
        // PseudoRandom(0): m_next = 12345, 12345 / 65536 = 0.
        let mut r = PseudoRandom(0);
        assert_eq!(r.next(), 0);
        // Then 12345 * 1103515245 + 12345 (mod 2^32), as signed, / 65536.
        let m = (12345u32).wrapping_mul(1_103_515_245).wrapping_add(12345) as i32;
        assert_eq!(r.next(), ((m / 65536) as u32 % 32768) as i32);
        let mut r = PseudoRandom(-5);
        for _ in 0..1000 {
            let v = r.range(-3, 9);
            assert!((-3..=9).contains(&v));
        }
    }

    #[test]
    fn turns_and_facedirs() {
        assert_eq!(turn_xz((1, 0, 0), 0), (0, 0, -1));
        assert_eq!(turn_xz((1, 0, 0), 1), (0, 0, 1));
        assert_eq!(dir_to_facedir((0, 0, 1)), 0);
        assert_eq!(dir_to_facedir((1, 0, 0)), 1);
        assert_eq!(dir_to_facedir((0, 0, -1)), 2);
        assert_eq!(dir_to_facedir((-1, 0, 0)), 3);
    }

    #[test]
    fn mapchunks() {
        assert_eq!((chunk_of(-32), chunk_of(47), chunk_of(48), chunk_of(-33)), (0, 0, 1, -1));
        // Each mapchunk's box is in it, whole map blocks in, and holds
        // 3x2x3 of its 125 blocks; the boxes move about from mapchunk to
        // mapchunk.
        let mut places = std::collections::HashSet::new();
        for c in [(0, 0, 0), (1, 0, 0), (0, -1, 3), (-7, 2, 9), (12, -4, -5), (3, 3, 3)] {
            let m = box_min(c);
            places.insert((m.0 - chunk_start(c.0), m.1 - chunk_start(c.1), m.2 - chunk_start(c.2)));
            let mut inside = 0;
            for bz in 0..5 {
                for by in 0..5 {
                    for bx in 0..5 {
                        let b = ((chunk_start(c.0) / 16 + bx) as i16, (chunk_start(c.1) / 16 + by) as i16, (chunk_start(c.2) / 16 + bz) as i16);
                        if let Some(found) = chunk_with_box(b) {
                            assert_eq!(found, c);
                            inside += 1;
                        }
                    }
                }
            }
            assert_eq!(inside, 18, "{c:?}");
        }
        assert!(places.len() > 1, "the boxes are all in the same place");
    }

    /// Solid stone everywhere: dungeons have room, and they come out the
    /// same every time.
    #[test]
    fn dungeons_in_solid_stone() {
        let mut stone = |_: NodePos| content::CONTENT_STONE;
        let mut found = 0;
        for c in 0..40 {
            let a = generate((c, -3, 7), &mut stone);
            let b = generate((c, -3, 7), &mut stone);
            assert_eq!(a.nodes(), b.nodes());
            if a.nodes() == 0 {
                continue;
            }
            found += 1;
            let mut ids = std::collections::HashSet::new();
            for (bp, placed) in &a.blocks {
                assert!(placed.windows(2).all(|w| w[0].index < w[1].index));
                for p in placed.iter() {
                    ids.insert(p.id);
                    let (x, y, z) = (p.index as i32 % 16, p.index as i32 / 16 % 16, p.index as i32 / 256);
                    let node = (bp.0 as i32 * 16 + x, bp.1 as i32 * 16 + y, bp.2 as i32 * 16 + z);
                    // Within the mapchunk's dungeon box.
                    let min = box_min((c, -3, 7));
                    assert!((0..BOX.0).contains(&(node.0 - min.0)) && (0..BOX.1).contains(&(node.1 - min.1)) && (0..BOX.2).contains(&(node.2 - min.2)));
                }
            }
            // Walls of the biome's kind: cobblestone, or sandstone brick in the
            // sandstone desert.
            assert!(ids.contains(&CONTENT_AIR) && ids.iter().any(|&id| is_wall(id)), "{ids:?}");
            assert!(a.rooms.len() >= 2);
        }
        assert!(found > 3, "only {found} of 40 mapchunks had dungeons");
    }

    /// In air there's nowhere for them.
    #[test]
    fn no_dungeons_in_the_air() {
        let mut air = |_: NodePos| CONTENT_AIR;
        for c in 0..10 {
            assert_eq!(generate((c, -3, 7), &mut air).nodes(), 0);
        }
    }

    #[test]
    fn loot() {
        for name in loot_names() {
            assert!(content::id_of(name).is_some(), "{name}");
        }
        let mut kinds = std::collections::HashSet::new();
        let mut empty = 0;
        for i in 0..200 {
            let slots = loot_for((i * 7, -600, i), Kind::Normal);
            if slots.iter().all(|s| s.is_none()) {
                empty += 1;
            }
            for s in slots.iter().flatten() {
                kinds.insert(s.id);
                if content::is_tool(s.id) {
                    assert!(s.count == 1 && (13107..=49151).contains(&s.wear));
                }
            }
        }
        assert!(empty < 5);
        // Deep down: obsidian and mese blocks, no sand or dirt.
        assert!(kinds.contains(&content::id_of("esp:obsidian").unwrap()));
        assert!(!kinds.contains(&content::CONTENT_DIRT));
        // Near the surface, sand and dirt; desert cobble only in the desert.
        let near: Vec<u16> = (0..100).flat_map(|i| loot_for((i, -10, 3), Kind::Normal)).flatten().map(|s| s.id).collect();
        assert!(near.contains(&content::CONTENT_DIRT) && !near.contains(&content::CONTENT_DESERT_COBBLE));
        let desert: Vec<u16> = (0..100).flat_map(|i| loot_for((i, -10, 3), Kind::Desert)).flatten().map(|s| s.id).collect();
        assert!(desert.contains(&content::CONTENT_DESERT_COBBLE));
        // The icesheet's: snow, and no dirt or sand.
        let ice: Vec<u16> = (0..100).flat_map(|i| loot_for((i, -10, 3), Kind::Ice)).flatten().map(|s| s.id).collect();
        assert!(ice.contains(&content::CONTENT_SNOW_LAYER) && !ice.contains(&content::CONTENT_DIRT) && !ice.contains(&content::CONTENT_SAND));
        // The same chest always holds the same.
        assert_eq!(loot_for((5, -20, 9), Kind::Normal), loot_for((5, -20, 9), Kind::Normal));
    }
}
