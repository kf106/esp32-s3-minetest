//! Luanti commands (the payload of ORIGINAL packets): a u16 command id
//! followed by command-specific fields, all big-endian. Floats are IEEE 754
//! f32, strings carry a u16 length prefix.
//!
//! Field layouts follow src/network/networkprotocol.h in the Luanti sources.

use crate::zstd::compress;

// Client to server
pub const TOSERVER_INIT: u16 = 0x02;
pub const TOSERVER_INIT2: u16 = 0x11;
pub const TOSERVER_PLAYERPOS: u16 = 0x23;
pub const TOSERVER_GOTBLOCKS: u16 = 0x24;
pub const TOSERVER_DELETEDBLOCKS: u16 = 0x25;
pub const TOSERVER_REQUEST_MEDIA: u16 = 0x40;
pub const TOSERVER_CLIENT_READY: u16 = 0x43;
pub const TOSERVER_FIRST_SRP: u16 = 0x50;
pub const TOSERVER_SRP_BYTES_A: u16 = 0x51;
pub const TOSERVER_SRP_BYTES_M: u16 = 0x52;
pub const TOSERVER_INTERACT: u16 = 0x39;
pub const TOSERVER_DAMAGE: u16 = 0x35;
pub const TOSERVER_RESPAWN_LEGACY: u16 = 0x38;
pub const TOSERVER_INVENTORY_ACTION: u16 = 0x31;
pub const TOSERVER_INVENTORY_FIELDS: u16 = 0x3c;
pub const TOSERVER_NODEMETA_FIELDS: u16 = 0x3b;
pub const TOSERVER_CHAT_MESSAGE: u16 = 0x32;
pub const TOSERVER_PLAYERITEM: u16 = 0x37;

// Server to client
pub const TOCLIENT_HELLO: u16 = 0x02;
pub const TOCLIENT_AUTH_ACCEPT: u16 = 0x03;
pub const TOCLIENT_ACCESS_DENIED: u16 = 0x0A;
pub const TOCLIENT_BLOCKDATA: u16 = 0x20;
pub const TOCLIENT_ADDNODE: u16 = 0x21;
pub const TOCLIENT_NODEMETA_CHANGED: u16 = 0x59;
pub const TOCLIENT_REMOVENODE: u16 = 0x22;
pub const TOCLIENT_INVENTORY: u16 = 0x27;
pub const TOCLIENT_TIME_OF_DAY: u16 = 0x29;
pub const TOCLIENT_CSM_RESTRICTION_FLAGS: u16 = 0x2A;
pub const TOCLIENT_ACTIVE_OBJECT_REMOVE_ADD: u16 = 0x31;
pub const TOCLIENT_ACTIVE_OBJECT_MESSAGES: u16 = 0x32;
pub const TOCLIENT_MOVE_PLAYER: u16 = 0x34;
pub const TOCLIENT_HP: u16 = 0x33;
pub const TOCLIENT_CHAT_MESSAGE: u16 = 0x2F;
pub const TOCLIENT_DEATHSCREEN_LEGACY: u16 = 0x37;
pub const TOCLIENT_HUDADD: u16 = 0x49;
pub const TOCLIENT_HUDCHANGE: u16 = 0x4b;
pub const TOCLIENT_HUDRM: u16 = 0x4a;
pub const TOCLIENT_BREATH: u16 = 0x4e;
pub const TOCLIENT_LOCAL_PLAYER_ANIMATIONS: u16 = 0x51;
pub const TOCLIENT_MEDIA: u16 = 0x38;
pub const TOCLIENT_NODEDEF: u16 = 0x3a;
pub const TOCLIENT_ANNOUNCE_MEDIA: u16 = 0x3c;
pub const TOCLIENT_ITEMDEF: u16 = 0x3d;
pub const TOCLIENT_PLAY_SOUND: u16 = 0x3f;
pub const TOCLIENT_ADD_PARTICLESPAWNER: u16 = 0x47;
pub const TOCLIENT_DELETE_PARTICLESPAWNER: u16 = 0x53;
pub const TOCLIENT_STOP_SOUND: u16 = 0x40;
pub const TOCLIENT_PLAYER_SPEED: u16 = 0x2b;
pub const TOCLIENT_FADE_SOUND: u16 = 0x55;
pub const TOCLIENT_PRIVILEGES: u16 = 0x41;
pub const TOCLIENT_INVENTORY_FORMSPEC: u16 = 0x42;
pub const TOCLIENT_DETACHED_INVENTORY: u16 = 0x43;
pub const TOCLIENT_SHOW_FORMSPEC: u16 = 0x44;
pub const TOCLIENT_MOVEMENT: u16 = 0x45;
pub const TOCLIENT_HUD_SET_FLAGS: u16 = 0x4c;
pub const TOCLIENT_EYE_OFFSET: u16 = 0x52;
pub const TOCLIENT_CLOUD_PARAMS: u16 = 0x54;
pub const TOCLIENT_SET_LIGHTING: u16 = 0x63;
pub const TOCLIENT_SRP_BYTES_S_B: u16 = 0x60;

/// Highest map serialisation format we speak.
pub const SER_FMT_VER_HIGHEST: u8 = 29;
/// Network protocol versions we accept.
pub const SERVER_PROTO_MIN: u16 = 37;
pub const SERVER_PROTO_MAX: u16 = 53;

pub const AUTH_MECHANISM_SRP: u32 = 1 << 1;
pub const AUTH_MECHANISM_FIRST_SRP: u32 = 1 << 2;

/// Reasons for TOCLIENT_ACCESS_DENIED (AccessDeniedCode in Luanti).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum AccessDenied {
    WrongPassword = 0,
    UnexpectedData = 1,
    Singleplayer = 2,
    WrongVersion = 3,
    WrongCharsInName = 4,
    WrongName = 5,
    TooManyUsers = 6,
    EmptyPassword = 7,
    AlreadyConnected = 8,
    ServerFail = 9,
    CustomString = 10,
    Shutdown = 11,
    Crash = 12,
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.buf.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }

    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn i16(&mut self) -> Option<i16> {
        Some(self.u16()? as i16)
    }

    pub fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Option<i32> {
        let b = self.take(4)?;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// Byte string with a u16 length prefix.
    pub fn bytes16(&mut self) -> Option<Vec<u8>> {
        let len = self.u16()? as usize;
        Some(self.take(len)?.to_vec())
    }

    /// Byte string with a u32 length prefix.
    pub fn bytes32(&mut self) -> Option<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    /// Everything not yet read.
    pub fn rest(&mut self) -> &'a [u8] {
        let r = &self.buf[self.pos.min(self.buf.len())..];
        self.pos = self.buf.len();
        r
    }

    /// Text string with a u16 length prefix.
    pub fn string16(&mut self) -> Option<String> {
        let b = self.bytes16()?;
        Some(String::from_utf8_lossy(&b).into_owned())
    }
}

pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// An empty buffer with room for `n` bytes.
    pub fn with_capacity(n: usize) -> Self {
        Self { buf: Vec::with_capacity(n) }
    }

    /// A command payload, starting with its u16 command id.
    pub fn new(command: u16) -> Self {
        let mut w = Self::plain();
        w.u16(command);
        w
    }

    /// A plain buffer with no command id, for nested structures.
    pub fn plain() -> Self {
        Self {
            buf: Vec::with_capacity(64),
        }
    }

    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    /// Bytes with a u16 length prefix (Luanti's serializeString16).
    pub fn bytes16(&mut self, b: &[u8]) -> &mut Self {
        self.u16(b.len() as u16);
        self.raw(b)
    }

    /// Bytes with a u32 length prefix (serializeString32 / putLongString).
    pub fn bytes32(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.raw(b)
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_bits().to_be_bytes());
        self
    }

    pub fn string16(&mut self, s: &str) -> &mut Self {
        self.u16(s.len() as u16);
        self.buf.extend_from_slice(s.as_bytes());
        self
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}

pub fn command_id(payload: &[u8]) -> Option<u16> {
    Reader::new(payload).u16()
}

// ---------------------------------------------------------------------------
// Client to server
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ToServerInit {
    pub max_ser_ver: u8,
    pub min_proto: u16,
    pub max_proto: u16,
    pub name: String,
}

pub fn parse_init(payload: &[u8]) -> Option<ToServerInit> {
    let mut r = Reader::new(payload);
    if r.u16()? != TOSERVER_INIT {
        return None;
    }
    let max_ser_ver = r.u8()?;
    r.u16()?; // unused: network compression modes
    Some(ToServerInit {
        max_ser_ver,
        min_proto: r.u16()?,
        max_proto: r.u16()?,
        name: r.string16()?,
    })
}

pub struct ToServerFirstSrp {
    pub salt: Vec<u8>,
    pub verifier: Vec<u8>,
    pub is_empty: bool,
}

pub fn parse_first_srp(payload: &[u8]) -> Option<ToServerFirstSrp> {
    let mut r = Reader::new(payload);
    if r.u16()? != TOSERVER_FIRST_SRP {
        return None;
    }
    Some(ToServerFirstSrp {
        salt: r.bytes16()?,
        verifier: r.bytes16()?,
        is_empty: r.u8()? != 0,
    })
}

/// TOSERVER_INIT2 optionally carries the client's language code.
pub fn parse_init2_lang(payload: &[u8]) -> Option<String> {
    let mut r = Reader::new(payload);
    r.u16()?;
    r.string16()
}

// ---------------------------------------------------------------------------
// Server to client
// ---------------------------------------------------------------------------

/// TOCLIENT_HELLO: the server's reply to TOSERVER_INIT.
pub fn build_hello(ser_ver: u8, proto: u16, auth_mechanisms: u32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HELLO);
    w.u8(ser_ver)
        .u16(0) // unused (network compression)
        .u16(proto)
        .u32(auth_mechanisms)
        .string16(""); // unused (used to be the username)
    w.into_inner()
}

/// TOCLIENT_AUTH_ACCEPT: login succeeded.
pub fn build_auth_accept(map_seed: u64, send_interval: f32, sudo_auth_mechanisms: u32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_AUTH_ACCEPT);
    w.f32(0.0)
        .f32(0.0)
        .f32(0.0) // v3f, unused
        .u64(map_seed)
        .f32(send_interval)
        .u32(sudo_auth_mechanisms);
    w.into_inner()
}

/// TOCLIENT_ACCESS_DENIED: the client shows `reason` (or `custom` text) and disconnects.
pub fn build_access_denied(reason: AccessDenied, custom: &str, reconnect: bool) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_ACCESS_DENIED);
    w.u8(reason as u8).string16(custom).u8(reconnect as u8);
    w.into_inner()
}

// ---------------------------------------------------------------------------
// Content (sent after TOSERVER_INIT2)
// ---------------------------------------------------------------------------

/// TOCLIENT_ITEMDEF: the zstd-compressed ItemDefManager.
pub fn build_itemdef(serialized: &[u8]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_ITEMDEF);
    w.bytes32(&compress(serialized));
    w.into_inner()
}

/// TOCLIENT_NODEDEF: the zstd-compressed NodeDefManager.
pub fn build_nodedef(serialized: &[u8]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_NODEDEF);
    w.bytes32(&compress(serialized));
    w.into_inner()
}

/// TOCLIENT_ANNOUNCE_MEDIA (protocol >= 48): a compressed array of file
/// names, then the raw SHA-1 of each file, then remote media servers.
pub fn build_announce_media<'a>(files: impl Iterator<Item = (&'a str, &'a [u8; 20])> + Clone) -> Vec<u8> {
    // serializeString16Array: u32 count, all u16 lengths, then all strings
    let names: Vec<&str> = files.clone().map(|(n, _)| n).collect();
    let mut array = Writer::plain();
    array.u32(names.len() as u32);
    for n in &names {
        array.u16(n.len() as u16);
    }
    for n in &names {
        array.raw(n.as_bytes());
    }

    let mut w = Writer::new(TOCLIENT_ANNOUNCE_MEDIA);
    w.bytes32(&compress(&array.into_inner()));
    for (_, sha1) in files {
        w.raw(sha1);
    }
    w.string16(""); // no remote media servers
    w.into_inner()
}

/// TOCLIENT_MEDIA packets for the requested files, in bunches of about
/// 5 KB like the Luanti server. Each file is zstd-compressed.
pub fn build_media_bunches(files: &[(&str, &[u8])]) -> Vec<Vec<u8>> {
    const BYTES_PER_BUNCH: usize = 5000;

    let mut bunches: Vec<Vec<(&str, Vec<u8>)>> = vec![Vec::new()];
    let mut bunch_size = 0;
    for (name, data) in files {
        let compressed = compress(data);
        bunch_size += compressed.len();
        bunches.last_mut().unwrap().push((name, compressed));
        if bunch_size >= BYTES_PER_BUNCH {
            bunches.push(Vec::new());
            bunch_size = 0;
        }
    }
    if bunches.len() > 1 && bunches.last().unwrap().is_empty() {
        bunches.pop();
    }

    let count = bunches.len() as u16;
    bunches
        .iter()
        .enumerate()
        .map(|(i, bunch)| {
            let mut w = Writer::new(TOCLIENT_MEDIA);
            w.u16(count).u16(i as u16).u32(bunch.len() as u32);
            for (name, data) in bunch {
                w.string16(name).bytes32(data);
            }
            w.into_inner()
        })
        .collect()
}

/// TOCLIENT_MOVEMENT with Luanti's default physics settings.
pub fn build_movement() -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_MOVEMENT);
    for v in [
        3.0,  // acceleration_default
        2.0,  // acceleration_air
        10.0, // acceleration_fast
        4.0,  // speed_walk
        1.35, // speed_crouch
        20.0, // speed_fast
        3.0,  // speed_climb
        6.5,  // speed_jump
        1.0,  // liquid_fluidity
        0.5,  // liquid_fluidity_smooth
        10.0, // liquid_sink
        9.81, // gravity
    ] {
        w.f32(v);
    }
    w.into_inner()
}

/// TOCLIENT_TIME_OF_DAY: time in 0..24000 (6000 = 06:00), and how many
/// times faster than real time the day runs (Luanti default 72).
pub fn build_time_of_day(time: u16, time_speed: f32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_TIME_OF_DAY);
    w.u16(time).f32(time_speed);
    w.into_inner()
}

/// TOCLIENT_CSM_RESTRICTION_FLAGS: no client-side mod restrictions.
pub fn build_csm_restriction_flags() -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_CSM_RESTRICTION_FLAGS);
    w.u64(0).u32(8); // flags, node range
    w.into_inner()
}

pub fn parse_request_media(payload: &[u8]) -> Option<Vec<String>> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let count = r.u16()?;
    (0..count).map(|_| r.string16()).collect()
}

#[derive(Debug)]
pub struct ClientReady {
    pub version: String,
    pub formspec_version: u16,
}

pub fn parse_client_ready(payload: &[u8]) -> Option<ClientReady> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let (major, minor, patch) = (r.u8()?, r.u8()?, r.u8()?);
    r.u8()?; // reserved
    let full = r.string16()?;
    let formspec_version = if r.remaining() >= 2 { r.u16()? } else { 1 };
    Some(ClientReady {
        version: format!("{major}.{minor}.{patch} ({full})"),
        formspec_version,
    })
}

// ---------------------------------------------------------------------------
// In game
// ---------------------------------------------------------------------------

/// TOCLIENT_BLOCKDATA: `payload` is position + compressed block, from
/// `world::block_payload`.
pub fn build_blockdata(payload: &[u8]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_BLOCKDATA);
    w.raw(payload);
    w.into_inner()
}

/// TOCLIENT_MOVE_PLAYER: position in BS units (10 per node), pitch and yaw
/// in degrees.
pub fn build_move_player(pos: [f32; 3], pitch: f32, yaw: f32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_MOVE_PLAYER);
    w.f32(pos[0]).f32(pos[1]).f32(pos[2]).f32(pitch).f32(yaw);
    w.into_inner()
}

pub fn build_privileges(privs: &[&str]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_PRIVILEGES);
    w.u16(privs.len() as u16);
    for p in privs {
        w.string16(p);
    }
    w.into_inner()
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerPos {
    /// Position in BS units (10 per node); the packet carries it x 100.
    pub pos_bs: [f32; 3],
    /// Velocity in BS units per second.
    pub speed_bs: [f32; 3],
    pub pitch: f32,
    pub yaw: f32,
    /// Bit 0-3 up/down/left/right, 4 jump, 5 aux1, 6 sneak, 7 dig, 8 place.
    pub keys: u32,
    /// View range the client wants, in map blocks.
    pub wanted_range: u8,
    /// Analogue movement 0..1 (joysticks), sent by clients since 5.10.
    pub movement_speed: f32,
}

impl PlayerPos {
    /// Position in nodes.
    pub fn pos_nodes(&self) -> [f32; 3] {
        self.pos_bs.map(|v| v / 10.0)
    }
}

pub fn parse_playerpos(payload: &[u8]) -> Option<PlayerPos> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let mut pos_bs = [0f32; 3];
    for p in pos_bs.iter_mut() {
        *p = r.i32()? as f32 / 100.0;
    }
    let mut speed_bs = [0f32; 3];
    for v in speed_bs.iter_mut() {
        *v = r.i32()? as f32 / 100.0;
    }
    let pitch = r.i32()? as f32 / 100.0;
    let yaw = r.i32()? as f32 / 100.0;
    let keys = r.u32()?;
    r.u8()?; // fov
    let wanted_range = r.u8()?;
    // Optional: u8 flags (>= 5.8), then movement speed and direction (>= 5.10)
    let mut movement_speed = 0.0;
    if r.remaining() >= 1 + 8 {
        r.u8()?;
        let v = f32::from_bits(r.u32()?);
        if v.is_finite() {
            movement_speed = v.clamp(0.0, 1.0);
        }
    }
    Some(PlayerPos { pos_bs, speed_bs, pitch, yaw, keys, wanted_range, movement_speed })
}

/// TOSERVER_GOTBLOCKS / TOSERVER_DELETEDBLOCKS: u8 count, then v3s16 positions.
pub fn parse_block_list(payload: &[u8]) -> Option<Vec<(i16, i16, i16)>> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let count = r.u8()?;
    (0..count).map(|_| Some((r.i16()?, r.i16()?, r.i16()?))).collect()
}

// ---------------------------------------------------------------------------
// Active objects (players)
// ---------------------------------------------------------------------------

pub const ACTIVEOBJECT_TYPE_GENERIC: u8 = 101;
const AO_CMD_SET_PROPERTIES: u8 = 0;
const AO_CMD_UPDATE_POSITION: u8 = 1;
const AO_CMD_SET_ANIMATION: u8 = 6;
const AO_CMD_SET_BONE_POSITION: u8 = 7;
const AO_CMD_UPDATE_ARMOR_GROUPS: u8 = 5;
const AO_CMD_ATTACH_TO: u8 = 8;
const AO_CMD_SET_PHYSICS_OVERRIDE: u8 = 9;
pub const PLAYER_MAX_HP: u16 = 20;

/// The object properties we use (ObjectProperties in Luanti), in nodes.
/// Anything not here is sent at the engine's default.
pub struct Props<'a> {
    pub hp_max: u16,
    pub physical: bool,
    pub collisionbox: [f32; 6],
    pub selectionbox: [f32; 6],
    pub visual: &'a str,
    pub visual_size: [f32; 3],
    pub textures: &'a [&'a str],
    pub makes_footstep_sound: bool,
    /// Radians per second.
    pub automatic_rotate: f32,
    pub mesh: &'a str,
    pub collide_with_objects: bool,
    /// In BS units.
    pub stepheight: f32,
    pub infotext: &'a str,
    pub wield_item: &'a str,
    pub glow: i8,
    pub eye_height: f32,
    pub damage_texture_modifier: &'a str,
    /// The zoomed view, in degrees (0: no zoom): binoculars.rs.
    pub zoom_fov: f32,
    /// A sprite's frames: columns and rows of its texture.
    pub spritediv: [i16; 2],
    /// Turn to face the way it moves, with this offset in degrees.
    pub face_movement: Option<f32>,
    pub use_texture_alpha: bool,
}

/// AO_CMD_SET_PROPERTIES (ObjectProperties::serialize).
pub fn properties_message(p: &Props) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_SET_PROPERTIES);
    w.u8(4) // ObjectProperties version
        .u16(p.hp_max)
        .u8(p.physical as u8)
        .f32(0.0); // removed property (weight)
    for v in p.collisionbox.into_iter().chain(p.selectionbox) {
        w.f32(v);
    }
    w.u8(1) // pointable
        .string16(p.visual);
    for v in p.visual_size {
        w.f32(v);
    }
    w.u16(p.textures.len() as u16);
    for t in p.textures {
        w.string16(t);
    }
    w.i16(p.spritediv[0])
        .i16(p.spritediv[1]) // spritediv
        .i16(0)
        .i16(0) // initial sprite position
        .u8(1) // is_visible
        .u8(p.makes_footstep_sound as u8)
        .f32(p.automatic_rotate)
        .string16(p.mesh)
        .u16(0) // colours
        .u8(p.collide_with_objects as u8)
        .f32(p.stepheight)
        .u8(p.face_movement.is_some() as u8) // automatic_face_movement_dir
        .f32(p.face_movement.unwrap_or(0.0)) // ... offset
        .u8(0) // backface_culling
        .string16("") // nametag (the client uses a player's name)
        .u32(0xFFFF_FFFF) // nametag colour
        .f32(if p.face_movement.is_some() { 360.0 } else { -1.0 }) // automatic_face_movement_max_rotation_per_sec
        .string16(p.infotext)
        .string16(p.wield_item)
        .u8(p.glow as u8)
        .u16(10) // breath_max
        .f32(p.eye_height)
        .f32(p.zoom_fov) // zoom_fov (0 = no zoom)
        .u8(p.use_texture_alpha as u8) // use_texture_alpha
        // >= 5.3
        .string16(p.damage_texture_modifier)
        .u8(1) // shaded
        // >= 5.4
        .u8(1) // show_on_minimap
        .u32(0x0001_0101) // nametag background: "unset" marker
        // >= 5.7
        .u8(0) // rotate_selectionbox
        // >= 5.12: node (for the "node" visual): ignore
        .u16(127)
        .u8(0)
        .u8(0)
        // >= 5.14
        .u32(u32::MAX) // nametag font size: unset
        .u8(0) // nametag_scale_z
        // >= 5.16
        .u8(0); // step_up_mode: legacy
    w.into_inner()
}

/// The Luanti engine's default player properties (PlayerSAO's
/// constructor), but physical, with player_api's model wearing `skin` (a texture). For
/// the local player the client also takes its collision box, eye height
/// and zoom from these. Sent again when the player changes skin, and when
/// they lie down in a bed or sit in a boat, or get up: player_api's "lay"
/// animation has a low, wide box and eyes 0.3 above the ground, "sit" a
/// box a node high and eyes at 0.8.
pub fn player_properties_message(skin: &str, posture: Posture, zoom_fov: f32) -> Vec<u8> {
    let (collisionbox, eye_height) = match posture {
        Posture::Lie => ([-0.6, 0.0, -0.6, 0.6, 0.3, 0.6], 0.3),
        Posture::Sit => ([-0.3, 0.0, -0.3, 0.3, 1.0, 0.3], 0.8),
        Posture::Stand => ([-0.3, 0.0, -0.3, 0.3, 1.7, 0.3], 1.47), // player_api's "stand" box
    };
    let lying = posture != Posture::Stand;
    properties_message(&Props {
        hp_max: PLAYER_MAX_HP,
        // Solid to other players' clients, so players can't walk through
        // each other. (Luanti leaves players non-physical, so they can.)
        // Not while lying in bed, so a bed beside someone asleep can still
        // be reached (nor sitting in a boat).
        physical: !lying,
        collisionbox,
        selectionbox: [-0.3, 0.0, -0.3, 0.3, 1.77, 0.3],
        visual: "mesh",
        visual_size: [1.0; 3],
        textures: &[skin],
        makes_footstep_sound: true,
        automatic_rotate: 0.0,
        mesh: "character.b3d",
        collide_with_objects: true,
        stepheight: 0.6 * 10.0, // as PlayerSAO sets it
        infotext: "",
        wield_item: "",
        glow: 0,
        eye_height, // player_api
        damage_texture_modifier: "^[brighten",
        zoom_fov,
        spritediv: [1, 1],
        face_movement: None,
        use_texture_alpha: false,
    })
}

/// How a player is: standing (or walking), lying in bed, or sitting in a
/// boat (player_api's "lay" and "sit" animations).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Posture {
    Stand,
    Lie,
    Sit,
}

/// A frame range of character.b3d, as player_api defines them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animation {
    pub start: f32,
    pub end: f32,
    pub fps: f32,
}

pub const ANIM_FPS: f32 = 30.0;
pub const ANIM_STAND: (f32, f32) = (0.0, 79.0);
pub const ANIM_WALK: (f32, f32) = (168.0, 187.0);
pub const ANIM_MINE: (f32, f32) = (189.0, 198.0);
pub const ANIM_WALK_MINE: (f32, f32) = (200.0, 219.0);
pub const ANIM_LAY: (f32, f32) = (162.0, 166.0);
pub const ANIM_SIT: (f32, f32) = (81.0, 160.0);

impl Animation {
    pub fn new(range: (f32, f32), fps: f32) -> Self {
        Self { start: range.0, end: range.1, fps }
    }
}

/// AO_CMD_SET_ANIMATION in the 5.17 form: looping, on track 0, starting
/// at the first frame.
pub fn set_animation_message(anim: Animation) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_SET_ANIMATION)
        .f32(anim.start)
        .f32(anim.end)
        .f32(anim.fps)
        .f32(0.0) // blend duration
        .u8(0) // "no loop", sent inverted: 0 = loop
        // >= 5.17
        .u16(1) // track id: numeric track 0, written as 0 + 1
        .i32(0) // priority
        .f32(anim.start); // current frame
    w.into_inner()
}

/// TOCLIENT_LOCAL_PLAYER_ANIMATIONS: the ranges the client uses to animate
/// its own player (seen in third-person view), as player_api sets them.
/// With `on` false, none at all, which suspends them (player_api, while
/// the player lies in a bed).
pub fn build_local_player_animations(on: bool) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_LOCAL_PLAYER_ANIMATIONS);
    for (a, b) in [ANIM_STAND, ANIM_WALK, ANIM_MINE, ANIM_WALK_MINE] {
        if on {
            w.f32(a).f32(b);
        } else {
            w.f32(0.0).f32(0.0);
        }
    }
    w.f32(if on { ANIM_FPS } else { 1.0 });
    w.into_inner()
}

/// The HUD flag for the wielded item in the player's hand.
pub const HUD_FLAG_WIELDITEM_VISIBLE: u32 = 1 << 3;

/// TOCLIENT_HUD_SET_FLAGS: the flags in `mask` are set as in `flags`.
pub fn build_hud_set_flags(flags: u32, mask: u32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HUD_SET_FLAGS);
    w.u32(flags).u32(mask);
    w.into_inner()
}

/// AO_CMD_UPDATE_ARMOR_GROUPS as PlayerSAO sends it with damage enabled.
/// (An "immortal" group would stop the client applying fall damage.)
pub fn armor_groups_message() -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_UPDATE_ARMOR_GROUPS).u16(1).string16("fleshy").i16(100);
    w.into_inner()
}

/// AO_CMD_ATTACH_TO: to object `parent` (no bone), `pos` from it in its
/// own units, not turned, not shown to a local player in first person.
pub fn attach_message(parent: u16, pos: [f32; 3]) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_ATTACH_TO).i16(parent as i16).string16("");
    for v in pos {
        w.f32(v);
    }
    w.f32(0.0).f32(0.0).f32(0.0).u8(0);
    w.into_inner()
}

/// TOCLIENT_EYE_OFFSET: the player's eyes moved by `offset` (BS units) in
/// first person and both third-person views (set_eye_offset).
pub fn build_eye_offset(offset: [f32; 3]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_EYE_OFFSET);
    for _ in 0..3 {
        for v in offset {
            w.f32(v);
        }
    }
    w.into_inner()
}

/// AO_CMD_UPDATE_ARMOR_GROUPS {immortal = 1}: punches do it no harm (a
/// boat's on_activate).
pub fn immortal_message() -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_UPDATE_ARMOR_GROUPS).u16(1).string16("immortal").i16(1);
    w.into_inner()
}

/// AO_CMD_ATTACH_TO with no parent. Required: the client keeps a remote
/// player's figure hidden (GenericCAO::m_is_visible starts false) until it
/// processes an attachment message, as UnitSAO always sends one.
pub fn not_attached_message() -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_ATTACH_TO)
        .i16(0) // parent id: none
        .string16("") // bone
        .f32(0.0)
        .f32(0.0)
        .f32(0.0) // position
        .f32(0.0)
        .f32(0.0)
        .f32(0.0) // rotation
        .u8(0); // force_visible
    w.into_inner()
}

/// AO_CMD_SET_PHYSICS_OVERRIDE with every override at its default, or
/// with speed, jump and gravity 0 (`frozen`: beds' lay_down).
pub fn physics_override_message(frozen: bool) -> Vec<u8> {
    let v = if frozen { 0.0 } else { 1.0 };
    let mut w = Writer::plain();
    w.u8(AO_CMD_SET_PHYSICS_OVERRIDE)
        .f32(v) // speed
        .f32(v) // jump
        .f32(v) // gravity
        .u8(0) // !sneak (sent inverted)
        .u8(1) // !sneak_glitch
        .u8(0); // !new_move
    for _ in 0..10 {
        // speed_climb, speed_crouch, liquid_fluidity, liquid_fluidity_smooth,
        // liquid_sink, acceleration_default, acceleration_air, speed_fast,
        // acceleration_fast, speed_walk
        w.f32(1.0);
    }
    w.into_inner()
}

/// AO_CMD_UPDATE_POSITION: position and velocity in BS units, yaw in degrees.
pub fn update_position_message(pos_bs: [f32; 3], speed_bs: [f32; 3], yaw: f32) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_UPDATE_POSITION);
    for v in pos_bs {
        w.f32(v);
    }
    for v in speed_bs {
        w.f32(v);
    }
    w.f32(0.0).f32(0.0).f32(0.0); // acceleration
    w.f32(0.0).f32(yaw).f32(0.0); // rotation
    w.u8(1) // interpolate
        .u8(0) // not the end of a movement
        .f32(0.1); // update interval
    w.into_inner()
}

/// GenericCAO initialisation data for a player.
/// What a client needs to show a player. Passed as one struct to keep the
/// argument count low (see server::Setup).
pub struct PlayerInit<'a> {
    pub name: &'a str,
    pub id: u16,
    pub pos_bs: [f32; 3],
    pub yaw: f32,
    pub anim: Animation,
    /// The skin texture.
    pub skin: &'a str,
    /// The zoom binoculars give (binoculars.rs).
    pub zoom_fov: f32,
    pub posture: Posture,
    /// What they are attached to (a boat), and where on it.
    pub attached: Option<(u16, [f32; 3])>,
}

pub fn player_init_data(p: &PlayerInit) -> Vec<u8> {
    let PlayerInit { name, id, pos_bs, yaw, anim, skin, zoom_fov, posture, attached } = *p;
    let mut w = Writer::plain();
    w.u8(1) // version
        .string16(name)
        .u8(1) // is_player
        .u16(id);
    for v in pos_bs {
        w.f32(v);
    }
    w.f32(0.0).f32(yaw).f32(0.0) // rotation
        .u16(PLAYER_MAX_HP);
    // The same messages, in the same order, as PlayerSAO's init data.
    let messages = [
        player_properties_message(skin, posture, zoom_fov),
        armor_groups_message(),
        set_animation_message(anim),
        attached.map_or_else(not_attached_message, |(parent, at)| attach_message(parent, at)),
        physics_override_message(false),
    ];
    w.u8(messages.len() as u8);
    for m in &messages {
        w.bytes32(m);
    }
    w.into_inner()
}

// ---------------------------------------------------------------------------
// Dropped items (builtin's "__builtin:item" entity)
// ---------------------------------------------------------------------------

const AO_CMD_SET_TEXTURE_MOD: u8 = 2;
/// Luanti's default movement_gravity, in nodes per second squared.
pub const GRAVITY: f32 = 9.81;

/// What a client needs to show a dropped item.
pub struct ItemInit<'a> {
    pub id: u16,
    /// "name count", as an ItemStack string.
    pub itemstring: &'a str,
    pub item_name: &'a str,
    pub description: &'a str,
    pub count: u16,
    pub light_source: u8,
    pub pos_bs: [f32; 3],
    pub vel_bs: [f32; 3],
}

/// The item entity's half size in nodes: builtin's set_item.
pub fn item_size(count: u16) -> f32 {
    0.2 + 0.1 * (count.min(99) as f32 / 99.0).cbrt()
}

/// builtin's set_item properties: a small spinning copy of the item.
pub fn item_properties_message(init: &ItemInit) -> Vec<u8> {
    let size = item_size(init.count);
    let b = [-size, -size, -size, size, size, size];
    properties_message(&Props {
        hp_max: 1,
        physical: true,
        collisionbox: b,
        selectionbox: b,
        visual: "wielditem",
        visual_size: [size; 3],
        textures: &[init.item_name],
        makes_footstep_sound: false,
        automatic_rotate: std::f32::consts::PI * 0.5 * 0.2 / size,
        mesh: "",
        collide_with_objects: false,
        stepheight: 0.0,
        infotext: init.description,
        wield_item: init.itemstring,
        glow: ((init.light_source as f32 / 2.0 + 0.5).floor()) as i8,
        eye_height: 1.625,
        damage_texture_modifier: "^[brighten",
        zoom_fov: 0.0,
        spritediv: [1, 1],
        face_movement: None,
        use_texture_alpha: false,
    })
}

/// AO_CMD_UPDATE_POSITION for a falling (or resting) item: the client
/// moves it itself, with gravity, until the next update.
pub fn item_position_message(pos_bs: [f32; 3], vel_bs: [f32; 3]) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_UPDATE_POSITION);
    for v in pos_bs.into_iter().chain(vel_bs) {
        w.f32(v);
    }
    w.f32(0.0).f32(-GRAVITY * 10.0).f32(0.0); // acceleration
    w.f32(0.0).f32(0.0).f32(0.0); // rotation
    w.u8(1) // interpolate
        .u8(0) // not the end of a movement
        .f32(0.09); // update interval
    w.into_inner()
}

/// GenericCAO initialisation data for a dropped item, with the messages
/// LuaEntitySAO sends: properties, armour groups, attachment, texture mod.
pub fn item_init_data(init: &ItemInit) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(1) // version
        .string16("__builtin:item")
        .u8(0) // not a player
        .u16(init.id);
    for v in init.pos_bs {
        w.f32(v);
    }
    w.f32(0.0).f32(0.0).f32(0.0) // rotation
        .u16(1); // hp
    let mut immortal = Writer::plain();
    immortal.u8(AO_CMD_UPDATE_ARMOR_GROUPS).u16(1).string16("immortal").i16(1);
    let mut texture_mod = Writer::plain();
    texture_mod.u8(AO_CMD_SET_TEXTURE_MOD).string16("");
    let messages = [
        item_properties_message(init),
        immortal.into_inner(),
        not_attached_message(),
        texture_mod.into_inner(),
        // Not in LuaEntitySAO's data, but it soon sends the same: gravity,
        // and the velocity of an item still in the air.
        item_position_message(init.pos_bs, init.vel_bs),
    ];
    w.u8(messages.len() as u8);
    for m in &messages {
        w.bytes32(m);
    }
    w.into_inner()
}

// ---------------------------------------------------------------------------
// Mobs (mobs.rs)
// ---------------------------------------------------------------------------

const AO_CMD_SET_SPRITE: u8 = 3;
const AO_CMD_PUNCHED: u8 = 4;

/// AO_CMD_UPDATE_POSITION with an acceleration: the client moves the object
/// on itself (colliding, if physical) until the next update. BS units, yaw
/// in degrees.
pub fn object_position_message(pos_bs: [f32; 3], vel_bs: [f32; 3], acc_bs: [f32; 3], yaw: f32) -> Vec<u8> {
    moved_message(pos_bs, vel_bs, acc_bs, yaw, Some(0.25))
}

/// AO_CMD_UPDATE_POSITION, saying how the client gets there: gliding over
/// `glide` seconds (an entity's move_to, Luanti sending its step time), or
/// at once (None: set_pos).
pub fn moved_message(pos_bs: [f32; 3], vel_bs: [f32; 3], acc_bs: [f32; 3], yaw: f32, glide: Option<f32>) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_UPDATE_POSITION);
    for v in pos_bs.into_iter().chain(vel_bs).chain(acc_bs) {
        w.f32(v);
    }
    w.f32(0.0).f32(yaw).f32(0.0); // rotation
    w.u8(glide.is_some() as u8) // interpolate
        .u8(glide.is_none() as u8) // the end of a movement (set_pos)
        .f32(glide.unwrap_or(0.0)); // update interval
    w.into_inner()
}

/// AO_CMD_PUNCHED: the object's HP after a hit. Clients flash its
/// damage_texture_modifier, or at 0 make a puff of smoke.
pub fn punched_message(hp: u16) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_PUNCHED).u16(hp);
    w.into_inner()
}

/// AO_CMD_SET_SPRITE: show `frames` frames of a sprite from `start` (a
/// column and row of its spritediv), each for `frame_length` seconds, over
/// and over (the frames go down the texture).
pub fn set_sprite_message(start: [i16; 2], frames: u16, frame_length: f32) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_SET_SPRITE).i16(start[0]).i16(start[1]).u16(frames).f32(frame_length).u8(0);
    w.into_inner()
}

/// AO_CMD_SET_BONE_POSITION in the >= 5.9 form: turn a bone of the model
/// by `rotation` (degrees) beyond what its animation does, getting there
/// over `over` seconds; position and scale left to the animation.
pub fn bone_rotation_message(bone: &str, rotation: [f32; 3], over: f32) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_SET_BONE_POSITION).string16(bone);
    for v in [0.0, 0.0, 0.0].into_iter().chain(rotation).chain([1.0, 1.0, 1.0]) {
        w.f32(v);
    }
    w.f32(0.0) // position: at once
        .f32(over) // rotation
        .f32(0.0) // scale
        .u8(0); // all relative to the animation
    w.into_inner()
}

/// AO_CMD_SET_TEXTURE_MOD: a modifier added to the object's textures.
pub fn texture_mod_message(modifier: &str) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(AO_CMD_SET_TEXTURE_MOD).string16(modifier);
    w.into_inner()
}

/// GenericCAO initialisation data for a Lua-entity-like object (not a
/// player), followed by `messages` (properties first).
pub fn entity_init_data(name: &str, id: u16, pos_bs: [f32; 3], yaw: f32, hp: u16, messages: &[Vec<u8>]) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(1) // version
        .string16(name)
        .u8(0) // not a player
        .u16(id);
    for v in pos_bs {
        w.f32(v);
    }
    w.f32(0.0).f32(yaw).f32(0.0) // rotation
        .u16(hp);
    w.u8(messages.len() as u8);
    for m in messages {
        w.bytes32(m);
    }
    w.into_inner()
}

/// TOCLIENT_ACTIVE_OBJECT_REMOVE_ADD
pub fn build_remove_add(removed: &[u16], added: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_ACTIVE_OBJECT_REMOVE_ADD);
    w.u16(removed.len() as u16);
    for id in removed {
        w.u16(*id);
    }
    w.u16(added.len() as u16);
    for (id, init) in added {
        w.u16(*id).u8(ACTIVEOBJECT_TYPE_GENERIC).bytes32(init);
    }
    w.into_inner()
}

/// TOCLIENT_ACTIVE_OBJECT_MESSAGES: (object id, message) pairs.
pub fn build_object_messages(messages: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_ACTIVE_OBJECT_MESSAGES);
    for (id, msg) in messages {
        w.u16(*id).bytes16(msg);
    }
    w.into_inner()
}

/// The animation player_api would play for these controls.
pub fn animation_for(pp: &PlayerPos) -> Animation {
    let walking = pp.keys & 0b1111 != 0 || pp.movement_speed > 0.01;
    let digging = pp.keys & (1 << 7 | 1 << 8) != 0;
    let sneaking = pp.keys & (1 << 6) != 0;
    let fps = if sneaking { ANIM_FPS / 2.0 } else { ANIM_FPS };
    let range = match (walking, digging) {
        (true, true) => ANIM_WALK_MINE,
        (true, false) => ANIM_WALK,
        (false, true) => ANIM_MINE,
        (false, false) => ANIM_STAND,
    };
    Animation::new(range, fps)
}

// ---------------------------------------------------------------------------
// SRP login (existing accounts)
// ---------------------------------------------------------------------------

/// TOSERVER_SRP_BYTES_A: the client's A, and whether it is based on an SRP
/// verifier (1) or a legacy password hash (0).
pub fn parse_srp_bytes_a(payload: &[u8]) -> Option<(Vec<u8>, u8)> {
    let mut r = Reader::new(payload);
    r.u16()?;
    Some((r.bytes16()?, r.u8()?))
}

/// TOSERVER_SRP_BYTES_M: the client's proof.
pub fn parse_srp_bytes_m(payload: &[u8]) -> Option<Vec<u8>> {
    let mut r = Reader::new(payload);
    r.u16()?;
    r.bytes16()
}

/// TOCLIENT_SRP_BYTES_S_B: the account's salt and our B.
pub fn build_srp_bytes_s_b(salt: &[u8], b: &[u8]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_SRP_BYTES_S_B);
    w.bytes16(salt).bytes16(b);
    w.into_inner()
}

// ---------------------------------------------------------------------------
// Interaction and node changes
// ---------------------------------------------------------------------------

/// Punching an object (picking up items) or starting to dig a node.
/// The sneak key's bit in PlayerPos::keys.
pub const KEY_SNEAK: u32 = 1 << 6;

pub const INTERACT_START_DIGGING: u8 = 0;
// Also 1 (stop digging), which needs no handling.
pub const INTERACT_DIGGING_COMPLETED: u8 = 2;
pub const INTERACT_PLACE: u8 = 3;
/// Left-click with a usable item (an apple): use it instead of digging.
pub const INTERACT_USE: u8 = 4;

#[derive(Debug)]
pub enum Pointed {
    Nothing,
    Node { under: [i16; 3], above: [i16; 3] },
    Object(u16),
}

#[derive(Debug)]
pub struct Interact {
    pub action: u8,
    /// Hotbar slot of the wielded item.
    pub item: u16,
    pub pointed: Pointed,
    /// The player's position, sent along with every interaction.
    pub pos: Option<PlayerPos>,
}

/// TOSERVER_INTERACT: action, item index, PointedThing (u32-prefixed), then
/// the same fields as TOSERVER_PLAYERPOS.
pub fn parse_interact(payload: &[u8]) -> Option<Interact> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let action = r.u8()?;
    let item = r.u16()?;
    let mut p = Reader::new(r.bytes32()?);
    if p.u8()? != 0 {
        return None; // PointedThing version
    }
    let v3 = |p: &mut Reader| -> Option<[i16; 3]> { Some([p.i16()?, p.i16()?, p.i16()?]) };
    let pointed = match p.u8()? {
        0 => Pointed::Nothing,
        1 => Pointed::Node { under: v3(&mut p)?, above: v3(&mut p)? },
        2 => Pointed::Object(p.u16()?),
        _ => return None,
    };
    let mut pp = TOSERVER_PLAYERPOS.to_be_bytes().to_vec();
    pp.extend_from_slice(r.rest());
    Some(Interact { action, item, pointed, pos: parse_playerpos(&pp) })
}

/// TOCLIENT_ADDNODE: position, MapNode (param0 u16, param1, param2), then
/// whether to keep the node's metadata.
pub fn build_addnode(pos: [i16; 3], id: u16, param2: u8) -> Vec<u8> {
    addnode(pos, id, param2, false)
}

/// TOCLIENT_ADDNODE that leaves the client's copy of the node's metadata
/// alone: for putting back what the client predicted wrongly, where the
/// node itself is unchanged.
pub fn build_addnode_keeping_meta(pos: [i16; 3], id: u16, param2: u8) -> Vec<u8> {
    addnode(pos, id, param2, true)
}

fn addnode(pos: [i16; 3], id: u16, param2: u8, keep_metadata: bool) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_ADDNODE);
    w.i16(pos[0]).i16(pos[1]).i16(pos[2]);
    w.u16(id).u8(0).u8(param2); // param1 0: the client recalculates light itself
    w.u8(keep_metadata as u8);
    w.into_inner()
}

/// TOCLIENT_REMOVENODE: the node becomes air.
pub fn build_removenode(pos: [i16; 3]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_REMOVENODE);
    w.i16(pos[0]).i16(pos[1]).i16(pos[2]);
    w.into_inner()
}

/// TOCLIENT_PLAY_SOUND: a sound at a position in BS units, as
/// Server::playSound sends it. `id` -1 makes it a one-off ("ephemeral")
/// sound; a positive id can be faded out later.
pub fn build_play_sound(id: i32, name: &str, gain: f32, pos_bs: [f32; 3]) -> Vec<u8> {
    play_sound(id, name, gain, pos_bs, false)
}

/// TOCLIENT_PLAY_SOUND for a sound that plays over and over until
/// stopped (the fire's crackle).
pub fn build_play_sound_looped(id: i32, name: &str, gain: f32, pos_bs: [f32; 3]) -> Vec<u8> {
    play_sound(id, name, gain, pos_bs, true)
}

fn play_sound(id: i32, name: &str, gain: f32, pos_bs: [f32; 3], looped: bool) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_PLAY_SOUND);
    w.i32(id).string16(name).f32(gain).u8(1); // positional
    w.f32(pos_bs[0]).f32(pos_bs[1]).f32(pos_bs[2]);
    w.u16(0) // object
        .u8(looped as u8) // loop
        .f32(0.0) // fade
        .f32(1.0) // pitch
        .u8((id < 0) as u8) // ephemeral
        .f32(0.0); // start time
    w.into_inner()
}

/// TOCLIENT_PLAY_SOUND for a sound that follows an object (a mob): the
/// client places it where the object is and moves it with it.
pub fn build_play_sound_object(id: i32, name: &str, gain: f32, pos_bs: [f32; 3], object: u16, looped: bool) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_PLAY_SOUND);
    w.i32(id).string16(name).f32(gain).u8(2); // SoundLocation::Object
    w.f32(pos_bs[0]).f32(pos_bs[1]).f32(pos_bs[2]);
    w.u16(object)
        .u8(looped as u8)
        .f32(0.0) // fade
        .f32(1.0) // pitch
        .u8((id < 0) as u8) // ephemeral
        .f32(0.0); // start time
    w.into_inner()
}

/// A particle spawner (Lua's add_particlespawner): `amount` particles over
/// `time` seconds (0: for ever, `amount` a second), each from a random
/// point and with a random velocity, acceleration, life and size within
/// these ranges (positions in nodes, relative to the object it is attached
/// to, if any). A texture, or a node (id and param2) whose own texture the
/// client breaks into bits.
#[derive(Clone, Debug)]
pub struct Particles<'a> {
    pub amount: u16,
    pub time: f32,
    pub pos: ([f32; 3], [f32; 3]),
    pub vel: ([f32; 3], [f32; 3]),
    pub acc: ([f32; 3], [f32; 3]),
    pub life: (f32, f32),
    pub size: (f32, f32),
    pub collide: bool,
    pub texture: &'a str,
    pub node: Option<(u16, u8)>,
    pub glow: u8,
    /// The object they come from (0: none).
    pub attached: u16,
}

impl Default for Particles<'_> {
    fn default() -> Self {
        Self {
            amount: 1,
            time: 0.001,
            pos: ([0.0; 3], [0.0; 3]),
            vel: ([0.0; 3], [0.0; 3]),
            acc: ([0.0; 3], [0.0; 3]),
            life: (1.0, 1.0),
            size: (1.0, 1.0),
            collide: false,
            texture: "",
            node: None,
            glow: 0,
            attached: 0,
        }
    }
}

/// TOCLIENT_ADD_PARTICLESPAWNER (Server::SendAddParticleSpawner, protocol
/// >= 42): every value a "tween" that stays the same throughout, and a
/// range that is evenly picked from (bias 0). Textures are drawn as
/// minetest_game asks, with blend "clip".
pub fn build_particle_spawner(id: u32, p: &Particles) -> Vec<u8> {
    fn v3_range(w: &mut Writer, (min, max): ([f32; 3], [f32; 3])) {
        w.u8(0).u16(1).f32(0.0); // tween: forwards, once, from the start
        for _ in 0..2 {
            for v in min.into_iter().chain(max) {
                w.f32(v);
            }
            w.f32(0.0); // bias
        }
    }
    fn f32_range(w: &mut Writer, (min, max): (f32, f32)) {
        w.u8(0).u16(1).f32(0.0);
        for _ in 0..2 {
            w.f32(min).f32(max).f32(0.0);
        }
    }
    let mut w = Writer::new(TOCLIENT_ADD_PARTICLESPAWNER);
    w.u16(p.amount).f32(p.time);
    v3_range(&mut w, p.pos);
    v3_range(&mut w, p.vel);
    v3_range(&mut w, p.acc);
    f32_range(&mut w, p.life);
    f32_range(&mut w, p.size);
    w.u8(p.collide as u8).bytes32(p.texture.as_bytes());
    w.u32(id)
        .u8(0) // vertical
        .u8(0) // collision_removal
        .u16(p.attached)
        .u8(0) // animation: none
        .u8(p.glow)
        .u8(0); // object_collision
    let (node, param2) = p.node.unwrap_or((127, 0)); // CONTENT_IGNORE: no node
    w.u16(node).u8(param2).u8(0); // node, param2, tile (0: any)
    // The texture's newer properties: flags (blend mode clip, 4, shifted
    // one up), alpha 1 throughout, scale 1 throughout.
    w.u8(4 << 1);
    w.u8(0).u16(1).f32(0.0).f32(1.0).f32(1.0);
    w.u8(0).u16(1).f32(0.0).f32(1.0).f32(1.0).f32(1.0).f32(1.0);
    v3_range(&mut w, ([0.0; 3], [0.0; 3])); // drag
    v3_range(&mut w, ([0.0; 3], [0.0; 3])); // jitter
    f32_range(&mut w, (0.0, 0.0)); // bounce
    w.u8(0); // attractor: none
    v3_range(&mut w, ([0.0; 3], [0.0; 3])); // radius
    w.u16(0); // texture pool: none
    w.into_inner()
}

/// TOCLIENT_DELETE_PARTICLESPAWNER: stop a spawner early.
pub fn build_delete_particle_spawner(id: u32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_DELETE_PARTICLESPAWNER);
    w.u32(id);
    w.into_inner()
}

/// TOCLIENT_PLAYER_SPEED: add to the player's velocity (BS units per
/// second), as player:add_velocity (a blast throwing them back).
pub fn build_player_speed(added_bs: [f32; 3]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_PLAYER_SPEED);
    w.f32(added_bs[0]).f32(added_bs[1]).f32(added_bs[2]);
    w.into_inner()
}

/// TOCLIENT_STOP_SOUND: stop a sound at once (minetest.sound_stop).
pub fn build_stop_sound(id: i32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_STOP_SOUND);
    w.i32(id);
    w.into_inner()
}

/// TOCLIENT_FADE_SOUND: fade a sound by `step` gain per second down to
/// `gain` (minetest_game fades the furnace fire with -1, 0).
pub fn build_fade_sound(id: i32, step: f32, gain: f32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_FADE_SOUND);
    w.i32(id).f32(step).f32(gain);
    w.into_inner()
}

/// TOCLIENT_INVENTORY: the player's inventory in Luanti's text format, as
/// a u32-prefixed string, then "skip next wield animation" (protocol > 51).
/// Each list is (name, width, slots as item strings, None for empty).
pub fn build_inventory(lists: &[(&str, u32, Vec<Option<String>>)]) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_INVENTORY);
    w.bytes32(inventory_text(lists).as_bytes()).u8(0);
    w.into_inner()
}

/// TOCLIENT_DETACHED_INVENTORY: an inventory that belongs to no player
/// (a furnace's here): name, "update" (not remove), a u16 the client
/// ignores, then the inventory text (Server::sendDetachedInventory).
pub fn build_detached_inventory(name: &str, lists: &[(&str, u32, Vec<Option<String>>)]) -> Vec<u8> {
    let text = inventory_text(lists);
    let mut w = Writer::new(TOCLIENT_DETACHED_INVENTORY);
    w.string16(name).u8(1).u16(text.len().min(0xFFFF) as u16).raw(text.as_bytes());
    w.into_inner()
}

/// TOCLIENT_SHOW_FORMSPEC: open (or, if one with that name is open,
/// update) a window; an empty formspec closes it.
pub fn build_show_formspec(formspec: &str, formname: &str) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_SHOW_FORMSPEC);
    w.bytes32(formspec.as_bytes()).string16(formname);
    w.into_inner()
}

/// The furnace window's form name, which the client sends back with its
/// fields (TOSERVER_INVENTORY_FIELDS) when it is closed.
pub const FURNACE_FORM: &str = "esp:furnace";

/// minetest_game's furnace window: the item to smelt above the flame and
/// the fuel below it, an arrow to the 2x2 output, and the player's
/// inventory underneath. The listrings make shift-click move output and
/// furnace contents to the inventory, and from the inventory into "src".
/// While it burns, `display` is the fuel left and the cooking progress in
/// percent: the flame and the arrow fill up from below.
pub fn furnace_formspec(inv: &str, display: Option<(u8, u8)>) -> String {
    let at = format!("detached:{inv}");
    let (fire, arrow) = match display {
        Some((fuel, item)) => (
            format!("default_furnace_fire_bg.png^[lowpart:{fuel}:default_furnace_fire_fg.png"),
            format!("gui_furnace_arrow_bg.png^[lowpart:{item}:gui_furnace_arrow_fg.png^[transformR270"),
        ),
        None => ("default_furnace_fire_bg.png".to_string(), "gui_furnace_arrow_bg.png^[transformR270".to_string()),
    };
    format!(
        "formspec_version[6]size[10.75,10]\
         list[{at};src;3,0.5;1,1;]\
         image[3,1.75;1,1;{fire}]\
         list[{at};fuel;3,3;1,1;]\
         image[4.25,1.75;1,1;{arrow}]\
         list[{at};dst;5.5,1.125;2,2;]\
         list[current_player;main;0.5,4.75;8,4;]\
         listring[{at};dst]listring[current_player;main]\
         listring[{at};src]listring[current_player;main]\
         listring[{at};fuel]listring[current_player;main]"
    )
}

/// The chest window's form name, sent back when it is closed.
pub const CHEST_FORM: &str = "esp:chest";

/// minetest_game's chest window: the chest's 32 slots above the player's
/// inventory, with a listring so shift-click moves items between them.
pub fn chest_formspec(inv: &str) -> String {
    let at = format!("detached:{inv}");
    format!(
        "formspec_version[6]size[10.75,11]\
         list[{at};main;0.5,0.5;8,4;]\
         list[current_player;main;0.5,5.75;8,4;]\
         listring[{at};main]listring[current_player;main]"
    )
}

/// minetest_game's bookshelf window (or vessels' shelf's): its 16 slots,
/// with a book (or bottle) outline in the empty ones, over the player's
/// inventory.
pub fn shelf_formspec(inv: &str, empty: &[bool], kind: crate::chest::Shelf) -> String {
    let slot = match kind {
        crate::chest::Shelf::Books => "default_bookshelf_slot.png",
        crate::chest::Shelf::Vessels => "vessels_shelf_slot.png",
    };
    let at = format!("detached:{inv}");
    let mut f = format!(
        "size[8,7;]list[{at};main;0,0.3;8,2;]\
         list[current_player;main;0,2.85;8,1;]\
         list[current_player;main;0,4.08;8,3;8]\
         listring[{at};main]listring[current_player;main]"
    );
    for i in 0..8 {
        f.push_str(&format!("image[{i},2.85;1,1;gui_hb_bg.png]"));
    }
    for (i, _) in empty.iter().enumerate().filter(|(_, e)| **e) {
        f.push_str(&format!("image[{},{};1,1;{slot}]", i % 8, if i < 8 { "0.3" } else { "1.3" }));
    }
    f
}

/// Luanti's inventory text: each list as (name, width, item strings).
fn inventory_text(lists: &[(&str, u32, Vec<Option<String>>)]) -> String {
    let mut text = String::new();
    for (name, width, slots) in lists {
        text.push_str(&format!("List {name} {}\nWidth {width}\n", slots.len()));
        for slot in slots {
            match slot {
                Some(item) => text.push_str(&format!("Item {item}\n")),
                None => text.push_str("Empty\n"),
            }
        }
        text.push_str("EndInventoryList\n");
    }
    text.push_str("EndInventory\n");
    text
}

/// The skin chooser beside the inventory: what the player wears now.
pub struct SkinChoice<'a> {
    pub label: &'a str,
    pub texture: &'a str,
}

pub const FIELD_SKIN_PREVIOUS: &str = "skin_prev";
pub const FIELD_SKIN_NEXT: &str = "skin_next";

/// Formspec text needs these escaped (and a texture's commas, inside a
/// comma-separated list of textures).
pub fn formspec_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(']', "\\]")
        .replace('[', "\\[")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('$', "\\$")
}

/// TOCLIENT_INVENTORY_FORMSPEC: the window the inventory key opens, laid
/// out like minetest_game's: the 3x3 craft grid with an arrow to what it
/// makes at the top, the 32 main slots below (the hotbar row first), and
/// on the right a turnable preview of the player's character with buttons
/// to change skin. The listrings make shift-click move items between the
/// main list and the craft grid.
pub fn build_inventory_formspec(skin: &SkinChoice) -> Vec<u8> {
    let fs = format!(
        "formspec_version[6]size[14.5,10.25]{tabs}\
         list[current_player;craft;2.375,0.5;3,3;]\
         image[6.375,1.75;1,1;gui_furnace_arrow_bg.png^[transformR270]\
         list[current_player;craftpreview;7.625,1.75;1,1;]\
         list[current_player;main;0.5,5;8,4;]\
         listring[current_player;main]\
         listring[current_player;craft]\
         model[10.85,0.3;3.2,3.7;skin_preview;character.b3d;{texture};0,180;false;true;0,79;30]\
         label[10.85,4.25;{label}]\
         button[10.85,4.6;1.4,0.75;{FIELD_SKIN_PREVIOUS};<]\
         button[12.65,4.6;1.4,0.75;{FIELD_SKIN_NEXT};>]",
        tabs = inventory_tabs(1),
        texture = formspec_escape(skin.texture),
        label = formspec_escape(skin.label),
    );
    let mut w = Writer::new(TOCLIENT_INVENTORY_FORMSPEC);
    w.bytes32(fs.as_bytes());
    w.into_inner()
}

/// sfinv's tabs along the top of the inventory window: Crafting (1) and
/// the crafting guide's Recipes (2).
fn inventory_tabs(current: u8) -> String {
    format!("tabheader[0,0;{};Crafting,Recipes;{current};true;false]", crate::craftguide::TABS)
}

/// The inventory window on its Recipes tab (the crafting guide).
pub fn build_recipes_formspec(guide: &crate::craftguide::Guide) -> Vec<u8> {
    let fs = format!("formspec_version[6]size[14.5,10.25]{}{}", inventory_tabs(2), crate::craftguide::formspec(guide));
    let mut w = Writer::new(TOCLIENT_INVENTORY_FORMSPEC);
    w.bytes32(fs.as_bytes());
    w.into_inner()
}

/// TOCLIENT_CLOUD_PARAMS: the weather's clouds (player:set_clouds with
/// density, thickness and speed; the rest as Luanti's defaults: colours,
/// and 120 nodes up).
pub fn build_cloud_params(w: &crate::weather::Weather) -> Vec<u8> {
    let mut wr = Writer::new(TOCLIENT_CLOUD_PARAMS);
    wr.f32(w.density)
        .u32(0xE5F0_F0FF) // bright: ARGB (229, 240, 240, 255)
        .u32(0xFF00_0000) // ambient
        .f32(120.0) // height
        .f32(w.thickness)
        .f32(w.speed.0)
        .f32(w.speed.1)
        .u32(0xFFCC_CCCC); // shadow
    wr.into_inner()
}

/// TOCLIENT_SET_LIGHTING: the weather's shadows (player:set_lighting with
/// shadow intensity, a little bloom and volumetric light; the rest as
/// Luanti's Lighting defaults).
pub fn build_set_lighting(shadow_intensity: f32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_SET_LIGHTING);
    w.f32(shadow_intensity)
        .f32(1.0) // saturation
        .f32(-3.0) // exposure: luminance_min
        .f32(-3.0) // luminance_max
        .f32(0.0) // exposure_correction
        .f32(1000.0) // speed_dark_bright
        .f32(1000.0) // speed_bright_dark
        .f32(1.0) // center_weight_power
        .f32(0.2) // volumetric_light_strength
        .u32(0xFF00_0000) // shadow_tint
        .f32(0.05) // bloom_intensity
        .f32(1.0) // bloom_strength_factor
        .f32(1.0) // bloom_radius
        .f32(0.0)
        .f32(0.0)
        .f32(0.0); // shadow_direction: unset
    w.into_inner()
}

/// TOSERVER_NODEMETA_FIELDS: the node, the form's name (unused), then
/// (field name, value) pairs from a node's own formspec (a sign's text
/// box).
pub fn parse_nodemeta_fields(payload: &[u8]) -> Option<([i16; 3], Vec<(String, String)>)> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let pos = [r.i16()?, r.i16()?, r.i16()?];
    r.string16()?;
    let count = r.u16()?;
    let mut fields = Vec::new();
    for _ in 0..count {
        let name = r.string16()?;
        let value = String::from_utf8_lossy(r.bytes32()?).into_owned();
        fields.push((name, value));
    }
    Some((pos, fields))
}

/// TOSERVER_INVENTORY_FIELDS: the formspec's name ("" for the inventory
/// window), then (field name, value) pairs; a pressed button is a field.
pub fn parse_inventory_fields(payload: &[u8]) -> Option<(String, Vec<(String, String)>)> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let form = r.string16()?;
    let count = r.u16()?;
    let mut fields = Vec::new();
    for _ in 0..count {
        let name = r.string16()?;
        let value = String::from_utf8_lossy(r.bytes32()?).into_owned();
        fields.push((name, value));
    }
    Some((form, fields))
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// TOSERVER_DAMAGE: fall damage the client worked out itself.
pub fn parse_damage(payload: &[u8]) -> Option<u16> {
    let mut r = Reader::new(payload);
    r.u16()?;
    r.u16()
}

/// TOCLIENT_HP: the player's health, and whether to show the red flash.
pub fn build_hp(hp: u16, effect: bool) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HP);
    w.u16(hp).u8(effect as u8);
    w.into_inner()
}

/// TOCLIENT_DEATHSCREEN_LEGACY: the client's own "You died" screen, whose
/// Respawn button sends TOSERVER_RESPAWN_LEGACY. (A Lua server shows a
/// formspec instead.) The fields are unused by current clients.
pub fn build_deathscreen() -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_DEATHSCREEN_LEGACY);
    w.u8(0).i32(0).i32(0).i32(0);
    w.into_inner()
}

/// Our HUD elements' ids: the health and breath bars, and the minimap.
pub const HUD_HEALTH: u32 = 0;
pub const HUD_BREATH: u32 = 1;
pub const HUD_MINIMAP: u32 = 2;
const HUD_ELEM_MINIMAP: u8 = 7;
/// A player's breath when full (Luanti's PLAYER_MAX_BREATH_DEFAULT).
pub const PLAYER_MAX_BREATH: u16 = 10;
const HUD_ELEM_STATBAR: u8 = 2;
const HUD_STAT_NUMBER: u8 = 4;

/// TOCLIENT_HUDADD for the health bar, laid out as builtin/game/hud.lua
/// does it: hearts above the hotbar, half a heart per HP.
pub fn build_health_bar(hp: u16) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HUDADD);
    w.u32(HUD_HEALTH)
        .u8(HUD_ELEM_STATBAR)
        .f32(0.5)
        .f32(1.0) // position: bottom centre of the screen
        .string16("health")
        .f32(0.0)
        .f32(0.0) // scale
        .string16("heart.png") // built into the client
        .u32(hp as u32) // number: half-hearts to show
        .u32(PLAYER_MAX_HP as u32) // item: half-hearts when full
        .u32(0) // direction: left to right
        .f32(0.0)
        .f32(0.0) // align
        .f32(-10.0 * 24.0 - 25.0)
        .f32(-(48.0 + 24.0 + 16.0)) // offset
        .f32(0.0)
        .f32(0.0)
        .f32(0.0) // world position
        .f32(24.0)
        .f32(24.0) // size (v2f for protocol >= 52)
        .i16(0) // z index
        .string16("heart_gone.png") // text2: empty hearts behind
        .u32(0) // style
        .u8(1); // flags: hideable
    w.into_inner()
}

/// TOCLIENT_HUDCHANGE: a new HP figure for the health bar.
/// builtin's breath bar (hud.lua): bubbles to the right of the hearts,
/// two units a bubble, shown only while breath isn't full.
pub fn build_breath_bar(breath: u16) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HUDADD);
    w.u32(HUD_BREATH)
        .u8(HUD_ELEM_STATBAR)
        .f32(0.5)
        .f32(1.0) // position: bottom centre of the screen
        .string16("breath")
        .f32(0.0)
        .f32(0.0) // scale
        .string16("bubble.png") // built into the client
        .u32(breath_units(breath)) // number
        .u32(breath_units(PLAYER_MAX_BREATH)) // item: when full
        .u32(0) // direction: left to right
        .f32(0.0)
        .f32(0.0) // align
        .f32(25.0)
        .f32(-(48.0 + 24.0 + 16.0)) // offset
        .f32(0.0)
        .f32(0.0)
        .f32(0.0) // world position
        .f32(24.0)
        .f32(24.0) // size
        .i16(0) // z index
        .string16("bubble_gone.png") // text2
        .u32(0) // style
        .u8(1); // flags: hideable
    w.into_inner()
}

/// Half-bubbles: breath scaled to the bar's 20 (builtin's scale_to_hud_max).
fn breath_units(breath: u16) -> u32 {
    breath as u32 * 2
}

pub fn build_breath_bar_change(breath: u16) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HUDCHANGE);
    w.u32(HUD_BREATH).u8(HUD_STAT_NUMBER).u32(breath_units(breath));
    w.into_inner()
}

/// TOCLIENT_HUDADD for builtin's minimap element (hud.lua): since
/// protocol 44 the client draws the minimap only as a HUD element, which
/// builtin adds while the "minimap" HUD flag is set; in the top right
/// corner, a quarter of the screen's height.
pub fn build_minimap_hud() -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HUDADD);
    w.u32(HUD_MINIMAP)
        .u8(HUD_ELEM_MINIMAP)
        .f32(1.0)
        .f32(0.0) // position: top right
        .string16("")
        .f32(0.0)
        .f32(0.0) // scale
        .string16("")
        .u32(0) // number
        .u32(0) // item
        .u32(0) // direction
        .f32(-1.0)
        .f32(1.0) // align: left of and below the position
        .f32(-10.0)
        .f32(10.0) // offset
        .f32(0.0)
        .f32(0.0)
        .f32(0.0) // world position
        .f32(0.0)
        .f32(-25.0) // size: 25% of the screen's height
        .i16(0) // z index
        .string16("") // text2
        .u32(0) // style
        .u8(1); // flags: hideable
    w.into_inner()
}

pub fn build_hud_remove(id: u32) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HUDRM);
    w.u32(id);
    w.into_inner()
}

/// TOCLIENT_BREATH: the player's breath, for the client's own use.
pub fn build_breath(breath: u16) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_BREATH);
    w.u16(breath);
    w.into_inner()
}

pub fn build_health_bar_change(hp: u16) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_HUDCHANGE);
    w.u32(HUD_HEALTH).u8(HUD_STAT_NUMBER).u32(hp as u32);
    w.into_inner()
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

/// Chat message types (ChatMessageType in Luanti's chatmessage.h). Players'
/// lines and command replies go out as "raw", as the Luanti server sends
/// them; the welcome is a system message.
pub const CHAT_RAW: u8 = 0;
pub const CHAT_SYSTEM: u8 = 3;

/// TOSERVER_CHAT_MESSAGE: what the player typed, as a UTF-16 string
/// (u16 length in code units).
pub fn parse_chat_message(payload: &[u8]) -> Option<String> {
    let mut r = Reader::new(payload);
    r.u16()?;
    let n = r.u16()? as usize;
    let units: Option<Vec<u16>> = (0..n).map(|_| r.u16()).collect();
    Some(char::decode_utf16(units?).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect())
}

/// TOCLIENT_CHAT_MESSAGE: version 1, type, sender and text as UTF-16
/// strings (u16 length in code units), then a Unix timestamp.
pub fn build_chat_message(kind: u8, sender: &str, text: &str) -> Vec<u8> {
    let mut w = Writer::new(TOCLIENT_CHAT_MESSAGE);
    w.u8(1).u8(kind);
    for s in [sender, text] {
        let units: Vec<u16> = s.encode_utf16().take(u16::MAX as usize).collect();
        w.u16(units.len() as u16);
        for u in units {
            w.u16(u);
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    w.u64(now);
    w.into_inner()
}

#[cfg(test)]
mod player_tests {
    use super::*;

    #[test]
    fn players_are_solid_except_in_bed() {
        // AO_CMD_SET_PROPERTIES, version, hp_max (u16), then physical.
        assert_eq!(player_properties_message("character.png", Posture::Stand, 0.0)[4], 1);
        assert_eq!(player_properties_message("character.png", Posture::Lie, 0.0)[4], 0);
        assert_eq!(player_properties_message("character.png", Posture::Sit, 0.0)[4], 0);
    }
}
