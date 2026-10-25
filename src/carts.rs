// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates
// Based on minetest_game's carts mod (mods/carts: cart_entity.lua,
// functions.lua, rails.lua), itself based on boost_cart:
//   Copyright (C) 2012-2016 PilzAdam
//   Copyright (C) 2014-2016 SmallJoker
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

//! Carts and rails, as minetest_game's carts mod has them.
//!
//! - Rails (plain, powered and brake) lie on the ground and join up with
//!   their neighbours (raillike): straight, curved, T-junctions and
//!   crossings, and slopes of one node.
//! - A cart is put on a rail, pushed by punching it (the way the player
//!   looks, harder for a full swing), ridden (right-click), and picked up
//!   with a sneak-punch. It follows the rails: up and down slopes (slowing
//!   on the way up), round curves, and at a junction straight on, or left
//!   or right with the left and right keys held, else whichever way there
//!   is. Back is a handbrake. It slows on its own (0.4 n/s/s), a powered
//!   rail speeds it up (5), a brake rail slows it (3); 7 nodes a second at
//!   most. At the end of the line it turns back.
//! - A cart punched picks up the dropped items it is on, and carries them
//!   (items.rs keeps which).
//! - The world moves it as Luanti moves an entity that isn't physical: by
//!   its velocity and acceleration, through anything; the cart's on_step
//!   keeps it on the rails, looking ahead for when it skipped a node.
//!
//! The pure parts are here; the server seats and unseats the driver, and
//! tells clients.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use log::{info, warn};

use crate::content;
use crate::messages::{self, Animation, Props};
use crate::storage;

/// The rails: plain, powered (carts:powerrail) and brake (carts:brakerail).
pub const RAIL: u16 = content::CONTENT_DIRT_DRY_GRASS + 1;
pub const POWERRAIL: u16 = RAIL + 1;
pub const BRAKERAIL: u16 = RAIL + 2;
pub const RAILS: [u16; 3] = [RAIL, POWERRAIL, BRAKERAIL];
/// minetest.raillike_group("rail"): rails join each other, not gunpowder
/// (whose group is tnt::RAILLIKE_GROUP).
pub const RAILLIKE_GROUP: i16 = 2;

/// Carts' object ids, just below the boats'.
pub const FIRST_ID: u16 = crate::boats::FIRST_ID - 0x100;
pub const LAST_ID: u16 = crate::boats::FIRST_ID - 1;
/// How many carts the world can hold (minetest_game has no limit).
pub const MAX_CARTS: usize = if cfg!(pc) { 256 } else { 32 };

/// carts.speed_max and punch_speed_max (nodes per second), and
/// path_distance_max (nodes).
pub const SPEED_MAX: f32 = 7.0;
pub const PUNCH_SPEED_MAX: f32 = 5.0;
const PATH_DISTANCE_MAX: i32 = 3;

/// Where the driver stands (set_attach), and how far their eyes are
/// lowered (set_eye_offset), in the cart's own units.
pub const SEAT: [f32; 3] = [0.0, -4.5, 0.0];
pub const EYES: [f32; 3] = [0.0, -4.0, 0.0];
pub const MESH: &str = "carts_cart.b3d";
pub const TEXTURE: &str = "carts_cart.png";
pub const ENTITY: &str = "carts:cart";
pub const SOUND: &str = "carts_cart_moving";

/// The physics step, as the boats'.
const STEP: f32 = 0.05;
/// How long clients take to glide to a cart's new place (Luanti sends
/// its server step, dedicated_server_step: 0.09 s).
const STEP_SENT: f32 = 0.09;
/// Clients are told where a moving cart is at least this often.
const RESYNC: f32 = 1.0;
const SAVE_DELAY: Duration = Duration::from_secs(10);

pub fn is_rail_node(id: u16) -> bool {
    RAILS.contains(&id)
}

/// A rail's acceleration (railparams): a powered rail's 5, a brake rail's
/// -3, else none.
pub fn acceleration(id: u16) -> f32 {
    match id {
        POWERRAIL => 5.0,
        BRAKERAIL => -3.0,
        _ => 0.0,
    }
}

/// What the carts need to know of the world.
pub trait Rails {
    fn node(&self, p: [i32; 3]) -> u16;
}

/// The driver's keys that a cart heeds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    pub left: bool,
    pub right: bool,
    pub down: bool,
}

impl Controls {
    /// From the key bits of TOSERVER_PLAYERPOS (bit 1 down, 2 left, 3
    /// right).
    pub fn from_keys(keys: u32) -> Self {
        Self { down: keys & 2 != 0, left: keys & 4 != 0, right: keys & 8 != 0 }
    }
}

type V = [f32; 3];
type D = [i32; 3];

const NONE: D = [0, 0, 0];

/// vector.round: halves up.
fn round(p: V) -> D {
    p.map(|v| (v + 0.5).floor() as i32)
}

fn to_v(d: D) -> V {
    d.map(|v| v as f32)
}

fn add(a: D, b: D) -> D {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn len(v: V) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// carts:get_sign.
fn sign(z: f32) -> i32 {
    if z == 0.0 {
        0
    } else if z > 0.0 {
        1
    } else {
        -1
    }
}

/// carts:velocity_to_dir: the axis it mostly goes along, and up or down.
pub fn velocity_to_dir(v: V) -> D {
    if v[0].abs() > v[2].abs() {
        [sign(v[0]), sign(v[1]), 0]
    } else {
        [0, sign(v[1]), sign(v[2])]
    }
}

/// carts:is_rail: a rail, of `railtype` if given.
fn is_rail(t: &dyn Rails, p: D, railtype: Option<i16>) -> bool {
    let id = t.node(p);
    if !is_rail_node(id) {
        return false;
    }
    railtype.map_or(true, |r| content::group_of(id, "connect_to_raillike") == r)
}

/// carts:check_front_up_down: a rail ahead, or (if `check_up`) a node up,
/// or one down; the way to it.
fn check_front_up_down(t: &dyn Rails, pos: D, dir: D, check_up: bool, railtype: Option<i16>) -> Option<D> {
    let mut dir = dir;
    dir[1] = 0;
    if is_rail(t, add(pos, dir), railtype) {
        return Some(dir);
    }
    if check_up {
        dir[1] = 1;
        if is_rail(t, add(pos, dir), railtype) {
            return Some(dir);
        }
    }
    dir[1] = -1;
    if is_rail(t, add(pos, dir), railtype) {
        return Some(dir);
    }
    None
}

/// carts:get_rail_direction: which way a cart going `dir` at `pos` goes
/// on: straight on (straight on first on a slope, with someone in it),
/// left or right if the driver holds that key (the key then ignored on
/// the next node: `old_switch`), else straight, left, right, and (unless
/// switching) back. Also the key that switched it (1 left, 2 right).
pub fn rail_direction(t: &dyn Rails, pos: V, dir: D, ctrl: Option<Controls>, old_switch: Option<u8>, railtype: Option<i16>) -> (D, Option<u8>) {
    let pos = round(pos);
    let (mut left_check, mut right_check) = (true, true);
    let mut left = NONE;
    let mut right = NONE;
    if dir[2] != 0 && dir[0] == 0 {
        left[0] = -dir[2];
        right[0] = dir[2];
    } else if dir[0] != 0 && dir[2] == 0 {
        left[2] = dir[0];
        right[2] = -dir[0];
    }
    let straight_priority = ctrl.is_some() && dir[1] != 0;
    // Normal, to disallow rail switching up- and downhill.
    if straight_priority {
        if let Some(cur) = check_front_up_down(t, pos, dir, true, railtype) {
            return (cur, None);
        }
    }
    if let Some(ctrl) = ctrl {
        if old_switch == Some(1) {
            left_check = false;
        } else if old_switch == Some(2) {
            right_check = false;
        }
        if ctrl.left && left_check {
            if let Some(cur) = check_front_up_down(t, pos, left, false, railtype) {
                return (cur, Some(1));
            }
            left_check = false;
        }
        if ctrl.right && right_check {
            if let Some(cur) = check_front_up_down(t, pos, right, false, railtype) {
                return (cur, Some(2));
            }
            right_check = true;
        }
    }
    if !straight_priority {
        if let Some(cur) = check_front_up_down(t, pos, dir, true, railtype) {
            return (cur, None);
        }
    }
    if left_check {
        if let Some(cur) = check_front_up_down(t, pos, left, false, railtype) {
            return (cur, None);
        }
    }
    if right_check {
        if let Some(cur) = check_front_up_down(t, pos, right, false, railtype) {
            return (cur, None);
        }
    }
    if old_switch.is_none() {
        if let Some(cur) = check_front_up_down(t, pos, [-dir[0], dir[1], -dir[2]], true, railtype) {
            return (cur, None);
        }
    }
    (NONE, None)
}

/// carts:pathfinder: after a step that took the cart more than a node
/// from `old_pos`, follow the rails from there as far as it went (at
/// most 3 nodes): None if they lead where it is; else where they lead
/// (and which way), for it to be put there.
fn pathfinder(t: &dyn Rails, pos: V, old_pos: D, old_dir: D, distance: f32, ctrl: Option<Controls>, switch: Option<u8>, railtype: Option<i16>) -> Option<(D, D)> {
    let pos = round(pos);
    if old_pos == pos {
        return None;
    }
    let mut pf_pos = old_pos;
    let mut pf_dir = old_dir;
    let mut pf_switch = switch;
    let distance = PATH_DISTANCE_MAX.min((distance + 1.0).floor() as i32);
    for _ in 0..distance.max(0) {
        let (d, s) = rail_direction(t, to_v(pf_pos), pf_dir, ctrl, Some(pf_switch.unwrap_or(0)), railtype);
        pf_dir = d;
        pf_switch = s;
        if pf_dir == NONE {
            // No way forwards.
            return Some((pf_pos, pf_dir));
        }
        pf_pos = add(pf_pos, pf_dir);
        if pf_pos == pos {
            return None;
        }
    }
    Some((pf_pos, pf_dir))
}

#[derive(Clone, Debug)]
pub struct Cart {
    pub id: u16,
    /// Nodes.
    pub pos: V,
    /// Nodes per second.
    pub vel: V,
    pub acc: V,
    /// The rails it runs on (connect_to_raillike), once pushed.
    pub railtype: Option<i16>,
    pub old_dir: D,
    old_pos: Option<D>,
    old_switch: Option<u8>,
    /// Pushed: the velocity added on the next step.
    punched: bool,
    punch_vel: V,
    pub driver: Option<u16>,
    /// Facing (radians) and tilt (the model's frame: 0 flat, 1 down, 2 up).
    pub yaw: f32,
    pub frame: f32,
    since_told: f32,
    /// Clients are to put it where it is at once (it changed direction:
    /// set_pos), not glide there (move_to).
    jump: bool,
    /// Solid to players: stopped (a moving cart isn't physical, or it
    /// sticks going uphill; minetest_game's never is).
    pub solid: bool,
    sound_ttl: Option<f32>,
    /// Its rolling sound is playing.
    sounding: bool,
}

/// What a step did, for the server to pass on.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Tell clients where it is and how it moves.
    Moved(u16),
    /// It tilted or turned; round a corner as it went, by this many
    /// radians, anticlockwise (else 0).
    Turned(u16, f32),
    /// It stopped (solid) or started (not).
    Solid(u16),
    /// Time to renew its rolling sound: at this gain, or stop it (None).
    Sound(u16, Option<f32>),
    /// It was pushed: it picks up the items it is on.
    Collect(u16),
}

impl Cart {
    pub fn new(id: u16, pos: V) -> Self {
        Self {
            id,
            pos,
            vel: [0.0; 3],
            acc: [0.0; 3],
            railtype: None,
            old_dir: [1, 0, 0],
            old_pos: None,
            old_switch: Some(0),
            punched: false,
            punch_vel: [0.0; 3],
            driver: None,
            yaw: 0.0,
            frame: 0.0,
            since_told: 0.0,
            jump: false,
            solid: true,
            sound_ttl: None,
            sounding: false,
        }
    }

    pub fn pos_bs(&self) -> V {
        self.pos.map(|v| v * 10.0)
    }

    /// on_punch, by a player: unless the driver punches it going fast,
    /// it is pushed the way the player looks (along the rails, if they
    /// go that way), by 2 n/s for a full swing (`swing`, 0 to 1). Not on
    /// a rail, or stopped, it learns which rails it runs on first.
    pub fn punch(&mut self, t: &dyn Rails, by: u16, look: V, swing: f32) {
        if self.railtype.is_none() || self.vel == [0.0; 3] {
            self.railtype = Some(content::group_of(t.node(round(self.pos)), "connect_to_raillike"));
        }
        if self.driver == Some(by) && (self.vel[0] + self.vel[2]).abs() > PUNCH_SPEED_MAX {
            return;
        }
        let mut punch_dir = velocity_to_dir(look);
        punch_dir[1] = 0;
        let (cart_dir, _) = rail_direction(t, self.pos, punch_dir, None, None, self.railtype);
        if cart_dir == NONE {
            return;
        }
        let f = 2.0 * swing.clamp(0.0, 1.0);
        self.punch_vel = to_v(cart_dir).map(|v| v * f);
        self.old_dir = cart_dir;
        self.punched = true;
    }

    /// Luanti's movement of an entity that isn't physical: on by its
    /// velocity and acceleration, through anything.
    fn travel(&mut self, dt: f32) {
        let mut time = 0.0;
        while time < dt {
            let h = STEP.min(dt - time);
            time += h;
            for i in 0..3 {
                self.pos[i] += (self.vel[i] + self.acc[i] * 0.5 * h) * h;
                self.vel[i] += self.acc[i] * h;
            }
        }
    }

    /// One step of `dt` seconds: moved on as it was going, then the cart's
    /// on_step (rail_on_step and rail_sound).
    pub fn step(&mut self, dt: f32, ctrl: Option<Controls>, t: &dyn Rails, events: &mut Vec<Event>) {
        if !(self.vel == [0.0; 3] && self.acc == [0.0; 3]) {
            self.travel(dt);
        }
        self.since_told += dt;
        self.on_rails(dt, ctrl, t, events);
        self.sound(dt, events);
        let solid = self.vel == [0.0; 3];
        if solid != self.solid {
            self.solid = solid;
            events.push(Event::Solid(self.id));
        }
        if self.vel != [0.0; 3] && self.since_told >= RESYNC {
            self.tell(events, false);
        }
    }

    /// rail_sound: every second, the rolling sound again, louder the
    /// faster it goes.
    fn sound(&mut self, dt: f32, events: &mut Vec<Event>) {
        match self.sound_ttl {
            None => {
                self.sound_ttl = Some(1.0);
                return;
            }
            Some(ttl) if ttl > 0.0 => {
                self.sound_ttl = Some(ttl - dt);
                return;
            }
            _ => {}
        }
        self.sound_ttl = Some(1.0);
        let speed = len(self.vel);
        if speed > 0.0 || self.sounding {
            self.sounding = speed > 0.0;
            events.push(Event::Sound(self.id, (speed > 0.0).then_some(speed / SPEED_MAX / 2.0)));
        }
    }

    /// rail_on_step.
    fn on_rails(&mut self, dt: f32, ctrl: Option<Controls>, t: &dyn Rails, events: &mut Vec<Event>) {
        let mut vel = self.vel;
        if self.punched {
            for i in 0..3 {
                vel[i] += self.punch_vel[i];
            }
            self.vel = vel;
            self.old_dir[1] = 0;
        } else if vel == [0.0; 3] {
            return;
        }
        let punched = self.punched;
        let mut pos = self.pos;
        let mut dir = velocity_to_dir(vel);
        let dir_changed = dir != self.old_dir;
        let (mut update_pos, mut update_vel) = (false, false);
        if let Some(old) = self.old_pos {
            if !punched && !dir_changed && round(pos) == old {
                // Don't look at one node more than once.
                return;
            }
        }
        let mut stop_wiggle = false;
        match self.old_pos {
            Some(old) if !dir_changed => {
                // Skipped nodes? Follow the rails as far as it went.
                let distance = dt * (len(vel) + 0.5 * dt * len(self.acc));
                if let Some((new_pos, new_dir)) = pathfinder(t, pos, old, self.old_dir, distance, ctrl, self.old_switch, self.railtype) {
                    // No rail found: put it where it should be.
                    pos = to_v(new_pos);
                    update_pos = true;
                    dir = new_dir;
                }
            }
            Some(_) if self.old_dir[1] != 1 && !punched => stop_wiggle = true,
            _ => {}
        }
        let (d, switch_keys) = rail_direction(t, pos, dir, ctrl, self.old_switch, self.railtype);
        dir = d;
        let dir_changed = dir != self.old_dir;
        let mut acc = 0.0;
        if stop_wiggle || dir == NONE {
            dir = self.old_dir;
            vel = [0.0; 3];
            let pos_r = round(pos);
            match self.old_pos {
                Some(old) if !is_rail(t, pos_r, self.railtype) => pos = to_v(old),
                _ if !stop_wiggle => {
                    // End of the rail: smooth out.
                    pos = to_v(pos_r);
                    dir[1] = 0;
                }
                _ => pos[1] = (pos[1] + 0.5).floor(),
            }
            update_pos = true;
            update_vel = true;
        } else {
            if dir_changed {
                let speed = (vel[0] + vel[2]).abs();
                vel = to_v(dir).map(|v| v * speed);
                update_vel = true;
                if dir[1] != self.old_dir[1] {
                    pos = to_v(round(pos));
                    update_pos = true;
                }
            }
            // Centred on the rail.
            if dir[2] != 0 && (pos[0] + 0.5).floor() != pos[0] {
                pos[0] = (pos[0] + 0.5).floor();
                update_pos = true;
            }
            if dir[0] != 0 && (pos[2] + 0.5).floor() != pos[2] {
                pos[2] = (pos[2] + 0.5).floor();
                update_pos = true;
            }
            // Slower uphill, faster down; a powered or brake rail, the
            // handbrake, or coasting.
            acc = dir[1] as f32 * -4.0;
            let speed_mod = acceleration(t.node(round(pos)));
            if speed_mod != 0.0 {
                acc += speed_mod;
            } else if ctrl.is_some_and(|c| c.down) {
                acc -= 3.0;
            } else {
                acc -= 0.4;
            }
        }
        // At most SPEED_MAX.
        let vel_len = len(vel);
        if vel_len > SPEED_MAX {
            vel = vel.map(|v| v * SPEED_MAX / vel_len);
            update_vel = true;
        }
        if vel_len >= SPEED_MAX && acc > 0.0 {
            acc = 0.0;
        }
        let new_acc = to_v(dir).map(|v| v * acc);
        let acc_changed = new_acc != self.acc;
        self.acc = new_acc;
        self.old_pos = Some(round(pos));
        self.old_dir = dir;
        self.old_switch = switch_keys;
        if punched {
            events.push(Event::Collect(self.id));
            self.punched = false;
            update_vel = true;
        }
        if !(update_vel || update_pos) {
            if acc_changed {
                self.tell(events, false);
            }
            return;
        }
        let yaw = if dir[0] < 0 {
            0.5
        } else if dir[0] > 0 {
            1.5
        } else if dir[2] < 0 {
            1.0
        } else {
            0.0
        } * std::f32::consts::PI;
        let frame = match dir[1] {
            -1 => 1.0,
            1 => 2.0,
            _ => 0.0,
        };
        if (yaw, frame) != (self.yaw, self.frame) {
            let pi = std::f32::consts::PI;
            let by = (yaw - self.yaw + 3.0 * pi).rem_euclid(2.0 * pi) - pi;
            self.yaw = yaw;
            self.frame = frame;
            // Round a corner, as it goes: a quarter turn. (Pushed off,
            // or turned back at the end of the line, it isn't a turn its
            // rider takes.)
            let corner = !punched && (by.abs() - pi / 2.0).abs() < 0.01;
            events.push(Event::Turned(self.id, if corner { by } else { 0.0 }));
        }
        if update_vel {
            self.vel = vel;
        }
        if update_pos {
            self.pos = pos;
        }
        // As the mod: set_pos when it changed direction, else move_to.
        self.tell(events, update_pos && dir_changed);
    }

    fn tell(&mut self, events: &mut Vec<Event>, jump: bool) {
        self.since_told = 0.0;
        self.jump = jump;
        events.push(Event::Moved(self.id));
    }
}

/// AO_CMD_SET_PROPERTIES for a cart (its initial_properties, but solid
/// when stopped; the rest Luanti's defaults).
pub fn properties(c: &Cart) -> Vec<u8> {
    messages::properties_message(&Props {
        hp_max: 10,
        physical: c.solid,
        collisionbox: [-0.5, -0.5, -0.5, 0.5, 0.5, 0.5],
        selectionbox: [-0.5, -0.5, -0.5, 0.5, 0.5, 0.5],
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

/// Where a cart is and how it moves, for clients.
pub fn position(c: &Cart) -> Vec<u8> {
    let glide = (!c.jump).then_some(STEP_SENT);
    messages::moved_message(c.pos_bs(), c.vel.map(|v| v * 10.0), c.acc.map(|v| v * 10.0), c.yaw.to_degrees(), glide)
}

/// Its tilt: the model's frame for flat, downhill or uphill.
pub fn animation(c: &Cart) -> Vec<u8> {
    messages::set_animation_message(Animation { start: c.frame, end: c.frame, fps: 1.0 })
}

/// What a client needs to show a cart.
pub fn init_data(c: &Cart) -> Vec<u8> {
    let messages = [properties(c), messages::immortal_message(), animation(c), messages::not_attached_message(), position(c)];
    messages::entity_init_data(ENTITY, c.id, c.pos_bs(), c.yaw.to_degrees(), 10, &messages)
}

pub fn is_cart_id(id: u16) -> bool {
    (FIRST_ID..=LAST_ID).contains(&id)
}

/// The world's carts, saved at a path.
pub struct Carts {
    list: Vec<Cart>,
    next_id: u16,
    path: Option<PathBuf>,
    changed: Option<Instant>,
}

impl Default for Carts {
    fn default() -> Self {
        Self { list: Vec::new(), next_id: FIRST_ID, path: None, changed: None }
    }
}

impl Carts {
    /// The carts saved at `path` (one per line: x y z, velocity, the way
    /// it last went, its rails or "-"). Its staticdata keeps the last two;
    /// Luanti's the rest.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut carts = Self { path: Some(path.clone()), ..Self::default() };
        let text = match storage::read(&path) {
            Ok(text) => String::from_utf8_lossy(&text).into_owned(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return carts,
            Err(e) => {
                warn!("Reading {}: {e}", path.display());
                return carts;
            }
        };
        for line in text.lines() {
            match parse(line) {
                Some((pos, vel, dir, railtype)) if carts.list.len() < MAX_CARTS => {
                    let id = carts.new_id();
                    let mut c = Cart::new(id, pos);
                    c.vel = vel;
                    c.old_dir = dir;
                    c.railtype = railtype;
                    carts.list.push(c);
                }
                Some(_) => {}
                None if line.trim().is_empty() => {}
                None => warn!("Skipping cart line \"{line}\""),
            }
        }
        if !carts.list.is_empty() {
            info!("{} carts", carts.list.len());
        }
        carts
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

    /// A cart put on the rail at `pos`; None if there are as many as
    /// there can be.
    pub fn add(&mut self, pos: [i32; 3]) -> Option<u16> {
        if self.list.len() >= MAX_CARTS {
            return None;
        }
        let id = self.new_id();
        self.list.push(Cart::new(id, to_v(pos)));
        self.touch();
        Some(id)
    }

    pub fn remove(&mut self, id: u16) -> Option<Cart> {
        let i = self.list.iter().position(|c| c.id == id)?;
        self.touch();
        Some(self.list.remove(i))
    }

    pub fn get(&self, id: u16) -> Option<&Cart> {
        self.list.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: u16) -> Option<&mut Cart> {
        self.list.iter_mut().find(|c| c.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Cart> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Step every cart; `controls` gives a driver's keys.
    pub fn step(&mut self, dt: f32, t: &dyn Rails, controls: &dyn Fn(u16) -> Option<Controls>) -> Vec<Event> {
        let mut events = Vec::new();
        for c in &mut self.list {
            let ctrl = c.driver.and_then(controls);
            let before = c.pos;
            c.step(dt, ctrl, t, &mut events);
            if c.pos != before {
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
        for c in &self.list {
            let [x, y, z] = c.pos;
            let [vx, vy, vz] = c.vel;
            let [dx, dy, dz] = c.old_dir;
            let rails = c.railtype.map_or("-".to_string(), |r| r.to_string());
            text.push_str(&format!("{x:.2} {y:.2} {z:.2} {vx:.2} {vy:.2} {vz:.2} {dx} {dy} {dz} {rails}\n"));
        }
        if let Err(e) = storage::write(path, text.as_bytes()) {
            warn!("Saving {}: {e}", path.display());
            crate::status::problem();
        }
    }
}

fn parse(line: &str) -> Option<(V, V, D, Option<i16>)> {
    let w: Vec<&str> = line.split_whitespace().collect();
    if w.len() != 10 {
        return None;
    }
    let f: Vec<f32> = w[..6].iter().map(|s| s.parse().ok().filter(|v: &f32| v.is_finite())).collect::<Option<_>>()?;
    let d: Vec<i32> = w[6..9].iter().map(|s| s.parse().ok().filter(|v: &i32| (-1..=1).contains(v))).collect::<Option<_>>()?;
    let railtype = if w[9] == "-" { None } else { Some(w[9].parse().ok()?) };
    Some(([f[0], f[1], f[2]], [f[3], f[4], f[5]], [d[0], d[1], d[2]], railtype))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Rails laid out by hand; everything else is air.
    struct Track(HashMap<D, u16>);

    impl Track {
        fn new(rails: &[(D, u16)]) -> Self {
            Self(rails.iter().copied().collect())
        }
        /// A straight line of plain rail along x from `from` to `to`, at
        /// y 0, z 0.
        fn line(from: i32, to: i32) -> Vec<(D, u16)> {
            (from..=to).map(|x| ([x, 0, 0], RAIL)).collect()
        }
    }

    impl Rails for Track {
        fn node(&self, p: D) -> u16 {
            self.0.get(&p).copied().unwrap_or(crate::world::CONTENT_AIR)
        }
    }

    fn run(c: &mut Cart, t: &Track, secs: f32, ctrl: Option<Controls>) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..(secs / 0.05).round() as usize {
            c.step(0.05, ctrl, t, &mut events);
        }
        events
    }

    #[test]
    fn directions() {
        assert_eq!(velocity_to_dir([2.0, 0.0, 0.5]), [1, 0, 0]);
        assert_eq!(velocity_to_dir([0.1, -1.0, -3.0]), [0, -1, -1]);
        // A T-junction: from x -1 going +x at (0,0,0), rails on to +x and
        // to +z (left, facing +x, is +z).
        let t = Track::new(&[([-1, 0, 0], RAIL), ([0, 0, 0], RAIL), ([1, 0, 0], RAIL), ([0, 0, 1], RAIL)]);
        let go = |ctrl| rail_direction(&t, [0.0; 3], [1, 0, 0], ctrl, None, None);
        assert_eq!(go(None), ([1, 0, 0], None));
        assert_eq!(go(Some(Controls { left: true, ..Default::default() })), ([0, 0, 1], Some(1)));
        assert_eq!(go(Some(Controls { right: true, ..Default::default() })), ([1, 0, 0], None));
        // A slope: a rail a node up ahead.
        let t = Track::new(&[([0, 0, 0], RAIL), ([1, 1, 0], RAIL)]);
        assert_eq!(rail_direction(&t, [0.0; 3], [1, 0, 0], None, None, None).0, [1, 1, 0]);
        // The end of the line: back the way it came, unless it switched.
        let t = Track::new(&[([-1, 0, 0], RAIL), ([0, 0, 0], RAIL)]);
        assert_eq!(rail_direction(&t, [0.0; 3], [1, 0, 0], None, None, None).0, [-1, 0, 0]);
        assert_eq!(rail_direction(&t, [0.0; 3], [1, 0, 0], None, Some(0), None).0, NONE);
        // Gunpowder isn't a rail; another rail type isn't this one.
        let t = Track::new(&[([0, 0, 0], RAIL), ([1, 0, 0], crate::tnt::GUNPOWDER)]);
        assert_eq!(rail_direction(&t, [0.0; 3], [1, 0, 0], None, Some(0), None).0, NONE);
        let t = Track::new(&[([0, 0, 0], RAIL), ([1, 0, 0], RAIL)]);
        assert_eq!(rail_direction(&t, [0.0; 3], [1, 0, 0], None, Some(0), Some(RAILLIKE_GROUP + 1)).0, NONE);
    }

    #[test]
    fn pushed_along() {
        let t = Track::new(&Track::line(0, 30));
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        // Punched looking +x, a full swing: 2 n/s, coasting to a stop
        // (0.4 n/s/s: 5 nodes) on the rails, centred on them.
        c.punch(&t, 2, [1.0, -0.2, 0.1], 1.0);
        assert_eq!(c.railtype, Some(RAILLIKE_GROUP));
        let events = run(&mut c, &t, 1.0, None);
        assert!(events.contains(&Event::Collect(FIRST_ID)));
        assert!((c.vel[0] - 1.6).abs() < 0.1 && c.vel[2] == 0.0, "{:?}", c.vel);
        assert_eq!(c.yaw, 1.5 * std::f32::consts::PI);
        run(&mut c, &t, 10.0, None);
        assert_eq!(c.vel, [0.0; 3]);
        assert!((c.pos[0] - 5.0).abs() < 0.6 && c.pos[1] == 0.0 && c.pos[2] == 0.0, "{:?}", c.pos);
        // Punched from the side, it goes along the rails anyway, to the
        // left first (as minetest_game's get_rail_direction).
        let mut c = Cart::new(FIRST_ID, [3.0, 0.0, 0.0]);
        c.punch(&t, 2, [0.0, 0.0, 1.0], 1.0);
        run(&mut c, &t, 1.0, None);
        assert!(c.pos[0] < 3.0, "{:?}", c.pos);
        // On a lone rail, with nowhere to go, it doesn't move.
        let lone = Track::new(&[([0, 0, 0], RAIL)]);
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.punch(&lone, 2, [1.0, 0.0, 0.0], 1.0);
        run(&mut c, &lone, 1.0, None);
        assert_eq!((c.pos, c.vel), ([0.0; 3], [0.0; 3]));
    }

    #[test]
    fn powered_braked_and_capped() {
        // Powered rail all the way: up to 7 n/s and no faster.
        let mut rails = Track::line(0, 200);
        for r in &mut rails {
            r.1 = POWERRAIL;
        }
        let t = Track::new(&rails);
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.punch(&t, 2, [1.0, 0.0, 0.0], 1.0);
        run(&mut c, &t, 6.0, None);
        assert!(c.vel[0] > 6.5 && c.vel[0] <= SPEED_MAX + 0.01, "{:?}", c.vel);
        // Then brake rail: it stops sooner than coasting would.
        let mut rails = Track::line(0, 60);
        for r in rails.iter_mut().skip(1) {
            r.1 = BRAKERAIL;
        }
        let t = Track::new(&rails);
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.punch(&t, 2, [1.0, 0.0, 0.0], 1.0);
        c.punch_vel = [6.0, 0.0, 0.0];
        run(&mut c, &t, 10.0, None);
        assert_eq!(c.vel, [0.0; 3]);
        assert!(c.pos[0] < 8.0, "{:?}", c.pos);
        // The handbrake: back held.
        let t = Track::new(&Track::line(0, 60));
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.driver = Some(2);
        c.punch(&t, 3, [1.0, 0.0, 0.0], 1.0);
        c.punch_vel = [6.0, 0.0, 0.0];
        run(&mut c, &t, 10.0, Some(Controls { down: true, ..Default::default() }));
        assert!(c.pos[0] < 8.0 && c.vel == [0.0; 3], "{:?}", c.pos);
    }

    #[test]
    fn slopes_curves_and_junctions() {
        // Up a slope: along x to 5, then up a node at a time to (8, 3).
        let mut rails = Track::line(0, 5);
        rails.extend([([6, 1, 0], RAIL), ([7, 2, 0], RAIL), ([8, 3, 0], RAIL)]);
        rails.extend((9..=20).map(|x| ([x, 3, 0], RAIL)));
        let t = Track::new(&rails);
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.punch(&t, 2, [1.0, 0.0, 0.0], 1.0);
        c.punch_vel = [7.0, 0.0, 0.0];
        let events = run(&mut c, &t, 3.0, None);
        assert!(c.pos[1] > 2.9 && c.pos[0] > 8.5, "{:?}", c.pos);
        assert!(events.iter().any(|e| matches!(e, Event::Turned(FIRST_ID, _))) && c.frame == 0.0);
        // A curve: along x to 3, then +z: a quarter turn anticlockwise
        // (pushed off, from facing +z to +x, isn't one). Stopped, it is
        // solid; moving, not.
        let mut rails = Track::line(0, 3);
        rails.extend((1..=10).map(|z| ([3, 0, z], RAIL)));
        let t = Track::new(&rails);
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.punch(&t, 2, [1.0, 0.0, 0.0], 1.0);
        c.punch_vel = [4.0, 0.0, 0.0];
        assert!(c.solid);
        let events = run(&mut c, &t, 2.0, None);
        assert!(c.pos[0] == 3.0 && c.pos[2] > 2.0, "{:?}", c.pos);
        assert!(c.vel[2] > 0.0 && c.vel[0] == 0.0 && c.yaw == 0.0 && !c.solid);
        let turns: Vec<f32> = events.iter().filter_map(|e| match e {
            Event::Turned(_, by) if *by != 0.0 => Some(*by),
            _ => None,
        }).collect();
        assert!(turns.len() == 1 && (turns[0] - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{turns:?}");
        assert!(events.contains(&Event::Solid(FIRST_ID)));
        // A junction: straight on, or left with the key held.
        let mut rails = Track::line(0, 12);
        rails.extend((1..=10).map(|z| ([5, 0, z], RAIL)));
        let t = Track::new(&rails);
        for (ctrl, along_z) in [(Controls::default(), false), (Controls { left: true, ..Default::default() }, true)] {
            let mut c = Cart::new(FIRST_ID, [0.0; 3]);
            c.driver = Some(2);
            c.punch(&t, 3, [1.0, 0.0, 0.0], 1.0);
            c.punch_vel = [4.0, 0.0, 0.0];
            run(&mut c, &t, 2.5, Some(ctrl));
            assert_eq!(c.pos[2] > 1.0, along_z, "{ctrl:?} {:?}", c.pos);
        }
        // Coming back the other way (-x), right takes it onto the branch.
        let mut c = Cart::new(FIRST_ID, [7.0, 0.0, 0.0]);
        c.driver = Some(2);
        c.punch(&t, 2, [-1.0, 0.0, 0.0], 1.0);
        run(&mut c, &t, 2.0, Some(Controls { right: true, ..Default::default() }));
        assert!(c.pos[0] == 5.0 && c.pos[2] > 1.0, "{:?}", c.pos);
        // The same after a first push the other way, as in the network
        // test: track x 0 to 7 with the branch at 4.
        let mut rails = Track::line(0, 7);
        rails.extend((1..=2).map(|z| ([4, 0, z], RAIL)));
        let t = Track::new(&rails);
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.punch(&t, 2, [1.0, 0.0, 0.0], 1.0);
        run(&mut c, &t, 10.0, None);
        c.driver = Some(2);
        c.punch(&t, 2, [-1.0, 0.0, 0.0], 1.0);
        let right = Some(Controls { right: true, ..Default::default() });
        let mut furthest: f32 = 0.0;
        for _ in 0..40 {
            c.step(0.05, right, &t, &mut Vec::new());
            furthest = furthest.max(c.pos[2]);
        }
        // Onto the branch (and, as it is short, off its end and back).
        assert!(furthest > 1.0, "{furthest}");
    }

    #[test]
    fn end_of_the_line_and_sound() {
        let t = Track::new(&Track::line(0, 4));
        let mut c = Cart::new(FIRST_ID, [0.0; 3]);
        c.punch(&t, 2, [1.0, 0.0, 0.0], 1.0);
        c.punch_vel = [7.0, 0.0, 0.0];
        // At the end of the line it turns back (backwards is the last way
        // get_rail_direction tries), and so on until it stops.
        let mut events = run(&mut c, &t, 1.0, None);
        assert!(c.vel[0] < 0.0 && c.pos[0] <= 4.0, "{:?} {:?}", c.pos, c.vel);
        events.extend(run(&mut c, &t, 30.0, None));
        assert_eq!(c.vel, [0.0; 3]);
        assert!((0.0..=4.0).contains(&c.pos[0]) && c.pos[1] == 0.0 && c.pos[2] == 0.0, "{:?}", c.pos);
        let sounds: Vec<_> = events.iter().filter_map(|e| match e {
            Event::Sound(_, g) => Some(*g),
            _ => None,
        }).collect();
        assert!(sounds.first().is_some_and(|g| g.is_some()) && sounds.last() == Some(&None), "{sounds:?}");
    }

    #[test]
    fn saving() {
        let dir = std::env::temp_dir().join(format!("carts-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("carts.txt");
        let mut carts = Carts::open(&path);
        let a = carts.add([1, 2, 3]).unwrap();
        carts.get_mut(a).unwrap().vel = [0.0, 0.0, -2.5];
        carts.get_mut(a).unwrap().old_dir = [0, 0, -1];
        carts.get_mut(a).unwrap().railtype = Some(RAILLIKE_GROUP);
        carts.save();
        let again = Carts::open(&path);
        let c = again.iter().next().unwrap();
        assert!(is_cart_id(c.id) && LAST_ID < crate::boats::FIRST_ID);
        assert_eq!((c.pos, c.vel, c.old_dir, c.railtype), ([1.0, 2.0, 3.0], [0.0, 0.0, -2.5], [0, 0, -1], Some(RAILLIKE_GROUP)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
