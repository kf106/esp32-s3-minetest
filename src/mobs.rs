// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates

//! Hostile mobs: not in minetest_game, so off unless an admin or moderator
//! turns them on (`/mobs on`; see settings.rs). There are two:
//! - the Mossling, a squat creature of bark and moss that hunts in packs
//!   of two or three at night near trees, and sinks back into the earth
//!   at dawn;
//! - the Wisp, a floating orb of pale blue flame that hangs about at a
//!   distance, then rushes a player, flickers faster with a rising whine
//!   for a moment and bursts: a small blast that sets things alight
//!   (unless TNT is off), like a little TNT. A player who backs off in
//!   time stops it. It fades away at dawn, and water puts it out.
//!
//! The server decides; the client draws. A mob is a GenericCAO using the
//! player model (character.b3d), small (visual_size), physical, turning
//! to face the way it moves (automatic_face_movement_dir). It is sent a
//! position, velocity and gravity only when its velocity changes (and now
//! and then to keep it right), and the client moves it on in between,
//! colliding with the nodes as the server does.
//!
//! The server's mob is a box (half_width by height) that falls, stops
//! against nodes (taken as whole cubes, and fences, walls and closed
//! gates as a node and a half), jumps up one node, and won't walk off a
//! drop of more than three nodes or into water. Four times a second each
//! one thinks: it wanders, or once it sees a player (a line of sight
//! from 0.8 above its feet, past leaves but not solid nodes), it and its
//! pack chase them, the others flanking (aiming at points to either side),
//! and swipe at them when close. One badly hurt with no pack near flees.
//!
//! A wisp is a sprite (8 frames of mobs_wisp.png, the client cycling
//! them), glowing, with no gravity: it flies straight where it wants,
//! stopping against nodes and rising over them, keeping a few nodes above
//! the ground while it wanders.
//!
//! They spawn only in the dark (night, and torch light of 7 or less),
//! 24 to 48 nodes from players, over a solid floor with two nodes of air
//! above (near trees, for mosslings), up to PER_PLAYER near each player
//! (WISPS_PER_PLAYER of them wisps) and MAX_MOBS in all. They go when no player is within DESPAWN nodes, and aren't kept
//! over a restart.

use std::time::Duration;

use crate::content::{self, ToolCaps};
use crate::messages::{self, Animation, Props};
use crate::nature::Rng;
use crate::world::NodePos;

/// Object ids for mobs: above the players' (peer ids) and below the
/// dropped items' (0x8000 on).
pub const FIRST_ID: u16 = 0x6000;
const LAST_ID: u16 = 0x7FFF;

/// The most mobs at once, and near each player (within NEAR nodes).
pub const MAX_MOBS: usize = if cfg!(pc) { 64 } else { 16 };
pub const PER_PLAYER: usize = if cfg!(pc) { 12 } else { 8 };
pub const WISPS_PER_PLAYER: usize = 2;
/// How loud a wisp's hum is.
pub const HUM_GAIN: f32 = 0.35;
const NEAR: f32 = 64.0;
/// How often spawning is tried, and how far from players.
pub const SPAWN_EVERY: Duration = Duration::from_secs(4);
const SPAWN_MIN: f32 = 24.0;
const SPAWN_MAX: f32 = 48.0;
/// The darkest a spawning place may be lit (by torches) and still spawn.
const SPAWN_LIGHT: u8 = 7;
/// Mobs further than this from every player go.
const DESPAWN: f32 = 80.0;
/// How often mobs think, in seconds.
const THINK: f32 = 0.25;
/// Physics time step, in seconds.
const STEP: f32 = 0.05;
const JUMP_SPEED: f32 = 5.0;
/// How long sinking into the ground at dawn takes, and how fast.
const SINK_TIME: f32 = 2.0;
const SINK_SPEED: f32 = 0.5;
/// A wisp lit this far from the player it is after stops (they got away).
const FUSE_ABORT: f32 = 4.5;
/// How far a wisp keeps from a player before it rushes them, and how long.
const STALK_RANGE: f32 = 7.0;
/// How high above the ground a wandering wisp keeps.
const HOVER: (f32, f32) = (1.5, 4.0);
/// A target further than this is lost; one out of sight this long, too.
const LOSE_RANGE: f32 = 24.0;
const LOSE_UNSEEN: f32 = 5.0;
/// How far a hit throws a mob back, and up, in nodes per second.
const KNOCKBACK: f32 = 4.0;
const KNOCK_UP: f32 = 3.0;
/// The "fleshy" armour of every mob (100: full damage).
const ARMOR: f32 = 100.0;
/// Resend a moving mob's position this often, to keep clients right.
const RESYNC: f32 = 2.0;
/// The night, when mobs spawn (time of day, 0 to 24000), and the day,
/// when they sink back into the ground.
pub fn dark(time_of_day: u16) -> bool {
    !(4500..19500).contains(&time_of_day)
}

pub fn daytime(time_of_day: u16) -> bool {
    (5000..19000).contains(&time_of_day)
}

pub fn is_mob_id(id: u16) -> bool {
    (FIRST_ID..=LAST_ID).contains(&id)
}

/// What the mobs need to know of the world.
pub trait Terrain {
    /// Whether a mob can't be in a node.
    fn solid(&self, p: NodePos) -> bool;
    fn liquid(&self, p: NodePos) -> bool;
    /// Whether a node blocks a mob's sight.
    fn opaque(&self, p: NodePos) -> bool;
    fn node(&self, p: NodePos) -> u16;
    /// The highest solid node in a column (leaves included).
    fn top(&self, x: i32, z: i32) -> i32;
    /// The light at a node at night.
    fn light(&self, p: NodePos) -> u8;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mossling,
    Wisp,
}

/// What kind of mob it is.
pub struct Def {
    /// The entity's name.
    pub name: &'static str,
    pub description: &'static str,
    pub hp_max: u16,
    /// Its box: half width, height, and how far its bottom is below its
    /// position (0: the position is its feet).
    pub half_width: f32,
    pub height: f32,
    pub below: f32,
    /// Its look: a mesh (or "" for a sprite), the texture, the size, and
    /// how much it glows.
    pub mesh: &'static str,
    pub visual_size: [f32; 3],
    pub texture: &'static str,
    pub glow: i8,
    /// Whether it flies (no gravity).
    pub flies: bool,
    /// A wisp's fuse (seconds), and its blast's radius (0: it doesn't
    /// burst).
    pub fuse: f32,
    pub burst: i32,
    /// Nodes per second.
    pub walk_speed: f32,
    pub run_speed: f32,
    /// How far it sees a player, and reaches one to hit.
    pub view_range: f32,
    pub reach: f32,
    /// HP a hit takes, and how often it can hit.
    pub damage: u16,
    pub attack_every: f32,
    /// Below this HP, alone, it flees.
    pub flee_hp: u16,
    /// What its death says, after the player's name.
    pub kills: &'static str,
    /// Its sounds ("": none): idle chatter, the call on seeing a player,
    /// a swipe, hurt, death, going at dawn, a hum all the time it is
    /// about, and the whine of its fuse.
    pub chatter: &'static str,
    pub call: &'static str,
    pub swipe: &'static str,
    pub hurt: &'static str,
    pub death: &'static str,
    pub sink: &'static str,
    pub hum: &'static str,
    pub whine: &'static str,
    /// What it drops: an item, and from how many to how many.
    pub drops: (u16, u32, u32),
}

pub const MOSSLING: Def = Def {
    name: "mobs:mossling",
    description: "Mossling",
    hp_max: 10,
    half_width: 0.25,
    height: 0.9,
    below: 0.0,
    mesh: "character.b3d",
    visual_size: [0.7, 0.55, 0.7],
    texture: "mobs_mossling.png",
    glow: 0,
    flies: false,
    fuse: 0.0,
    burst: 0,
    walk_speed: 1.5,
    run_speed: 3.2,
    view_range: 16.0,
    reach: 1.5,
    damage: 2,
    attack_every: 1.5,
    flee_hp: 3,
    kills: "was mauled by mosslings",
    chatter: "mobs_mossling_chatter",
    call: "mobs_mossling_call",
    swipe: "mobs_mossling_swipe",
    hurt: "mobs_mossling_hurt",
    death: "mobs_mossling_death",
    sink: "default_dig_crumbly",
    hum: "",
    whine: "",
    drops: (content::ITEM_STICK, 1, 3),
};

pub const WISP: Def = Def {
    name: "mobs:wisp",
    description: "Wisp",
    hp_max: 6,
    half_width: 0.3,
    height: 0.6,
    below: 0.3,
    mesh: "",
    visual_size: [0.8, 0.8, 0.8],
    texture: "mobs_wisp.png",
    glow: 14,
    flies: true,
    fuse: 1.5,
    burst: 2,
    walk_speed: 0.8,
    run_speed: 3.5,
    view_range: 20.0,
    // How close it gets before it lights its fuse.
    reach: 2.2,
    damage: 0,
    attack_every: 0.0,
    flee_hp: 0,
    kills: "was burnt up by a wisp",
    chatter: "",
    call: "",
    swipe: "",
    hurt: "mobs_wisp_fizzle",
    death: "mobs_wisp_fizzle",
    sink: "mobs_wisp_fizzle",
    hum: "mobs_wisp_hum",
    whine: "mobs_wisp_whine",
    drops: (content::ITEM_COAL_LUMP, 1, 1),
};

pub fn def(kind: Kind) -> &'static Def {
    match kind {
        Kind::Mossling => &MOSSLING,
        Kind::Wisp => &WISP,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// Standing or wandering about.
    Wander,
    Chase,
    Flee,
    /// A wisp's fuse burning.
    Lit,
    /// Going at dawn: sinking into the ground, or (a wisp) fading.
    Sinking,
}

/// The animation it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anim {
    Stand,
    Walk,
    Run,
    Attack,
    /// A wisp's fast flicker, its fuse lit.
    Charge,
}

pub fn animation(a: Anim) -> Animation {
    use messages::*;
    match a {
        Anim::Stand | Anim::Charge => Animation::new(ANIM_STAND, ANIM_FPS),
        Anim::Walk => Animation::new(ANIM_WALK, ANIM_FPS),
        Anim::Run => Animation::new(ANIM_WALK, ANIM_FPS * 1.5),
        Anim::Attack => Animation::new(ANIM_MINE, ANIM_FPS * 1.25),
    }
}

/// The message showing a mob's animation: the model's frames, or a
/// sprite's (a wisp: frames 0 to 3 slowly, 4 to 7 fast with its fuse lit).
pub fn animation_message(m: &Mob) -> Vec<u8> {
    if m.def().mesh.is_empty() {
        match m.anim {
            Anim::Charge => messages::set_sprite_message([0, 4], 4, 0.05),
            _ => messages::set_sprite_message([0, 0], 4, 0.15),
        }
    } else {
        messages::set_animation_message(animation(m.anim))
    }
}

pub struct Mob {
    pub id: u16,
    pub kind: Kind,
    /// Its feet (a wisp: its middle), in nodes.
    pub pos: [f32; 3],
    /// Nodes per second.
    pub vel: [f32; 3],
    /// Degrees, as a player's (facing -sin, cos).
    pub yaw: f32,
    pub hp: u16,
    pub state: State,
    pub anim: Anim,
    /// Its pack, and which way it flanks (-1, 0, 1).
    pack: u16,
    flank: f32,
    target: Option<u16>,
    unseen: f32,
    /// The horizontal velocity it wants, and (flying) the vertical.
    want: [f32; 2],
    want_y: f32,
    /// Until it next picks a way to wander, stops fleeing, or (sinking)
    /// is gone.
    timer: f32,
    think: f32,
    looks: u32,
    cooldown: f32,
    swing: f32,
    /// A wisp's fuse: seconds left.
    fuse: f32,
    chatter: f32,
    /// How long it has been pushing against something, and a way round
    /// it being tried (seconds left, and which side).
    stuck: f32,
    detour: f32,
    detour_side: f32,
    /// Until it sinks, once it is day.
    dawn: Option<f32>,
    on_ground: bool,
    /// Thrown by a hit: it flies where it was thrown until it lands.
    thrown: bool,
    /// The velocity clients last had (as they collide too, a part of it
    /// may since have stopped), and when they had it.
    sent: [f32; 3],
    since_sent: f32,
    /// Clients need telling where it is (it turned to face a player).
    dirty: bool,
}

impl Mob {
    pub fn def(&self) -> &'static Def {
        def(self.kind)
    }

    pub fn pos_bs(&self) -> [f32; 3] {
        self.pos.map(|v| v * 10.0)
    }
}

/// A player, as the mobs see them.
#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub id: u16,
    /// Feet, in nodes.
    pub pos: [f32; 3],
    pub alive: bool,
}

/// What clients need to be told, and what happens to players.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Added(u16),
    Moved(u16),
    Animated(u16),
    /// It goes at dawn: no longer physical, sinking (or a wisp rising and
    /// fading).
    Sinking(u16),
    /// A sound at the mob (at `pos`, nodes, if it has gone): name, gain
    /// and how far it is heard.
    Sound(u16, &'static str, f32, f32, [f32; 3]),
    /// A wisp's fuse is lit (true), or put out (false).
    Fuse(u16, bool),
    /// A wisp bursts at `pos` (nodes), with a blast of `radius`; it is
    /// gone (Removed follows).
    Burst { mob: u16, pos: [f32; 3], radius: i32 },
    /// A mob hit a player: HP lost, and how they are thrown (nodes/s).
    Hit { mob: u16, player: u16, damage: u16, push: [f32; 3] },
    Removed(u16),
}

/// A punch's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Punched {
    pub hp: u16,
    pub kind: Kind,
    pub pos: [f32; 3],
}

pub struct Mobs {
    list: Vec<Mob>,
    next_id: u16,
    next_pack: u16,
    /// Seconds since the mobs started, and when each player last punched
    /// one (for time_from_last_punch).
    clock: f32,
    punches: Vec<(u16, f32)>,
}

impl Default for Mobs {
    fn default() -> Self {
        Self::new()
    }
}

impl Mobs {
    pub fn new() -> Self {
        Self { list: Vec::new(), next_id: FIRST_ID, next_pack: 0, clock: 0.0, punches: Vec::new() }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Mob> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn get(&self, id: u16) -> Option<&Mob> {
        self.list.iter().find(|m| m.id == id)
    }

    /// Remove them all; their ids.
    pub fn clear(&mut self) -> Vec<u16> {
        self.list.drain(..).map(|m| m.id).collect()
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

    /// Add a pack of `count` at these places (feet).
    pub fn add_pack(&mut self, kind: Kind, at: &[[f32; 3]], rng: &mut Rng) -> Vec<Event> {
        let pack = self.next_pack;
        self.next_pack = self.next_pack.wrapping_add(1);
        let mut events = Vec::new();
        for (i, &pos) in at.iter().enumerate() {
            if self.list.len() >= MAX_MOBS {
                break;
            }
            let id = self.new_id();
            let flank = [0.0, 1.0, -1.0][i % 3];
            self.list.push(Mob {
                id,
                kind,
                pos,
                vel: [0.0; 3],
                yaw: rng.range(0, 359) as f32,
                hp: def(kind).hp_max,
                state: State::Wander,
                anim: Anim::Stand,
                pack,
                flank,
                target: None,
                unseen: 0.0,
                want: [0.0; 2],
                want_y: 0.0,
                timer: 0.0,
                think: rng.range(0, 250) as f32 / 1000.0,
                looks: i as u32,
                cooldown: 0.0,
                swing: 0.0,
                fuse: 0.0,
                chatter: rng.range(3, 10) as f32,
                stuck: 0.0,
                detour: 0.0,
                detour_side: 1.0,
                dawn: None,
                on_ground: false,
                thrown: false,
                sent: [0.0; 3],
                since_sent: 0.0,
                dirty: false,
            });
            events.push(Event::Added(id));
        }
        events
    }

    /// Seconds since `player` last punched a mob (time_from_last_punch),
    /// and now they have.
    pub fn since_punch(&mut self, player: u16) -> f32 {
        let now = self.clock;
        match self.punches.iter_mut().find(|(p, _)| *p == player) {
            Some((_, t)) => {
                let since = now - *t;
                *t = now;
                since
            }
            None => {
                self.punches.push((player, now));
                1_000_000.0
            }
        }
    }

    /// A player at `from` (feet) hits a mob for `damage`: it is thrown
    /// back and turns on them. At 0 HP it is gone. None if there is no
    /// such mob.
    pub fn punch(&mut self, id: u16, player: u16, from: [f32; 3], damage: u16) -> Option<Punched> {
        self.harm(id, Some(player), from, damage)
    }

    /// A mob takes `damage` from something at `from` (a player, or a
    /// blast): thrown back, and turning on the player, if one.
    pub fn harm(&mut self, id: u16, player: Option<u16>, from: [f32; 3], damage: u16) -> Option<Punched> {
        let i = self.list.iter().position(|m| m.id == id)?;
        let m = &mut self.list[i];
        m.hp = m.hp.saturating_sub(damage);
        let result = Punched { hp: m.hp, kind: m.kind, pos: m.pos };
        if m.hp == 0 {
            self.list.remove(i);
            return Some(result);
        }
        if m.state != State::Sinking {
            let (dx, dz) = (m.pos[0] - from[0], m.pos[2] - from[2]);
            let d = (dx * dx + dz * dz).sqrt().max(0.01);
            m.vel = [dx / d * KNOCKBACK, KNOCK_UP, dz / d * KNOCKBACK];
            m.want = [0.0; 2];
            m.want_y = 0.0;
            m.thrown = true;
            m.on_ground = false;
            m.sent = m.vel;
            m.since_sent = 0.0;
            if let Some(player) = player {
                m.target = Some(player);
                m.unseen = 0.0;
                if m.state == State::Wander {
                    m.state = State::Chase;
                }
            }
            // It reels for a moment before going on.
            m.think = 0.4;
        }
        Some(result)
    }

    /// Move the mobs on by `dt` seconds.
    pub fn step(&mut self, dt: f32, t: &dyn Terrain, players: &[Player], day: bool, rng: &mut Rng) -> Vec<Event> {
        let dt = dt.min(0.5);
        self.clock += dt;
        let mut events = Vec::new();
        // Far from everyone: gone.
        self.list.retain(|m| {
            let near = players.iter().any(|p| dist2(p.pos, m.pos) <= DESPAWN * DESPAWN);
            if !near {
                events.push(Event::Removed(m.id));
            }
            near
        });
        let others: Vec<(u16, u16, [f32; 3])> = self.list.iter().map(|m| (m.id, m.pack, m.pos)).collect();
        let mut spotted: Vec<(u16, u16)> = Vec::new();
        for m in &mut self.list {
            m.think -= dt;
            if m.think <= 0.0 {
                m.think += THINK;
                if m.def().flies {
                    think_wisp(m, players, t, day, rng, &mut events);
                } else {
                    think(m, &others, players, t, day, rng, &mut events, &mut spotted);
                }
            }
        }
        // A pack hunts together: whoever sees a player, the rest join in.
        for (pack, target) in spotted {
            for m in self.list.iter_mut().filter(|m| m.pack == pack && m.target.is_none() && m.state == State::Wander) {
                m.target = Some(target);
                m.state = State::Chase;
                m.unseen = 0.0;
            }
        }
        let mut gone = Vec::new();
        for m in &mut self.list {
            let done = if m.def().flies { fly(m, dt, t, &mut events) } else { physics(m, dt, t, &mut events) };
            if done {
                gone.push(m.id);
            }
        }
        self.list.retain(|m| !gone.contains(&m.id));
        events.extend(gone.into_iter().map(Event::Removed));
        events
    }

    /// Try to spawn a pack somewhere near each player, if it is dark and
    /// there is room.
    pub fn spawn(&mut self, t: &dyn Terrain, players: &[Player], rng: &mut Rng) -> Vec<Event> {
        let mut events = Vec::new();
        for p in players.iter().filter(|p| p.alive) {
            if self.list.len() >= MAX_MOBS {
                break;
            }
            let near = self.list.iter().filter(|m| dist2(m.pos, p.pos) <= NEAR * NEAR).count();
            if near >= PER_PLAYER {
                continue;
            }
            let wisps = self.list.iter().filter(|m| m.kind == Kind::Wisp && dist2(m.pos, p.pos) <= NEAR * NEAR).count();
            // One time in three a wisp, if there is room for one.
            let kind = if wisps < WISPS_PER_PLAYER && rng.one_in(3) { Kind::Wisp } else { Kind::Mossling };
            for _ in 0..4 {
                if let Some(floor) = spawn_spot(t, p.pos, players, kind == Kind::Mossling, rng) {
                    let count = if kind == Kind::Wisp { 1 } else { rng.range(2, 3) as usize };
                    let count = count.min(PER_PLAYER - near).min(MAX_MOBS - self.list.len());
                    let at = if kind == Kind::Wisp { vec![wisp_place(floor)] } else { pack_places(t, floor, count) };
                    events.extend(self.add_pack(kind, &at, rng));
                    break;
                }
            }
        }
        events
    }

    /// A pack of mosslings, or a wisp, beside a player (feet at `pos`,
    /// looking along `dir`): a few nodes in front, on the ground (a wisp
    /// above it). (/mobs spawn.)
    pub fn spawn_near(&mut self, kind: Kind, t: &dyn Terrain, pos: [f32; 3], dir: [f32; 2], rng: &mut Rng) -> Vec<Event> {
        let (x, z) = ((pos[0] + dir[0] * 5.0).round() as i32, (pos[2] + dir[1] * 5.0).round() as i32);
        let y0 = (pos[1] + 0.5).floor() as i32;
        let floor = (y0 - 6..=y0 + 6).rev().map(|y| (x, y - 1, z)).find(|&f| fits(t, f));
        let Some(floor) = floor else {
            return Vec::new();
        };
        if kind == Kind::Wisp {
            return self.add_pack(kind, &[wisp_place(floor)], rng);
        }
        let count = 3.min(MAX_MOBS.saturating_sub(self.list.len()));
        let at = pack_places(t, floor, count);
        self.add_pack(kind, &at, rng)
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

fn norm2(v: [f32; 2]) -> [f32; 2] {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if l < 1e-4 {
        [0.0, 0.0]
    } else {
        [v[0] / l, v[1] / l]
    }
}

/// The yaw (degrees) of facing along `dir`.
fn yaw_of(dir: [f32; 2]) -> f32 {
    (-dir[0]).atan2(dir[1]).to_degrees().rem_euclid(360.0)
}

/// The node a point is in.
fn node_at(p: [f32; 3]) -> NodePos {
    ((p[0] + 0.5).floor() as i32, (p[1] + 0.5).floor() as i32, (p[2] + 0.5).floor() as i32)
}

/// Whether nothing opaque lies between two points (a 3D DDA through the
/// nodes the line crosses, the ends' own nodes excepted).
pub fn clear_line(a: [f32; 3], b: [f32; 3], t: &dyn Terrain) -> bool {
    let start = node_at(a);
    let end = node_at(b);
    let mut n = [start.0, start.1, start.2];
    let e = [end.0, end.1, end.2];
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let mut step = [0i32; 3];
    let mut t_max = [f32::INFINITY; 3];
    let mut t_delta = [f32::INFINITY; 3];
    for i in 0..3 {
        if d[i] > 0.0 {
            step[i] = 1;
            t_max[i] = ((n[i] as f32 + 0.5) - a[i]) / d[i];
            t_delta[i] = 1.0 / d[i];
        } else if d[i] < 0.0 {
            step[i] = -1;
            t_max[i] = ((n[i] as f32 - 0.5) - a[i]) / d[i];
            t_delta[i] = -1.0 / d[i];
        }
    }
    for _ in 0..200 {
        let i = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
        if t_max[i] > 1.0 {
            return true;
        }
        n[i] += step[i];
        t_max[i] += t_delta[i];
        if n == e {
            return true;
        }
        if t.opaque((n[0], n[1], n[2])) {
            return false;
        }
    }
    true
}

/// Whether a mob sees a player's eyes (from 0.8 above its feet, or a
/// wisp's middle).
fn sees(m: &Mob, p: &Player, t: &dyn Terrain) -> bool {
    let eye = if m.def().flies { m.pos } else { [m.pos[0], m.pos[1] + 0.8, m.pos[2]] };
    let target = [p.pos[0], p.pos[1] + 1.45, p.pos[2]];
    clear_line(eye, target, t)
}

#[allow(clippy::too_many_arguments)]
fn think(m: &mut Mob, others: &[(u16, u16, [f32; 3])], players: &[Player], t: &dyn Terrain, day: bool, rng: &mut Rng, events: &mut Vec<Event>, spotted: &mut Vec<(u16, u16)>) {
    if m.state == State::Sinking {
        return;
    }
    let d = m.def();
    m.cooldown = (m.cooldown - THINK).max(0.0);
    m.swing = (m.swing - THINK).max(0.0);
    m.looks = m.looks.wrapping_add(1);
    let look = m.looks % 2 == 0;
    // At dawn, each in its own time, it sinks into the ground.
    if day {
        let left = *m.dawn.get_or_insert_with(|| rng.range(0, 80) as f32 / 10.0);
        if left <= 0.0 {
            m.state = State::Sinking;
            m.timer = SINK_TIME;
            m.vel = [0.0, -SINK_SPEED, 0.0];
            m.want = [0.0; 2];
            m.sent = m.vel;
            m.since_sent = 0.0;
            events.push(Event::Sinking(m.id));
            events.push(Event::Sound(m.id, d.sink, 0.6, 16.0, m.pos));
            return;
        }
        m.dawn = Some(left - THINK);
    } else {
        m.dawn = None;
    }
    // Its target: kept while near and seen lately.
    let mut target = m.target.and_then(|id| players.iter().find(|p| p.id == id && p.alive)).copied();
    if let Some(p) = target {
        if dist2(p.pos, m.pos) > LOSE_RANGE * LOSE_RANGE {
            target = None;
        } else if look {
            if sees(m, &p, t) {
                m.unseen = 0.0;
            } else {
                m.unseen += 2.0 * THINK;
                if m.unseen > LOSE_UNSEEN {
                    target = None;
                }
            }
        }
    }
    if target.is_none() && m.target.is_some() {
        m.target = None;
        m.state = State::Wander;
        m.timer = 0.0;
    }
    // Looking for one: the nearest player it can see.
    if target.is_none() && look && !day {
        let mut near: Vec<&Player> = players.iter().filter(|p| p.alive && dist2(p.pos, m.pos) <= d.view_range * d.view_range).collect();
        near.sort_by(|a, b| dist2(a.pos, m.pos).total_cmp(&dist2(b.pos, m.pos)));
        if let Some(p) = near.into_iter().find(|p| sees(m, p, t)) {
            target = Some(*p);
            m.target = Some(p.id);
            m.state = State::Chase;
            m.unseen = 0.0;
            spotted.push((m.pack, p.id));
            events.push(Event::Sound(m.id, d.call, 1.0, 32.0, m.pos));
        }
    }
    let packmates_near = others.iter().any(|&(id, pack, pos)| id != m.id && pack == m.pack && dist2(pos, m.pos) <= 64.0);
    // Badly hurt and alone, it runs.
    if m.hp <= d.flee_hp && !packmates_near && target.is_some() && m.state == State::Chase {
        m.state = State::Flee;
        m.timer = 4.0;
    }
    let mut anim = Anim::Stand;
    let mut face: Option<[f32; 2]> = None;
    match (m.state, target) {
        (State::Flee, Some(p)) => {
            m.timer -= THINK;
            if m.timer <= 0.0 {
                m.state = State::Chase;
            }
            let away = norm2([m.pos[0] - p.pos[0], m.pos[2] - p.pos[2]]);
            m.want = [away[0] * d.run_speed, away[1] * d.run_speed];
            anim = Anim::Run;
        }
        (State::Chase, Some(p)) => {
            let to = [p.pos[0] - m.pos[0], p.pos[2] - m.pos[2]];
            let dist = (to[0] * to[0] + to[1] * to[1]).sqrt();
            let dy = p.pos[1] - m.pos[1];
            let dir = norm2(to);
            if dist <= d.reach && (-1.0..1.6).contains(&dy) {
                m.want = [0.0; 2];
                face = Some(dir);
                anim = if m.swing > 0.0 { Anim::Attack } else { Anim::Stand };
                if m.cooldown <= 0.0 {
                    m.cooldown = d.attack_every;
                    m.swing = 0.5;
                    anim = Anim::Attack;
                    let push = [dir[0] * 2.0, 1.5, dir[1] * 2.0];
                    events.push(Event::Hit { mob: m.id, player: p.id, damage: d.damage, push });
                    events.push(Event::Sound(m.id, d.swipe, 0.8, 16.0, m.pos));
                }
            } else {
                // The others in the pack come at the player from the sides.
                let aim = if m.flank != 0.0 && dist > 4.0 {
                    let side = (dist * 0.4).min(3.0) * m.flank;
                    [to[0] - dir[1] * side, to[1] + dir[0] * side]
                } else {
                    to
                };
                let mut go = norm2(aim);
                if m.stuck > 0.75 && m.detour <= 0.0 {
                    m.detour = 1.0;
                    m.detour_side = if rng.one_in(2) { 1.0 } else { -1.0 };
                }
                if m.detour > 0.0 {
                    m.detour -= THINK;
                    go = [-go[1] * m.detour_side, go[0] * m.detour_side];
                }
                m.want = [go[0] * d.run_speed, go[1] * d.run_speed];
                anim = if m.swing > 0.0 { Anim::Attack } else { Anim::Run };
            }
        }
        _ => {
            m.state = State::Wander;
            m.timer -= THINK;
            if m.timer <= 0.0 || m.stuck > 0.5 {
                m.timer = rng.range(2, 6) as f32;
                m.stuck = 0.0;
                if rng.one_in(3) {
                    m.want = [0.0; 2];
                } else {
                    let a = (rng.range(0, 359) as f32).to_radians();
                    m.want = [a.cos() * d.walk_speed, a.sin() * d.walk_speed];
                }
            }
            if m.want != [0.0; 2] {
                anim = Anim::Walk;
            }
            // Now and then it chatters (one in each pack: the first left).
            m.chatter -= THINK;
            let first = others.iter().filter(|o| o.1 == m.pack).map(|o| o.0).min() == Some(m.id);
            if m.chatter <= 0.0 {
                m.chatter = rng.range(5, 15) as f32;
                if first {
                    events.push(Event::Sound(m.id, d.chatter, 0.7, 20.0, m.pos));
                }
            }
        }
    }
    // Keep a little apart from the others.
    for &(id, _, pos) in others {
        let (dx, dz) = (m.pos[0] - pos[0], m.pos[2] - pos[2]);
        let d2 = dx * dx + dz * dz;
        if id != m.id && d2 < 0.64 && dist2(pos, m.pos) < 1.0 {
            let away = if d2 < 1e-4 { [1.0, 0.0] } else { norm2([dx, dz]) };
            m.want = [m.want[0] + away[0], m.want[1] + away[1]];
        }
    }
    // Not over a drop or into water.
    if m.want != [0.0; 2] && !safe_ahead(m, t) {
        m.want = [0.0; 2];
        if m.state == State::Wander {
            m.timer = 0.0;
        }
        if anim != Anim::Attack {
            anim = Anim::Stand;
        }
    }
    if m.want != [0.0; 2] {
        m.yaw = yaw_of(m.want);
    } else if let Some(f) = face {
        // Facing a player without moving: clients only turn it to face
        // the way it moves, so tell them.
        let yaw = yaw_of(f);
        let turn = (yaw - m.yaw).rem_euclid(360.0);
        if turn > 20.0 && turn < 340.0 {
            m.dirty = true;
        }
        m.yaw = yaw;
    }
    if anim != m.anim {
        m.anim = anim;
        events.push(Event::Animated(m.id));
    }
}

fn norm3(v: [f32; 3]) -> ([f32; 3], f32) {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-4 {
        ([0.0; 3], 0.0)
    } else {
        (v.map(|x| x / l), l)
    }
}

/// A wisp thinks: hovers about; once it sees a player, it hangs about
/// some way off for a few seconds, then rushes them; close enough, it
/// lights its fuse and comes on slowly, bursting when the fuse is out,
/// unless the player has got away.
fn think_wisp(m: &mut Mob, players: &[Player], t: &dyn Terrain, day: bool, rng: &mut Rng, events: &mut Vec<Event>) {
    if m.state == State::Sinking || m.hp == 0 {
        return;
    }
    let d = m.def();
    m.looks = m.looks.wrapping_add(1);
    let look = m.looks % 2 == 0;
    let mut anim = Anim::Stand;
    // At dawn it fades away, each in its own time.
    if day {
        let left = *m.dawn.get_or_insert_with(|| rng.range(0, 80) as f32 / 10.0);
        if left <= 0.0 {
            if m.state == State::Lit {
                events.push(Event::Fuse(m.id, false));
            }
            m.state = State::Sinking;
            m.timer = SINK_TIME;
            m.vel = [0.0, SINK_SPEED, 0.0];
            m.sent = m.vel;
            m.since_sent = 0.0;
            m.anim = Anim::Stand;
            events.push(Event::Animated(m.id));
            events.push(Event::Sinking(m.id));
            events.push(Event::Sound(m.id, d.sink, 0.6, 16.0, m.pos));
            return;
        }
        m.dawn = Some(left - THINK);
    } else {
        m.dawn = None;
    }
    let mut target = m.target.and_then(|id| players.iter().find(|p| p.id == id && p.alive)).copied();
    if let Some(p) = target {
        if dist2(p.pos, m.pos) > LOSE_RANGE * LOSE_RANGE {
            target = None;
        } else if look && m.state != State::Lit {
            if sees(m, &p, t) {
                m.unseen = 0.0;
            } else {
                m.unseen += 2.0 * THINK;
                if m.unseen > LOSE_UNSEEN {
                    target = None;
                }
            }
        }
    }
    if target.is_none() && m.target.is_some() {
        if m.state == State::Lit {
            events.push(Event::Fuse(m.id, false));
        }
        m.target = None;
        m.state = State::Wander;
        m.timer = 0.0;
    }
    if target.is_none() && look && !day {
        let mut near: Vec<&Player> = players.iter().filter(|p| p.alive && dist2(p.pos, m.pos) <= d.view_range * d.view_range).collect();
        near.sort_by(|a, b| dist2(a.pos, m.pos).total_cmp(&dist2(b.pos, m.pos)));
        if let Some(p) = near.into_iter().find(|p| sees(m, p, t)) {
            target = Some(*p);
            m.target = Some(p.id);
            m.state = State::Chase;
            m.unseen = 0.0;
            // It hangs about a while first, looking harmless.
            m.timer = rng.range(20, 50) as f32 / 10.0;
        }
    }
    let mut want = [0.0f32; 3];
    match (m.state, target) {
        (State::Lit, Some(p)) => {
            let chest = [p.pos[0], p.pos[1] + 1.0, p.pos[2]];
            let (dir, dist) = norm3([chest[0] - m.pos[0], chest[1] - m.pos[1], chest[2] - m.pos[2]]);
            if dist > FUSE_ABORT {
                // They got away: it goes out, and hangs back a while.
                events.push(Event::Fuse(m.id, false));
                m.state = State::Chase;
                m.timer = 3.0;
            } else {
                m.fuse -= THINK;
                anim = Anim::Charge;
                want = dir.map(|v| v * d.walk_speed * 2.0);
                if m.fuse <= 0.0 {
                    events.push(Event::Burst { mob: m.id, pos: m.pos, radius: d.burst });
                    m.hp = 0;
                    return;
                }
            }
        }
        (State::Chase, Some(p)) => {
            let chest = [p.pos[0], p.pos[1] + 1.0, p.pos[2]];
            let (dir, dist) = norm3([chest[0] - m.pos[0], chest[1] - m.pos[1], chest[2] - m.pos[2]]);
            m.timer -= THINK;
            if dist <= d.reach {
                m.state = State::Lit;
                m.fuse = d.fuse;
                anim = Anim::Charge;
                events.push(Event::Fuse(m.id, true));
            } else if m.timer > 0.0 {
                // Keeping its distance, drifting round them, at their eyes.
                let flat = norm2([dir[0], dir[2]]);
                let across = [-flat[1] * d.walk_speed, flat[0] * d.walk_speed];
                let toward = if dist > STALK_RANGE + 1.0 {
                    d.walk_speed * 2.0
                } else if dist < STALK_RANGE - 1.0 {
                    -d.walk_speed * 2.0
                } else {
                    0.0
                };
                want = [flat[0] * toward + across[0], (chest[1] + 0.5 - m.pos[1]).clamp(-1.0, 1.0), flat[1] * toward + across[1]];
            } else {
                want = dir.map(|v| v * d.run_speed);
            }
        }
        _ => {
            m.state = State::Wander;
            m.timer -= THINK;
            if m.timer <= 0.0 || m.stuck > 0.5 {
                m.timer = rng.range(3, 7) as f32;
                m.stuck = 0.0;
                m.want = if rng.one_in(3) {
                    [0.0; 2]
                } else {
                    let a = (rng.range(0, 359) as f32).to_radians();
                    [a.cos() * d.walk_speed, a.sin() * d.walk_speed]
                };
            }
            // Keeping a few nodes above the ground.
            let (x, y, z) = node_at(m.pos);
            let gap = (0..8).find(|k| t.solid((x, y - k, z))).map_or(8.0, |k| k as f32 + m.pos[1] - y as f32 + 0.5);
            let rise = if gap < HOVER.0 {
                0.6
            } else if gap > HOVER.1 {
                -0.6
            } else {
                0.0
            };
            want = [m.want[0], rise, m.want[1]];
        }
    }
    // Something in the way: up over it.
    if m.stuck > 0.3 {
        want[1] = want[1].max(1.5);
    }
    m.want = [want[0], want[2]];
    m.want_y = want[1];
    if anim != m.anim {
        m.anim = anim;
        events.push(Event::Animated(m.id));
    }
}

/// Move a flying mob on by `dt`: straight where it wants, stopping against
/// nodes. Returns whether it is gone (burst, faded, or put out by water).
fn fly(m: &mut Mob, dt: f32, t: &dyn Terrain, events: &mut Vec<Event>) -> bool {
    m.since_sent += dt;
    if m.hp == 0 {
        return true;
    }
    if m.state == State::Sinking {
        m.pos[1] += m.vel[1] * dt;
        m.timer -= dt;
        return m.timer <= 0.0;
    }
    let d = m.def();
    if t.liquid(node_at(m.pos)) {
        if m.state == State::Lit {
            events.push(Event::Fuse(m.id, false));
        }
        events.push(Event::Sound(m.id, d.death, 0.8, 16.0, m.pos));
        return true;
    }
    let size = (d.half_width, d.height, d.below);
    let mut blocked = false;
    let mut time = 0.0;
    while time < dt {
        let step = STEP.min(dt - time);
        time += step;
        if m.thrown {
            // Knocked back: it slows, then goes on.
            m.vel = m.vel.map(|v| v * 0.85);
            if m.vel.iter().all(|v| v.abs() < 0.5) {
                m.thrown = false;
            }
        } else {
            m.vel = [m.want[0], m.want_y, m.want[1]];
        }
        for axis in [1, 0, 2] {
            let delta = m.vel[axis] * step;
            if delta != 0.0 && move_axis(&mut m.pos, axis, delta, size, t) {
                m.vel[axis] = 0.0;
                m.sent[axis] = 0.0;
                blocked = true;
            }
        }
    }
    m.stuck = if blocked { m.stuck + dt } else { 0.0 };
    let differs = (0..3).any(|i| (m.vel[i] - m.sent[i]).abs() > 0.05);
    let moving = m.vel.iter().any(|v| v.abs() > 0.01);
    if m.dirty || (differs && m.since_sent >= 0.2) || (moving && m.since_sent >= RESYNC) || m.since_sent >= RESYNC * 4.0 {
        m.sent = m.vel;
        m.since_sent = 0.0;
        m.dirty = false;
        events.push(Event::Moved(m.id));
    }
    false
}

/// Whether the ground goes on ahead: a solid node at most three below,
/// and no water first.
fn safe_ahead(m: &Mob, t: &dyn Terrain) -> bool {
    let dir = norm2(m.want);
    let ahead = [m.pos[0] + dir[0] * 0.7, m.pos[1], m.pos[2] + dir[1] * 0.7];
    let (x, fy, z) = node_at([ahead[0], ahead[1] + 0.01, ahead[2]]);
    if t.solid((x, fy, z)) {
        return true; // a step up
    }
    for k in 0..=3 {
        if t.liquid((x, fy - k, z)) {
            return false;
        }
        if t.solid((x, fy - k - 1, z)) {
            return true;
        }
    }
    false
}

/// Move a mob on by `dt`. Returns whether it is gone (sunk).
fn physics(m: &mut Mob, dt: f32, t: &dyn Terrain, events: &mut Vec<Event>) -> bool {
    m.since_sent += dt;
    if m.state == State::Sinking {
        m.pos[1] += m.vel[1] * dt;
        m.timer -= dt;
        return m.timer <= 0.0;
    }
    let d = m.def();
    let size = (d.half_width, d.height, d.below);
    let mut blocked = false;
    let mut jumped = false;
    let mut time = 0.0;
    while time < dt {
        let step = STEP.min(dt - time);
        time += step;
        // It goes the way it wants, unless thrown.
        if !m.thrown {
            m.vel[0] = m.want[0];
            m.vel[2] = m.want[1];
        }
        m.vel[1] -= messages::GRAVITY * step;
        let mut landed = false;
        for axis in [1, 0, 2] {
            let delta = m.vel[axis] * step;
            if delta != 0.0 && move_axis(&mut m.pos, axis, delta, size, t) {
                m.vel[axis] = 0.0;
                m.sent[axis] = 0.0;
                if axis == 1 {
                    landed = delta < 0.0;
                } else {
                    blocked = true;
                }
            }
        }
        m.on_ground = landed;
        if landed {
            m.thrown = false;
        }
        // Against a one-node step with room above: jump.
        if blocked && m.on_ground && m.want != [0.0; 2] && can_jump(m, t) {
            m.vel[1] = JUMP_SPEED;
            m.vel[0] = m.want[0];
            m.vel[2] = m.want[1];
            m.on_ground = false;
            jumped = true;
        }
    }
    if m.pos[1] < -2000.0 {
        return true;
    }
    m.stuck = if blocked && !jumped { m.stuck + dt } else { 0.0 };
    let moving = m.want != [0.0; 2];
    let differs = (m.vel[0] - m.sent[0]).abs() > 0.05 || (m.vel[2] - m.sent[2]).abs() > 0.05;
    if jumped || m.dirty || (differs && !blocked && m.since_sent >= 0.2) || ((moving || !m.on_ground) && m.since_sent >= RESYNC) || m.since_sent >= RESYNC * 4.0 {
        m.sent = m.vel;
        m.since_sent = 0.0;
        m.dirty = false;
        events.push(Event::Moved(m.id));
    }
    false
}

fn can_jump(m: &Mob, t: &dyn Terrain) -> bool {
    let dir = norm2(m.want);
    let (x, fy, z) = node_at([m.pos[0], m.pos[1] + 0.01, m.pos[2]]);
    let (ax, az) = ((m.pos[0] + dir[0] * 0.6 + 0.5).floor() as i32, (m.pos[2] + dir[1] * 0.6 + 0.5).floor() as i32);
    !t.solid((x, fy + 1, z)) && !t.solid((ax, fy + 1, az)) && (!t.solid((x, fy + 2, z)) || m.def().height <= 1.0)
}

/// The range of node coordinates from `min` to `max`.
fn node_range(min: f32, max: f32) -> (i32, i32) {
    ((min + 0.5).floor() as i32, (max + 0.5).ceil() as i32 - 1)
}

/// Move a box (half width `w`, height `h`, its bottom `below` under `pos`)
/// by `delta` along one axis, stopping against solid nodes. Returns
/// whether it hit one.
fn move_axis(pos: &mut [f32; 3], axis: usize, delta: f32, (w, h, below): (f32, f32, f32), t: &dyn Terrain) -> bool {
    let old = pos[axis];
    let mut new = *pos;
    new[axis] += delta;
    let lo = [new[0] - w, new[1] - below, new[2] - w];
    let hi = [new[0] + w, new[1] - below + h, new[2] + w];
    let r: [(i32, i32); 3] = std::array::from_fn(|i| node_range(lo[i], hi[i]));
    let mut hit: Option<i32> = None;
    for x in r[0].0..=r[0].1 {
        for y in r[1].0..=r[1].1 {
            for z in r[2].0..=r[2].1 {
                if t.solid((x, y, z)) {
                    let n = [x, y, z][axis];
                    hit = Some(match hit {
                        None => n,
                        Some(o) if delta < 0.0 => o.max(n),
                        Some(o) => o.min(n),
                    });
                }
            }
        }
    }
    let Some(n) = hit else {
        *pos = new;
        return false;
    };
    // Up against the nearest one, but never backwards.
    let (below, above) = if axis == 1 { (below, h - below) } else { (w, w) };
    let snap = if delta < 0.0 { n as f32 + 0.5 + below + 1e-4 } else { n as f32 - 0.5 - above - 1e-4 };
    pos[axis] = if delta < 0.0 { snap.clamp(new[axis], old) } else { snap.clamp(old, new[axis]) };
    true
}

/// Whether a mob can stand on `floor`: solid (not leaves, nor the top
/// of a tree) with two nodes of air above.
fn fits(t: &dyn Terrain, floor: NodePos) -> bool {
    let free = |p: NodePos| !t.solid(p) && !t.liquid(p);
    let id = t.node(floor);
    t.solid(floor) && content::group_of(id, "leaves") == 0 && content::group_of(id, "tree") == 0 && !t.liquid(floor) && free((floor.0, floor.1 + 1, floor.2)) && free((floor.0, floor.1 + 2, floor.2))
}

/// A place to spawn near a player at `around`: 24 to 48 nodes away from
/// them (and SPAWN_MIN from everyone), dark, on the ground (and near trees,
/// if `trees`).
fn spawn_spot(t: &dyn Terrain, around: [f32; 3], players: &[Player], near_trees: bool, rng: &mut Rng) -> Option<NodePos> {
    let a = (rng.range(0, 359) as f32).to_radians();
    let r = rng.range(SPAWN_MIN as u32, SPAWN_MAX as u32) as f32;
    let (x, z) = ((around[0] + a.cos() * r).round() as i32, (around[2] + a.sin() * r).round() as i32);
    let top = t.top(x, z);
    let floor = (top - 10..=top).rev().map(|y| (x, y, z)).find(|&f| fits(t, f))?;
    let feet = [x as f32, floor.1 as f32 + 0.5, z as f32];
    if (feet[1] - around[1]).abs() > 24.0 || players.iter().any(|p| dist2(p.pos, feet) < SPAWN_MIN * SPAWN_MIN) {
        return None;
    }
    if t.light((x, floor.1 + 1, z)) > SPAWN_LIGHT {
        return None;
    }
    if !near_trees {
        return Some(floor);
    }
    let trees = (-3..=3).any(|dx| (1..=6).any(|dy| (-3..=3).any(|dz| {
        let id = t.node((x + dx, floor.1 + dy, z + dz));
        content::group_of(id, "tree") > 0 || content::group_of(id, "leaves") > 0
    })));
    trees.then_some(floor)
}

/// Where a wisp floats over `floor`: in the second node of air above it.
fn wisp_place(floor: NodePos) -> [f32; 3] {
    [floor.0 as f32, floor.1 as f32 + 2.0, floor.2 as f32]
}

/// Where a pack of `count` stands around `floor`: on it and the free
/// places beside it.
fn pack_places(t: &dyn Terrain, floor: NodePos, count: usize) -> Vec<[f32; 3]> {
    let feet = |f: NodePos| [f.0 as f32, f.1 as f32 + 0.5, f.2 as f32];
    let mut at = vec![feet(floor)];
    for (dx, dz) in [(1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)] {
        if at.len() >= count {
            break;
        }
        let f = (floor.0 + dx, floor.1, floor.2 + dz);
        if fits(t, f) {
            at.push(feet(f));
        }
    }
    while at.len() < count {
        let n = at.len() as f32;
        at.push([floor.0 as f32 + 0.2 * n, floor.1 as f32 + 0.5, floor.2 as f32 - 0.2 * n]);
    }
    at.truncate(count);
    at
}

/// Damage and wear when a player hits a mob (Luanti's getHitParams, with
/// the "fleshy" group): less for a punch sooner than the tool's
/// full_punch_interval after the last. Wear as builtin's
/// punch_attack_uses: the first group's uses, times 3 to the level.
pub fn hit_params(caps: &ToolCaps, since_last: f32, wear: u16) -> (u16, u32) {
    let mult = (since_last / caps.full_punch_interval).clamp(0.0, 1.0);
    let damage = (caps.fleshy as f32 * mult * ARMOR / 100.0) as i32;
    let uses = caps.groupcaps.iter().find(|c| c.uses != 0 && c.maxlevel >= 1).map_or(0, |c| c.uses * 3u32.pow((c.maxlevel - 1) as u32));
    let wear = if uses > 0 { (content::use_wear(uses, wear) as f32 * mult) as u32 } else { 0 };
    (damage.clamp(0, u16::MAX as i32) as u16, wear)
}

/// A mob's properties: the player model, small (a mossling), or a glowing
/// sprite of 8 frames (a wisp); physical unless going at dawn.
pub fn properties(m: &Mob, physical: bool) -> Vec<u8> {
    let d = m.def();
    let (w, h, b) = (d.half_width, d.height, d.below);
    let sprite = d.mesh.is_empty();
    messages::properties_message(&Props {
        hp_max: d.hp_max,
        physical,
        collisionbox: [-w, -b, -w, w, h - b, w],
        selectionbox: [-w - 0.05, -b, -w - 0.05, w + 0.05, h - b, w + 0.05],
        visual: if sprite { "sprite" } else { "mesh" },
        visual_size: d.visual_size,
        textures: &[d.texture],
        makes_footstep_sound: !d.flies,
        automatic_rotate: 0.0,
        mesh: d.mesh,
        collide_with_objects: false,
        stepheight: if d.flies { 0.0 } else { 0.6 * 10.0 },
        infotext: "",
        wield_item: "",
        glow: d.glow,
        eye_height: 0.8,
        damage_texture_modifier: if sprite { "^[brighten" } else { "^[colorize:#ff000060" },
        zoom_fov: 0.0,
        spritediv: if sprite { [1, 8] } else { [1, 1] },
        face_movement: if sprite { None } else { Some(-90.0) },
        use_texture_alpha: sprite,
    })
}

/// Where it is and how it moves (with gravity, unless it flies or is
/// going at dawn).
pub fn position(m: &Mob) -> Vec<u8> {
    let acc = if m.state == State::Sinking || m.def().flies { [0.0; 3] } else { [0.0, -messages::GRAVITY * 10.0, 0.0] };
    messages::object_position_message(m.pos_bs(), m.vel.map(|v| v * 10.0), acc, m.yaw)
}

/// What a client needs to show a mob.
pub fn init_data(m: &Mob) -> Vec<u8> {
    let messages = [
        properties(m, m.state != State::Sinking),
        messages::armor_groups_message(),
        animation_message(m),
        messages::not_attached_message(),
        position(m),
    ];
    messages::entity_init_data(m.def().name, m.id, m.pos_bs(), m.yaw, m.hp, &messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground (everything below y = 0.5), with some walls, water and
    /// trees, all at night.
    struct Flat {
        walls: Vec<NodePos>,
        water: Vec<NodePos>,
        trees: Vec<NodePos>,
        torch: Option<NodePos>,
    }

    impl Flat {
        fn new() -> Self {
            Self { walls: Vec::new(), water: Vec::new(), trees: Vec::new(), torch: None }
        }
    }

    impl Terrain for Flat {
        fn solid(&self, p: NodePos) -> bool {
            (p.1 <= 0 && !self.water.contains(&p)) || self.walls.contains(&p) || self.trees.contains(&p)
        }
        fn liquid(&self, p: NodePos) -> bool {
            self.water.contains(&p)
        }
        fn opaque(&self, p: NodePos) -> bool {
            self.solid(p)
        }
        fn node(&self, p: NodePos) -> u16 {
            if self.trees.contains(&p) {
                content::CONTENT_FIR_TREE
            } else if self.solid(p) {
                content::CONTENT_STONE
            } else {
                crate::world::CONTENT_AIR
            }
        }
        fn top(&self, x: i32, z: i32) -> i32 {
            (0..10).rev().find(|&y| self.solid((x, y, z))).unwrap_or(0)
        }
        fn light(&self, p: NodePos) -> u8 {
            self.torch.map_or(0, |t| 14u8.saturating_sub(((t.0 - p.0).abs() + (t.1 - p.1).abs() + (t.2 - p.2).abs()) as u8))
        }
    }

    fn run(mobs: &mut Mobs, t: &Flat, players: &[Player], secs: f32, rng: &mut Rng) -> Vec<Event> {
        let mut all = Vec::new();
        for _ in 0..(secs / 0.05) as usize {
            all.extend(mobs.step(0.05, t, players, false, rng));
        }
        all
    }

    #[test]
    fn falls_and_stands() {
        let t = Flat::new();
        let mut rng = Rng::new();
        let mut mobs = Mobs::new();
        let ev = mobs.add_pack(Kind::Mossling, &[[0.0, 3.0, 0.0]], &mut rng);
        assert_eq!(ev, vec![Event::Added(FIRST_ID)]);
        let far = [Player { id: 2, pos: [60.0, 0.5, 0.0], alive: true }];
        run(&mut mobs, &t, &far, 2.0, &mut rng);
        let m = mobs.get(FIRST_ID).unwrap();
        assert!((m.pos[1] - 0.5).abs() < 0.01, "{:?}", m.pos);
        // Gone once nobody is near.
        let gone = run(&mut mobs, &t, &[Player { pos: [200.0, 0.5, 0.0], ..far[0] }], 0.1, &mut rng);
        assert!(gone.contains(&Event::Removed(FIRST_ID)) && mobs.is_empty());
    }

    #[test]
    fn hunts_and_hits() {
        let t = Flat::new();
        let mut rng = Rng::new();
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Mossling, &[[0.0, 0.5, 0.0], [1.0, 0.5, 0.0]], &mut rng);
        let player = [Player { id: 2, pos: [10.0, 0.5, 0.0], alive: true }];
        let ev = run(&mut mobs, &t, &player, 6.0, &mut rng);
        assert!(ev.iter().any(|e| matches!(e, Event::Sound(_, s, _, _, _) if *s == MOSSLING.call)));
        let hits = ev.iter().filter(|e| matches!(e, Event::Hit { player: 2, damage: 2, .. })).count();
        assert!(hits >= 2, "{hits} hits");
        for m in mobs.iter() {
            assert!(dist2(m.pos, player[0].pos) < 4.0, "{:?}", m.pos);
            assert_eq!(m.state, State::Chase);
        }
        // Not through a wall: the player behind one isn't seen.
        let mut t = Flat::new();
        for y in 1..=3 {
            for z in -8..=8 {
                t.walls.push((5, y, z));
            }
        }
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Mossling, &[[0.0, 0.5, 0.0]], &mut rng);
        let ev = run(&mut mobs, &t, &player, 5.0, &mut rng);
        assert!(!ev.iter().any(|e| matches!(e, Event::Hit { .. })));
        assert!(mobs.iter().all(|m| m.pos[0] < 5.0));
    }

    #[test]
    fn jumps_a_step_but_not_into_water() {
        let mut t = Flat::new();
        for z in -3..=3 {
            t.walls.push((3, 1, z));
        }
        let mut rng = Rng::new();
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Mossling, &[[0.0, 0.5, 0.0]], &mut rng);
        let player = [Player { id: 2, pos: [8.0, 1.5, 0.0], alive: true }];
        // Standing on a block in the open: on top of the step's row.
        let mut t2 = Flat { walls: t.walls.clone(), ..Flat::new() };
        t2.walls.extend((4..=10).flat_map(|x| (-3..=3).map(move |z| (x, 1, z))));
        let ev = run(&mut mobs, &t2, &player, 6.0, &mut rng);
        assert!(ev.iter().any(|e| matches!(e, Event::Hit { .. })), "{:?}", mobs.iter().next().map(|m| m.pos));
        // Water between: it waits at the edge.
        let mut t3 = Flat::new();
        for x in 3..=5 {
            for z in -10..=10 {
                t3.water.push((x, 0, z));
            }
        }
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Mossling, &[[0.0, 0.5, 0.0]], &mut rng);
        let player = [Player { id: 2, pos: [8.0, 0.5, 0.0], alive: true }];
        let ev = run(&mut mobs, &t3, &player, 6.0, &mut rng);
        assert!(!ev.iter().any(|e| matches!(e, Event::Hit { .. })));
        let m = mobs.iter().next().unwrap();
        assert!(m.pos[0] < 2.8 && m.pos[1] > 0.4, "{:?}", m.pos);
    }

    #[test]
    fn punched() {
        let mut rng = Rng::new();
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Mossling, &[[0.0, 0.5, 0.0]], &mut rng);
        assert!(mobs.since_punch(2) > 100.0);
        let r = mobs.punch(FIRST_ID, 2, [-2.0, 0.5, 0.0], 4).unwrap();
        assert_eq!(r.hp, 6);
        let m = mobs.get(FIRST_ID).unwrap();
        assert!(m.vel[0] > 3.0 && m.target == Some(2));
        let r = mobs.punch(FIRST_ID, 2, [-2.0, 0.5, 0.0], 7).unwrap();
        assert_eq!(r.hp, 0);
        assert!(mobs.is_empty() && mobs.punch(FIRST_ID, 2, [0.0; 3], 1).is_none());
        // A stone sword: 4 at full strength, 2 if swung again too soon.
        let sword = content::caps_of(Some(content::TOOL_SWORD_STONE));
        let (full, wear) = hit_params(sword, 5.0, 0);
        assert_eq!(full, sword.fleshy as u16);
        assert!(wear > 0);
        let (half, _) = hit_params(sword, sword.full_punch_interval / 2.0, 0);
        assert_eq!(half, (sword.fleshy / 2) as u16);
        assert_eq!(hit_params(&content::HAND, 5.0, 0), (1, 0));
    }

    #[test]
    fn spawning() {
        let mut t = Flat::new();
        let mut rng = Rng::new();
        let player = [Player { id: 2, pos: [0.0, 0.5, 0.0], alive: true }];
        // No trees: no mosslings, but wisps (no more than two).
        let mut mobs = Mobs::new();
        for _ in 0..50 {
            mobs.spawn(&t, &player, &mut rng);
        }
        assert!(mobs.iter().all(|m| m.kind == Kind::Wisp));
        assert_eq!(mobs.len(), WISPS_PER_PLAYER);
        assert!(mobs.iter().all(|m| (m.pos[1] - 2.0).abs() < 0.01));
        let mut mobs = Mobs::new();
        // A forest all round: packs, up to the cap, at a distance.
        for x in (-50..=50).step_by(4) {
            for z in (-50..=50).step_by(4) {
                for y in 1..=4 {
                    t.trees.push((x, y, z));
                }
            }
        }
        for _ in 0..50 {
            mobs.spawn(&t, &player, &mut rng);
        }
        assert_eq!(mobs.len(), PER_PLAYER.min(MAX_MOBS));
        for m in mobs.iter() {
            let d = dist2(m.pos, player[0].pos).sqrt();
            assert!((SPAWN_MIN - 1.0..=SPAWN_MAX + 2.0).contains(&d), "{d}");
            assert!((m.pos[1] - if m.kind == Kind::Wisp { 2.0 } else { 0.5 }).abs() < 0.01);
        }
        assert!(mobs.iter().filter(|m| m.kind == Kind::Wisp).count() <= WISPS_PER_PLAYER);
        // Lit by a torch: none.
        let mut lit = Flat { trees: t.trees.clone(), ..Flat::new() };
        lit.torch = Some((0, 1, 0));
        assert_eq!(lit.light((0, 1, 30)), 0);
        assert!(lit.light((0, 1, 3)) > SPAWN_LIGHT);
        // The night and the day.
        assert!(dark(20000) && dark(1000) && !dark(12000));
        assert!(daytime(6000) && !daytime(23000));
    }

    fn run_wisp(mobs: &mut Mobs, t: &Flat, player: &mut Player, secs: f32, away: Option<f32>, rng: &mut Rng) -> Vec<Event> {
        let mut all = Vec::new();
        for _ in 0..(secs / 0.05) as usize {
            let ev = mobs.step(0.05, t, &[*player], false, rng);
            // The player backs off as soon as the fuse is lit.
            if let Some(speed) = away {
                if all.iter().chain(&ev).any(|e| matches!(e, Event::Fuse(_, true))) {
                    player.pos[0] += speed * 0.05;
                }
            }
            all.extend(ev);
        }
        all
    }

    #[test]
    fn wisp_bursts() {
        let t = Flat::new();
        let mut rng = Rng::new();
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Wisp, &[[0.0, 2.0, 0.0]], &mut rng);
        let mut player = Player { id: 2, pos: [12.0, 0.5, 0.0], alive: true };
        // It hangs back a few seconds, then rushes, lights and bursts.
        let ev = run_wisp(&mut mobs, &t, &mut player, 15.0, None, &mut rng);
        let lit = ev.iter().position(|e| matches!(e, Event::Fuse(_, true))).expect("never lit");
        let burst = ev.iter().position(|e| matches!(e, Event::Burst { radius: 2, .. })).expect("never burst");
        assert!(lit < burst && ev[burst + 1..].contains(&Event::Removed(FIRST_ID)));
        assert!(mobs.is_empty());
        let Event::Burst { pos, .. } = ev[burst] else { unreachable!() };
        assert!(dist2(pos, [12.0, 1.5, 0.0]) < 3.0 * 3.0, "{pos:?}");
        // Backing off as it lights, the player gets away: it goes out.
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Wisp, &[[0.0, 2.0, 0.0]], &mut rng);
        let mut player = Player { id: 2, pos: [12.0, 0.5, 0.0], alive: true };
        let ev = run_wisp(&mut mobs, &t, &mut player, 3.0 + 6.0, Some(8.0), &mut rng);
        assert!(ev.iter().any(|e| matches!(e, Event::Fuse(_, false))), "{ev:?}");
        assert!(!ev.iter().any(|e| matches!(e, Event::Burst { .. })));
    }

    #[test]
    fn wisp_floats_and_water_puts_it_out() {
        let mut t = Flat::new();
        let mut rng = Rng::new();
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Wisp, &[[0.0, 2.0, 0.0]], &mut rng);
        let far = [Player { id: 2, pos: [60.0, 0.5, 0.0], alive: true }];
        run(&mut mobs, &t, &far, 20.0, &mut rng);
        let m = mobs.get(FIRST_ID).unwrap();
        assert!((1.4..=5.5).contains(&m.pos[1]), "{:?}", m.pos);
        // Into water: gone.
        let (x, y, z) = node_at(m.pos);
        t.water.push((x, y, z));
        let ev = run(&mut mobs, &t, &far, 0.1, &mut rng);
        assert!(ev.contains(&Event::Removed(FIRST_ID)), "{ev:?}");
        // Hit: knocked back, and hurt (6 HP: a stone sword's two blows).
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Wisp, &[[0.0, 2.0, 0.0]], &mut rng);
        assert_eq!(mobs.punch(FIRST_ID, 2, [-1.0, 1.5, 0.0], 4).unwrap().hp, 2);
        assert_eq!(mobs.harm(FIRST_ID, None, [-1.0, 1.5, 0.0], 3).unwrap().hp, 0);
    }

    #[test]
    fn sinks_at_dawn() {
        let t = Flat::new();
        let mut rng = Rng::new();
        let mut mobs = Mobs::new();
        mobs.add_pack(Kind::Mossling, &[[0.0, 0.5, 0.0], [2.0, 0.5, 0.0]], &mut rng);
        let player = [Player { id: 2, pos: [30.0, 0.5, 0.0], alive: true }];
        let mut all = Vec::new();
        for _ in 0..240 {
            all.extend(mobs.step(0.05, &t, &player, true, &mut rng));
        }
        assert!(mobs.is_empty());
        assert_eq!(all.iter().filter(|e| matches!(e, Event::Sinking(_))).count(), 2);
        assert_eq!(all.iter().filter(|e| matches!(e, Event::Removed(_))).count(), 2);
    }

    #[test]
    fn sight() {
        let mut t = Flat::new();
        assert!(clear_line([0.0, 1.3, 0.0], [10.0, 2.0, 3.0], &t));
        t.walls.push((5, 2, 1));
        t.walls.push((5, 1, 1));
        t.walls.push((5, 2, 2));
        t.walls.push((5, 1, 2));
        assert!(!clear_line([0.0, 1.3, 0.0], [10.0, 2.0, 3.0], &t));
        assert!(clear_line([0.0, 1.3, 0.0], [10.0, 1.3, -3.0], &t));
        assert_eq!(yaw_of([0.0, 1.0]), 0.0);
        assert_eq!(yaw_of([1.0, 0.0]), 270.0);
    }
}
