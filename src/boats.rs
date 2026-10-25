// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates
// Based on minetest_game's boats mod (mods/boats/init.lua):
//   Copyright (C) 2012-2016 PilzAdam
//   Copyright (C) 2012-2016 Various Minetest Game developers and contributors
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

//! Boats, as minetest_game's boats mod has them: a wooden boat placed on
//! water, which a player right-clicks to sit in and steers with the
//! movement keys (forward and back speed it up and slow it down, left and
//! right turn it; forward and back together set it cruising), and punches
//! to pick up again.
//!
//! - On water it floats at the surface; out of it, it falls, and on the
//!   ground it stops.
//! - Water drags on it, so it slows to a stop on its own; its top speed,
//!   with forward held, is about 5 nodes a second.
//! - The world moves it as Luanti's server moves a physical entity: by its
//!   velocity and acceleration, stopped by solid nodes.
//!
//! The pure parts are here; the server seats and unseats the driver, and
//! tells clients where the boats are.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use log::{info, warn};

use crate::content;
use crate::messages::{self, Props};
use crate::mobs::{move_axis, Terrain};
use crate::storage;

/// Boats' object ids, just below the mobs' (players have those below).
pub const FIRST_ID: u16 = crate::mobs::FIRST_ID - 0x100;
pub const LAST_ID: u16 = crate::mobs::FIRST_ID - 1;
/// How many boats the world can hold (minetest_game has no limit).
pub const MAX_BOATS: usize = if cfg!(pc) { 256 } else { 32 };

/// The boat entity's collision box (do not move its top, the mod warns:
/// lower, and a boat under water falls through the world).
pub const BOX: [f32; 6] = [-0.5, -0.35, -0.5, 0.5, 0.3, 0.5];
/// Where the driver sits, in the boat's own units (set_attach).
pub const SEAT: [f32; 3] = [0.5, 1.0, -3.0];
pub const MESH: &str = "boats_boat.obj";
/// The planks it is made of ("default_wood.png").
pub const TEXTURE: &str = "esp_apple_wood.png";
pub const ENTITY: &str = "boats:boat";

/// Speeding up or slowing down, nodes per second per second.
const THRUST: f32 = 2.0;
/// Turning, radians per second.
const TURN: f32 = 0.9;
/// The physics step (as the mobs').
const STEP: f32 = 0.05;
/// Clients are told where a moving boat is at least this often.
const RESYNC: f32 = 0.2;
const SAVE_DELAY: Duration = Duration::from_secs(10);

/// The driver's movement keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

impl Controls {
    /// From the key bits of TOSERVER_PLAYERPOS (bit 0-3: up, down, left,
    /// right).
    pub fn from_keys(keys: u32) -> Self {
        Self { up: keys & 1 != 0, down: keys & 2 != 0, left: keys & 4 != 0, right: keys & 8 != 0 }
    }
}

#[derive(Clone, Debug)]
pub struct Boat {
    pub id: u16,
    /// Nodes.
    pub pos: [f32; 3],
    /// Nodes per second.
    pub vel: [f32; 3],
    pub acc: [f32; 3],
    /// Radians (Luanti's: 0 faces +z, increasing anticlockwise from above).
    pub yaw: f32,
    /// Speed along the way it faces (negative: going backwards).
    pub v: f32,
    /// The driver's peer (and object) id.
    pub driver: Option<u16>,
    /// Cruising: going on as if forward were held.
    pub auto: bool,
    /// Clients' idea of it, to decide when to tell them again.
    told: ([f32; 3], [f32; 3], f32),
    since_told: f32,
}

/// What a step did, for the server to pass on.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Tell clients where it is and how it moves.
    Moved(u16),
    /// Cruise mode turned on (true) or off, for the driver.
    Cruise { driver: u16, on: bool },
}

/// Luanti's math.sign: 0 for 0.
fn sign(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// The velocity of speed `v` facing `yaw`, going up at `y` (get_velocity).
pub fn velocity(v: f32, yaw: f32, y: f32) -> [f32; 3] {
    [-yaw.sin() * v, y, yaw.cos() * v]
}

fn is_water(t: &dyn Terrain, p: [f32; 3]) -> bool {
    content::group_of(t.node(node_at(p)), "water") > 0
}

fn node_at(p: [f32; 3]) -> (i32, i32, i32) {
    (p[0].round() as i32, p[1].round() as i32, p[2].round() as i32)
}

impl Boat {
    pub fn new(id: u16, pos: [f32; 3], yaw: f32) -> Self {
        Self { id, pos, vel: [0.0; 3], acc: [0.0; 3], yaw, v: 0.0, driver: None, auto: false, told: (pos, [0.0; 3], yaw), since_told: 0.0 }
    }

    pub fn pos_bs(&self) -> [f32; 3] {
        self.pos.map(|v| v * 10.0)
    }

    /// Whether it lies still: nobody in it, not moving.
    fn still(&self) -> bool {
        self.driver.is_none() && self.v == 0.0 && self.vel == [0.0; 3]
    }

    /// Luanti's physics for a physical entity: its velocity changed by its
    /// acceleration, and moved by it, stopped by solid nodes.
    /// Returns whether it came down on something.
    fn travel(&mut self, dt: f32, t: &dyn Terrain) -> bool {
        let size = (BOX[3], BOX[4] - BOX[1], -BOX[1]);
        let mut landed = false;
        let mut time = 0.0;
        while time < dt {
            let step = STEP.min(dt - time);
            time += step;
            for axis in 0..3 {
                self.vel[axis] += self.acc[axis] * step;
            }
            for axis in [1, 0, 2] {
                let delta = self.vel[axis] * step;
                if delta != 0.0 && move_axis(&mut self.pos, axis, delta, size, t) {
                    self.vel[axis] = 0.0;
                    landed |= axis == 1 && delta < 0.0;
                }
            }
        }
        landed
    }

    /// One step of `dt` seconds: moved as it was going, then the boat's
    /// on_step, steered by `ctrl` if someone is in it.
    pub fn step(&mut self, dt: f32, ctrl: Option<Controls>, t: &dyn Terrain, events: &mut Vec<Event>) {
        let landed = !self.still() && self.travel(dt, t);
        self.since_told += dt;
        self.v = (self.vel[0].powi(2) + self.vel[2].powi(2)).sqrt() * sign(self.v);
        if let (Some(driver), Some(c)) = (self.driver, ctrl) {
            if c.up && c.down {
                if !self.auto {
                    self.auto = true;
                    events.push(Event::Cruise { driver, on: true });
                }
            } else if c.down {
                self.v -= dt * THRUST;
                if self.auto {
                    self.auto = false;
                    events.push(Event::Cruise { driver, on: false });
                }
            } else if c.up || self.auto {
                self.v += dt * THRUST;
            }
            // Backwards, left and right turn it the other way.
            let back = self.v < -0.001;
            if c.left {
                self.yaw += if back { -dt * TURN } else { dt * TURN };
            } else if c.right {
                self.yaw += if back { dt * TURN } else { -dt * TURN };
            }
        }
        if self.still() {
            self.acc = [0.0; 3];
            self.tell_if_due(events);
            return;
        }
        // Drag, keeping the sign: past it, it stops.
        let drag = dt * sign(self.v) * (0.01 + 0.0796 * self.v * self.v);
        if self.v.abs() <= drag.abs() {
            self.v = 0.0;
        } else {
            self.v -= drag;
        }
        let below = [self.pos[0], self.pos[1] - 0.5, self.pos[2]];
        if !is_water(t, below) {
            if content::walkable(t.node(node_at(below))) {
                // Aground: stopped, and pushed up out of it. (Come to rest
                // on the ground with nobody in it, it lies still, rather
                // than bobbing on the push for ever.)
                self.v = 0.0;
                self.acc = [0.0, 1.0, 0.0];
                if landed && self.driver.is_none() {
                    self.acc = [0.0; 3];
                    self.vel = [0.0; 3];
                    self.tell_if_due(events);
                    return;
                }
            } else {
                self.acc = [0.0, -9.8, 0.0];
            }
            self.vel = velocity(self.v, self.yaw, self.vel[1]);
        } else if is_water(t, [below[0], below[1] + 1.0, below[2]]) {
            // Under water: up, faster the faster it sinks.
            let mut y = self.vel[1];
            if y >= 5.0 {
                y = 5.0;
                self.acc = [0.0; 3];
            } else if y < 0.0 {
                self.acc = [0.0, 20.0, 0.0];
            } else {
                self.acc = [0.0, 5.0, 0.0];
            }
            self.vel = velocity(self.v, self.yaw, y);
        } else {
            // At the surface: there it floats, unless it is going up or
            // down fast.
            self.acc = [0.0; 3];
            if self.vel[1].abs() < 1.0 {
                self.pos[1] = self.pos[1].floor() + 0.5;
                self.vel = velocity(self.v, self.yaw, 0.0);
            } else {
                self.vel = velocity(self.v, self.yaw, self.vel[1]);
            }
        }
        self.tell_if_due(events);
    }

    /// Clients follow a boat by its velocity and acceleration, so they are
    /// told when those or its heading change, and now and then while it
    /// moves, so they don't drift from it.
    fn tell_if_due(&mut self, events: &mut Vec<Event>) {
        let (_, vel, yaw) = self.told;
        let changed = (0..3).any(|i| (vel[i] - self.vel[i]).abs() > 0.2) || (yaw - self.yaw).abs() > 0.08;
        let moving = self.vel != [0.0; 3];
        if changed || (moving && self.since_told >= RESYNC) || (!moving && self.told.0 != self.pos) {
            self.told = (self.pos, self.vel, self.yaw);
            self.since_told = 0.0;
            events.push(Event::Moved(self.id));
        }
    }
}

/// AO_CMD_SET_PROPERTIES for a boat (its initial_properties; the rest
/// Luanti's defaults).
fn properties() -> Vec<u8> {
    messages::properties_message(&Props {
        hp_max: 10,
        physical: true,
        collisionbox: BOX,
        selectionbox: BOX,
        visual: "mesh",
        visual_size: [1.0; 3],
        textures: &[TEXTURE],
        makes_footstep_sound: false,
        automatic_rotate: 0.0,
        mesh: MESH,
        collide_with_objects: true,
        stepheight: 0.0,
        infotext: "",
        wield_item: "",
        glow: 0,
        eye_height: 1.625,
        damage_texture_modifier: "^[brighten",
        zoom_fov: 0.0,
        spritediv: [1, 1],
        face_movement: None,
        use_texture_alpha: false,
    })
}

/// Where a boat is and how it moves, for clients.
pub fn position(b: &Boat) -> Vec<u8> {
    messages::object_position_message(b.pos_bs(), b.vel.map(|v| v * 10.0), b.acc.map(|v| v * 10.0), b.yaw.to_degrees())
}

/// What a client needs to show a boat.
pub fn init_data(b: &Boat) -> Vec<u8> {
    let messages = [properties(), messages::immortal_message(), messages::not_attached_message(), position(b)];
    messages::entity_init_data(ENTITY, b.id, b.pos_bs(), b.yaw.to_degrees(), 10, &messages)
}

pub fn is_boat_id(id: u16) -> bool {
    (FIRST_ID..=LAST_ID).contains(&id)
}

/// The world's boats, saved at a path.
pub struct Boats {
    list: Vec<Boat>,
    next_id: u16,
    path: Option<PathBuf>,
    changed: Option<Instant>,
}

impl Default for Boats {
    fn default() -> Self {
        Self { list: Vec::new(), next_id: FIRST_ID, path: None, changed: None }
    }
}

impl Boats {
    /// The boats saved at `path` (one per line: x y z yaw).
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut boats = Self { path: Some(path.clone()), ..Self::default() };
        let text = match storage::read(&path) {
            Ok(text) => String::from_utf8_lossy(&text).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return boats,
            Err(e) => {
                warn!("Reading {}: {e}", path.display());
                return boats;
            }
        };
        for line in text.lines() {
            let v: Vec<f32> = line.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            if v.len() == 4 && v.iter().all(|x| x.is_finite()) && boats.list.len() < MAX_BOATS {
                let id = boats.new_id();
                boats.list.push(Boat::new(id, [v[0], v[1], v[2]], v[3]));
            } else if !line.trim().is_empty() {
                warn!("Skipping boat line \"{line}\"");
            }
        }
        if !boats.list.is_empty() {
            info!("{} boats", boats.list.len());
        }
        boats
    }

    fn new_id(&mut self) -> u16 {
        loop {
            let id = self.next_id;
            self.next_id = if id >= LAST_ID { FIRST_ID } else { id + 1 };
            if self.get(id).is_none() {
                return id;
            }
        }
    }

    /// A boat put on the water at `pos` facing `yaw`; None if there are
    /// as many as there can be.
    pub fn add(&mut self, pos: [f32; 3], yaw: f32) -> Option<u16> {
        if self.list.len() >= MAX_BOATS {
            return None;
        }
        let id = self.new_id();
        self.list.push(Boat::new(id, pos, yaw));
        self.touch();
        Some(id)
    }

    pub fn remove(&mut self, id: u16) -> Option<Boat> {
        let i = self.list.iter().position(|b| b.id == id)?;
        self.touch();
        Some(self.list.remove(i))
    }

    pub fn get(&self, id: u16) -> Option<&Boat> {
        self.list.iter().find(|b| b.id == id)
    }

    pub fn get_mut(&mut self, id: u16) -> Option<&mut Boat> {
        self.list.iter_mut().find(|b| b.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Boat> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Step every boat that moves or has someone in it; `controls` gives a
    /// driver's keys.
    pub fn step(&mut self, dt: f32, t: &dyn Terrain, controls: &dyn Fn(u16) -> Option<Controls>) -> Vec<Event> {
        let mut events = Vec::new();
        for b in &mut self.list {
            let ctrl = b.driver.and_then(controls);
            let before = b.pos;
            b.step(dt, ctrl, t, &mut events);
            if b.pos != before {
                self.changed.get_or_insert_with(Instant::now);
            }
        }
        events
    }

    fn touch(&mut self) {
        self.changed.get_or_insert_with(Instant::now);
    }

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
        for b in &self.list {
            let [x, y, z] = b.pos;
            text.push_str(&format!("{x:.2} {y:.2} {z:.2} {:.3}\n", b.yaw));
        }
        if let Err(e) = storage::write(path, text.as_bytes()) {
            warn!("Saving {}: {e}", path.display());
            crate::status::problem();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::NodePos;

    /// A lake: water up to y 0 where x >= 0, stone below and beside it,
    /// air above.
    struct Lake;

    impl Terrain for Lake {
        fn solid(&self, p: NodePos) -> bool {
            content::walkable(self.node(p))
        }
        fn liquid(&self, p: NodePos) -> bool {
            content::is_liquid(self.node(p))
        }
        fn opaque(&self, _: NodePos) -> bool {
            false
        }
        fn node(&self, p: NodePos) -> u16 {
            if p.1 < -3 || (p.0 < 0 && p.1 <= 0) {
                content::CONTENT_STONE
            } else if p.1 <= 0 {
                content::CONTENT_WATER
            } else {
                crate::world::CONTENT_AIR
            }
        }
        fn top(&self, _: i32, _: i32) -> i32 {
            0
        }
        fn light(&self, _: NodePos) -> u8 {
            0
        }
    }

    fn run(b: &mut Boat, secs: f32, ctrl: Option<Controls>) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..(secs / 0.05) as usize {
            b.step(0.05, ctrl, &Lake, &mut events);
        }
        events
    }

    #[test]
    fn floats_and_stays() {
        // Put on the water (at the water node + 0.5), it floats there.
        let mut b = Boat::new(FIRST_ID, [5.0, 0.5, 5.0], 0.0);
        let events = run(&mut b, 3.0, None);
        assert_eq!(b.pos, [5.0, 0.5, 5.0]);
        assert!(events.is_empty(), "{events:?}");
        // Dropped in from above, it bobs, and comes to float at the
        // surface.
        let mut b = Boat::new(FIRST_ID, [5.0, 3.0, 5.0], 0.0);
        b.vel[1] = -0.1;
        run(&mut b, 10.0, None);
        assert!((b.pos[1] - 0.5).abs() < 0.01 && b.vel[1] == 0.0, "{:?} {:?}", b.pos, b.vel);
        // Under water, it rises to the surface.
        let mut b = Boat::new(FIRST_ID, [5.0, -2.0, 5.0], 0.0);
        b.vel[1] = -0.1;
        run(&mut b, 10.0, None);
        assert!((b.pos[1] - 0.5).abs() < 0.01, "{:?}", b.pos);
    }

    #[test]
    fn steering() {
        let fwd = Some(Controls { up: true, ..Default::default() });
        let mut b = Boat::new(FIRST_ID, [50.0, 0.5, 5.0], 0.0);
        b.driver = Some(7);
        // Forward speeds it up along +z (yaw 0) to about 5 n/s.
        run(&mut b, 20.0, fwd);
        assert!(b.v > 4.5 && b.v < 5.1, "{}", b.v);
        assert!(b.vel[0].abs() < 1e-3 && b.vel[2] > 4.5);
        // Let go, it slows to a stop.
        let z = b.pos[2];
        run(&mut b, 60.0, Some(Controls::default()));
        assert_eq!(b.v, 0.0);
        assert!(b.pos[2] > z + 5.0);
        // Left turns it anticlockwise: the yaw grows.
        let yaw = b.yaw;
        run(&mut b, 1.0, Some(Controls { left: true, up: true, ..Default::default() }));
        assert!((b.yaw - yaw - 0.9).abs() < 0.06, "{}", b.yaw - yaw);
        // Forward and back together: cruising, on without the keys.
        let events = run(&mut b, 0.1, Some(Controls { up: true, down: true, ..Default::default() }));
        assert!(events.contains(&Event::Cruise { driver: 7, on: true }));
        let v = b.v;
        run(&mut b, 2.0, Some(Controls::default()));
        assert!(b.auto && b.v > v, "{} {} {}", b.auto, b.v, v);
        let events = run(&mut b, 0.1, Some(Controls { down: true, ..Default::default() }));
        assert!(events.contains(&Event::Cruise { driver: 7, on: false }) && !b.auto);
    }

    #[test]
    fn runs_aground() {
        // Heading for the shore (x < 0) it stops against it.
        let mut b = Boat::new(FIRST_ID, [3.0, 0.5, 5.0], std::f32::consts::FRAC_PI_2);
        b.driver = Some(7);
        run(&mut b, 10.0, Some(Controls { up: true, ..Default::default() }));
        assert!(b.pos[0].abs() < 0.01, "{:?}", b.pos);
        // On land it falls to the ground and stops.
        let mut b = Boat::new(FIRST_ID, [-5.0, 3.0, 5.0], 0.0);
        b.vel[1] = -0.1;
        run(&mut b, 5.0, None);
        assert!((b.pos[1] - 0.85).abs() < 0.01, "{:?}", b.pos);
        // ... and lies still there.
        assert!(b.still() && run(&mut b, 2.0, None).is_empty());
    }

    #[test]
    fn ids_and_saving() {
        let dir = std::env::temp_dir().join(format!("boats-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("boats.txt");
        let mut boats = Boats::open(&path);
        let a = boats.add([1.0, 0.5, 2.0], 1.5).unwrap();
        let b = boats.add([3.0, 0.5, 4.0], 0.0).unwrap();
        assert!(is_boat_id(a) && is_boat_id(b) && a != b);
        assert!(LAST_ID < crate::mobs::FIRST_ID);
        boats.remove(a);
        boats.save();
        let again = Boats::open(&path);
        assert_eq!(again.len(), 1);
        let only = again.iter().next().unwrap();
        assert_eq!((only.pos, only.yaw), ([3.0, 0.5, 4.0], 0.0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
