// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2026 Keir Finlow-Bates
// Translated from Luanti's dropped item entity (builtin/game/item_entity.lua):
//   Copyright (C) 2010-2024 celeron55, Perttu Ahola <celeron55@gmail.com>, and Luanti contributors
// Licensed under the GNU Lesser General Public License, version 2.1 or (at
// your option) any later version: see COPYING.LESSER.

//! Dropped items lying in the world, like builtin's "__builtin:item"
//! entity: thrown by a player (or knocked off a wall), they fall and come to
//! rest on the ground, merge with matching items nearby, and are picked up
//! by punching them. They disappear after 15 minutes.
//!
//! They are kept over a restart in `world/items.txt`, as builtin's item
//! entity keeps its item and age in its static data: one line each,
//!   <x> <y> <z> <age> <item string>
//! rewritten a few seconds after they change. After a restart they fall
//! from where they were, so they settle wherever the ground now is.
//!
//! Clients move a falling item themselves (it is a physical object with
//! gravity); the server moves its own copy the same way so it knows where
//! the item ends up, and tells clients when it lands.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use log::{info, warn};

use crate::inventory::{Meta, Stack};
use crate::messages::{item_size, GRAVITY};
use crate::storage;

/// How soon after a change the items are saved: often enough to lose
/// little when the plug is pulled, seldom enough to spare the SD card
/// while items are being thrown around.
const SAVE_DELAY: Duration = Duration::from_secs(10);

/// builtin's default item_entity_ttl, in seconds.
pub const TIME_TO_LIVE: f32 = 900.0;
/// At most this many items at once; the oldest goes when another drops.
pub const MAX_ITEMS: usize = 128;
/// Object ids for items, well clear of the player ids (which are peer ids).
const FIRST_ID: u16 = 0x8000;
/// Physics time step, in seconds.
const STEP: f32 = 0.02;
/// Resting items this close (in nodes) with room for each other merge.
const MERGE_RADIUS: f32 = 1.0;
/// Items that fall this far are lost.
const LOWEST: f32 = -2000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Dropped {
    pub id: u16,
    pub item: u16,
    pub count: u16,
    /// A tool's wear.
    pub wear: u16,
    /// Its metadata (a written book's).
    pub meta: Meta,
    /// Centre, in nodes.
    pub pos: [f32; 3],
    /// Nodes per second.
    pub vel: [f32; 3],
    pub falling: bool,
    pub age: f32,
}

impl Dropped {
    pub fn stack(&self) -> Stack {
        Stack { id: self.item, count: self.count, wear: self.wear, meta: self.meta }
    }
    pub fn pos_bs(&self) -> [f32; 3] {
        self.pos.map(|v| v * 10.0)
    }
    pub fn vel_bs(&self) -> [f32; 3] {
        self.vel.map(|v| v * 10.0)
    }
}

/// What clients need to be told after a step.
#[derive(Debug, PartialEq)]
pub enum Change {
    /// New position and velocity (it landed, or started falling).
    Moved(u16),
    /// New count, so new properties.
    Resized(u16),
    Removed(u16),
}

pub struct Items {
    list: Vec<Dropped>,
    next_id: u16,
    /// Where they are saved (None: not saved).
    path: Option<PathBuf>,
    /// When they first changed since the last save.
    changed: Option<Instant>,
}

impl Items {
    pub fn new() -> Self {
        Self { list: Vec::new(), next_id: FIRST_ID, path: None, changed: None }
    }

    /// The items saved at `path`. They start falling, so call `settle`
    /// once the world is there to lift any that are now inside a block.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut items = Self { path: Some(path.clone()), ..Self::new() };
        let text = match storage::read(&path) {
            Ok(text) => String::from_utf8_lossy(&text).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return items,
            Err(e) => {
                warn!("Reading {}: {e}", path.display());
                return items;
            }
        };
        for line in text.lines() {
            match parse(line) {
                Some((pos, age, stack)) if items.list.len() < MAX_ITEMS => {
                    let id = items.next_id;
                    items.next_id += 1;
                    let (item, count, wear, meta) = (stack.id, stack.count, stack.wear, stack.meta);
                    items.list.push(Dropped { id, item, count, wear, meta, pos, vel: [0.0; 3], falling: true, age });
                }
                Some(_) => {}
                None => warn!("Skipping item line \"{line}\""),
            }
        }
        if !items.list.is_empty() {
            info!("{} dropped items", items.list.len());
        }
        items
    }

    /// Lift items out of anything solid they are in (built over while
    /// the server was off, or a save from before).
    pub fn settle(&mut self, solid: &dyn Fn(i32, i32, i32) -> bool) {
        for d in &mut self.list {
            lift_out(&mut d.pos, item_size(d.count), solid);
        }
    }

    /// Save, if the items changed SAVE_DELAY or more ago. Returns whether
    /// it saved.
    pub fn save_if_due(&mut self) -> bool {
        let due = self.changed.is_some_and(|t| t.elapsed() >= SAVE_DELAY);
        if due {
            self.save();
        }
        due
    }

    pub fn save(&mut self) {
        self.changed = None;
        let Some(path) = &self.path else {
            return;
        };
        let mut text = String::new();
        for d in &self.list {
            let [x, y, z] = d.pos;
            text.push_str(&format!("{x:.2} {y:.2} {z:.2} {:.0} {}\n", d.age, d.stack().item_string()));
        }
        if let Err(e) = storage::write(path, text.as_bytes()) {
            warn!("Saving {}: {e}", path.display());
            crate::status::problem();
        }
    }

    fn touch(&mut self) {
        self.changed.get_or_insert_with(Instant::now);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Dropped> {
        self.list.iter()
    }

    pub fn get(&self, id: u16) -> Option<&Dropped> {
        self.list.iter().find(|d| d.id == id)
    }

    fn get_mut(&mut self, id: u16) -> Option<&mut Dropped> {
        self.list.iter_mut().find(|d| d.id == id)
    }

    /// Set an item moving at `vel` (nodes per second), as a blast throws
    /// it. Returns whether there is such an item.
    pub fn throw(&mut self, id: u16, vel: [f32; 3]) -> bool {
        let Some(d) = self.get_mut(id) else {
            return false;
        };
        d.vel = vel;
        d.falling = true;
        true
    }

    /// Add an item stack at `pos` (nodes) moving at `vel`. If it starts
    /// inside something solid it is lifted out. Returns its id, and the id
    /// of the oldest item if that had to go to make room.
    pub fn add(&mut self, stack: Stack, motion: ([f32; 3], [f32; 3]), solid: &dyn Fn(i32, i32, i32) -> bool) -> (u16, Option<u16>) {
        let evicted = if self.list.len() >= MAX_ITEMS {
            Some(self.list.remove(0).id)
        } else {
            None
        };
        // The next id that isn't still on the ground (going round after
        // u16::MAX; there are at most MAX_ITEMS, so one is always free).
        let id = loop {
            let id = self.next_id;
            self.next_id = if id == u16::MAX { FIRST_ID } else { id + 1 };
            if self.get(id).is_none() {
                break id;
            }
        };
        let (mut pos, vel) = motion;
        lift_out(&mut pos, item_size(stack.count), solid);
        let (item, count, wear, meta) = (stack.id, stack.count, stack.wear, stack.meta);
        self.list.push(Dropped { id, item, count, wear, meta, pos, vel, falling: true, age: 0.0 });
        self.touch();
        (id, evicted)
    }

    pub fn remove(&mut self, id: u16) -> Option<Dropped> {
        let i = self.list.iter().position(|d| d.id == id)?;
        self.touch();
        Some(self.list.remove(i))
    }

    pub fn set_count(&mut self, id: u16, count: u16) {
        if let Some(d) = self.get_mut(id) {
            d.count = count;
            self.touch();
        }
    }

    /// The node at `node` changed: items resting on or near it start
    /// falling again (they stop at once if still supported).
    pub fn wake_near(&mut self, node: [i32; 3]) -> Vec<u16> {
        let mut woken = Vec::new();
        for d in &mut self.list {
            let near = (0..3).all(|i| (d.pos[i] - node[i] as f32).abs() < 1.5);
            if near && !d.falling {
                d.falling = true;
                woken.push(d.id);
            }
        }
        woken
    }

    /// Move falling items on by `dt` seconds, age everything, and merge
    /// items that came to rest next to matching ones.
    pub fn step(&mut self, dt: f32, solid: &dyn Fn(i32, i32, i32) -> bool) -> Vec<Change> {
        let mut changes = Vec::new();
        let dt = dt.min(0.5);
        let mut landed = Vec::new();
        for d in &mut self.list {
            d.age += dt;
            if !d.falling {
                continue;
            }
            let half = item_size(d.count);
            let mut t = 0.0;
            while t < dt {
                let h = STEP.min(dt - t);
                t += h;
                d.vel[1] -= GRAVITY * h;
                let mut on_ground = false;
                for axis in [1, 0, 2] {
                    let delta = d.vel[axis] * h;
                    if delta != 0.0 && move_axis(&mut d.pos, axis, delta, half, solid) {
                        on_ground |= axis == 1 && delta < 0.0;
                        d.vel[axis] = 0.0;
                    }
                }
                if on_ground {
                    // As builtin does on touching the ground: stop dead.
                    d.vel = [0.0; 3];
                    d.falling = false;
                    landed.push(d.id);
                    break;
                }
            }
        }
        let before = self.list.len();
        let mut gone = Vec::new();
        self.list.retain(|d| {
            let keep = d.age < TIME_TO_LIVE && d.pos[1] > LOWEST;
            if !keep {
                gone.push(d.id);
            }
            keep
        });
        if self.list.len() != before {
            landed.retain(|id| !gone.contains(id));
        }
        changes.extend(gone.into_iter().map(Change::Removed));
        for id in landed {
            changes.push(Change::Moved(id));
            changes.extend(self.merge_into(id));
        }
        if !changes.is_empty() {
            self.touch();
        }
        changes
    }

    /// builtin's try_merge_with: absorb resting items of the same kind
    /// within MERGE_RADIUS, while the stack has room.
    fn merge_into(&mut self, id: u16) -> Vec<Change> {
        let mut changes = Vec::new();
        loop {
            let Some(me) = self.get(id).cloned() else {
                break;
            };
            let other = self.list.iter().find(|o| {
                o.id != id
                    && !o.falling
                    && o.item == me.item
                    && o.wear == me.wear
                    && o.meta == me.meta
                    && o.count + me.count <= crate::content::stack_max(me.item)
                    && (0..3).map(|i| (o.pos[i] - me.pos[i]).powi(2)).sum::<f32>() <= MERGE_RADIUS * MERGE_RADIUS
            });
            let Some(other) = other.map(|o| (o.id, o.count, o.pos)) else {
                break;
            };
            self.remove(other.0);
            changes.push(Change::Removed(other.0));
            if let Some(d) = self.get_mut(id) {
                // builtin moves the merged item to where the other one was,
                // lifted a little as it grows.
                let old = item_size(d.count);
                d.count += other.1;
                d.pos = other.2;
                d.pos[1] += item_size(d.count) - old;
                d.age = 0.0;
            }
            changes.push(Change::Resized(id));
            changes.push(Change::Moved(id));
        }
        changes
    }
}

/// Lift an item of half size `half` out of solid nodes it is in, up to
/// three nodes.
fn lift_out(pos: &mut [f32; 3], half: f32, solid: &dyn Fn(i32, i32, i32) -> bool) {
    for _ in 0..3 {
        if !overlaps(*pos, half, solid) {
            break;
        }
        pos[1] = (pos[1] + 0.5).floor() + 0.5 + half + 1e-3;
    }
}

/// A saved item line: position, age and stack.
fn parse(line: &str) -> Option<([f32; 3], f32, Stack)> {
    let mut f = line.splitn(5, ' ');
    let mut num = || f.next()?.parse::<f32>().ok();
    let pos = [num()?, num()?, num()?];
    let age = num()?;
    let stack = Stack::parse(f.next()?)?;
    pos.iter().all(|v| v.is_finite()).then_some((pos, age, stack))
}

/// The range of node coordinates a box from `min` to `max` overlaps. Node
/// n spans n - 0.5 to n + 0.5.
fn node_range(min: f32, max: f32) -> (i32, i32) {
    ((min + 0.5).floor() as i32, (max + 0.5).ceil() as i32 - 1)
}

fn overlaps(pos: [f32; 3], half: f32, solid: &dyn Fn(i32, i32, i32) -> bool) -> bool {
    let r: [(i32, i32); 3] = std::array::from_fn(|i| node_range(pos[i] - half, pos[i] + half));
    (r[0].0..=r[0].1).any(|x| (r[1].0..=r[1].1).any(|y| (r[2].0..=r[2].1).any(|z| solid(x, y, z))))
}

/// Move a box of half size `half` at `pos` by `delta` along one axis,
/// stopping against solid nodes. Returns whether it hit one.
fn move_axis(pos: &mut [f32; 3], axis: usize, delta: f32, half: f32, solid: &dyn Fn(i32, i32, i32) -> bool) -> bool {
    let old = pos[axis];
    let mut new = *pos;
    new[axis] += delta;
    let r: [(i32, i32); 3] = std::array::from_fn(|i| node_range(new[i] - half, new[i] + half));
    let mut hit: Option<i32> = None;
    for x in r[0].0..=r[0].1 {
        for y in r[1].0..=r[1].1 {
            for z in r[2].0..=r[2].1 {
                if solid(x, y, z) {
                    let n = [x, y, z][axis];
                    hit = Some(match hit {
                        None => n,
                        Some(h) if delta < 0.0 => h.max(n),
                        Some(h) => h.min(n),
                    });
                }
            }
        }
    }
    match hit {
        None => {
            *pos = new;
            false
        }
        Some(n) => {
            // Up against the nearest solid node, but never backwards.
            let snap = if delta < 0.0 { n as f32 + 0.5 + half + 1e-4 } else { n as f32 - 0.5 - half - 1e-4 };
            pos[axis] = if delta < 0.0 { snap.clamp(new[axis], old) } else { snap.clamp(old, new[axis]) };
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::*;

    #[test]
    fn saved_over_a_restart() {
        let dir = std::env::temp_dir().join(format!("esp_items_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("items.txt");
        let _ = std::fs::remove_file(&path);
        // The ground is everything below y = 0.5.
        let ground = |_: i32, y: i32, _: i32| y <= 0;
        let mut items = Items::open(&path);
        items.add(Stack::new(CONTENT_COBBLE, 7), ([1.0, 3.0, 2.0], [0.0; 3]), &ground);
        items.add(Stack { wear: 1234, ..Stack::new(TOOL_PICK_STONE, 1) }, ([4.0, 3.0, 2.0], [0.0; 3]), &ground);
        for _ in 0..50 {
            items.step(0.1, &ground);
        }
        assert!(items.changed.is_some());
        items.save();
        assert!(items.changed.is_none());
        let before: Vec<(u16, u16, u16)> = items.iter().map(|d| (d.item, d.count, d.wear)).collect();

        // Since then a block was built where the cobblestone lies.
        let built = |x: i32, y: i32, z: i32| y <= 0 || (x, y, z) == (1, 1, 2);
        let mut again = Items::open(&path);
        again.settle(&built);
        let after: Vec<(u16, u16, u16)> = again.iter().map(|d| (d.item, d.count, d.wear)).collect();
        assert_eq!(before, after);
        let cobble = again.iter().next().unwrap();
        assert!(cobble.pos[1] > 1.5 && cobble.falling && (cobble.age - 5.0).abs() < 1.0);
        // They settle back on the ground (or the new block).
        for _ in 0..20 {
            again.step(0.1, &built);
        }
        assert!(again.iter().all(|d| !d.falling));
        assert!(parse("1 2 x 0 esp:cobble").is_none() && parse("1 2 3 0 esp:nothing 4").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
