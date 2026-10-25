//! The server loop: peers, the reliability layer, and command dispatch.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use log::{debug, info, warn};

use crate::boats;
use crate::auth::{self, Account, AuthStore, NameError};
use crate::content::{self, Media};
use crate::messages::{self, *};
use crate::protocol::*;
use crate::inventory::{self, Action, Inventory, InventoryStore, List, Stack};
use crate::items::{Change, Items};
use crate::player::{PlayerData, PlayerStore};
use crate::skins::{self, Skins};
use crate::srp;
use crate::craftguide;
use crate::weather;
use crate::env_sounds;
use crate::fire;
use crate::settings;
use crate::tnt;
use crate::insects;
use crate::mobs::{self, Mobs};
use crate::beds;
use crate::binoculars;
use crate::keys;
use crate::screwdriver;
use crate::bones;
use crate::doors::{self, Door};
use crate::falling::{self, Falling};
use crate::farming;
use crate::fences;
use crate::panes;
use crate::plants;
use crate::privs::{self, Privs};
use crate::stairs;
use crate::status::{self, SharedStatus};
use crate::chest::{self, Chests};
use crate::books;
use crate::dungeons;
use crate::meta;
use crate::clock::{self, Clock};
use crate::colours;
use crate::furnace::{self, FurnaceList, Furnaces, Lists, Place};
use crate::nature::{self, Job, Rng, Timers};
use crate::world::{self, BlockPos, NodePos, World};

/// Chat commands (commands.rs).
#[path = "commands.rs"]
mod commands;

/// The resend timeout before any round trip has been measured (Luanti's
/// starting resend_timeout).
const RESEND_INTERVAL: Duration = Duration::from_millis(500);
const PEER_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// minetest_game's furnaces run a one-second node timer.
const FURNACE_STEP_INTERVAL: Duration = Duration::from_secs(1);
/// Leaf decay and sapling timers handled per tick, at most (each may look
/// at a few hundred nodes).
const TIMERS_PER_TICK: usize = 8;
/// How far chest sounds carry, in nodes (minetest_game's max_hear_distance).
const CHEST_HEARING: f32 = 10.0;
/// How far furnace sounds carry, in nodes (minetest_game's max_hear_distance).
const FURNACE_HEARING: f32 = 16.0;
/// How far sounds carry by default (Luanti's max_hear_distance).
const HEARING: f32 = 32.0;
/// How far from players short-lived particles are sent.
const PARTICLE_RANGE: f32 = 64.0;
/// How far from players (map blocks) butterflies and fireflies look at the
/// light: less than Luanti's active block range (4), to keep the block
/// columns looked at within what the server keeps worked out.
const INSECT_RANGE: i16 = 3;
/// How many nodes a hexer's cast drains at most.
const DRAIN_NODES: u32 = 4;
/// How often the heaps are checked for damage.
const HEAP_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// How many recent events the heap check reports.
const RECENT: usize = 32;
/// How far a door is heard opening and closing (its max_hear_distance).
const DOOR_HEARING: f32 = 10.0;
/// How often liquids flow a step (Luanti's liquid_update).
const LIQUID_UPDATE: Duration = Duration::from_secs(1);
/// Liquid changes in a block sent node by node; more, and the whole block
/// is sent again.
const NODE_UPDATES_PER_BLOCK: usize = 8;
/// A player link silent this long, either way, is reported with the
/// board's own view of its memory and Wi-Fi (the client gives up at 30 s;
/// an idle client still pings every 5 s or so).
const LINK_QUIET: Duration = Duration::from_secs(8);
/// Underwater, a breath is lost every DROWN_INTERVAL; out of breath, the
/// node's drowning damage is taken as often. In the air a breath comes
/// back every BREATHE_INTERVAL (PlayerSAO::step).
const DROWN_INTERVAL: Duration = Duration::from_secs(2);
const BREATHE_INTERVAL: Duration = Duration::from_millis(500);
/// A changed inventory is written to the SD card at most this long after
/// the change (and when the player leaves), not after every dig.
const INVENTORY_SAVE: Duration = Duration::from_secs(5);
/// How often lava touching water may cool, and the chance each time
/// (minetest_game's lava cooling ABM: interval 2, chance 2).
const LAVA_COOLING: Duration = Duration::from_secs(2);
const LAVA_COOLING_CHANCE: u32 = 2;
/// Wi-Fi power saving goes on after this long with nobody doing anything
/// (no one connected, or every player standing still).
const IDLE_POWER_SAVE: Duration = Duration::from_secs(30);
/// How many reliable packets may be in flight (unacknowledged) per channel.
/// Luanti's own minimum window is 32.
const RELIABLE_WINDOW: usize = 32;
/// Reliable packets from a client that arrive ahead of a missing one are
/// kept (and acknowledged) if they are at most this far ahead, as Luanti
/// does. Dropping them instead makes the client resend them with growing
/// delays, and it gives up on a packet not acknowledged within 30 s.
const EARLY_WINDOW: u16 = 512;
/// How often to repeat "please reconnect" to a client from before a restart.
const STALE_NOTICE_INTERVAL: Duration = Duration::from_secs(3);
/// If the loop has been busy this long without waiting for packets, it
/// pauses for one scheduler tick so lower-priority tasks (including the
/// idle task the watchdog checks) get to run.
const MAX_BUSY: Duration = Duration::from_millis(200);
/// Time per pass of the main loop for generating map blocks, so incoming
/// packets are read often enough not to overflow the socket's queue.
const BLOCK_TIME_BUDGET: Duration = Duration::from_millis(15);
/// When a send fails because the Wi-Fi transmit buffers are full, hold
/// back non-urgent sending (new reliable packets, map blocks, resends) for
/// this long so the radio can catch up.
const CONGESTION_BACKOFF: Duration = Duration::from_millis(20);
/// A packet that couldn't be sent is tried again after this long (rather
/// than the full RESEND_INTERVAL).
const RETRY_UNSENT: Duration = Duration::from_millis(30);
/// Each resend of a packet waits this much longer than the last (Luanti).
const RESEND_SCALE_BASE: f32 = 1.5;
/// Report failed sends at most this often.
const SEND_FAILURE_REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// Sent to clients in TOCLIENT_AUTH_ACCEPT. Luanti's default
/// `dedicated_server_step` is 0.09 seconds.
const SEND_INTERVAL: f32 = 0.09;
const MAP_SEED: u64 = 0;
/// Start the day at 10:00, running at Luanti's default speed.

/// Horizontal view radius in map blocks (the client may ask for less).
const MAX_VIEW_RADIUS: i16 = 5;
/// Keep at most this many packets queued on the block channel, so blocks
/// are generated as the client takes them rather than all at once.
const BLOCK_QUEUE_LIMIT: usize = 8;
/// Blocks taking longer than this to build are reported in the log.
const SLOW_BLOCK: Duration = Duration::from_millis(500);
/// How many map blocks the builder thread may have in hand at once. Few,
/// so that newly wanted (nearer) blocks don't wait behind a long queue.
const BUILDS_IN_FLIGHT: usize = 4;
/// Built blocks kept for other players (or a return visit); emptied when
/// full. A block is about 1 to 3 KB, so up to about 16 MB on the PC.
const BUILT_CACHE_MAX: usize = if cfg!(pc) { 8192 } else { 600 };
/// Block data goes on channel 2, like the Luanti server.
const BLOCK_CHANNEL: usize = 2;
/// How far (in nodes, from the player's feet) a player may dig or build.
/// The hand's range is 4 nodes from the eyes; this leaves room for lag.
const MAX_INTERACT_DISTANCE: f32 = 8.0;
/// Hotbar slots shown by the client (Luanti's default).
const HOTBAR_SIZE: usize = 8;
/// How often a moving player's position is saved to the SD card.
const SAVE_INTERVAL: Duration = Duration::from_secs(30);
/// How many players can be in the game at once on the board (Luanti's
/// max_users, default 15 there). More would overload the ESP32: see
/// docs/features.md. The PC build can take more (Setup::max_players).
pub const MAX_PLAYERS: usize = 6;
/// Peer ids: 0 and 1 are Luanti's "none" and the server. A player's object
/// id is their peer id, so peer ids stay below the boats', mobs' and dropped
/// items' object ids (from boats::FIRST_ID up).
const FIRST_PEER_ID: u16 = 2;
const LAST_PEER_ID: u16 = crate::boats::FIRST_ID - 1;

/// The peer id for a new connection, as Luanti's Connection::createPeer
/// picks it: the next one after the last given out that nobody is using,
/// going round to the start after the last. None if all are in use.
fn free_peer_id(next: u16, in_use: &HashSet<u16>) -> Option<u16> {
    let next = next.clamp(FIRST_PEER_ID, LAST_PEER_ID);
    (next..=LAST_PEER_ID).chain(FIRST_PEER_ID..next).find(|id| !in_use.contains(id))
}
/// Luanti's chat limits (chat_message_max_size, and
/// chat_message_limit_per_10sec).
const CHAT_MAX_CHARS: usize = 500;
const CHAT_MAX_PER_10S: usize = 8;
/// The welcome message, shown in the chat area when a player joins, one
/// line per message. A /sd/motd.txt file replaces it.
fn welcome(max_players: usize) -> String {
    let what = if cfg!(target_os = "espidf") {
        format!("on an ESP32-S3 microcontroller: a {} board, with 512 KB of RAM plus 8 MB of PSRAM", crate::board::BOARD.name)
    } else {
        "as the PC build of a server made for the ESP32-S3 microcontroller".to_string()
    };
    format!(
        "Welcome! This server runs Minetest Game {what}.
It is written from scratch in Rust, with no Lua: the game is built in. Up to {max_players} players.
Source: https://github.com/kf106/esp32-s3-minetest
Chat works as usual; type /help for the commands."
    )
}

/// What the server calls itself in /status.
const SERVER_NAME: &str = if cfg!(target_os = "espidf") { "ESP32-S3 Minetest server" } else { "Minetest server (esp32-s3-minetest PC build)" };
/// How often dropped items are moved on.
const ITEM_STEP_INTERVAL: Duration = Duration::from_millis(50);
/// builtin's item_drop: thrown from 1.2 nodes above the feet, at 2.9
/// nodes per second along the look direction plus 2 upwards.
const DROP_HEIGHT: f32 = 1.2;
const DROP_SPEED: f32 = 2.9;
const DROP_LIFT: f32 = 2.0;

struct Unacked {
    seqnum: u16,
    data: Vec<u8>,
    /// When it first went out (None: not yet, the radio was full), for the
    /// round-trip time when its ACK comes.
    first_sent: Option<Instant>,
    /// When to send it (again) if no ACK has come.
    due: Instant,
    /// Times it has been resent.
    resends: u32,
}

struct Channel {
    /// Next seqnum we will use for an outgoing reliable packet.
    next_out: u16,
    /// Next seqnum we expect from the client.
    next_in: u16,
    /// Next sequence number for an outgoing split packet.
    next_split: u16,
    /// Reliable packets we have sent that the client hasn't acknowledged.
    unacked: Vec<Unacked>,
    /// Packet bodies waiting for room in the reliable window.
    queue: VecDeque<Vec<u8>>,
    /// Reliable packets from the client that arrived before `next_in`'s,
    /// by sequence number: the inner packet's bytes.
    early: HashMap<u16, Vec<u8>>,
    /// Commands from the client that came split into chunks, by their
    /// split sequence number, while chunks are still to come.
    splits: HashMap<u16, IncomingSplit>,
}

/// One chunk of a split command, as it arrived. (A struct, as Xtensa
/// function calls carry only six argument words.)
struct SplitChunk<'a> {
    seqnum: u16,
    count: u16,
    num: u16,
    data: &'a [u8],
}

/// A command arriving in chunks (Luanti's IncomingSplitPacket).
struct IncomingSplit {
    chunks: Vec<Option<Vec<u8>>>,
    received: usize,
    started: Instant,
}

/// At most this many chunks in one split command: a client's biggest is a
/// request for every media file, a few kilobytes.
const MAX_SPLIT_CHUNKS: u16 = 64;
/// At most this many split commands being put together on a channel.
const MAX_SPLITS: usize = 4;
/// Unfinished split commands are dropped after this long (Luanti drops
/// unreliable ones after 30 s; reliable ones always complete).
const SPLIT_TIMEOUT: Duration = Duration::from_secs(30);

impl Channel {
    fn new() -> Self {
        Self {
            next_out: SEQNUM_INITIAL,
            next_in: SEQNUM_INITIAL,
            next_split: SEQNUM_INITIAL,
            unacked: Vec::new(),
            queue: VecDeque::new(),
            early: HashMap::new(),
            splits: HashMap::new(),
        }
    }
}

struct Peer {
    id: u16,
    channels: [Channel; CHANNEL_COUNT],
    last_seen: Instant,
    name: Option<String>,
    hello_sent: bool,
    /// The protocol version agreed (in TOSERVER_INIT).
    proto: u16,
    /// Auth mechanisms we offered in TOCLIENT_HELLO.
    allowed_auth_mechs: u32,
    authenticated: bool,
    /// Where /home takes them (/sethome), from their player file.
    home: Option<[f32; 3]>,
    /// What the player may do, from login (privs.rs).
    privs: Privs,
    content_sent: bool,
    ready: bool,
    /// Blocks the client has (sent, and not reported deleted).
    sent_blocks: crate::fasthash::FastSet<BlockPos>,
    /// Blocks still to send, nearest first.
    pending_blocks: VecDeque<BlockPos>,
    /// Blocks being built for this player, in the order wanted.
    awaiting: VecDeque<BlockPos>,
    /// Block the player was in when `pending_blocks` was planned.
    view_center: Option<BlockPos>,
    view_radius: i16,
    /// The player's position in BS units (10 per node), velocity and yaw,
    /// as last reported. The player's object id is the peer id.
    pos_bs: [f32; 3],
    speed_bs: [f32; 3],
    yaw: f32,
    /// The animation other clients are showing for this player.
    anim: Animation,
    /// SRP login in progress: set by SRP_BYTES_A, checked by SRP_BYTES_M.
    srp: Option<srp::Challenge>,
    /// SRP_BYTES_A was received; Luanti allows only one try per connection.
    srp_started: bool,
    /// Loaded from the SD card when the player joins.
    inventory: Inventory,
    pitch: f32,
    hp: u16,
    /// What was last written to the player file, and when.
    saved: Option<PlayerData>,
    saved_at: Instant,
    /// The key of the skin they wear (see skins.rs).
    skin: String,
    /// When their recent chat messages were sent, for the flood limit.
    chat_times: VecDeque<Instant>,
    /// The furnace whose window they have open.
    furnace: Option<NodePos>,
    /// When the furnace window was last sent (opened or updated).
    furnace_form_sent: Option<Instant>,
    /// Keys held, as last reported.
    keys: u32,
    /// The chest whose window they have open.
    chest: Option<NodePos>,
    /// The hotbar slot of the book whose window they have open
    /// (minetest_game's book_writers), and the tab they last chose.
    book_slot: Option<usize>,
    book_tab: u8,
    /// The hotbar slot in hand (TOSERVER_PLAYERITEM, and each interaction).
    wield: usize,
    /// The zoom their properties last gave them, and whether they were
    /// last allowed the minimap (binoculars.rs).
    zoom_fov: f32,
    minimap: Option<bool>,
    /// The fire's crackle playing to this player, if any.
    fire_sound: Option<i32>,
    /// The inventory window's Recipes tab: what it shows.
    guide: craftguide::Guide,
    /// The bed they lie in, if any.
    in_bed: Option<InBed>,
    /// The boat they sit in, if any.
    in_boat: Option<u16>,
    /// Already reported: nothing heard from them for LINK_QUIET, and none
    /// of our packets acknowledged for LINK_QUIET (see watch_links).
    quiet_reported: bool,
    unacked_reported: bool,
    /// The inventory changed and the client hasn't been sent it yet; and
    /// when it first changed since it was last saved (see INVENTORY_SAVE).
    inventory_unsent: bool,
    inventory_unsaved: Option<Instant>,
    /// Breath (0 to PLAYER_MAX_BREATH), whether the breath bar is shown,
    /// and since when it has been full again (it goes a second later).
    breath: u16,
    breath_bar: bool,
    breath_full_since: Option<Instant>,
    /// Average round-trip time (seconds) from ACKs, and the resend timeout
    /// it gives (Luanti's UDPPeer::reportRTT).
    avg_rtt: Option<f32>,
    resend_timeout: Duration,
}

impl Peer {
    /// Lying in bed, sitting in a boat, or neither.
    fn posture(&self) -> Posture {
        if self.in_bed.is_some() {
            Posture::Lie
        } else if self.in_boat.is_some() {
            Posture::Sit
        } else {
            Posture::Stand
        }
    }

    fn new(id: u16) -> Self {
        Self {
            id,
            channels: std::array::from_fn(|_| Channel::new()),
            last_seen: Instant::now(),
            name: None,
            hello_sent: false,
            proto: 0,
            allowed_auth_mechs: 0,
            authenticated: false,
            home: None,
            privs: Privs::NONE,
            content_sent: false,
            ready: false,
            sent_blocks: Default::default(),
            pending_blocks: VecDeque::new(),
            awaiting: VecDeque::new(),
            view_center: None,
            view_radius: MAX_VIEW_RADIUS,
            pos_bs: [0.0; 3],
            speed_bs: [0.0; 3],
            yaw: 0.0,
            anim: Animation::new(ANIM_STAND, ANIM_FPS),
            srp: None,
            srp_started: false,
            inventory: Inventory::new(),
            pitch: 0.0,
            hp: PLAYER_MAX_HP,
            saved: None,
            saved_at: Instant::now(),
            skin: skins::DEFAULT_SKIN.to_string(),
            chat_times: VecDeque::new(),
            furnace: None,
            furnace_form_sent: None,
            keys: 0,
            chest: None,
            book_slot: None,
            book_tab: 1,
            wield: 0,
            zoom_fov: 0.0,
            minimap: None,
            fire_sound: None,
            guide: craftguide::Guide::default(),
            in_bed: None,
            in_boat: None,
            quiet_reported: false,
            unacked_reported: false,
            inventory_unsent: false,
            inventory_unsaved: None,
            breath: PLAYER_MAX_BREATH,
            breath_bar: false,
            breath_full_since: None,
            avg_rtt: None,
            resend_timeout: RESEND_INTERVAL,
        }
    }

    /// A round-trip time from an ACK: keep a running average, and resend
    /// after twice it, from 0.1 to 2 seconds (Luanti's reportRTT).
    fn report_rtt(&mut self, rtt: f32) {
        let avg = match self.avg_rtt {
            Some(avg) => avg * 0.9 + rtt * 0.1,
            None => rtt,
        };
        self.avg_rtt = Some(avg);
        self.resend_timeout = Duration::from_secs_f32((avg * 2.0).clamp(0.1, 2.0));
    }

    /// Write the inventory to the SD card, if it changed since last time.
    fn save_inventory(&mut self, store: &InventoryStore) {
        if self.inventory_unsaved.take().is_some() {
            store.save(self.display_name(), &self.inventory);
        }
    }

    fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("?")
    }

    fn player_data(&self) -> PlayerData {
        PlayerData {
            pos: self.pos_bs.map(|v| v / 10.0),
            yaw: self.yaw,
            pitch: self.pitch,
            hp: self.hp,
            skin: self.skin.clone(),
            home: self.home,
        }
    }

    /// Write the player file if this is a player in the game and anything
    /// changed since the last save.
    fn save(&mut self, store: &PlayerStore) {
        if !self.ready {
            return;
        }
        let data = self.player_data();
        if self.saved.as_ref() != Some(&data) {
            store.save(self.display_name(), &data);
            self.saved = Some(data);
        }
        self.saved_at = Instant::now();
    }
}

/// Content packets built once at startup and sent to every client.
struct Content {
    itemdef: Vec<u8>,
    nodedef: Vec<u8>,
    announce_media: Vec<u8>,
    media: Media,
}

pub struct Server {
    socket: UdpSocket,
    auth: AuthStore,
    /// Privileges, the administrator and bans (privs.rs).
    privs: privs::Store,
    world: World,
    inventories: InventoryStore,
    players: PlayerStore,
    furnaces: Furnaces,
    chests: Chests,
    /// Chests shown with their lids up (someone has them open).
    open_chests: HashSet<NodePos>,
    status: SharedStatus,
    content: Content,
    peers: HashMap<SocketAddr, Peer>,
    next_peer_id: u16,
    /// Which player send_blocks starts with, rotating.
    block_turn: usize,
    /// Addresses of clients left over from before a restart, and when we
    /// last told them to reconnect.
    stale_told: HashMap<SocketAddr, Instant>,
    /// SRP challenges are computed on another thread (they take about a
    /// second on the ESP32); results come back here.
    srp_results: Receiver<SrpResult>,
    /// Logins waiting for the SRP thread; None if it couldn't be started.
    srp_jobs: Option<Sender<SrpJob>>,
    /// The thread on the second core that builds map blocks (None if it
    /// couldn't be started: then blocks are built in the server loop).
    builder: Option<BlockBuilder>,
    /// Blocks the builder is working on.
    building: HashSet<BlockPos>,
    /// Built blocks (TOCLIENT_BLOCKDATA commands), with the world version
    /// they were built from.
    built: crate::fasthash::FastMap<BlockPos, (u64, Arc<Vec<u8>>)>,
    /// Dropped items, and when they were last moved on.
    items: Items,
    items_stepped: Instant,
    /// Boats (boats.rs), saved beside the items.
    boats: boats::Boats,
    /// Hostile mobs (mobs.rs), when on; when spawning was last tried.
    mobs: Mobs,
    mobs_spawned: Instant,
    /// The looping hum of each wisp, and the whine of its fuse while lit
    /// (sound ids), and when a wisp last blasted nodes.
    mob_sounds: HashMap<u16, (i32, i32)>,
    /// The id for the next particle spawner, and a lit wisp's sparks.
    next_spawner: u32,
    mob_sparks: HashMap<u16, u32>,
    mob_blasted: Option<Instant>,
    /// Sand and gravel on their way down (falling.rs).
    falling: Vec<Falling>,
    /// When the furnaces' timers last ran.
    furnaces_stepped: Instant,
    /// When bones were last checked for having become old.
    bones_checked: Instant,
    /// The time of day, kept over restarts.
    clock: Clock,
    /// Who owns each steel door.
    door_owners: doors::Owners,
    /// The bed each player respawns at.
    bed_spawns: beds::Spawns,
    /// When everyone online lay down in bed: the night is skipped two
    /// seconds later (beds' schedule_update).
    night_skip_at: Option<Instant>,
    /// Farmed soil, looked after every farming::SOIL_INTERVAL.
    fields: BTreeSet<NodePos>,
    fields_checked: Instant,
    /// Cactus and papyrus players planted, which grow (plants.rs).
    climbers: BTreeSet<NodePos>,
    climbers_checked: Instant,
    /// Fire and TNT: the settings, every flame there is, and when the fire mod's
    /// ABMs, crackle and burning of dropped items last ran.
    settings: settings::Settings,
    flames: BTreeSet<NodePos>,
    fire_ignited: Instant,
    fire_burned: Instant,
    fire_sounded: Instant,
    tnt_ignited: Instant,
    /// When the butterflies and fireflies near players last looked at the
    /// light (insects.rs).
    insects_tended: Instant,
    items_burned: Instant,
    /// When the liquids last flowed.
    liquids_flowed: Instant,
    /// When lava touching water was last looked at.
    lava_cooled: Instant,
    moss_checked: Instant,
    /// When players were last checked for binoculars, and for a mapping
    /// kit.
    zoom_checked: Instant,
    map_checked: Instant,
    /// When the weather last changed the clouds.
    weather_set: Instant,
    /// When env_sounds last listened round the players.
    sounds_played: Instant,
    /// When grass last spread, and last died under cover.
    grass_spread: Instant,
    grass_covered: Instant,
    /// When breath was last lost underwater, and last regained.
    drowned_at: Instant,
    breathed_at: Instant,
    /// Liquid activity since it was last reported: nodes changed, blocks
    /// sent again, and when.
    liquid_changes: u32,
    liquid_blocks_resent: u32,
    liquids_reported: Instant,
    /// The last RECENT things the server did, for the heap check.
    recent: VecDeque<(Instant, &'static str, u16)>,
    heap_checked: Instant,
    heap_check_timed: bool,
    heap_damaged: bool,
    /// Id for the next sound that may need fading out.
    next_sound_id: i32,
    /// When a player last did something (moved, looked, dug, chatted...)
    /// or a client was logging in, and whether Wi-Fi power saving is on.
    last_activity: Instant,
    power_saving: bool,
    /// Running its own hotspot (so no power saving: that is for a client).
    hotspot: bool,
    /// Leaf decay and sapling timers.
    timers: Timers,
    rng: Rng,
    skins: Skins,
    /// Welcome message lines.
    welcome: Vec<String>,
    /// How many players may be in the game at once.
    max_players: usize,
    /// Sending is held back until then (see CONGESTION_BACKOFF).
    congested_until: Instant,
    /// Sends that failed since the last report, and when that was.
    send_failures: u32,
    send_failures_reported: Instant,
    /// Reliable packets resent, and ACKs received, since the last report.
    resends: u32,
    acks: u32,
}

/// Everything the server needs, gathered up by main().
pub struct Setup {
    pub socket: UdpSocket,
    /// The welcome message from the SD card, replacing welcome().
    pub motd: Option<String>,
    pub auth: AuthStore,
    pub world: World,
    pub inventories: InventoryStore,
    pub players: PlayerStore,
    pub furnaces: Furnaces,
    pub chests: Chests,
    pub items: Items,
    pub clock: Clock,
    pub door_owners: doors::Owners,
    pub bed_spawns: beds::Spawns,
    /// Whether fire spreads and TNT can be lit (world/settings.txt).
    pub settings: settings::Settings,
    pub media: Media,
    pub status: SharedStatus,
    /// How many players may be in the game at once (MAX_PLAYERS on the
    /// board).
    pub max_players: usize,
}

/// A login for the SRP thread to work out.
struct SrpJob {
    addr: SocketAddr,
    peer_id: u16,
    /// The name exactly as the client sent it: it is part of the proof.
    name: String,
    account: auth::Account,
    bytes_a: Vec<u8>,
}

/// The SRP thread, started once, at startup: starting a thread per login
/// needs 16 KB of internal RAM in one piece, which may not be there once
/// the server has been running a while. Logins queue up and are worked
/// out one at a time.
fn start_srp_thread(results: Sender<SrpResult>) -> Option<Sender<SrpJob>> {
    let (jobs, queue) = mpsc::channel::<SrpJob>();
    // Measured at 2 KB used, but once 6.5 KB (see the "Stack left"
    // report): room to spare.
    let spawned = spawn_on_second_core(b"srp\0", 12 * 1024, 4, move || {
            for job in queue {
                let started = Instant::now();
                let challenge = srp::start(&srp::Login {
                    username: &job.name,
                    salt: &job.account.salt,
                    verifier: &job.account.verifier,
                    bytes_a: &job.bytes_a,
                });
                let _ = results.send(SrpResult {
                    addr: job.addr,
                    peer_id: job.peer_id,
                    salt: job.account.salt,
                    challenge,
                    took: started.elapsed(),
                });
            }
        });
    match spawned {
        Ok(_) => Some(jobs),
        Err(e) => {
            log::error!("Starting the SRP thread: {e}; existing accounts won't be able to log in");
            None
        }
    }
}

/// A map block for the builder to make, from a snapshot of the world's
/// changes.
struct BuildJob {
    pos: BlockPos,
    state: Arc<world::State>,
}

/// A finished block: the TOCLIENT_BLOCKDATA command, and the world version
/// it was built from.
struct Built {
    pos: BlockPos,
    version: u64,
    payload: Vec<u8>,
}

struct BlockBuilder {
    jobs: Sender<BuildJob>,
    done: Receiver<Built>,
}

/// Start the thread that builds (generates and compresses) map blocks, on
/// the second CPU core, which otherwise has little to do: the server loop
/// on the first core then only sends them.
fn start_block_builder(status: SharedStatus, port: u16) -> Option<BlockBuilder> {
    let (jobs, queue) = mpsc::channel::<BuildJob>();
    let (finished, done) = mpsc::channel::<Built>();
    // Each finished block is followed by an empty datagram to the server's
    // own port, which wakes the server loop from waiting for packets so it
    // sends the block straight away.
    let nudge = UdpSocket::bind(("127.0.0.1", 0))
        .map_err(|e| warn!("Block builder: no wake-up socket ({e}); blocks may wait up to {POLL_INTERVAL:?}"))
        .ok();
    // Priority 1, just above core 1's idle task; logins and the display
    // come first. After each block it pauses for a scheduler tick so the
    // idle task (which the task watchdog checks on) gets a turn.
    // Measured: about 7.5 KB used (see the "Stack left" report).
    let spawned = spawn_on_second_core(b"blocks\0", 16 * 1024, 1, move || {
        // Its own copy of the terrain caches; the changes come with each job.
        let mut world = World::builder();
        for job in queue {
            let started = Instant::now();
            let version = job.state.version();
            world.use_snapshot(job.state);
            let payload = build_blockdata(&world.block_payload(job.pos));
            let took = started.elapsed();
            if took >= SLOW_BLOCK {
                warn!("Block {:?} took {} ms to build", job.pos, took.as_millis());
            }
            {
                let mut st = status::lock(&status);
                st.blocks_built += 1;
                st.block_us += took.as_micros() as u64;
            }
            if finished.send(Built { pos: job.pos, version, payload }).is_err() {
                break;
            }
            if let Some(n) = &nudge {
                let _ = n.send_to(&[], ("127.0.0.1", port));
            }
            let_other_tasks_run();
        }
    });
    match spawned {
        Ok(()) => Some(BlockBuilder { jobs, done }),
        Err(e) => {
            warn!("Starting the block builder: {e}; building blocks in the server loop instead");
            None
        }
    }
}

/// Start a thread pinned to the second core (core 1). The server loop runs
/// on core 0, with the Wi-Fi driver; core 1 is otherwise nearly idle.
/// `priority` is FreeRTOS's (the server loop has 1; higher runs first).
#[cfg(target_os = "espidf")]
fn spawn_on_second_core<F>(name: &'static [u8], stack: usize, priority: u8, f: F) -> std::io::Result<()>
where
    F: FnOnce() + Send + 'static,
{
    use esp_idf_svc::sys::{esp_pthread_get_default_config, esp_pthread_set_cfg};
    // The next thread this one starts takes these settings.
    unsafe {
        let mut cfg = esp_pthread_get_default_config();
        cfg.thread_name = name.as_ptr() as *const _;
        cfg.pin_to_core = 1;
        cfg.prio = priority as _;
        cfg.stack_size = stack as _;
        esp_pthread_set_cfg(&cfg);
    }
    let spawned = std::thread::Builder::new().stack_size(stack).spawn(f).map(|_| ());
    unsafe {
        let cfg = esp_pthread_get_default_config();
        esp_pthread_set_cfg(&cfg);
    }
    spawned
}

#[cfg(not(target_os = "espidf"))]
fn spawn_on_second_core<F>(name: &'static [u8], stack: usize, _priority: u8, f: F) -> std::io::Result<()>
where
    F: FnOnce() + Send + 'static,
{
    let name = String::from_utf8_lossy(name.strip_suffix(b"\0").unwrap_or(name)).into_owned();
    std::thread::Builder::new().name(name).stack_size(stack).spawn(f).map(|_| ())
}

struct SrpResult {
    addr: SocketAddr,
    /// The peer id when the work started, in case the address was reused.
    peer_id: u16,
    salt: Vec<u8>,
    challenge: Option<srp::Challenge>,
    took: Duration,
}

/// Pause for one FreeRTOS tick. (A short std sleep would busy-wait.)
#[cfg(target_os = "espidf")]
fn let_other_tasks_run() {
    unsafe { esp_idf_svc::sys::vTaskDelay(1) };
}

#[cfg(not(target_os = "espidf"))]
fn let_other_tasks_run() {
    std::thread::yield_now();
}

impl Server {
    /// Takes one struct rather than six arguments: with the hidden return
    /// pointer that made seven, one more than the Xtensa calling
    /// convention passes in registers, and the seventh (the status) arrived
    /// as garbage on the ESP32.
    pub fn new(setup: Setup) -> Self {
        let Setup { socket, motd, auth, world, inventories, players, furnaces, chests, mut items, clock, door_owners, bed_spawns, settings, media, status, max_players } = setup;
        // Items dropped before a restart: out of any block built over them.
        items.settle(&|x, y, z| content::walkable(world.node((x, y, z))));
        let itemdef = build_itemdef(&content::itemdef_bytes());
        let nodedef = build_nodedef(&content::nodedef_bytes());
        let announce_media = build_announce_media(
            media.files().iter().map(|f| (f.name.as_str(), &f.sha1)),
        );
        info!(
            "Content: {} nodes, ITEMDEF {} bytes, NODEDEF {} bytes, ANNOUNCE_MEDIA {} bytes",
            content::NODES.len(),
            itemdef.len(),
            nodedef.len(),
            announce_media.len()
        );
        let skins = Skins::from_media(&media);
        let (srp_sender, srp_results) = mpsc::channel();
        let srp_jobs = start_srp_thread(srp_sender);
        let port = socket.local_addr().map(|a| a.port()).unwrap_or(0);
        let builder = start_block_builder(status.clone(), port);
        // On its own hotspot the board is the access point, which never dozes.
        let hotspot = status::lock(&status).hotspot.is_some();
        // Saplings planted before a restart: timers again, as when planted
        // (minetest_game's node timers survive a restart; ours don't).
        let mut timers = Timers::new();
        let mut rng = Rng::new();
        let saplings = world.changed_nodes(content::is_sapling);
        for &pos in &saplings {
            timers.start(pos, Job::Grow, nature::grow_delay(&mut rng));
        }
        if !saplings.is_empty() {
            info!("{} saplings growing", saplings.len());
        }
        for pos in world.changed_nodes(content::is_mark) {
            timers.start(pos, Job::Regrow, nature::grow_delay(&mut rng));
        }
        // Flames go on burning, and look for something to burn again.
        let flames: BTreeSet<NodePos> = world.changed_nodes(fire::is_flame).into_iter().collect();
        for &pos in &flames {
            if world.node(pos) == fire::BASIC_FLAME {
                timers.start(pos, Job::Flame, fire::flame_timer(&mut rng));
            }
        }
        if !flames.is_empty() {
            info!("Fire: {} flames burning", flames.len());
        }
        // Lit TNT and gunpowder carry on burning.
        for pos in world.changed_nodes(tnt::is_lit) {
            let job = if world.node(pos) == tnt::TNT_BURNING { Job::Fuse } else { Job::Powder };
            timers.start(pos, job, if job == Job::Fuse { tnt::FUSE } else { tnt::POWDER_BURNS });
        }
        // Seeds and crops carry on growing; soil is looked after again.
        let crops = world.changed_nodes(farming::is_growing);
        for &pos in &crops {
            timers.start(pos, Job::Crop, farming::grow_retry(&mut rng));
        }
        let fields: BTreeSet<NodePos> = world.changed_nodes(farming::is_soil).into_iter().collect();
        if !crops.is_empty() || !fields.is_empty() {
            info!("Farming: {} soil, {} crops growing", fields.len(), crops.len());
        }
        if nature::TEST_BUILD {
            info!("Test build: saplings, apples and crops grow ten times faster");
        }
        // Planted cactus and papyrus carry on growing; picked blueberry
        // bushes bear again.
        let climbers: BTreeSet<NodePos> = world.changed_nodes(plants::is_climber).into_iter().collect();
        for pos in world.changed_nodes(|id| id == plants::BLUEBERRY_LEAVES) {
            if world.param2(pos) != nature::PLACED_LEAVES {
                timers.start(pos, Job::Berries, nature::grow_delay(&mut rng));
            }
        }
        // Chests left open when the server stopped are closed again
        // (minetest_game's "close opened chests on load").
        let mut world = world;
        for pos in world.changed_nodes(|id| content::chest_pair(id).is_some_and(|(_, open)| open == id)) {
            if let Some((closed, _)) = content::chest_pair(world.node(pos)) {
                let param2 = world.param2(pos);
                world.set_node(pos, closed, param2);
            }
        }
        let privs = privs::Store::open(auth.dir());
        if let Some(admin) = privs.admin() {
            info!("The administrator is {admin}");
        }
        Self {
            socket,
            privs,
            auth,
            world,
            inventories,
            players,
            furnaces,
            chests,
            open_chests: HashSet::new(),
            status,
            content: Content {
                itemdef,
                nodedef,
                announce_media,
                media,
            },
            peers: HashMap::new(),
            next_peer_id: FIRST_PEER_ID,
            block_turn: 0,
            stale_told: HashMap::new(),
            srp_results,
            srp_jobs,
            builder,
            building: HashSet::new(),
            built: Default::default(),
            boats: items.path().map_or_else(boats::Boats::default, |p| boats::Boats::open(p.with_file_name("boats.txt"))),
            items,
            clock,
            door_owners,
            bed_spawns,
            night_skip_at: None,
            fields,
            fields_checked: Instant::now(),
            climbers,
            climbers_checked: Instant::now(),
            settings,
            flames,
            fire_ignited: Instant::now(),
            fire_burned: Instant::now(),
            fire_sounded: Instant::now(),
            tnt_ignited: Instant::now(),
            insects_tended: Instant::now(),
            items_burned: Instant::now(),
            liquids_flowed: Instant::now(),
            lava_cooled: Instant::now(),
            moss_checked: Instant::now(),
            zoom_checked: Instant::now(),
            map_checked: Instant::now(),
            weather_set: Instant::now(),
            sounds_played: Instant::now(),
            grass_spread: Instant::now(),
            grass_covered: Instant::now(),
            drowned_at: Instant::now(),
            breathed_at: Instant::now(),
            liquid_changes: 0,
            liquid_blocks_resent: 0,
            liquids_reported: Instant::now(),
            recent: VecDeque::new(),
            heap_checked: Instant::now(),
            heap_check_timed: false,
            heap_damaged: false,
            items_stepped: Instant::now(),
            mobs: Mobs::new(),
            mobs_spawned: Instant::now(),
            mob_sounds: HashMap::new(),
            next_spawner: 1,
            mob_sparks: HashMap::new(),
            mob_blasted: None,
            falling: Vec::new(),
            furnaces_stepped: Instant::now(),
            bones_checked: Instant::now(),
            next_sound_id: 1,
            last_activity: Instant::now(),
            power_saving: false,
            hotspot,
            timers,
            rng,
            skins,
            max_players,
            welcome: motd
                .unwrap_or_else(|| welcome(max_players))
                .lines()
                .map(str::trim_end)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect(),
            congested_until: Instant::now(),
            send_failures: 0,
            send_failures_reported: Instant::now(),
            resends: 0,
            acks: 0,
        }
    }

    /// Where the shared status data is, as seen through the `status` field.
    pub fn status_data_addr(&self) -> usize {
        Arc::as_ptr(&self.status) as usize
    }

    pub fn run(&mut self) -> std::io::Result<()> {
        self.socket.set_read_timeout(Some(POLL_INTERVAL))?;
        let mut buf = [0u8; 1500];
        let mut last_wait = Instant::now();
        // For the status screen: how long the loop works rather than waits.
        let (mut busy, mut reported) = (Duration::ZERO, Instant::now());
        loop {
            // Wait up to POLL_INTERVAL for a packet, then take everything
            // else already waiting before doing anything slow: the socket's
            // queue is short, and packets that don't fit are lost.
            let pass = Instant::now();
            let received = self.socket.recv_from(&mut buf);
            let waited = pass.elapsed();
            match received {
                Ok((len, from)) => {
                    self.handle_datagram(&buf[..len], from);
                    self.socket.set_nonblocking(true)?;
                    let drained = loop {
                        match self.socket.recv_from(&mut buf) {
                            Ok((len, from)) => self.handle_datagram(&buf[..len], from),
                            Err(e) => break e,
                        }
                    };
                    self.socket.set_nonblocking(false)?;
                    if !matches!(drained.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
                        return Err(drained);
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    last_wait = Instant::now(); // we were idle for a while
                }
                Err(e) => return Err(e),
            }
            self.tick();
            if last_wait.elapsed() >= MAX_BUSY {
                let_other_tasks_run();
                last_wait = Instant::now();
            }
            busy += pass.elapsed().saturating_sub(waited);
            if reported.elapsed() >= Duration::from_millis(250) {
                status::lock(&self.status).busy_us += busy.as_micros() as u64;
                busy = Duration::ZERO;
                reported = Instant::now();
            }
        }
    }

    fn handle_datagram(&mut self, data: &[u8], from: SocketAddr) {
        // The block builder's "a block is ready" nudge (see BlockBuilder):
        // it only needed to wake the loop.
        if data.is_empty() && from.ip().is_loopback() {
            return;
        }
        status::lock(&self.status).packets_in += 1;
        let dg = match parse_datagram(data) {
            Ok(dg) => dg,
            Err(e) => {
                warn!("Bad datagram from {from}: {e}");
                hexdump(data);
                return;
            }
        };

        match self.peers.get(&from) {
            // A client uses peer id 0 until we give it one. A non-zero id
            // from an address we don't know is a session from before the
            // server restarted: Luanti ignores those, and so do we, after
            // telling the client to reconnect.
            None if dg.peer_id != PEER_ID_INEXISTENT => {
                self.reject_stale(from, dg.peer_id);
                return;
            }
            None => {
                if matches!(dg.packet, Packet::Control(Control::Disco)) {
                    return;
                }
                self.add_peer(from);
            }
            // The same address with someone else's id: also stale.
            Some(peer) if dg.peer_id != PEER_ID_INEXISTENT && dg.peer_id != peer.id => {
                debug!("Packet from {from} with peer id {} (expected {}); ignored", dg.peer_id, peer.id);
                return;
            }
            Some(_) => {}
        }
        if let Some(peer) = self.peers.get_mut(&from) {
            if peer.quiet_reported {
                peer.quiet_reported = false;
                let gap = peer.last_seen.elapsed().as_secs_f32();
                let name = peer.display_name().to_string();
                info!("Link: heard from {name} again after {gap:.1} s; {}", link_state());
            }
            peer.last_seen = Instant::now();
        }

        self.handle_packet(from, dg.channel as usize, dg.packet);
    }

    /// Handle kept packets that are now next in order.
    fn process_early(&mut self, from: SocketAddr, channel: usize) {
        loop {
            let Some(peer) = self.peers.get_mut(&from) else {
                return;
            };
            let ch = &mut peer.channels[channel];
            let Some(raw) = ch.early.remove(&ch.next_in) else {
                return;
            };
            ch.next_in = ch.next_in.wrapping_add(1);
            match parse_inner(&raw) {
                Ok(packet) => self.handle_packet(from, channel, packet),
                Err(e) => warn!("Bad kept packet from {from}: {e}"),
            }
        }
    }

    /// Tell a client still connected to a previous run of the server that
    /// it must reconnect: an unreliable ACCESS_DENIED (we don't know its
    /// sequence numbers) with the reconnect button, then DISCO. At most
    /// every few seconds per address, as it keeps sending until it stops.
    fn reject_stale(&mut self, from: SocketAddr, peer_id: u16) {
        let now = Instant::now();
        if let Some(last) = self.stale_told.get(&from) {
            if now.duration_since(*last) < STALE_NOTICE_INTERVAL {
                return;
            }
        } else {
            info!("{from} is still using peer id {peer_id} from before the server restarted; telling it to reconnect");
        }
        if self.stale_told.len() >= 64 {
            self.stale_told.retain(|_, t| now.duration_since(*t) < Duration::from_secs(60));
        }
        self.stale_told.insert(from, now);
        let denied = build_access_denied(
            AccessDenied::CustomString,
            "The server restarted. Please reconnect.",
            true,
        );
        self.send_raw(from, &unreliable_datagram(0, &denied));
        self.send_raw(from, &disco_datagram());
    }

    fn handle_packet(&mut self, from: SocketAddr, channel: usize, packet: Packet<'_>) {
        match packet {
            Packet::Reliable { seqnum, inner, raw } => {
                let Some(peer) = self.peers.get_mut(&from) else {
                    return;
                };
                let ch = &mut peer.channels[channel];
                let expected = ch.next_in;

                if seqnum == expected {
                    ch.next_in = expected.wrapping_add(1);
                    self.send_raw(from, &ack_datagram(channel as u8, seqnum));
                    self.handle_packet(from, channel, *inner);
                    self.process_early(from, channel);
                } else if seqnum_higher(expected, seqnum) {
                    // Already processed; our ACK must have been lost, so resend it.
                    debug!("Duplicate reliable {seqnum} on ch{channel}");
                    self.send_raw(from, &ack_datagram(channel as u8, seqnum));
                } else if seqnum.wrapping_sub(expected) < EARLY_WINDOW {
                    // Arrived ahead of one that was lost: keep it for later
                    // and acknowledge it now.
                    debug!("Early reliable {seqnum} on ch{channel} (expecting {expected}), kept");
                    ch.early.entry(seqnum).or_insert_with(|| raw.to_vec());
                    self.send_raw(from, &ack_datagram(channel as u8, seqnum));
                } else {
                    warn!("Reliable {seqnum} on ch{channel} is too far ahead of {expected}; dropped");
                }
            }

            Packet::Control(Control::Ack(seqnum)) => {
                if let Some(peer) = self.peers.get_mut(&from) {
                    let unacked = &mut peer.channels[channel].unacked;
                    if let Some(i) = unacked.iter().position(|u| u.seqnum == seqnum) {
                        // Round-trip time from its first sending ("a bit off
                        // for re-sent packets but that's okay", as Luanti says).
                        if let Some(sent) = unacked.remove(i).first_sent {
                            peer.report_rtt(sent.elapsed().as_secs_f32());
                            self.acks += 1;
                        }
                    }
                }
                self.flush(from, channel);
            }

            Packet::Control(Control::Ping) => {
                // Nothing to do: if it came reliably, it has already been ACKed.
            }

            Packet::Control(Control::Disco) => self.remove_peer(from, "disconnected", "left"),

            Packet::Control(other) => {
                warn!("Unexpected control packet from {from}: {other}");
            }

            Packet::Original(payload) => self.handle_command(from, channel, payload),

            Packet::Split {
                seqnum,
                chunk_count,
                chunk_num,
                data,
            } => {
                let chunk = SplitChunk { seqnum, count: chunk_count, num: chunk_num, data };
                if let Some(payload) = self.add_split_chunk(from, channel, chunk) {
                    self.handle_command(from, channel, &payload);
                }
            }
        }
    }

    /// One chunk of a split command (Luanti's IncomingSplitBuffer::insert):
    /// the whole command once its last chunk is in.
    fn add_split_chunk(&mut self, from: SocketAddr, channel: usize, chunk: SplitChunk<'_>) -> Option<Vec<u8>> {
        let SplitChunk { seqnum, count, num, data } = chunk;
        let ch = &mut self.peers.get_mut(&from)?.channels[channel];
        if count == 0 || count > MAX_SPLIT_CHUNKS || num >= count {
            warn!("Split packet {seqnum} from {from}: chunk {num} of {count} refused");
            return None;
        }
        ch.splits.retain(|_, s| s.started.elapsed() < SPLIT_TIMEOUT);
        if !ch.splits.contains_key(&seqnum) && ch.splits.len() >= MAX_SPLITS {
            warn!("Split packet {seqnum} from {from}: too many unfinished; dropped");
            return None;
        }
        let split = ch.splits.entry(seqnum).or_insert_with(|| IncomingSplit {
            chunks: vec![None; count as usize],
            received: 0,
            started: Instant::now(),
        });
        if split.chunks.len() != count as usize {
            warn!("Split packet {seqnum} from {from}: chunk count changed from {} to {count}", split.chunks.len());
            return None;
        }
        let slot = &mut split.chunks[num as usize];
        if slot.is_none() {
            *slot = Some(data.to_vec());
            split.received += 1;
        }
        if split.received < split.chunks.len() {
            return None;
        }
        let split = ch.splits.remove(&seqnum)?;
        let payload: Vec<u8> = split.chunks.into_iter().flatten().flatten().collect();
        debug!("Split packet {seqnum} from {from}: {count} chunks, {} bytes", payload.len());
        Some(payload)
    }

    fn handle_command(&mut self, from: SocketAddr, channel: usize, payload: &[u8]) {
        let Some(cmd) = messages::command_id(payload) else {
            return; // empty ORIGINAL packet: the client's "hello", nothing to do
        };
        let peer_id = self.peers.get(&from).map_or(0, |p| p.id);
        self.note_command(cmd, peer_id);
        // Logging in and loading, and anything a player does, keep Wi-Fi
        // fully awake. (Positions count only if something changed: see
        // on_playerpos.)
        let ready = self.peers.get(&from).is_some_and(|p| p.ready);
        let acting = matches!(
            cmd,
            TOSERVER_INTERACT
                | TOSERVER_INVENTORY_ACTION
                | TOSERVER_INVENTORY_FIELDS
                | TOSERVER_NODEMETA_FIELDS
                | TOSERVER_CHAT_MESSAGE
                | TOSERVER_DAMAGE
                | TOSERVER_RESPAWN_LEGACY
        );
        if !ready || acting {
            self.wake();
        }
        match cmd {
            TOSERVER_INIT => self.on_init(from, payload),
            TOSERVER_FIRST_SRP => self.on_first_srp(from, payload),
            TOSERVER_SRP_BYTES_A => self.on_srp_bytes_a(from, payload),
            TOSERVER_SRP_BYTES_M => self.on_srp_bytes_m(from, payload),
            TOSERVER_INTERACT => self.on_interact(from, payload),
            TOSERVER_INVENTORY_ACTION => self.on_inventory_action(from, payload),
            TOSERVER_DAMAGE => self.on_damage(from, payload),
            TOSERVER_RESPAWN_LEGACY => self.on_respawn(from),
            TOSERVER_INIT2 => self.on_init2(from, payload),
            TOSERVER_REQUEST_MEDIA => self.on_request_media(from, payload),
            TOSERVER_CLIENT_READY => self.on_client_ready(from, payload),
            TOSERVER_PLAYERPOS => self.on_playerpos(from, payload),
            TOSERVER_DELETEDBLOCKS => self.on_deleted_blocks(from, payload),
            // The client confirms blocks it has meshed; we don't need that yet.
            TOSERVER_GOTBLOCKS => {}
            // The hotbar slot in hand (digging and placing say it too):
            // a key in hand opens what it fits.
            TOSERVER_PLAYERITEM => {
                if let (Some(peer), [a, b, ..]) = (self.peers.get_mut(&from), payload) {
                    peer.wield = u16::from_be_bytes([*a, *b]) as usize;
                }
            }
            TOSERVER_INVENTORY_FIELDS => self.on_inventory_fields(from, payload),
            TOSERVER_NODEMETA_FIELDS => self.on_nodemeta_fields(from, payload),
            TOSERVER_CHAT_MESSAGE => self.on_chat(from, payload),
            // Window size and similar client details: nothing to do.
            0x0053 => {}
            // TOSERVER_REMOVED_SOUNDS: sounds the client finished playing.
            0x003a => {}
            other => {
                info!(
                    "Command 0x{other:04x} on ch{channel}, {} bytes:",
                    payload.len()
                );
                hexdump(&payload[..payload.len().min(64)]);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Login sequence
    // -----------------------------------------------------------------------

    fn on_init(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(init) = parse_init(payload) else {
            warn!("Malformed TOSERVER_INIT from {from}");
            hexdump(payload);
            return;
        };
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if peer.hello_sent {
            return; // the client resends INIT until it sees our HELLO
        }
        let id = peer.id;
        info!("TOSERVER_INIT from peer {id}: {init:?}");

        if init.max_proto < SERVER_PROTO_MIN || init.min_proto > SERVER_PROTO_MAX {
            warn!(
                "Client protocol range {}..={} is incompatible with ours {}..={}",
                init.min_proto, init.max_proto, SERVER_PROTO_MIN, SERVER_PROTO_MAX
            );
            self.deny(from, AccessDenied::WrongVersion, "");
            return;
        }

        match auth::check_name(&init.name) {
            Ok(()) => {}
            Err(NameError::BadChars) => {
                self.deny(from, AccessDenied::WrongCharsInName, "");
                return;
            }
            Err(NameError::Empty | NameError::TooLong) => {
                self.deny(from, AccessDenied::WrongName, "");
                return;
            }
        }

        // The administrator is never shut out (they may share an address).
        if let Some(ban) = self.privs.ban_on(&init.name, from.ip()).filter(|_| !self.privs.is_admin(&init.name)) {
            let reason = format!("Your IP is banned. Banned name was {}", ban.name);
            info!("{} at {from} is banned (as {})", init.name, ban.name);
            self.deny(from, AccessDenied::CustomString, &reason);
            return;
        }

        // Checked now so a client isn't taken through logging in only to be
        // turned away, and again when it is ready to join (Luanti does both).
        if self.user_limit_reached(&init.name) {
            info!("{} tried to join, but the game is full ({} players)", init.name, self.max_players);
            self.deny(from, AccessDenied::TooManyUsers, "");
            return;
        }

        let auth_mechs = match self.auth.load(&init.name) {
            // A cleared password (/clearpassword): the player sets a new
            // one by logging in, as with a new account.
            Ok(Some(a)) if a.verifier.is_empty() => AUTH_MECHANISM_FIRST_SRP,
            Ok(Some(_)) => AUTH_MECHANISM_SRP,
            Ok(None) => AUTH_MECHANISM_FIRST_SRP,
            Err(e) => {
                warn!("Reading account {}: {e}", init.name);
                self.deny(from, AccessDenied::ServerFail, "");
                return;
            }
        };

        let proto = init.max_proto.min(SERVER_PROTO_MAX);
        let ser_ver = init.max_ser_ver.min(SER_FMT_VER_HIGHEST);
        let hello = build_hello(ser_ver, proto, auth_mechs);

        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        peer.name = Some(init.name);
        peer.hello_sent = true;
        peer.proto = proto;
        peer.allowed_auth_mechs = auth_mechs;
        info!(
            "Sending TOCLIENT_HELLO: protocol {proto}, serialisation {ser_ver}, auth {}",
            if auth_mechs == AUTH_MECHANISM_SRP { "SRP (existing account)" } else { "FIRST_SRP (new account)" }
        );
        self.send_command(from, 0, &hello);
    }

    fn on_first_srp(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(msg) = parse_first_srp(payload) else {
            warn!("Malformed TOSERVER_FIRST_SRP from {from}");
            self.deny(from, AccessDenied::UnexpectedData, "");
            return;
        };
        let Some(peer) = self.peers.get(&from) else {
            return;
        };
        if peer.authenticated {
            // The client's Change Password dialogue (Luanti's
            // handleCommand_FirstSrp for a player already in the game).
            let name = peer.display_name().to_string();
            let account = Account { salt: msg.salt, verifier: msg.verifier };
            match self.auth.set(&name, &account) {
                Ok(()) => {
                    info!("{name} changed their password");
                    self.tell(from, "Password change successful.");
                }
                Err(e) => {
                    warn!("Saving {name}'s new password: {e}");
                    self.tell(from, "Password change failed or unavailable.");
                }
            }
            return;
        }
        if !peer.hello_sent || peer.allowed_auth_mechs & AUTH_MECHANISM_FIRST_SRP == 0 {
            warn!("Unexpected TOSERVER_FIRST_SRP from {from}");
            self.deny(from, AccessDenied::UnexpectedData, "");
            return;
        }
        let name = peer.display_name().to_string();
        let mechs = peer.allowed_auth_mechs;

        if msg.is_empty {
            warn!("{name} is registering with an empty password");
        }
        let account = Account {
            salt: msg.salt,
            verifier: msg.verifier,
        };
        // An account whose password was cleared takes the new one.
        let cleared = matches!(self.auth.load(&name), Ok(Some(a)) if a.verifier.is_empty());
        let created = if cleared { self.auth.set(&name, &account).map(|_| true) } else { self.auth.create(&name, &account) };
        match created {
            Ok(true) => {
                info!(
                    "Registered account {name} (salt {} bytes, verifier {} bytes)",
                    account.salt.len(),
                    account.verifier.len()
                );
                self.event(format!("New account: {name}"));
            }
            Ok(false) => {
                warn!("{name} is already registered");
                self.deny(from, AccessDenied::AlreadyConnected, "");
                return;
            }
            Err(e) => {
                warn!("Saving account {name}: {e}");
                self.deny(from, AccessDenied::ServerFail, "");
                return;
            }
        }

        self.accept_auth(from, mechs);
    }

    /// Login succeeded: drop any older connection with the same name (most
    /// likely a client that crashed and hasn't timed out yet), then let the
    /// client in.
    fn accept_auth(&mut self, from: SocketAddr, mechs: u32) {
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        peer.authenticated = true;
        peer.srp = None;
        let display = peer.display_name().to_string();
        // The first player to log in to a world with no administrator
        // becomes it (privs.rs).
        if self.privs.claim_admin(&display) {
            info!("{display} is now the administrator");
            self.event(format!("Admin: {display}"));
        }
        let privs = self.privs.privs(&display);
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        peer.privs = privs;
        let name = peer.display_name().to_ascii_lowercase();
        let stale: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(a, p)| **a != from && p.authenticated && p.display_name().to_ascii_lowercase() == name)
            .map(|(a, _)| *a)
            .collect();
        for addr in stale {
            // Tell the old client why (if it is still there), then forget it.
            self.deny(addr, AccessDenied::AlreadyConnected, "");
            if let Some(mut old) = self.peers.remove(&addr) {
                old.save(&self.players);
                old.save_inventory(&self.inventories);
                info!("{} logged in again; dropped the old connection from {addr}", old.display_name());
                if old.ready {
                    self.remove_player_object(old.id);
                }
            }
            self.close_unwatched_lids();
            self.refresh_players();
        }
        info!("Sending TOCLIENT_AUTH_ACCEPT to {name}");
        let accept = build_auth_accept(MAP_SEED, SEND_INTERVAL, mechs);
        self.send_command(from, 0, &accept);
    }

    fn on_srp_bytes_a(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some((bytes_a, based_on)) = parse_srp_bytes_a(payload) else {
            warn!("Malformed TOSERVER_SRP_BYTES_A from {from}");
            self.deny(from, AccessDenied::UnexpectedData, "");
            return;
        };
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if peer.authenticated {
            info!("{} asked to change password (not supported yet)", peer.display_name());
            return;
        }
        // based_on 0 would mean a legacy password hash; we only offer SRP.
        if !peer.hello_sent
            || peer.srp_started
            || peer.allowed_auth_mechs & AUTH_MECHANISM_SRP == 0
            || based_on != 1
        {
            warn!("Unexpected TOSERVER_SRP_BYTES_A from {from} (based_on {based_on})");
            self.deny(from, AccessDenied::UnexpectedData, "");
            return;
        }
        peer.srp_started = true;
        let name = peer.display_name().to_string();

        let account = match self.auth.load(&name) {
            Ok(Some(a)) => a,
            Ok(None) => {
                warn!("Account {name} disappeared during login");
                self.deny(from, AccessDenied::ServerFail, "");
                return;
            }
            Err(e) => {
                warn!("Reading account {name}: {e}");
                self.deny(from, AccessDenied::ServerFail, "");
                return;
            }
        };

        // The maths takes about a second on the ESP32, so it runs on its
        // own thread and the server carries on; tick() picks up the result.
        let peer_id = self.peers.get(&from).map_or(0, |p| p.id);
        let job = SrpJob { addr: from, peer_id, name, account, bytes_a };
        let queued = self.srp_jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok());
        if !queued {
            warn!("The SRP thread isn't running; can't log {from} in");
            self.deny(from, AccessDenied::ServerFail, "");
        }
    }

    /// Send the salt and B for logins whose SRP maths has finished.
    fn finish_srp(&mut self) {
        while let Ok(r) = self.srp_results.try_recv() {
            let Some(peer) = self.peers.get_mut(&r.addr).filter(|p| p.id == r.peer_id) else {
                continue; // gone while we were working
            };
            let name = peer.display_name().to_string();
            let Some(challenge) = r.challenge else {
                warn!("{name}: SRP safety check failed on A");
                self.deny(r.addr, AccessDenied::UnexpectedData, "");
                continue;
            };
            info!("{name}: SRP challenge computed in {} ms", r.took.as_millis());
            let reply = build_srp_bytes_s_b(&r.salt, &challenge.bytes_b);
            peer.srp = Some(challenge);
            self.send_command(r.addr, 0, &reply);
        }
    }

    fn on_srp_bytes_m(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(m) = parse_srp_bytes_m(payload) else {
            warn!("Malformed TOSERVER_SRP_BYTES_M from {from}");
            self.deny(from, AccessDenied::UnexpectedData, "");
            return;
        };
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        let Some(challenge) = peer.srp.take() else {
            warn!("Unexpected TOSERVER_SRP_BYTES_M from {from}");
            self.deny(from, AccessDenied::UnexpectedData, "");
            return;
        };
        let name = peer.display_name().to_string();
        let mechs = peer.allowed_auth_mechs;
        if challenge.check(&m) {
            info!("{name} logged in");
            self.accept_auth(from, mechs);
        } else {
            info!("{name} supplied the wrong password");
            self.event(format!("Wrong password: {name}"));
            self.deny(from, AccessDenied::WrongPassword, "");
        }
    }

    /// Something worth showing on the status screen.
    fn event(&self, text: String) {
        status::lock(&self.status).event(text);
    }

    fn edit_event(&self, name: &str, verb: &str, id: u16) {
        let node = content::name_of(id);
        let node = node.split_once(':').map_or(node, |(_, n)| n).replace('_', " ");
        let mut st = status::lock(&self.status);
        st.edits += 1;
        st.event(format!("{name} {verb} {node}"));
    }

    /// Update the screen's player list: players in the game in joining
    /// order, and how many connections are still logging in or loading.
    fn refresh_players(&self) {
        let mut ready: Vec<(u16, String)> = self
            .peers
            .values()
            .filter(|p| p.ready)
            .map(|p| (p.id, p.display_name().to_string()))
            .collect();
        ready.sort();
        let mut st = status::lock(&self.status);
        st.joining = self.peers.len() - ready.len();
        st.players = ready.into_iter().map(|(_, n)| n).collect();
    }

    /// Forget a connection: save the player, and take them out of the game.
    /// `how` goes in the log, `event` on the status screen ("left").
    fn remove_peer(&mut self, addr: SocketAddr, how: &str, event: &str) {
        let Some(mut peer) = self.peers.remove(&addr) else {
            return;
        };
        peer.save(&self.players);
        peer.save_inventory(&self.inventories);
        info!("Peer {} ({}) {how}", peer.id, peer.display_name());
        // Their boat is left empty (its on_detach_child).
        if let Some(b) = peer.in_boat.and_then(|id| self.boats.get_mut(id)) {
            b.driver = None;
            b.auto = false;
        }
        if peer.ready {
            self.remove_player_object(peer.id);
            self.event(format!("{} {event}", peer.display_name()));
            status::lock(&self.status).leaves += 1;
        }
        self.close_unwatched_lids();
        self.refresh_players();
    }

    fn deny(&mut self, addr: SocketAddr, reason: AccessDenied, custom: &str) {
        info!("Denying access to {addr}: {reason:?} {custom}");
        let pkt = build_access_denied(reason, custom, false);
        self.send_command(addr, 0, &pkt);
    }

    // -----------------------------------------------------------------------
    // Content and joining
    // -----------------------------------------------------------------------

    fn on_init2(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if !peer.authenticated {
            warn!("TOSERVER_INIT2 from unauthenticated peer {}", peer.id);
            return;
        }
        if peer.content_sent {
            return;
        }
        peer.content_sent = true;
        let lang = parse_init2_lang(payload).unwrap_or_default();
        info!(
            "TOSERVER_INIT2 from {} (language \"{lang}\"): sending content",
            peer.display_name()
        );

        // Same order as the Luanti server, all on channel 0.
        let itemdef = self.content.itemdef.clone();
        let nodedef = self.content.nodedef.clone();
        let announce = self.content.announce_media.clone();
        self.send_command(from, 0, &itemdef);
        self.send_command(from, 0, &nodedef);
        self.send_command(from, 0, &announce);
        self.send_command(from, 0, &build_movement());
        // The time now, so every player's day matches.
        let now = self.time_of_day();
        self.send_command(from, 0, &build_time_of_day(now, clock::TIME_SPEED));
        self.send_command(from, 0, &build_csm_restriction_flags());
    }

    fn on_request_media(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(names) = parse_request_media(payload) else {
            warn!("Malformed TOSERVER_REQUEST_MEDIA from {from}");
            return;
        };
        let mut files = Vec::new();
        for name in &names {
            match self.content.media.get(name) {
                Some(f) => files.push((f.name.as_str(), &f.data[..])),
                None => warn!("Client asked for unknown media \"{name}\""),
            }
        }
        info!("Client requested {} media files: {}", names.len(), names.join(", "));
        let bunches = build_media_bunches(&files);
        for bunch in bunches {
            self.send_command(from, 2, &bunch);
        }
    }

    fn on_client_ready(&mut self, from: SocketAddr, payload: &[u8]) {
        let ready = parse_client_ready(payload);
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if !peer.content_sent || peer.ready {
            return;
        }
        let name = peer.display_name().to_string();
        // Two could have been logging in at once for the last place.
        if self.user_limit_reached(&name) {
            info!("{name} tried to join, but the game is full ({} players)", self.max_players);
            self.deny(from, AccessDenied::TooManyUsers, "");
            return;
        }
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        peer.ready = true;
        match ready {
            Some(r) => info!(
                "{} is in the game (client {}, formspec v{})",
                peer.display_name(),
                r.version,
                r.formspec_version
            ),
            None => info!("{} is in the game", peer.display_name()),
        }

        // Where they left off, or the spawn point for a newcomer.
        let id = peer.id;
        let name = peer.display_name().to_string();
        let saved = self.players.load(&name);
        let (spawn, yaw, pitch, hp) = match &saved {
            Some(d) => (self.safe_position(d.pos), d.yaw, d.pitch, d.hp),
            None => (self.spawn_point(id), 0.0, 0.0, PLAYER_MAX_HP),
        };
        // A skin that has since gone falls back to the default.
        let skin = saved.as_ref().map_or(skins::DEFAULT_SKIN, |d| d.skin.as_str());
        let skin = self.skins.get(self.skins.index_of(skin)).key.clone();
        info!(
            "{name} {} at ({:.1}, {:.1}, {:.1}) with {hp} HP",
            if saved.is_some() { "returns" } else { "spawns" },
            spawn[0],
            spawn[1],
            spawn[2]
        );

        let spawn_bs = spawn.map(|v| v * 10.0);
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.pos_bs = spawn_bs;
            peer.yaw = yaw;
            peer.pitch = pitch;
            peer.hp = hp;
            peer.skin = skin;
            peer.home = saved.as_ref().and_then(|d| d.home);
            peer.saved = saved;
            peer.saved_at = Instant::now();
            peer.inventory = self.inventories.load(peer.display_name());
            peer.zoom_fov = binoculars::zoom_fov(&peer.inventory.main);
            peer.minimap = None;
            let items: u32 = peer.inventory.main.iter().flatten().map(|s| s.count as u32).sum();
            info!("{} has {items} items", peer.display_name());
        }
        let privs = self.peers.get(&from).map_or(Privs::NONE, |p| p.privs);
        self.send_command(from, 0, &build_privileges(&privs.names()));
        // Frame ranges for the client's own model (third-person view)
        self.send_command(from, 0, &build_local_player_animations(true));
        self.send_command(from, 0, &build_move_player(spawn_bs, pitch, yaw));
        self.send_inventory(from);
        self.send_inventory_formspec(from);
        self.add_player_object(from);
        // The minimap only with a mapping kit (map's on_joinplayer).
        self.update_minimap(from);
        // The weather's clouds at once (weather's on_joinplayer), and the
        // sounds around (env_sounds').
        self.set_weather(from);
        self.play_env_sounds(from);
        self.send_command(from, 0, &build_health_bar(hp));
        self.send_command(from, 0, &build_hp(hp, false));
        self.send_command(from, 0, &build_breath(PLAYER_MAX_BREATH));
        if hp == 0 {
            // Left while dead: back to the death screen, as Luanti does.
            self.send_command(from, 0, &build_deathscreen());
        }
        self.plan_blocks(from, spawn, MAX_VIEW_RADIUS);
        self.send_welcome(from);
        if let Some(peer) = self.peers.get(&from) {
            let name = peer.display_name().to_string();
            self.event(format!("{name} joined"));
            status::lock(&self.status).joins += 1;
        }
        self.refresh_players();
    }

    /// Tell a newly joined client about every player (itself included), and
    /// everyone else about the new player.
    fn add_player_object(&mut self, new: SocketAddr) {
        let Some(peer) = self.peers.get(&new) else {
            return;
        };
        let new_id = peer.id;
        let new_init = player_init_data(&PlayerInit {
            name: peer.display_name(),
            id: new_id,
            pos_bs: peer.pos_bs,
            yaw: peer.yaw,
            anim: peer.anim,
            skin: &self.skin_texture(&peer.skin),
            zoom_fov: peer.zoom_fov,
            posture: peer.posture(),
            attached: peer.in_boat.map(|b| (b, boats::SEAT)),
        });

        // The client's own object must be in its first object packet: from
        // then on it calls LocalPlayer::isDead(), which needs that object.
        // Then the boats, before the players who may sit in them.
        let mut for_new = vec![(new_id, new_init.clone())];
        for b in self.boats.iter() {
            for_new.push((b.id, boats::init_data(b)));
        }
        let mut others = Vec::new();
        for (addr, p) in &self.peers {
            if *addr != new && p.ready {
                for_new.push((p.id, player_init_data(&PlayerInit {
                    name: p.display_name(),
                    id: p.id,
                    pos_bs: p.pos_bs,
                    yaw: p.yaw,
                    anim: p.anim,
                    skin: &self.skin_texture(&p.skin),
                    zoom_fov: p.zoom_fov,
                    posture: p.posture(),
                    attached: p.in_boat.map(|b| (b, boats::SEAT)),
                })));
                others.push(*addr);
            }
        }
        for d in self.items.iter() {
            for_new.push((d.id, item_init(d)));
        }
        let mut hums = Vec::new();
        for m in self.mobs.iter() {
            for_new.push((m.id, mobs::init_data(m)));
            if let Some(&(hum, _)) = self.mob_sounds.get(&m.id) {
                hums.push(build_play_sound_object(hum, m.def().hum, mobs::HUM_GAIN, m.pos_bs(), m.id, true));
            }
        }
        info!(
            "Adding player object {new_id}; {} other player(s) online",
            others.len()
        );
        self.send_command(new, 0, &build_remove_add(&[], &for_new));
        for hum in hums {
            self.send_command(new, 0, &hum);
        }
        let announce = build_remove_add(&[], &[(new_id, new_init)]);
        for addr in others {
            self.send_command(addr, 0, &announce);
        }
    }

    /// Tell the remaining clients that a player has gone.
    fn remove_player_object(&mut self, id: u16) {
        let packet = build_remove_add(&[id], &[]);
        let ready: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, p)| p.ready)
            .map(|(a, _)| *a)
            .collect();
        for addr in ready {
            self.send_command(addr, 0, &packet);
        }
    }

    /// Tell everyone else where a player the server has just moved now is
    /// (teleported, respawned, in or out of bed). Their client's next
    /// position report matches what the server set, so it doesn't count
    /// as a move to relay, and the others would go on seeing them where
    /// they were.
    pub(super) fn announce_position(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get(&addr) else {
            return;
        };
        let packet = build_object_messages(&[(peer.id, update_position_message(peer.pos_bs, [0.0; 3], peer.yaw))]);
        let others: Vec<SocketAddr> = self.peers.iter().filter(|(a, p)| **a != addr && p.ready).map(|(a, _)| *a).collect();
        for other in others {
            self.send_command(other, 0, &packet);
        }
    }

    fn on_playerpos(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(pp) = parse_playerpos(payload) else {
            return;
        };
        let Some(peer) = self.peers.get(&from) else {
            return;
        };
        if !peer.ready {
            return;
        }
        let id = peer.id;
        let radius = (pp.wanted_range as i16).clamp(1, MAX_VIEW_RADIUS);
        let pos = pp.pos_nodes();
        let center = node_to_block(pos);
        if peer.view_center != Some(center) || peer.view_radius != radius {
            self.plan_blocks(from, pos, radius);
        }

        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        let moved = peer.pos_bs != pp.pos_bs || peer.speed_bs != pp.speed_bs || peer.yaw != pp.yaw;
        let active = moved || peer.pitch != pp.pitch || peer.keys != pp.keys;
        peer.keys = pp.keys;
        peer.pos_bs = pp.pos_bs;
        peer.speed_bs = pp.speed_bs;
        peer.yaw = pp.yaw;
        peer.pitch = pp.pitch;

        // Walk, mine or stand, as player_api does from the pressed keys.
        // Animation changes go reliably on channel 0: if one were lost the
        // player would be stuck walking (or frozen) until the next change.
        // Lying in bed or sitting in a boat, the animation stays
        // (player_api's player_attached).
        let anim = if peer.in_bed.is_some() || peer.in_boat.is_some() { peer.anim } else { animation_for(&pp) };
        let in_boat = peer.in_boat.is_some();
        if anim != peer.anim {
            peer.anim = anim;
            let packet = build_object_messages(&[(id, set_animation_message(anim))]);
            let others: Vec<SocketAddr> = self
                .peers
                .iter()
                .filter(|(a, p)| **a != from && p.ready)
                .map(|(a, _)| *a)
                .collect();
            for addr in others {
                self.send_command(addr, 0, &packet);
            }
        }
        if active {
            self.wake();
        }
        // In a boat, where they are is where the boat is: other clients
        // put them there themselves (Luanti doesn't take an attached
        // player's position either).
        if !moved || in_boat {
            return;
        }

        // Relay to the other players, unreliably on channel 1 like the
        // Luanti server: a lost update is soon replaced by the next.
        let msg = build_object_messages(&[(
            id,
            update_position_message(pp.pos_bs, pp.speed_bs, pp.yaw),
        )]);
        // Skipped while the radio is behind: the next update replaces it.
        if self.congested() {
            return;
        }
        let datagram = unreliable_datagram(1, &msg);
        let others: Vec<SocketAddr> = self.peers.iter().filter(|(a, p)| **a != from && p.ready).map(|(a, _)| *a).collect();
        for addr in others {
            self.send_raw(addr, &datagram);
        }
    }

    /// Digging and building. The client has already shown the change; we
    /// check it, apply it to the world, and tell everyone who has that map
    /// block. If we refuse, we send the real node back to undo the
    /// client's prediction.
    fn on_interact(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(msg) = parse_interact(payload) else {
            warn!("Malformed TOSERVER_INTERACT from {from}");
            hexdump(payload);
            return;
        };
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if !peer.ready {
            return;
        }
        if let Some(pp) = &msg.pos {
            peer.pos_bs = pp.pos_bs;
            peer.speed_bs = pp.speed_bs;
        }
        let name = peer.display_name().to_string();
        let body = [peer.pos_bs[0] / 10.0, peer.pos_bs[1] / 10.0 + 1.0, peer.pos_bs[2] / 10.0];
        let dead = peer.hp == 0;
        // Without "interact", nothing happens, and a dug or placed node is
        // put back as it was (Luanti's handleCommand_Interact).
        if !peer.privs.has("interact") {
            info!("{name} attempted to interact without 'interact' privilege");
            if let Pointed::Node { under, above } = msg.pointed {
                match msg.action {
                    INTERACT_DIGGING_COMPLETED => self.resend_node(from, under),
                    INTERACT_PLACE => self.resend_node(from, above),
                    _ => {}
                }
            }
            return;
        }
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.wield = msg.item as usize;
        }
        if msg.action == INTERACT_USE {
            if !dead {
                // Screwdriver, skeleton key, binoculars and mapping kit
                // (their on_use).
                let slot = msg.item as usize;
                let item = self.peers.get(&from).and_then(|p| p.inventory.get(slot)).filter(|_| slot < HOTBAR_SIZE).map(|s| s.id);
                let near = |p: [i16; 3]| (0..3).map(|i| (p[i] as f32 - body[i]).powi(2)).sum::<f32>().sqrt() <= MAX_INTERACT_DISTANCE;
                match (item, &msg.pointed) {
                    (Some(content::TOOL_SCREWDRIVER), Pointed::Node { under, .. }) => {
                        if near(*under) {
                            self.screwdrive(from, slot, *under, screwdriver::Mode::Face);
                        }
                        return;
                    }
                    (Some(content::TOOL_SCREWDRIVER), _) => return,
                    (Some(content::ITEM_SKELETON_KEY), Pointed::Node { under, .. }) => {
                        if near(*under) {
                            self.cut_key(from, slot, *under);
                        }
                        return;
                    }
                    (Some(content::ITEM_SKELETON_KEY), _) => return,
                    (Some(content::TOOL_FLINT_AND_STEEL), pointed) => {
                        let target = match pointed {
                            Pointed::Node { under, above } if near(*under) => Some((*under, *above)),
                            _ => None,
                        };
                        self.strike(from, slot, target, body);
                        return;
                    }
                    (Some(content::ITEM_BINOCULARS), _) => {
                        self.update_zoom(from);
                        return;
                    }
                    (Some(content::ITEM_MAPPING_KIT), _) => {
                        self.update_minimap(from);
                        return;
                    }
                    _ => {}
                }
                // An empty bucket scoops up the water it points at.
                let scooped = match msg.pointed {
                    Pointed::Node { under, above } => {
                        let at = BucketUse { slot: msg.item as usize, under, above, body };
                        // ... and a hoe turns dirt into soil.
                        self.scoop_water(from, at) || self.hoe(from, at)
                    }
                    _ => false,
                };
                if !scooped {
                    self.use_item(from, msg.item as usize);
                }
            }
            return;
        }
        if let Pointed::Object(id) = msg.pointed {
            if boats::is_boat_id(id) && !dead {
                match msg.action {
                    INTERACT_START_DIGGING => self.punch_boat(from, id, body),
                    INTERACT_PLACE => self.board_boat(from, id, body),
                    _ => {}
                }
                return;
            }
            if msg.action == INTERACT_START_DIGGING && !dead {
                if mobs::is_mob_id(id) {
                    self.punch_mob(from, id, msg.item as usize, body);
                } else {
                    self.pick_up(from, id, body);
                }
            }
            return;
        }
        let Pointed::Node { under, above } = msg.pointed else {
            return;
        };
        if dead && matches!(msg.action, INTERACT_DIGGING_COMPLETED | INTERACT_PLACE) {
            let target = if msg.action == INTERACT_PLACE { above } else { under };
            self.resend_node(from, target);
            return;
        }
        let in_reach = |p: [i16; 3]| {
            let d: f32 = (0..3).map(|i| (p[i] as f32 - body[i]).powi(2)).sum();
            d.sqrt() <= MAX_INTERACT_DISTANCE
        };
        let node_pos = |p: [i16; 3]| (p[0] as i32, p[1] as i32, p[2] as i32);

        match msg.action {
            INTERACT_DIGGING_COMPLETED => {
                let current = self.world.node(node_pos(under));
                if current == world::CONTENT_AIR || !in_reach(under) {
                    info!("{name}: refused dig at {under:?}");
                    self.resend_node(from, under);
                    return;
                }
                // What the wielded item can dig, falling back to the hand
                // (as Luanti's server checks the client's dig).
                let slot = msg.item as usize;
                let wielded = self.peers.get(&from).and_then(|p| p.inventory.get(slot)).filter(|_| slot < HOTBAR_SIZE);
                let caps = content::caps_of(wielded.map(|s| s.id));
                let wear = content::dig_params(current, caps, wielded.map_or(0, |s| s.wear))
                    .map(|(_, wear)| wear)
                    .or_else(|| content::dig_params(current, &content::HAND, 0).map(|_| 0));
                let Some(wear) = wear else {
                    info!("{name}: {} can't be dug with that", content::name_of(current));
                    self.resend_node(from, under);
                    return;
                };
                // minetest_game's can_dig: a furnace must be emptied first.
                let furnace_at = node_pos(under);
                if content::is_furnace(current) && self.furnaces.get(furnace_at).is_some_and(|f| !f.is_empty()) {
                    info!("{name}: the furnace at {under:?} isn't empty");
                    self.resend_node(from, under);
                    return;
                }
                // A chest must be emptied first, and isn't dug while open; a
                // locked one only by its owner, or with its key.
                if content::is_chest(current) || current == bones::BONES || content::shelf_of(current).is_some() {
                    self.fill_dungeon_chest(furnace_at);
                    let open = content::chest_pair(current).is_some_and(|(_, o)| o == current);
                    let key = self.wielded_key(from);
                    let record = self.chests.get(furnace_at);
                    if open || record.is_some_and(|c| !c.is_empty() || !c.allows_with(&name, key.as_deref())) {
                        info!("{name}: can't dig the chest at {under:?} (open, not empty, or not theirs)");
                        self.resend_node(from, under);
                        return;
                    }
                    self.close_chest_windows(furnace_at);
                    self.chests.remove(furnace_at);
                }
                // A steel door or trapdoor only by its owner.
                if doors::is_door(current) || doors::is_trapdoor(current) {
                    let key = self.wielded_key(from);
                    if !self.door_owners.allows_with(furnace_at, &name, key.as_deref()) {
                        info!("{name}: can't dig the locked door at {under:?}");
                        self.resend_node(from, under);
                        return;
                    }
                    self.door_owners.remove(furnace_at);
                }
                if doors::is_door(current) {
                    // Its hidden top goes with it (after_dig_node).
                    let top = [under[0], under[1] + 1, under[2]];
                    if self.world.node(node_pos(top)) == doors::HIDDEN {
                        self.world.set_node(node_pos(top), world::CONTENT_AIR, 0);
                        self.broadcast_node(top, &build_removenode(top));
                    }
                }
                // A bed isn't dug while someone lies in it; its other half
                // goes with it, and nobody respawns there any more.
                if let Some((_, top)) = beds::bed_of(current) {
                    let param2 = self.world.param2(furnace_at);
                    let other = beds::other_half(furnace_at, param2, top);
                    let paired = beds::is_bed(self.world.node(other)) && self.world.param2(other) == param2;
                    let bottom = if top { paired.then_some(other) } else { Some(furnace_at) };
                    if bottom.is_some_and(|b| self.bed_occupied(b)) {
                        info!("{name}: someone is lying in the bed at {under:?}");
                        self.resend_node(from, under);
                        return;
                    }
                    if paired {
                        let o16 = [other.0 as i16, other.1 as i16, other.2 as i16];
                        self.world.set_node(other, world::CONTENT_AIR, 0);
                        self.broadcast_node(o16, &build_removenode(o16));
                        self.bed_spawns.remove_at(other);
                    }
                    self.bed_spawns.remove_at(furnace_at);
                }
                if content::is_furnace(current) {
                    self.close_furnace(furnace_at);
                    if let Some(gone) = self.furnaces.remove(furnace_at) {
                        self.fade_sounds(&gone.sounds);
                    }
                }
                info!("{name} dug {} at {under:?}", content::name_of(current));
                // Others see bits of it fly (builtin's handle_node_drops).
                self.node_bits(node_pos(under), current, 0, Some(from));
                // Wear out the tool (builtin's node_dig); a worn-out tool
                // breaks with its sound.
                if let Some(mut tool) = wielded.filter(|_| wear > 0) {
                    let intact = tool.add_wear(wear);
                    if let Some(peer) = self.peers.get_mut(&from) {
                        peer.inventory.set(slot, intact.then_some(tool));
                    }
                    if !intact {
                        info!("{name}'s {} broke", content::name_of(tool.id));
                        let at = under.map(|v| v as f32 * 10.0);
                        self.play_sound_near(-1, content::TOOL_BREAKS, 0.5, at, HEARING);
                    }
                    self.inventory_changed(from);
                }
                self.edit_event(&name, "dug", current);
                // What stands on it, before it goes: a generated plant
                // vanishes from the world's view with its ground (see
                // World::unless_unsupported), but clients still have it.
                let above_pos = (under[0] as i32, under[1] as i32 + 1, under[2] as i32);
                let on_top = self.world.node(above_pos);
                // A picked apple that grew on a tree leaves a mark where a
                // new one grows (minetest_game's after_dig_node).
                if content::is_apple(current) && self.world.param2(node_pos(under)) == 0 {
                    self.world.set_node(node_pos(under), content::CONTENT_APPLE_MARK, 0);
                    self.broadcast_node(under, &build_addnode(under, content::CONTENT_APPLE_MARK, 0));
                    let after = nature::grow_delay(&mut self.rng);
                    self.timers.start(node_pos(under), Job::Regrow, after);
                } else if let Some(left) = plants::after_dig(current) {
                    // Kelp leaves its sand, a coral its skeleton, picked
                    // blueberries their leaves, which bear again later.
                    self.world.set_node(node_pos(under), left, 0);
                    self.broadcast_node(under, &build_addnode(under, left, 0));
                    if left == plants::BLUEBERRY_LEAVES {
                        let after = nature::grow_delay(&mut self.rng);
                        self.timers.start(node_pos(under), Job::Berries, after);
                    }
                } else {
                    self.world.set_node(node_pos(under), world::CONTENT_AIR, 0);
                    self.broadcast_node(under, &build_removenode(under));
                }
                // A coal block's permanent flame goes with it (fire's
                // after_destruct).
                self.put_out_above(node_pos(under), current);
                // Items resting on the dug node fall.
                self.wake_items(node_pos(under));
                // Panes beside it take their new shape.
                self.update_panes_near(node_pos(under));
                // Sand or gravel that stood on it comes loose.
                self.check_falling((under[0] as i32, under[1] as i32 + 1, under[2] as i32));
                // Torches hanging on the dug node, and saplings standing on
                // it, fall off and drop as items, as builtin's attached
                // nodes do.
                self.drop_attached(under);
                // So does a generated plant that stood on it.
                if farming::is_attached_plant(on_top) && self.world.node(above_pos) == world::CONTENT_AIR {
                    let a16 = [under[0], under[1] + 1, under[2]];
                    info!("{} at {a16:?} lost its support", content::name_of(on_top));
                    self.world.set_node(above_pos, world::CONTENT_AIR, 0);
                    self.broadcast_node(a16, &build_removenode(a16));
                    for (item, count) in self.drops(on_top) {
                        self.spawn_item(Stack::new(item, count), (a16.map(|v| v as f32), [0.0; 3]));
                    }
                }
                // Its leaves may now decay (minetest_game's leafdecay).
                self.start_leaf_decay(node_pos(under), current);
                // Papyrus above dug papyrus goes too, to the digger
                // (default.dig_up).
                let world = &self.world;
                let up = plants::dug_up(node_pos(under), current, &|p| world.node(p));
                for p in &up {
                    let p16 = [p.0 as i16, p.1 as i16, p.2 as i16];
                    self.world.set_node(*p, world::CONTENT_AIR, 0);
                    self.broadcast_node(p16, &build_removenode(p16));
                }
                let mut drops = self.drops(current);
                if !up.is_empty() {
                    drops.push((plants::PAPYRUS, up.len() as u16));
                }
                // The drop goes into the digger's inventory; what doesn't
                // fit is dropped where the node was (builtin's
                // handle_node_drops). Leaves sometimes give a sapling.
                if farming::is_soil(current) {
                    self.fields.remove(&node_pos(under));
                }
                for (item, count) in drops {
                    let left = match self.peers.get_mut(&from) {
                        Some(peer) => peer.inventory.add(Stack::new(item, count)),
                        None => count,
                    };
                    if left > 0 {
                        info!("{name}'s inventory is full; dropping {left} {}", content::name_of(item));
                        self.spawn_item(Stack::new(item, left), (under.map(|v| v as f32), [0.0; 3]));
                    }
                    self.inventory_changed(from);
                }
            }
            INTERACT_PLACE => {
                // Right-clicking a furnace opens it, unless sneaking: then
                // the wielded item is placed against it (builtin's
                // item_place). The client doesn't predict either.
                let sneaking = msg.pos.as_ref().is_some_and(|p| p.keys & KEY_SNEAK != 0);
                if content::is_furnace(self.world.node(node_pos(under))) && !sneaking {
                    if in_reach(under) {
                        self.open_furnace(from, node_pos(under));
                    }
                    return;
                }
                if content::is_chest(self.world.node(node_pos(under))) && !sneaking {
                    if in_reach(under) {
                        self.open_chest(from, node_pos(under));
                    }
                    return;
                }
                if content::shelf_of(self.world.node(node_pos(under))).is_some() && !sneaking {
                    if in_reach(under) {
                        self.open_shelf(from, node_pos(under));
                    }
                    return;
                }
                if self.world.node(node_pos(under)) == bones::BONES && !sneaking {
                    if in_reach(under) {
                        self.open_bones(from, node_pos(under));
                    }
                    return;
                }
                // Doors, trapdoors and gates open and close the same way.
                let target = self.world.node(node_pos(under));
                if (doors::is_door(target) || doors::is_trapdoor(target) || fences::is_gate(target)) && !sneaking {
                    if in_reach(under) {
                        self.toggle_door(from, node_pos(under));
                    }
                    return;
                }
                // A firefly in a bottle is let out (its on_rightclick).
                if target == insects::FIREFLY_BOTTLE && !sneaking {
                    if in_reach(under) {
                        self.release_firefly(node_pos(under));
                    }
                    return;
                }
                // A bed's bottom is slept in (its top isn't rightclickable).
                if beds::is_bed_bottom(target) && !sneaking {
                    if in_reach(under) {
                        self.use_bed(from, node_pos(under));
                    }
                    return;
                }
                // Where the player looks (the stairs and doors use it).
                let (pitch, yaw) = match (&msg.pos, self.peers.get(&from)) {
                    (Some(pp), _) => (pp.pitch, pp.yaw),
                    (None, Some(p)) => (p.pitch, p.yaw),
                    (None, None) => (0.0, 0.0),
                };
                // The client predicts placing the wielded item, if it is a
                // node. An empty slot predicts nothing.
                let slot = msg.item as usize;
                let wielded = self.peers.get(&from).and_then(|p| p.inventory.get(slot));
                // The screwdriver turns the node about its axis (its
                // on_place).
                if wielded.is_some_and(|s| slot < HOTBAR_SIZE && s.id == content::TOOL_SCREWDRIVER) {
                    if in_reach(under) {
                        self.screwdrive(from, slot, under, screwdriver::Mode::Axis);
                    }
                    return;
                }
                // A key, sneaking, on a steel door or trapdoor: opens or
                // shuts it if it fits (its on_key_use). Placed, it is
                // nothing.
                if wielded.is_some_and(|s| slot < HOTBAR_SIZE && content::group_of(s.id, "key") == 1) {
                    if doors::is_locked(target) && in_reach(under) {
                        self.toggle_door(from, node_pos(under));
                    }
                    return;
                }
                // A water bucket pours out a water source.
                if wielded.is_some_and(|s| slot < HOTBAR_SIZE && matches!(s.id, content::ITEM_BUCKET_WATER | content::ITEM_BUCKET_LAVA)) {
                    if in_reach(above) {
                        self.pour_water(from, BucketUse { slot, under, above, body });
                    }
                    return;
                }
                // A door item places a door (it isn't a node itself).
                if let Some(door) = wielded.filter(|_| slot < HOTBAR_SIZE).and_then(|s| doors::door_item(s.id)) {
                    let place = DoorPlace { under, above, look: look_dir(pitch, yaw), slot };
                    if in_reach(above) {
                        self.place_door(from, door, place);
                    }
                    return;
                }
                // A waterlily goes on still water (its on_place), and
                // nowhere else.
                if wielded.is_some_and(|s| slot < HOTBAR_SIZE && s.id == colours::WATERLILY) {
                    let free = content::buildable_to(self.world.node(node_pos(above))) && !content::is_liquid(self.world.node(node_pos(above)));
                    if self.world.node(node_pos(under)) == content::CONTENT_WATER && free && in_reach(above) {
                        let param2 = (self.rng.next() % 4) as u8;
                        let name = self.peers.get(&from).map(|p| p.display_name().to_string()).unwrap_or_default();
                        info!("{name} placed a waterlily at {above:?}");
                        if let Some(peer) = self.peers.get_mut(&from) {
                            peer.inventory.take_one(slot);
                        }
                        self.inventory_changed(from);
                        self.world.set_node(node_pos(above), colours::WATERLILY, param2);
                        self.broadcast_node(above, &build_addnode(above, colours::WATERLILY, param2));
                    }
                    return;
                }
                // Kelp goes into sand under water, a coral into coral
                // skeleton (their on_place): into the node pointed at.
                if let Some(id) = wielded.filter(|s| slot < HOTBAR_SIZE && plants::is_rooted(s.id)).map(|s| s.id) {
                    if in_reach(under) {
                        self.place_rooted(from, id, slot, under);
                    } else {
                        self.resend_node(from, under);
                    }
                    return;
                }
                // A boat goes on water (its on_place), and nowhere else.
                if wielded.is_some_and(|s| slot < HOTBAR_SIZE && s.id == content::ITEM_BOAT) {
                    if in_reach(under) {
                        self.place_boat(from, slot, under, yaw);
                    }
                    return;
                }
                // A bed places its two halves.
                if let Some(bed) = wielded.filter(|s| slot < HOTBAR_SIZE && beds::is_bed_bottom(s.id)).map(|s| s.id) {
                    let place = DoorPlace { under, above, look: look_dir(pitch, yaw), slot };
                    if in_reach(above) {
                        self.place_bed(from, bed, place);
                    }
                    return;
                }
                let Some(stack) = wielded.filter(|s| slot < HOTBAR_SIZE && content::is_node(s.id)) else {
                    // Nothing to place (an empty hand, or an item such as a
                    // stick that isn't a node). If the client thought
                    // otherwise, correct the node and the inventory.
                    self.resend_node(from, above);
                    self.send_inventory(from);
                    return;
                };
                let id = stack.id;
                // Air, water, or an apple mark (buildable_to: placing
                // replaces it).
                let target = self.world.node(node_pos(above));
                let free = content::buildable_to(target);
                let support = self.world.node(node_pos(under));
                // A slab on the open face of the same kind of slab makes a
                // full block where that slab was. The client predicted a
                // slab in the node beyond, so put that back too.
                if let Some(block) = stairs::merged(id, under, above, support, self.world.param2(node_pos(under))) {
                    if !in_reach(under) {
                        self.resend_node(from, above);
                        return;
                    }
                    info!("{name} made a double slab of {} at {under:?}", content::name_of(id));
                    self.edit_event(&name, "placed", block);
                    if let Some(peer) = self.peers.get_mut(&from) {
                        peer.inventory.take_one(slot);
                    }
                    self.inventory_changed(from);
                    self.world.set_node(node_pos(under), block, 0);
                    self.broadcast_node(under, &build_addnode(under, block, 0));
                    self.resend_node(from, above);
                    return;
                }
                // A torch hangs on something solid (the client checks the
                // same); anything else just needs something to build on.
                let torch = content::is_torch(id);
                // A sapling stands on the node below it, which must be solid
                // (builtin's check_attached_node, as the client checks).
                let below = (above[0] as i32, above[1] as i32 - 1, above[2] as i32);
                let supported = if torch || content::is_sign(id) {
                    content::walkable(support)
                } else if farming::is_seed(id) {
                    // A seed goes on soil, directly on top (farming's
                    // place_seed).
                    farming::is_soil(support) && above[1] == under[1] + 1
                } else if content::stands_on_floor(id) {
                    content::walkable(self.world.node(below))
                } else {
                    support != world::CONTENT_AIR
                };
                // A furnace faces the player who places it.
                // Torches and ladders face what they were placed against.
                let param2 = if content::is_wallmounted(id) {
                    content::wallmounted_param2(under, above)
                } else if content::is_furnace(id) || content::is_chest(id) || content::shelf_of(id).is_some() || doors::is_trapdoor(id) || fences::is_gate(id) || panes::is_flat(id) {
                    content::facedir_param2(above, body)
                } else if stairs::shape_of(id).is_some() {
                    // Stairs and slabs: the way the player looks decides
                    // which way up they go (the stairs mod's on_place).
                    let placer = stairs::Placer { feet: [body[0], body[1] - 1.0, body[2]], pitch, yaw };
                    stairs::place_param2(id, under, above, support, self.world.param2(node_pos(under)), &placer)
                } else if content::is_leaves(id) || content::is_apple(id) {
                    // Placed leaves never decay (minetest_game's
                    // after_place_leaves), and placed apples don't regrow.
                    nature::PLACED_LEAVES
                } else if farming::is_seed(id) {
                    1 // lying on the floor (wallmounted)
                } else if id == content::DRY_SHRUB {
                    content::DRY_SHRUB_PARAM2 // minetest_game's place_param2
                } else {
                    0
                };
                // The item is always the floor torch; on a wall or ceiling
                // it becomes that torch node (minetest_game's on_place).
                // Tall grass comes up any of its five heights.
                let node = if id == farming::GRASS[0] || id == farming::DRY_GRASS[0] {
                    farming::placed_grass(id, &mut self.rng)
                } else if plants::is_small_plant(id) {
                    plants::placed_size(id, self.rng.next())
                } else {
                    content::torch_variant(id, param2)
                };
                // Water would wash a torch straight out again (it is
                // floodable), so it stays in the hand.
                let flooded = content::floodable(id) && content::is_liquid(target);
                if !free || !supported || flooded || !in_reach(above) {
                    info!("{name}: refused placing at {above:?}");
                    self.resend_node(from, above);
                    return;
                }
                info!("{name} placed {} at {above:?}", content::name_of(id));
                self.edit_event(&name, "placed", id);
                if let Some(peer) = self.peers.get_mut(&from) {
                    peer.inventory.take_one(slot);
                }
                self.inventory_changed(from);
                self.world.set_node(node_pos(above), node, param2);
                // Sand or gravel put over nothing falls.
                self.check_falling(node_pos(above));
                self.broadcast_node(above, &build_addnode(above, node, param2));
                // A pane, or a pane beside it, takes its new shape.
                self.update_panes_near(node_pos(above));
                // A shelf starts empty (its on_construct).
                if let Some(kind) = content::shelf_of(node) {
                    self.chests.get_or_add(node_pos(above)).shelf = Some(kind);
                    self.chests.save();
                    self.update_shelf(node_pos(above));
                }
                // A sign gets its text box (minetest_game's on_construct).
                if content::is_sign(node) {
                    self.world.set_meta(node_pos(above), Some(meta::new_sign()));
                    self.broadcast_meta(node_pos(above));
                }
                // Built into lava: the client lit the block up as the lava
                // went; send it again with the server's light.
                if content::is_lava(target) {
                    self.send_block_again(world::block_of_node(node_pos(above)));
                }
                if content::is_sapling(node) {
                    let after = nature::grow_delay(&mut self.rng);
                    self.timers.start(node_pos(above), Job::Grow, after);
                }
                if plants::is_climber(node) {
                    self.climbers.insert(node_pos(above));
                }
                if farming::is_seed(node) {
                    let after = farming::grow_delay(&mut self.rng);
                    self.timers.start(node_pos(above), Job::Crop, after);
                }
                // A locked chest belongs to whoever placed it; so does a
                // steel trapdoor.
                if content::is_locked_chest(node) {
                    self.chests.get_or_add(node_pos(above)).owner = Some(name.clone());
                    self.chests.save();
                }
                if doors::is_locked(node) {
                    self.door_owners.set(node_pos(above), &name);
                }
            }
            // Punching TNT or gunpowder with a torch lights it (their
            // on_punch).
            INTERACT_START_DIGGING if tnt::lights(self.world.node(node_pos(under))) && in_reach(under) => {
                let slot = msg.item as usize;
                let torch = self.peers.get(&from).and_then(|p| p.inventory.get(slot)).filter(|_| slot < HOTBAR_SIZE).is_some_and(|s| s.id == content::CONTENT_TORCH);
                if torch && self.light_tnt(node_pos(under)) {
                    info!("{name} lit {} at {under:?} with a torch", content::name_of(self.world.node(node_pos(under))));
                }
            }
            // Punching bones takes what is in them (their on_punch).
            INTERACT_START_DIGGING if self.world.node(node_pos(under)) == bones::BONES && in_reach(under) => {
                self.punch_bones(from, node_pos(under));
            }
            // Starting and stopping digging need nothing else from us; the
            // mining animation follows the dig key in TOSERVER_PLAYERPOS.
            _ => {}
        }
    }

    /// Right-clicking a door, trapdoor or gate: open it if it's shut, shut
    /// it if it's open (doors.door_toggle, trapdoor_toggle, the gate's
    /// on_rightclick), with its sound. A steel one only for its owner.
    fn toggle_door(&mut self, from: SocketAddr, pos: NodePos) {
        let name = self.peers.get(&from).map(|p| p.display_name().to_string()).unwrap_or_default();
        let key = self.wielded_key(from);
        if !self.door_owners.allows_with(pos, &name, key.as_deref()) {
            let owner = self.door_owners.owner(pos).unwrap_or_default().to_string();
            self.tell(from, &format!("This door is locked; it belongs to {owner}."));
            return;
        }
        let (node, param2) = (self.world.node(pos), self.world.param2(pos));
        // A door changes node and turn; a trapdoor or gate only node.
        let toggled = match (doors::door_of(node), doors::trapdoor_of(node)) {
            (Some((door, _)), _) => doors::toggle(node, param2).map(|(to, p2, opened)| (to, p2, opened, door.open, door.close)),
            (_, Some((trap, _))) => doors::trapdoor_toggle(node).map(|(to, opened)| (to, param2, opened, trap.open, trap.close)),
            _ => fences::gate_toggle(node).map(|(to, opened)| (to, param2, opened, fences::GATE_OPEN, fences::GATE_CLOSE)),
        };
        let hearing = if fences::is_gate(node) { fences::GATE_HEARING } else { DOOR_HEARING };
        let Some((to, to_param2, opened, open_sound, close_sound)) = toggled else {
            return;
        };
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, to, to_param2);
        self.broadcast_node(p16, &build_addnode(p16, to, to_param2));
        let (sound, gain) = if opened { open_sound } else { close_sound };
        self.play_sound_near(-1, sound, gain, p16.map(|v| v as f32 * 10.0), hearing);
    }

    /// Placing a door (the door item's on_place): in the node pointed at
    /// (or into an apple mark), with room above for its top, facing the
    /// way the player looks, and hinged to pair with a door beside it.
    fn place_door(&mut self, from: SocketAddr, door: &'static Door, at: DoorPlace) {
        let name = self.peers.get(&from).map(|p| p.display_name().to_string()).unwrap_or_default();
        let node_pos = |p: [i16; 3]| (p[0] as i32, p[1] as i32, p[2] as i32);
        let free = content::buildable_to;
        let pos = if free(self.world.node(node_pos(at.under))) { at.under } else { at.above };
        let top = [pos[0], pos[1] + 1, pos[2]];
        if !free(self.world.node(node_pos(pos))) || !free(self.world.node(node_pos(top))) {
            info!("{name}: no room for a door at {pos:?}");
            return;
        }
        let dir = doors::facedir_of(at.look);
        // minetest_game looks for anything in group "door": trapdoors too.
        let next_to_door = content::group_of(self.world.node(doors::beside(node_pos(pos), dir)), "door") > 0;
        let (node, param2, top_param2) = doors::placed(door, dir, next_to_door);
        info!("{name} placed {} at {pos:?}", content::name_of(door.item));
        self.edit_event(&name, "placed", door.item);
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.inventory.take_one(at.slot);
        }
        self.inventory_changed(from);
        self.world.set_node(node_pos(pos), node, param2);
        self.broadcast_node(pos, &build_addnode(pos, node, param2));
        self.world.set_node(node_pos(top), doors::HIDDEN, top_param2);
        self.broadcast_node(top, &build_addnode(top, doors::HIDDEN, top_param2));
        if door.locked {
            self.door_owners.set(node_pos(pos), &name);
        }
        let (sound, gain) = content::place_sound(node);
        self.play_sound_near(-1, sound, gain, pos.map(|v| v as f32 * 10.0), HEARING);
    }

    /// Placing a bed (beds' on_place): its bottom in the node pointed at (or
    /// one it can build into), its top beyond that the way the player
    /// looks, both turned that way. Nothing is checked underneath: beds
    /// can be put in the air, as in minetest_game.
    fn place_bed(&mut self, from: SocketAddr, bed: u16, at: DoorPlace) {
        let name = self.peers.get(&from).map(|p| p.display_name().to_string()).unwrap_or_default();
        let node_pos = |p: [i16; 3]| (p[0] as i32, p[1] as i32, p[2] as i32);
        let free = content::buildable_to;
        let pos = if free(self.world.node(node_pos(at.under))) { at.under } else { at.above };
        let dir = doors::facedir_of(at.look);
        let top = beds::other_half(node_pos(pos), dir, false);
        if !free(self.world.node(node_pos(pos))) || !free(self.world.node(top)) {
            info!("{name}: no room for a bed at {pos:?}");
            return;
        }
        let Some((kind, _)) = beds::bed_of(bed) else {
            return;
        };
        let top16 = [top.0 as i16, top.1 as i16, top.2 as i16];
        info!("{name} placed {} at {pos:?}", content::name_of(bed));
        self.edit_event(&name, "placed", bed);
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.inventory.take_one(at.slot);
        }
        self.inventory_changed(from);
        self.world.set_node(node_pos(pos), kind.bottom, dir);
        self.broadcast_node(pos, &build_addnode(pos, kind.bottom, dir));
        self.world.set_node(top, kind.top, dir);
        self.broadcast_node(top16, &build_addnode(top16, kind.top, dir));
        let (sound, gain) = content::place_sound(bed);
        self.play_sound_near(-1, sound, gain, pos.map(|v| v as f32 * 10.0), HEARING);
    }

    /// Whether someone lies in the bed whose bottom is at `bed`.
    fn bed_occupied(&self, bed: NodePos) -> bool {
        self.peers.values().any(|p| p.in_bed.is_some_and(|b| b.bed == bed))
    }

    /// Right-clicking a bed (beds.on_rightclick). By day it only becomes
    /// where the player respawns; at night they lie down in it, or get up
    /// if they already lie there, and everyone in bed sees how many are.
    fn use_bed(&mut self, from: SocketAddr, bed: NodePos) {
        let Some(peer) = self.peers.get(&from) else {
            return;
        };
        let (name, lying) = (peer.display_name().to_string(), peer.in_bed.is_some());
        if !beds::is_night(self.time_of_day()) {
            if lying {
                self.get_up(from, true);
            }
            if self.bed_spawns.get(&name) != Some(bed) {
                self.bed_spawns.set(&name, bed);
                self.tell(from, "Spawn point set.");
            } else {
                self.tell(from, "You can only sleep at night.");
            }
            return;
        }
        if lying {
            self.get_up(from, true);
        } else {
            self.lie_down(from, bed);
            // As minetest_game, the respawn point is set even if the
            // player couldn't lie down.
            self.bed_spawns.set(&name, bed);
        }
        self.update_bed_forms(false);
    }

    /// beds' lay_down: unless the bed is taken or the player is moving,
    /// put them on it, looking towards its foot, unable to move, lying
    /// down, without the item in their hand.
    fn lie_down(&mut self, from: SocketAddr, bed: NodePos) {
        if self.bed_occupied(bed) {
            self.tell(from, "This bed is already occupied!");
            return;
        }
        let param2 = self.world.param2(bed);
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        let speed = peer.speed_bs.iter().map(|v| (v / 10.0).powi(2)).sum::<f32>().sqrt();
        if speed > 0.05 {
            self.tell(from, "You have to stop moving before going to bed!");
            return;
        }
        let (pos, yaw) = beds::lying_position(bed, param2);
        let pos_bs = pos.map(|v| v * 10.0);
        peer.in_bed = Some(InBed { bed, from_bs: peer.pos_bs });
        peer.pos_bs = pos_bs;
        peer.yaw = yaw;
        peer.anim = Animation::new(ANIM_LAY, 0.0);
        let (id, name, pitch, skin, zoom) = (peer.id, peer.display_name().to_string(), peer.pitch, peer.skin.clone(), peer.zoom_fov);
        info!("{name} lies down in the bed at {bed:?}");
        let texture = self.skin_texture(&skin);
        self.send_command(from, 0, &build_object_messages(&[(id, physics_override_message(true))]));
        self.send_command(from, 0, &build_move_player(pos_bs, pitch, yaw));
        self.announce_position(from);
        self.send_command(from, 0, &build_local_player_animations(false));
        self.send_command(from, 0, &build_hud_set_flags(0, HUD_FLAG_WIELDITEM_VISIBLE));
        let lay = build_object_messages(&[
            (id, set_animation_message(Animation::new(ANIM_LAY, 0.0))),
            (id, player_properties_message(&texture, Posture::Lie, zoom)),
        ]);
        self.send_to_players(&lay);
    }

    /// beds' lay_down to stand up: back where they were (`move_back`; not
    /// when they died there), free to move, standing, with their item in
    /// hand, looking a random way. Returns whether they were in bed.
    fn get_up(&mut self, from: SocketAddr, move_back: bool) -> bool {
        let yaw = (self.rng.range(1, 180) as f32 / 100.0).to_degrees();
        let Some(peer) = self.peers.get_mut(&from) else {
            return false;
        };
        let Some(was) = peer.in_bed.take() else {
            return false;
        };
        let stand = Animation::new(ANIM_STAND, ANIM_FPS);
        peer.anim = stand;
        if move_back {
            peer.pos_bs = was.from_bs;
            peer.yaw = yaw;
        }
        let (id, name, pitch, skin, zoom) = (peer.id, peer.display_name().to_string(), peer.pitch, peer.skin.clone(), peer.zoom_fov);
        info!("{name} gets up from the bed at {:?}", was.bed);
        let texture = self.skin_texture(&skin);
        if move_back {
            self.send_command(from, 0, &build_move_player(was.from_bs, pitch, yaw));
            self.announce_position(from);
        }
        self.send_command(from, 0, &build_object_messages(&[(id, physics_override_message(false))]));
        self.send_command(from, 0, &build_local_player_animations(true));
        self.send_command(from, 0, &build_hud_set_flags(HUD_FLAG_WIELDITEM_VISIBLE, HUD_FLAG_WIELDITEM_VISIBLE));
        let up = build_object_messages(&[(id, set_animation_message(stand)), (id, player_properties_message(&texture, Posture::Stand, zoom))]);
        self.send_to_players(&up);
        true
    }

    /// beds' update_formspecs: everyone in bed sees how many of the
    /// players are, with "Force night skip" once more than half are; or
    /// "Good morning." when the night is over.
    fn update_bed_forms(&mut self, finished: bool) {
        let online = self.peers.values().filter(|p| p.ready).count();
        let sleepers: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.in_bed.is_some()).map(|(a, _)| *a).collect();
        let form = if finished {
            beds::formspec("Good morning.", 2.7, false)
        } else {
            let majority = online as f32 / 2.0 < sleepers.len() as f32;
            beds::formspec(&format!("{} of {online} players are in bed", sleepers.len()), 2.2, majority)
        };
        let packet = build_show_formspec(&form, beds::FORM);
        for addr in sleepers {
            self.send_command(addr, 0, &packet);
        }
    }

    /// The bed window's buttons: leaving the bed (or closing the window),
    /// and forcing the night to end if more than half are in bed.
    fn on_bed_fields(&mut self, from: SocketAddr, fields: &[(String, String)]) {
        let pressed = |name: &str| fields.iter().any(|(f, _)| f == name);
        let in_bed = self.peers.values().filter(|p| p.in_bed.is_some()).count();
        if pressed("quit") || pressed("leave") {
            self.get_up(from, true);
            self.update_bed_forms(false);
        }
        if pressed("force") {
            let online = self.peers.values().filter(|p| p.ready).count();
            if (online as f32 / 2.0) < in_bed as f32 {
                self.update_bed_forms(true);
                self.skip_night();
            } else {
                self.update_bed_forms(false);
            }
        }
    }

    /// The night is over (beds.skip_night and kick_players): morning for
    /// everyone, and everyone gets up.
    fn skip_night(&mut self) {
        info!("Everyone slept: the night is skipped");
        self.clock.set_time_of_day(beds::MORNING);
        self.send_to_players(&build_time_of_day(beds::MORNING, clock::TIME_SPEED));
        let sleepers: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.in_bed.is_some()).map(|(a, _)| *a).collect();
        for addr in sleepers {
            self.get_up(addr, true);
        }
    }

    /// When everyone online is in bed, the night is skipped two seconds
    /// later (beds' check_in_beds and schedule_update; also when someone
    /// still up leaves the game).
    fn tend_beds(&mut self) {
        match self.night_skip_at {
            Some(at) if Instant::now() >= at => {
                self.night_skip_at = None;
                self.update_bed_forms(true);
                self.skip_night();
            }
            Some(_) => {}
            None => {
                let mut ready = self.peers.values().filter(|p| p.ready).peekable();
                if ready.peek().is_some() && ready.all(|p| p.in_bed.is_some()) {
                    self.night_skip_at = Some(Instant::now() + Duration::from_secs(2));
                }
            }
        }
    }

    /// Let liquids flow a step (liquids.rs), and show everyone what
    /// changed. A torch the water reaches is washed out and dropped, with
    /// a hiss (minetest_game's torch on_flood).
    fn flow_liquids(&mut self) {
        let changes = self.world.flow_liquids();
        // Every 10 s while liquid is moving, say how much: water that
        // never settles would show here.
        if self.liquid_changes > 0 && self.liquids_reported.elapsed() >= Duration::from_secs(10) {
            info!(
                "Liquids: {} nodes changed in the last {}s ({} blocks sent again), {} places waiting",
                self.liquid_changes,
                self.liquids_reported.elapsed().as_secs(),
                self.liquid_blocks_resent,
                self.world.liquid_queue_len()
            );
            self.liquid_changes = 0;
            self.liquid_blocks_resent = 0;
            self.liquids_reported = Instant::now();
        }
        if changes.is_empty() {
            return;
        }
        if self.liquid_changes == 0 {
            self.liquids_reported = Instant::now();
        }
        self.liquid_changes += changes.len() as u32;
        self.note("liquids flowed", changes.len() as u16);
        debug!("Liquids: {} changed, {} queued", changes.len(), self.world.liquid_queue_len());
        // A few changes in a block go out node by node; a block where
        // more changed is sent again whole, in turn with the other blocks
        // players need, as Luanti does with every liquid change. One
        // packet per node would swamp the radio when water pours into a
        // cave.
        // Lava always goes as whole blocks: a single lava node would make
        // the client light up what is round it, which the server doesn't
        // (lava only glows itself; see content::glow).
        let mut per_block: HashMap<BlockPos, usize> = HashMap::new();
        for c in &changes {
            let extra = if content::is_lava(c.id) || content::is_lava(c.was) { NODE_UPDATES_PER_BLOCK + 1 } else { 1 };
            *per_block.entry(world::block_of_node(c.pos)).or_default() += extra;
        }
        for (&block, &n) in &per_block {
            if n > NODE_UPDATES_PER_BLOCK {
                self.send_block_again(block);
                self.liquid_blocks_resent += 1;
            }
        }
        for c in changes {
            let p16 = [c.pos.0 as i16, c.pos.1 as i16, c.pos.2 as i16];
            if per_block[&world::block_of_node(c.pos)] <= NODE_UPDATES_PER_BLOCK {
                if c.id == world::CONTENT_AIR {
                    self.broadcast_node(p16, &build_removenode(p16));
                } else {
                    self.broadcast_node(p16, &build_addnode(p16, c.id, c.param2));
                }
            }
            if content::is_torch(c.was) {
                let at = p16.map(|v| v as f32);
                self.spawn_item(Stack::new(content::CONTENT_TORCH, 1), (at, [0.0; 3]));
                // The hiss of a torch going out, unless lava took it.
                if !content::is_lava(c.id) {
                    self.play_sound_near(-1, "default_cool_lava", 0.07, at.map(|v| v * 10.0), 16.0);
                }
            }
            // A flame washed out goes with a hiss (on_flood), unless lava
            // took it.
            if fire::is_flame(c.was) {
                self.flames.remove(&c.pos);
                if !content::is_lava(c.id) {
                    self.play_sound_near(-1, "fire_extinguish_flame", 0.15, p16.map(|v| v as f32 * 10.0), 16.0);
                }
            }
            // Items lying where it went may now float off or fall.
            self.wake_items(c.pos);
        }
    }

    /// Report a player link going quiet, before the 30 s timeout drops it:
    /// nothing arriving from them, or nothing of ours acknowledged (our
    /// packets not reaching them, or their ACKs not reaching us). With the
    /// board's memory and Wi-Fi state, so that a disconnect shows whose
    /// side failed.
    fn watch_links(&mut self) {
        let mut reports = Vec::new();
        for peer in self.peers.values_mut().filter(|p| p.ready) {
            let name = peer.display_name().to_string();
            let heard = peer.last_seen.elapsed();
            if heard >= LINK_QUIET && !peer.quiet_reported {
                peer.quiet_reported = true;
                reports.push(format!("nothing from {name} for {:.1} s", heard.as_secs_f32()));
            }
            // The longest an unacknowledged packet of ours has been out.
            let waiting: usize = peer.channels.iter().map(|c| c.unacked.len()).sum();
            let oldest = peer.channels.iter().flat_map(|c| c.unacked.iter()).filter_map(|u| u.first_sent).min();
            let resent = peer.channels.iter().flat_map(|c| c.unacked.iter()).map(|u| u.resends).max().unwrap_or(0);
            match oldest {
                Some(t) if t.elapsed() >= LINK_QUIET => {
                    if !peer.unacked_reported {
                        peer.unacked_reported = true;
                        reports.push(format!(
                            "no ACKs from {name} for {:.1} s ({waiting} packets waiting, resent up to {resent} times)",
                            t.elapsed().as_secs_f32()
                        ));
                    }
                }
                _ => {
                    if peer.unacked_reported {
                        peer.unacked_reported = false;
                        reports.push(format!("ACKs from {name} again"));
                    }
                }
            }
        }
        if reports.is_empty() {
            return;
        }
        let st = status::lock(&self.status);
        let counts = format!("{} packets in, {} out, {} send failures since boot", st.packets_in, st.packets_out, st.send_failures);
        drop(st);
        for r in reports {
            warn!("Link: {r}; {}; {counts}", link_state());
        }
    }

    /// Breath, as PlayerSAO::step: with their eyes in water (or lava) a
    /// player loses a breath every 2 s, and with none left 1 HP every 2 s;
    /// in the air they get a breath back every half second. The breath
    /// bar shows while breath isn't full (builtin's hud.lua).
    fn step_breath(&mut self, now: Instant) {
        let drown = now.duration_since(self.drowned_at) >= DROWN_INTERVAL;
        let breathe = now.duration_since(self.breathed_at) >= BREATHE_INTERVAL;
        if drown {
            self.drowned_at = now;
        }
        if breathe {
            self.breathed_at = now;
        }
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        let mut hurt = Vec::new();
        for (addr, p) in self.peers.iter_mut() {
            if !p.ready || p.hp == 0 {
                continue;
            }
            // The eyes: player_api's eye height, lower lying in bed.
            let eye = if p.in_bed.is_some() { 0.3 } else { 1.47 };
            let at = (
                (p.pos_bs[0] / 10.0).round() as i32,
                (p.pos_bs[1] / 10.0 + eye).round() as i32,
                (p.pos_bs[2] / 10.0).round() as i32,
            );
            let drowning = content::drowning(self.world.node(at));
            let before = p.breath;
            if drown && drowning > 0 {
                p.breath = p.breath.saturating_sub(1);
                if p.breath == 0 {
                    hurt.push((*addr, drowning));
                }
            } else if breathe && drowning == 0 && p.breath < PLAYER_MAX_BREATH {
                p.breath += 1;
            }
            if p.breath != before {
                changed.push(*addr);
            }
            // A second after breath is full again, the bar goes.
            if p.breath_bar && p.breath_full_since.is_some_and(|t| t.elapsed() >= Duration::from_secs(1)) {
                p.breath_bar = false;
                p.breath_full_since = None;
                removed.push(*addr);
            }
        }
        for addr in changed {
            self.send_breath(addr);
        }
        for addr in removed {
            self.send_command(addr, 0, &build_hud_remove(HUD_BREATH));
        }
        for (addr, damage) in hurt {
            self.hurt(addr, damage, "drowned");
        }
    }

    /// Tell a player their breath, and show, update or remove the bar.
    fn send_breath(&mut self, addr: SocketAddr) {
        let Some(p) = self.peers.get_mut(&addr) else {
            return;
        };
        let breath = p.breath;
        let mut packets = vec![build_breath(breath)];
        if breath < PLAYER_MAX_BREATH {
            p.breath_full_since = None;
            packets.push(if p.breath_bar { build_breath_bar_change(breath) } else { build_breath_bar(breath) });
            p.breath_bar = true;
        } else if p.breath_bar && p.breath_full_since.is_none() {
            // Full again: show it full, and remove it a second later
            // (step_breath).
            p.breath_full_since = Some(Instant::now());
            packets.push(build_breath_bar_change(breath));
        }
        for packet in packets {
            self.send_command(addr, 0, &packet);
        }
    }

    /// minetest_game's moss growth: cobblestone beside water turns mossy,
    /// one time in MOSS_CHANCE each time it is looked at, keeping its turn.
    fn grow_moss(&mut self) {
        let chance = nature::moss_chance();
        for pos in self.world.moss_beside_water() {
            if !self.rng.one_in(chance) {
                continue;
            }
            let (was, param2) = (self.world.node(pos), self.world.param2(pos));
            let Some(mossy) = nature::mossy_of(was) else {
                continue;
            };
            self.world.set_node(pos, mossy, param2);
            let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
            self.broadcast_node(p16, &build_addnode(p16, mossy, param2));
        }
    }

    /// minetest_game's "Grass spread" (`spread`) or "Grass covered" ABM,
    /// on the dirt and grass noted near players (within 4 map blocks,
    /// Luanti's active_block_range): grass spreads onto dirt in daylight
    /// under the open sky (light 13 or more, at the node above), and
    /// covered grass goes back to dirt, each one time in 50.
    fn grow_grass(&mut self, spread: bool) {
        let players: Vec<NodePos> = self
            .peers
            .values()
            .filter(|p| p.ready)
            .map(|p| ((p.pos_bs[0] / 10.0).round() as i32, (p.pos_bs[1] / 10.0).round() as i32, (p.pos_bs[2] / 10.0).round() as i32))
            .collect();
        if players.is_empty() {
            return;
        }
        let active = |p: NodePos| {
            let b = world::block_of_node(p);
            players.iter().any(|&q| {
                let c = world::block_of_node(q);
                (b.0 - c.0).abs() <= 4 && (b.1 - c.1).abs() <= 4 && (b.2 - c.2).abs() <= 4
            })
        };
        let daytime = (6000..18000).contains(&self.time_of_day());
        let chance = nature::grass_chance();
        for (pos, turf) in self.world.turf() {
            let to = match turf {
                world::Turf::Spread(to) if spread => to,
                world::Turf::Covered(to) if !spread => to,
                _ => continue,
            };
            if !active(pos) || !self.rng.one_in(chance) {
                continue;
            }
            if spread && !(daytime && self.world.highest_solid(pos.0, pos.2) < pos.1 + 1) {
                continue;
            }
            self.world.set_node(pos, to, 0);
            let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
            self.broadcast_node(p16, &build_addnode(p16, to, 0));
        }
    }

    // -------------------------------------------------------------------
    // Fire (fire.rs)
    // -------------------------------------------------------------------

    /// Light a flame (basic or permanent) at `pos`, `step` from where its
    /// fire started, if there is air there and, for a basic flame, fewer
    /// than fire::MAX_FLAMES burning. Returns whether it did.
    fn light_flame(&mut self, pos: NodePos, flame: u16, step: u8) -> bool {
        if self.world.node(pos) != world::CONTENT_AIR {
            return false;
        }
        if flame == fire::BASIC_FLAME {
            let world = &self.world;
            self.flames.retain(|&p| fire::is_flame(world.node(p)));
            let basic = self.flames.iter().filter(|&&p| world.node(p) == fire::BASIC_FLAME).count();
            if basic >= fire::MAX_FLAMES {
                debug!("Fire: {basic} flames burning, the most there may be; none at {pos:?}");
                return false;
            }
        }
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, flame, step);
        self.broadcast_node(p16, &build_addnode(p16, flame, step));
        self.flames.insert(pos);
        if flame == fire::BASIC_FLAME {
            self.timers.start(pos, Job::Flame, fire::flame_timer(&mut self.rng));
        }
        true
    }

    /// Put a flame out: it simply goes.
    fn put_out(&mut self, pos: NodePos) {
        self.flames.remove(&pos);
        if !fire::is_flame(self.world.node(pos)) {
            return;
        }
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, world::CONTENT_AIR, 0);
        self.broadcast_node(p16, &build_removenode(p16));
    }

    /// A coal block gone from `pos`: the permanent flame on it goes too.
    fn put_out_above(&mut self, pos: NodePos, was: u16) {
        let above = (pos.0, pos.1 + 1, pos.2);
        if was == content::CONTENT_COAL_BLOCK && self.world.node(above) == fire::PERMANENT_FLAME {
            self.put_out(above);
        }
    }

    /// A basic flame's timer: with nothing flammable beside it, it goes
    /// out; otherwise it looks again later. While fire is off it goes out
    /// regardless (minetest_game's would burn on beside leaves or wood for
    /// ever, never spreading).
    fn check_flame(&mut self, pos: NodePos) {
        if self.world.node(pos) != fire::BASIC_FLAME {
            self.flames.remove(&pos);
            return;
        }
        if self.settings.fire() && self.flammable_near(pos).is_some() {
            self.timers.start(pos, Job::Flame, fire::flame_timer(&mut self.rng));
        } else {
            self.put_out(pos);
        }
    }

    /// The first flammable node beside `pos` (find_node_near, radius 1).
    fn flammable_near(&self, pos: NodePos) -> Option<NodePos> {
        nature::AROUND
            .iter()
            .map(|d| (pos.0 + d.0, pos.1 + d.1, pos.2 + d.2))
            .find(|&p| fire::flammable(self.world.node(p)) > 0)
    }

    /// Whether a node is in a block near a player (within 4 map blocks:
    /// Luanti's active_block_range), where ABMs run.
    fn active_test(&self) -> impl Fn(NodePos) -> bool {
        let players: Vec<BlockPos> = self
            .peers
            .values()
            .filter(|p| p.ready)
            .map(|p| world::block_of_node(((p.pos_bs[0] / 10.0).round() as i32, (p.pos_bs[1] / 10.0).round() as i32, (p.pos_bs[2] / 10.0).round() as i32)))
            .collect();
        move |pos| {
            let b = world::block_of_node(pos);
            players.iter().any(|c| (b.0 - c.0).abs() <= 4 && (b.1 - c.1).abs() <= 4 && (b.2 - c.2).abs() <= 4)
        }
    }

    /// The "Ignite flame" ABM: a flammable node beside a flame or lava
    /// catches fire, one time in `chance`, in the air beside it; the new
    /// flame is a step further from where the fire began than the flame
    /// that lit it (lava starts afresh), and at MAX_STEPS it lights no
    /// more. Not while fire is off.
    fn ignite(&mut self, chance: u32) {
        if !self.settings.fire() {
            return;
        }
        let active = self.active_test();
        let world = &self.world;
        let mut igniters: Vec<NodePos> = self.flames.iter().copied().filter(|&p| active(p)).collect();
        igniters.extend(world.changed_nodes(content::is_lava).into_iter().filter(|&p| active(p)));
        // Each flammable node beside an igniter, with the igniter nearest
        // the start of its fire. A new flame is as many steps further on
        // as it is nodes from the igniter (the air it takes may be two
        // nodes off), so it is never further than its step from where the
        // fire began; lava starts a fire afresh.
        let mut burning: std::collections::BTreeMap<NodePos, (NodePos, Option<u8>)> = std::collections::BTreeMap::new();
        for q in igniters {
            let id = world.node(q);
            if fire::igniter(id) == 0 {
                continue;
            }
            let base = (!content::is_lava(id)).then(|| world.param2(q));
            for d in nature::AROUND {
                let p = (q.0 + d.0, q.1 + d.1, q.2 + d.2);
                if fire::flammable(world.node(p)) > 0 {
                    let e = burning.entry(p).or_insert((q, base));
                    if base.map_or(0, |b| b as u16 + 1) < e.1.map_or(0, |b| b as u16 + 1) {
                        *e = (q, base);
                    }
                }
            }
        }
        for (p, (q, base)) in burning {
            if base.is_some_and(|b| b >= fire::MAX_STEPS) || !self.rng.one_in(chance) {
                continue;
            }
            let air = nature::AROUND
                .iter()
                .map(|d| (p.0 + d.0, p.1 + d.1, p.2 + d.2))
                .find(|&a| self.world.node(a) == world::CONTENT_AIR);
            let Some(a) = air else {
                continue;
            };
            let step = match base {
                None => 0,
                Some(b) => b as u16 + fire::distance(q, a) as u16,
            };
            if step <= fire::MAX_STEPS as u16 {
                self.light_flame(a, fire::BASIC_FLAME, step as u8);
            }
        }
    }

    /// The "Remove flammable nodes" ABM: a basic flame burns away a
    /// flammable node beside it, one time in `chance`. Not while fire is
    /// off.
    fn burn_flammables(&mut self, chance: u32) {
        if !self.settings.fire() {
            return;
        }
        let active = self.active_test();
        let flames: Vec<NodePos> =
            self.flames.iter().copied().filter(|&p| active(p) && self.world.node(p) == fire::BASIC_FLAME).collect();
        for pos in flames {
            if !self.rng.one_in(chance) {
                continue;
            }
            if let Some(p) = self.flammable_near(pos) {
                self.burn_away(p);
            }
        }
    }

    /// A flammable node burnt away (minetest.remove_node, and
    /// check_for_falling): as if dug, but with no drops, and what was in
    /// it (a bookshelf's books) is lost. A bed someone lies in doesn't
    /// burn.
    fn burn_away(&mut self, pos: NodePos) {
        let id = self.world.node(pos);
        // TNT and gunpowder catch light instead (their on_burn), or with TNT
        // off, don't burn at all.
        if tnt::lights(id) {
            self.light_tnt(pos);
            return;
        }
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        if let Some((_, top)) = beds::bed_of(id) {
            let param2 = self.world.param2(pos);
            let other = beds::other_half(pos, param2, top);
            let paired = beds::is_bed(self.world.node(other)) && self.world.param2(other) == param2;
            let bottom = if top { paired.then_some(other) } else { Some(pos) };
            if bottom.is_some_and(|b| self.bed_occupied(b)) {
                return;
            }
            if paired {
                let o16 = [other.0 as i16, other.1 as i16, other.2 as i16];
                self.world.set_node(other, world::CONTENT_AIR, 0);
                self.broadcast_node(o16, &build_removenode(o16));
                self.bed_spawns.remove_at(other);
            }
            self.bed_spawns.remove_at(pos);
        }
        if doors::is_door(id) {
            let top = (pos.0, pos.1 + 1, pos.2);
            if self.world.node(top) == doors::HIDDEN {
                let t16 = [top.0 as i16, top.1 as i16, top.2 as i16];
                self.world.set_node(top, world::CONTENT_AIR, 0);
                self.broadcast_node(t16, &build_removenode(t16));
            }
        }
        if doors::is_door(id) || doors::is_trapdoor(id) {
            self.door_owners.remove(pos);
        }
        if content::is_chest(id) || content::shelf_of(id).is_some() {
            self.close_chest_windows(pos);
            self.chests.remove(pos);
        }
        debug!("Fire burnt {} at {pos:?}", content::name_of(id));
        let above = (pos.0, pos.1 + 1, pos.2);
        let on_top = self.world.node(above);
        self.world.set_node(pos, world::CONTENT_AIR, 0);
        self.broadcast_node(p16, &build_removenode(p16));
        self.wake_items(pos);
        self.update_panes_near(pos);
        self.check_falling(above);
        self.drop_attached(p16);
        // A generated plant that stood on it (see the digging).
        if farming::is_attached_plant(on_top) && self.world.node(above) == world::CONTENT_AIR {
            let a16 = [p16[0], p16[1] + 1, p16[2]];
            self.broadcast_node(a16, &build_removenode(a16));
        }
        self.start_leaf_decay(pos, id);
    }

    /// Flint and steel (its on_use): pointed at something flammable with
    /// air above, a flame there; at a coal block, a permanent flame on it.
    /// It strikes, and wears, whatever it points at.
    fn strike(&mut self, from: SocketAddr, slot: usize, target: Option<([i16; 3], [i16; 3])>, body: [f32; 3]) {
        let at = target.map_or(body.map(|v| v * 10.0), |(_, above)| above.map(|v| v as f32 * 10.0));
        self.play_sound_near(-1, "fire_flint_and_steel", 0.2, at, 8.0);
        if let Some((under, above)) = target {
            let (u, a) = ((under[0] as i32, under[1] as i32, under[2] as i32), (above[0] as i32, above[1] as i32, above[2] as i32));
            let node = self.world.node(u);
            let name = self.peers.get(&from).map(|p| p.display_name().to_string()).unwrap_or_default();
            if tnt::lights(node) {
                // TNT and gunpowder are lit (on_ignite).
                if self.light_tnt(u) {
                    info!("{name} lit {} at {u:?}", content::name_of(node));
                }
            } else if node == content::CONTENT_COAL_BLOCK {
                if self.light_flame((u.0, u.1 + 1, u.2), fire::PERMANENT_FLAME, 0) {
                    info!("{name} lit a permanent flame at {:?}", (u.0, u.1 + 1, u.2));
                }
            } else if fire::flammable(node) >= 1 && self.light_flame(a, fire::BASIC_FLAME, 0) {
                info!("{name} lit a fire at {a:?}");
            }
        }
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        let Some(mut tool) = peer.inventory.get(slot) else {
            return;
        };
        let intact = tool.add_wear(content::use_wear(fire::FLINT_USES, tool.wear));
        peer.inventory.set(slot, intact.then_some(tool));
        if !intact {
            self.play_sound_near(-1, content::TOOL_BREAKS, 0.5, at, HEARING);
        }
        self.inventory_changed(from);
    }

    /// The fire's crackle (fire.update_player_sound): for each player, the
    /// last one stops, and if flames burn within SOUND_RADIUS a new one
    /// plays from the middle of them, louder the more there are.
    fn crackle(&mut self) {
        let world = &self.world;
        self.flames.retain(|&p| fire::is_flame(world.node(p)));
        let r = fire::SOUND_RADIUS;
        let mut plays = Vec::new();
        for (&addr, peer) in self.peers.iter_mut().filter(|(_, p)| p.ready) {
            let me = peer.pos_bs.map(|v| (v / 10.0).round() as i32);
            let near: Vec<&NodePos> = self
                .flames
                .iter()
                .filter(|p| (p.0 - me[0]).abs() <= r && (p.1 - me[1]).abs() <= r && (p.2 - me[2]).abs() <= r)
                .collect();
            let stop = peer.fire_sound.take();
            if near.is_empty() {
                plays.push((addr, stop, None));
                continue;
            }
            let lo = [near.iter().map(|p| p.0).min().unwrap(), near.iter().map(|p| p.1).min().unwrap(), near.iter().map(|p| p.2).min().unwrap()];
            let hi = [near.iter().map(|p| p.0).max().unwrap(), near.iter().map(|p| p.1).max().unwrap(), near.iter().map(|p| p.2).max().unwrap()];
            let mid = [0, 1, 2].map(|i| (lo[i] + hi[i]) as f32 * 5.0);
            let id = self.next_sound_id;
            self.next_sound_id = if id == i32::MAX { 1 } else { id + 1 };
            peer.fire_sound = Some(id);
            plays.push((addr, stop, Some((id, fire::sound_gain(near.len()), mid))));
        }
        for (addr, stop, play) in plays {
            if let Some(id) = stop {
                self.send_command(addr, 0, &build_stop_sound(id));
            }
            if let Some((id, gain, mid)) = play {
                self.send_command(addr, 0, &build_play_sound_looped(id, "fire_fire", gain, mid));
            }
        }
    }

    /// Dropped items that can burn, burning up in lava, or now and then in
    /// a flame, with a puff of smoke (default's item_entity).
    fn burn_items(&mut self) {
        let world = &self.world;
        let mut burnt = Vec::new();
        for d in self.items.iter() {
            let node = world.node((d.pos[0].round() as i32, d.pos[1].round() as i32, d.pos[2].round() as i32));
            if fire::igniter(node) > 0 && fire::item_burns(d.item, node, &mut self.rng) {
                burnt.push((d.id, d.pos_bs()));
            }
        }
        for (id, at) in burnt {
            if let Some(d) = self.items.remove(id) {
                debug!("{} {} burnt up", d.count, content::name_of(d.item));
                self.send_to_players(&build_remove_add(&[id], &[]));
                self.play_sound_near(-1, "default_item_smoke", 1.0, at, 8.0);
                self.item_smoke(at.map(|v| v / 10.0));
            }
        }
    }

    // -------------------------------------------------------------------
    // TNT (tnt.rs)
    // -------------------------------------------------------------------

    /// Light TNT or gunpowder (tnt.burn): TNT goes off in 4 s, gunpowder
    /// burns for a second. Not while TNT is off. Returns whether it did.
    fn light_tnt(&mut self, pos: NodePos) -> bool {
        if !self.settings.tnt() {
            return false;
        }
        let id = self.world.node(pos);
        let Some(lit) = tnt::lit(id) else {
            return false;
        };
        let param2 = self.world.param2(pos);
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, lit, param2);
        self.broadcast_node(p16, &build_addnode(p16, lit, param2));
        let at = p16.map(|v| v as f32 * 10.0);
        if lit == tnt::TNT_BURNING {
            self.play_sound_near(-1, "tnt_ignite", 1.0, at, HEARING);
            self.timers.start(pos, Job::Fuse, tnt::FUSE);
        } else {
            self.play_sound_near(-1, "tnt_gunpowder_burning", 1.0, at, HEARING);
            self.timers.start(pos, Job::Powder, tnt::POWDER_BURNS);
        }
        true
    }

    /// Lit TNT's fuse has burnt down: it goes off, or with TNT turned off
    /// meanwhile, goes back to being TNT.
    fn fuse(&mut self, pos: NodePos) {
        if self.world.node(pos) != tnt::TNT_BURNING {
            return;
        }
        if !self.settings.tnt() {
            let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
            self.world.set_node(pos, tnt::TNT, 0);
            self.broadcast_node(p16, &build_addnode(p16, tnt::TNT, 0));
            return;
        }
        self.explode(pos);
    }

    /// Lit gunpowder has burnt: it lights what is next to it along the
    /// trail, and is gone.
    fn powder_burns(&mut self, pos: NodePos) {
        if self.world.node(pos) != tnt::GUNPOWDER_BURNING {
            return;
        }
        for p in tnt::powder_reach(pos) {
            self.light_tnt(p);
        }
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, world::CONTENT_AIR, 0);
        self.broadcast_node(p16, &build_removenode(p16));
    }

    /// The "TNT ignition" ABM: TNT and gunpowder beside a flame (not a
    /// permanent one) or lava, near players, are lit.
    fn ignite_tnt(&mut self) {
        if !self.settings.tnt() {
            return;
        }
        let active = self.active_test();
        let world = &self.world;
        let mut igniters: Vec<NodePos> = self.flames.iter().copied().filter(|&p| active(p) && world.node(p) == fire::BASIC_FLAME).collect();
        igniters.extend(world.changed_nodes(content::is_lava).into_iter().filter(|&p| active(p)));
        let mut lit: Vec<NodePos> = Vec::new();
        for q in igniters {
            for d in nature::AROUND {
                let p = (q.0 + d.0, q.1 + d.1, q.2 + d.2);
                if tnt::lights(world.node(p)) && !lit.contains(&p) {
                    lit.push(p);
                }
            }
        }
        for p in lit {
            self.light_tnt(p);
        }
    }

    /// TNT goes off at `center` (tnt.boom): all the TNT within 2 nodes
    /// with it, in a blast that grows with how many; what is in the blast
    /// is destroyed (and thrown out), set alight, lit or set off in turn;
    /// sand and gravel above come down; players near are hurt and thrown
    /// back, and items thrown.
    fn explode(&mut self, center: NodePos) {
        let mut count = 0;
        for dz in -2..=2 {
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let p = (center.0 + dx, center.1 + dy, center.2 + dz);
                    if matches!(self.world.node(p), tnt::TNT | tnt::TNT_BURNING) {
                        count += 1;
                        self.world.set_node(p, world::CONTENT_AIR, 0);
                    }
                }
            }
        }
        let radius = tnt::blast_radius(count);
        self.blast(center, radius, 2, &format!("TNT exploded at {center:?} ({count} together, radius {radius})"), "blown up");
    }

    /// A blast of `radius` at `center` (TNT's, or a wisp's): what is in it
    /// is destroyed (and thrown out), set alight, lit or set off in turn;
    /// sand and gravel above come down; players and mobs near are hurt
    /// (`death` says how a player died) and thrown back, and items thrown.
    /// The TNT within `middle` nodes went with it.
    fn blast(&mut self, center: NodePos, radius: i32, middle: i32, what: &str, death: &str) {
        let started = Instant::now();
        let mut drops: Vec<Stack> = Vec::new();
        let mut chained = Vec::new();
        let mut powder = Vec::new();
        let mut changed: Vec<NodePos> = Vec::new();
        let mut trunks = Vec::new();
        let world_basic = |s: &Self| s.flames.iter().filter(|&&p| s.world.node(p) == fire::BASIC_FLAME).count();
        let mut basic = world_basic(self);
        for dz in -radius..=radius {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if !tnt::reaches((dx, dy, dz), radius, &mut self.rng) {
                        continue;
                    }
                    let p = (center.0 + dx, center.1 + dy, center.2 + dz);
                    let id = self.world.node(p);
                    match tnt::blasted(id) {
                        tnt::Blasted::Spared => continue,
                        tnt::Blasted::Chain => {
                            chained.push(p);
                            continue;
                        }
                        tnt::Blasted::Light => {
                            powder.push(p);
                            continue;
                        }
                        tnt::Blasted::Emptied => {
                            if content::is_furnace(id) {
                                self.close_furnace(p);
                                if let Some(f) = self.furnaces.remove(p) {
                                    self.fade_sounds(&f.sounds);
                                    drops.extend(f.src.iter().chain(f.fuel.iter()).chain(f.dst.iter()).flatten().copied());
                                }
                                drops.push(Stack::new(content::CONTENT_FURNACE, 1));
                            } else {
                                self.fill_dungeon_chest(p);
                                self.close_chest_windows(p);
                                if let Some(c) = self.chests.get(p) {
                                    drops.extend(c.slots().iter().flatten().copied());
                                }
                                self.chests.remove(p);
                                let item = content::chest_pair(id).map_or(id, |(closed, _)| closed);
                                drops.push(Stack::new(item, 1));
                            }
                            self.world.set_node(p, world::CONTENT_AIR, 0);
                        }
                        tnt::Blasted::Flame => {
                            let flame = basic < fire::MAX_FLAMES;
                            self.world.set_node(p, if flame { fire::BASIC_FLAME } else { world::CONTENT_AIR }, 0);
                            if flame {
                                basic += 1;
                                self.flames.insert(p);
                                self.timers.start(p, Job::Flame, fire::flame_timer(&mut self.rng));
                            }
                        }
                        tnt::Blasted::Destroyed => {
                            for (item, n) in self.drops(id) {
                                drops.push(Stack::new(item, n));
                            }
                            // A bed's other half and a door's hidden top go
                            // too; nobody respawns at a blown-up bed.
                            if let Some((_, top)) = beds::bed_of(id) {
                                let other = beds::other_half(p, self.world.param2(p), top);
                                if beds::is_bed(self.world.node(other)) {
                                    self.world.set_node(other, world::CONTENT_AIR, 0);
                                    changed.push(other);
                                }
                                self.bed_spawns.remove_at(other);
                                self.bed_spawns.remove_at(p);
                            }
                            if doors::is_door(id) || doors::is_trapdoor(id) {
                                self.door_owners.remove(p);
                                let top = (p.0, p.1 + 1, p.2);
                                if doors::is_door(id) && self.world.node(top) == doors::HIDDEN {
                                    self.world.set_node(top, world::CONTENT_AIR, 0);
                                    changed.push(top);
                                }
                            }
                            if content::is_trunk(id) {
                                trunks.push((p, id));
                            }
                            self.put_out_above(p, id);
                            self.world.set_node(p, world::CONTENT_AIR, 0);
                        }
                    }
                    changed.push(p);
                }
            }
        }
        // TNT round the middle went with it.
        for dz in -middle..=middle {
            for dy in -middle..=middle {
                for dx in -middle..=middle {
                    changed.push((center.0 + dx, center.1 + dy, center.2 + dz));
                }
            }
        }
        let worked_out = started.elapsed().as_millis();
        // Every map block it changed goes out again whole.
        let blocks: BTreeSet<BlockPos> = changed.iter().map(|&p| world::block_of_node(p)).collect();
        for b in blocks {
            self.send_block_again(b);
        }
        let at = [center.0 as f32 * 10.0, center.1 as f32 * 10.0, center.2 as f32 * 10.0];
        self.play_sound_near(-1, "tnt_explode", 2.5, at, (radius as f32 * 20.0).min(128.0));
        // What stood on what went falls, or drops off.
        let reach = radius * 3 / 2;
        for dz in -reach..=reach {
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let r = ((dx * dx + dy * dy + dz * dz) as f32).sqrt();
                    if r / (radius as f32) < 1.4 {
                        self.check_falling((center.0 + dx, center.1 + dy, center.2 + dz));
                    }
                }
            }
        }
        for &p in &changed {
            self.drop_attached([p.0 as i16, p.1 as i16, p.2 as i16]);
            self.wake_items(p);
        }
        for (p, trunk) in trunks {
            self.start_leaf_decay(p, trunk);
        }
        for p in chained {
            self.timers.start(p, Job::Chain, tnt::CHAINED);
        }
        for p in powder {
            self.light_tnt(p);
        }
        // Bits of what there was most of fly out (add_effects).
        let mut most: Vec<(u16, u32)> = Vec::new();
        for d in drops.iter().filter(|d| content::is_node(d.id)) {
            match most.iter_mut().find(|(id, _)| *id == d.id) {
                Some((_, n)) => *n += d.count as u32,
                None => most.push((d.id, d.count as u32)),
            }
        }
        let bits = most.iter().max_by_key(|(_, n)| *n).map(|(id, _)| *id);
        self.blast_effects(center, radius, bits);
        self.blast_physics(center, radius, death);
        self.eject(center, radius, drops);
        info!(
            "{what}: {} nodes, worked out in {worked_out} ms, {} ms in all",
            changed.len(),
            started.elapsed().as_millis()
        );
    }

    /// A blast's push (entity_physics): players and mobs within its damage
    /// radius lose HP and are thrown back; dropped items are thrown.
    fn blast_physics(&mut self, center: NodePos, radius: i32, death: &str) {
        let c = [center.0 as f32, center.1 as f32, center.2 as f32];
        let reach = tnt::damage_radius(radius);
        let away = |p: [f32; 3]| {
            let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            (len, if len > 0.0 { d.map(|v| v / len) } else { [0.0, 1.0, 0.0] })
        };
        let hurt: Vec<(SocketAddr, f32, [f32; 3])> = self
            .peers
            .iter()
            .filter(|(_, p)| p.ready && p.hp > 0)
            .filter_map(|(a, p)| {
                let (dist, dir) = away(p.pos_bs.map(|v| v / 10.0));
                (dist <= reach).then_some((*a, dist, dir))
            })
            .collect();
        for (addr, dist, dir) in hurt {
            let push = tnt::knockback(dir, dist, radius);
            self.send_command(addr, 0, &build_player_speed(push.map(|v| v * 10.0)));
            let damage = tnt::damage(dist, radius).round() as u16;
            if damage > 0 {
                self.hurt(addr, damage, death);
            }
        }
        let mobs: Vec<(u16, f32)> = self.mobs.iter().map(|m| (m.id, away(m.pos).0)).filter(|m| m.1 <= reach).collect();
        for (id, dist) in mobs {
            let damage = tnt::damage(dist, radius).round() as u16;
            if let Some(hit) = self.mobs.harm(id, None, c, damage.max(1)) {
                self.mob_hit(id, hit);
            }
        }
        let items: Vec<(u16, f32, [f32; 3], [f32; 3])> =
            self.items.iter().map(|d| (d.id, away(d.pos).0, away(d.pos).1, d.vel)).filter(|i| i.1 <= reach).collect();
        for (id, dist, dir, old) in items {
            // calc_velocity: power radius * 10, over the distance, a little
            // at random.
            let power = radius as f32 * 10.0 / dist.max(1.0);
            let mut v = [0, 1, 2].map(|i| dir[i] * power + old[i] + (self.rng.range(0, 100) as f32 / 100.0 - 0.5));
            let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if speed > 250.0 {
                v = v.map(|x| x * 250.0 / speed);
            }
            if self.items.throw(id, v) {
                self.send_item_position(id);
            }
        }
    }

    /// What a blast destroyed, thrown out in handfuls (eject_drops) from
    /// open places within its radius, upwards and outwards.
    fn eject(&mut self, center: NodePos, radius: i32, drops: Vec<Stack>) {
        for stack in drops {
            for n in tnt::handfuls(stack.count, radius, content::stack_max(stack.id)) {
                let mut at = center;
                for _ in 0..5 {
                    let x = center.0 + self.rng.range(0, (2 * radius) as u32) as i32 - radius;
                    let z = center.2 + self.rng.range(0, (2 * radius) as u32) as i32 - radius;
                    if !content::walkable(self.world.node((x, center.1, z))) {
                        at = (x, center.1, z);
                        break;
                    }
                }
                let vel = [self.rng.range(0, 6) as f32 - 3.0, self.rng.range(0, 10) as f32, self.rng.range(0, 6) as f32 - 3.0];
                self.spawn_item(Stack { count: n, ..stack }, ([at.0 as f32, at.1 as f32, at.2 as f32], vel));
            }
        }
    }

    /// minetest_game's cool_lava: lava touching water turns, half the time
    /// it is looked at, into obsidian (a source) or stone (flowing lava),
    /// with a hiss.
    fn cool_lava(&mut self) {
        for pos in self.world.lava_touching_water() {
            if !self.rng.one_in(LAVA_COOLING_CHANCE) {
                continue;
            }
            let was = self.world.node(pos);
            let to = if was == content::CONTENT_LAVA { content::CONTENT_OBSIDIAN } else { content::CONTENT_STONE };
            self.world.set_node(pos, to, 0);
            self.send_block_again(world::block_of_node(pos));
            let at = [pos.0 as f32 * 10.0, pos.1 as f32 * 10.0, pos.2 as f32 * 10.0];
            self.play_sound_near(-1, "default_cool_lava", 0.2, at, 16.0);
        }
    }

    /// Damage from the nodes a player is in, once a second (PlayerSAO's
    /// node damage): the worst of the points 0.1 above the feet, every
    /// node up from there, and 0.1 under the top of the player's box.
    fn burn_players(&mut self) {
        let mut hurt = Vec::new();
        for (addr, p) in &self.peers {
            if !p.ready || p.hp == 0 {
                continue;
            }
            let top = if p.in_bed.is_some() { 0.3 } else { 1.7 } - 0.1;
            let feet = p.pos_bs.map(|v| v / 10.0);
            let mut heights = Vec::new();
            let mut h = 0.1;
            while h < top {
                heights.push(h);
                h += 1.0;
            }
            heights.push(top);
            let damage = heights
                .iter()
                .map(|h| {
                    let n = (feet[0].round() as i32, (feet[1] + h).round() as i32, feet[2].round() as i32);
                    content::damage_per_second(self.world.node(n))
                })
                .max()
                .unwrap_or(0);
            if damage > 0 {
                hurt.push((*addr, damage));
            }
        }
        for (addr, damage) in hurt {
            self.hurt(addr, damage, "burned in lava");
        }
    }

    /// Tell everyone who has it that a node changed from `was` to `id`:
    /// node by node, except for lava, whose block is sent again whole (see
    /// flow_liquids).
    fn show_node(&mut self, pos: [i16; 3], was: u16, id: u16, param2: u8) {
        if content::is_lava(was) || content::is_lava(id) {
            self.send_block_again(world::block_of_node((pos[0] as i32, pos[1] as i32, pos[2] as i32)));
        } else if id == world::CONTENT_AIR {
            self.broadcast_node(pos, &build_removenode(pos));
        } else {
            self.broadcast_node(pos, &build_addnode(pos, id, param2));
        }
    }

    /// Players who have a block get it again, before anything else they
    /// are waiting for (Luanti's SetBlockNotSent).
    fn send_block_again(&mut self, block: BlockPos) {
        for peer in self.peers.values_mut() {
            if peer.ready && peer.sent_blocks.remove(&block) && !peer.pending_blocks.contains(&block) {
                peer.pending_blocks.push_front(block);
            }
        }
    }

    /// What digging a node gives: its drop, except that leaves sometimes
    /// give a sapling, and farming's plants roll for theirs.
    fn drops(&mut self, id: u16) -> Vec<(u16, u16)> {
        if farming::is_attached_plant(id) || farming::is_soil(id) {
            return farming::drops(id, &mut self.rng);
        }
        // Gravel gives flint one time in 16 (minetest_game's drop table).
        if id == content::CONTENT_GRAVEL && self.rng.one_in(16) {
            return vec![(content::ITEM_FLINT, 1)];
        }
        match content::drop_of(id) {
            Some((leaf, 1)) if content::is_leaves(id) => vec![(nature::leaf_drop(leaf, &mut self.rng), 1)],
            other => other.into_iter().collect(),
        }
    }

    /// A hoe used on dirt or grass with air above (farming.hoe_on_use):
    /// dry soil, with the sound of digging, and wear on the hoe. Returns
    /// whether the wielded item was a hoe.
    fn hoe(&mut self, addr: SocketAddr, at: BucketUse) -> bool {
        let BucketUse { slot, under, body, .. } = at;
        let Some(peer) = self.peers.get(&addr) else {
            return false;
        };
        let Some(mut hoe) = peer.inventory.get(slot).filter(|_| slot < HOTBAR_SIZE) else {
            return false;
        };
        let Some(uses) = farming::hoe_uses(hoe.id) else {
            return false;
        };
        let name = peer.display_name().to_string();
        let pos = (under[0] as i32, under[1] as i32, under[2] as i32);
        let near = (0..3).map(|i| (under[i] as f32 - body[i]).powi(2)).sum::<f32>().sqrt() <= MAX_INTERACT_DISTANCE;
        let soil = farming::hoed(self.world.node(pos));
        let clear = farming::clear_above(self.world.node((pos.0, pos.1 + 1, pos.2)));
        let Some(soil) = soil.filter(|_| clear && near) else {
            return true;
        };
        info!("{name} hoed {} at {under:?}", content::name_of(self.world.node(pos)));
        self.edit_event(&name, "hoed", soil);
        self.world.set_node(pos, soil, 0);
        self.broadcast_node(under, &build_addnode(under, soil, 0));
        self.fields.insert(pos);
        let at_bs = under.map(|v| v as f32 * 10.0);
        self.play_sound_near(-1, "default_dig_crumbly", 0.3, at_bs, HEARING);
        let intact = hoe.add_wear(content::use_wear(uses, hoe.wear));
        if let Some(peer) = self.peers.get_mut(&addr) {
            peer.inventory.set(slot, intact.then_some(hoe));
        }
        if !intact {
            info!("{name}'s {} broke", content::name_of(hoe.id));
            self.play_sound_near(-1, content::TOOL_BREAKS, 0.5, at_bs, HEARING);
        }
        self.inventory_changed(addr);
        true
    }

    /// farming.grow_plant: a seed sprouts on soil; a crop grows a stage on
    /// wet soil in daylight under the open sky, otherwise it tries again
    /// in a minute or so. A seed off soil stays a seed.
    fn grow_crop(&mut self, pos: NodePos) {
        let id = self.world.node(pos);
        if !farming::is_growing(id) {
            return; // dug or eaten meanwhile
        }
        let below = self.world.node((pos.0, pos.1 - 1, pos.2));
        let daytime = (6000..18000).contains(&self.time_of_day());
        let lit = daytime && self.world.highest_solid(pos.0, pos.2) < pos.1;
        match farming::grow(id, below, lit) {
            Some((next, param2)) => {
                let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
                self.world.set_node(pos, next, param2);
                self.broadcast_node(p16, &build_addnode(p16, next, param2));
                if farming::is_growing(next) {
                    let after = farming::grow_delay(&mut self.rng);
                    self.timers.start(pos, Job::Crop, after);
                }
            }
            None if farming::is_seed(id) && !farming::is_soil(below) => {}
            None => {
                let after = farming::grow_retry(&mut self.rng);
                self.timers.start(pos, Job::Crop, after);
            }
        }
    }

    /// The "Farming soil" ABM, every SOIL_INTERVAL for one soil node in
    /// SOIL_CHANCE: wet with water within 3 nodes, drying without, and
    /// back to dirt when dry and bare or covered by something solid.
    fn tend_fields(&mut self) {
        let fields: Vec<NodePos> = self.fields.iter().copied().collect();
        for pos in fields {
            let soil = self.world.node(pos);
            if !farming::is_soil(soil) {
                self.fields.remove(&pos);
                continue;
            }
            if !self.rng.one_in(farming::soil_check().1) {
                continue;
            }
            let above = self.world.node((pos.0, pos.1 + 1, pos.2));
            let r = farming::WATER_REACH;
            let water_near = (-r..=r).any(|dy| {
                (-r..=r).any(|dz| (-r..=r).any(|dx| farming::is_water(self.world.node((pos.0 + dx, pos.1 + dy, pos.2 + dz)))))
            });
            let Some(next) = farming::soil_update(soil, above, water_near) else {
                continue;
            };
            let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
            self.world.set_node(pos, next, 0);
            self.broadcast_node(p16, &build_addnode(p16, next, 0));
            if !farming::is_soil(next) {
                self.fields.remove(&pos);
            }
        }
    }

    /// Remove what was attached to the node at `pos` (just removed):
    /// torches and signs hanging on it, and a sapling standing on it. They drop as
    /// items where they were.
    fn drop_attached(&mut self, pos: [i16; 3]) {
        for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let n = [pos[0] + dx, pos[1] + dy, pos[2] + dz];
            let np = (n[0] as i32, n[1] as i32, n[2] as i32);
            let id = self.world.node(np);
            let attached = if content::is_torch(id) || content::is_sign(id) {
                let s = content::wallmounted_support(self.world.param2(np));
                (np.0 + s.0, np.1 + s.1, np.2 + s.2) == (pos[0] as i32, pos[1] as i32, pos[2] as i32)
            } else {
                content::stands_on_floor(id) && dy == 1
            };
            if !attached {
                continue;
            }
            info!("{} at {n:?} lost its support", content::name_of(id));
            self.world.set_node(np, world::CONTENT_AIR, 0);
            self.broadcast_node(n, &build_removenode(n));
            for (item, count) in self.drops(id) {
                self.spawn_item(Stack::new(item, count), (n.map(|v| v as f32), [0.0; 3]));
            }
        }
    }

    /// A trunk was dug at `pos`: its leaves nearby get decay timers.
    fn start_leaf_decay(&mut self, pos: NodePos, trunk: u16) {
        let world = &self.world;
        let node = |p: NodePos| (world.node(p), world.param2(p));
        for leaf in nature::leaves_to_check(pos, trunk, &node) {
            let after = nature::decay_delay(&mut self.rng);
            self.timers.start(leaf, Job::Decay, after);
        }
    }

    /// Run the leaf decay and sapling timers that are due (a few per tick).
    fn run_timers(&mut self) {
        let now = Instant::now();
        for _ in 0..TIMERS_PER_TICK {
            let Some((pos, job)) = self.timers.pop_due(now) else {
                return;
            };
            match job {
                Job::Decay => self.decay_leaf(pos),
                Job::Grow => self.grow_sapling(pos),
                Job::Regrow => self.regrow_apple(pos),
                Job::Crop => self.grow_crop(pos),
                Job::Berries => self.grow_berries(pos),
                Job::Flame => self.check_flame(pos),
                Job::Fuse => self.fuse(pos),
                Job::Powder => self.powder_burns(pos),
                Job::Chain => {
                    if self.world.node(pos) == tnt::TNT && self.settings.tnt() {
                        self.explode(pos);
                    }
                }
            }
        }
    }

    /// minetest_game's leafdecay_on_timer: a leaf with no trunk of its tree
    /// near it goes, dropping a sapling now and then (never the leaf).
    fn decay_leaf(&mut self, pos: NodePos) {
        let world = &self.world;
        let node = |p: NodePos| (world.node(p), world.param2(p));
        if !nature::should_decay(pos, &node) {
            return;
        }
        let (leaf, leaf_param2) = (self.world.node(pos), self.world.param2(pos));
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, world::CONTENT_AIR, 0);
        self.broadcast_node(p16, &build_removenode(p16));
        self.node_bits(pos, leaf, leaf_param2, None);
        self.wake_items(pos);
        self.drop_attached(p16);
        if let Some(drop) = nature::decay_drop(leaf, &mut self.rng) {
            let r = |rng: &mut Rng| rng.range(0, 99) as f32 / 100.0 - 0.5;
            let at = [pos.0 as f32 + r(&mut self.rng), pos.1 as f32 + r(&mut self.rng), pos.2 as f32 + r(&mut self.rng)];
            self.spawn_item(Stack::new(drop, 1), (at, [0.0; 3]));
        }
    }

    /// A picked blueberry bush's leaves bear berries again, when light
    /// enough (daylight reaching them); otherwise they wait 200 s more.
    fn grow_berries(&mut self, pos: NodePos) {
        if self.world.node(pos) != plants::BLUEBERRY_LEAVES {
            return;
        }
        let lit = (6000..18000).contains(&self.time_of_day()) && self.world.highest_solid(pos.0, pos.2) <= pos.1;
        if !lit {
            self.timers.start(pos, Job::Berries, nature::quick(plants::BERRIES_RETRY));
            return;
        }
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, plants::BLUEBERRY_LEAVES_WITH_BERRIES, 0);
        self.broadcast_node(p16, &build_addnode(p16, plants::BLUEBERRY_LEAVES_WITH_BERRIES, 0));
    }

    /// The "Grow cactus" and "Grow papyrus" ABMs, for what players
    /// planted: now and then one grows a node, on its ground, in daylight.
    fn tend_climbers(&mut self) {
        let daytime = (6000..18000).contains(&self.time_of_day());
        for pos in self.climbers.clone() {
            if !plants::is_climber(self.world.node(pos)) {
                self.climbers.remove(&pos);
                continue;
            }
            if !self.rng.one_in(plants::CLIMB_CHANCE) {
                continue;
            }
            let world = &self.world;
            let lit = |p: NodePos| daytime && world.highest_solid(p.0, p.2) < p.1;
            let Some(next) = plants::climb(pos, &|p| world.node(p), &lit) else {
                continue;
            };
            let id = self.world.node(pos);
            let p16 = [next.0 as i16, next.1 as i16, next.2 as i16];
            self.world.set_node(next, id, 0);
            self.broadcast_node(p16, &build_addnode(p16, id, 0));
        }
    }

    /// Kelp or a coral put into the node pointed at: kelp into sand with
    /// water 4 to 6 nodes up (its height, picked at random), a coral into
    /// coral skeleton with water above.
    fn place_rooted(&mut self, from: SocketAddr, id: u16, slot: usize, under: [i16; 3]) {
        let pos = (under[0] as i32, under[1] as i32, under[2] as i32);
        let height = self.rng.range(4, 6) as i32;
        let world = &self.world;
        let node = |p: NodePos| world.node(p);
        let param2 = if id == plants::KELP { plants::kelp_place(pos, height, &node) } else { plants::coral_place(id, pos, &node).then_some(0) };
        let Some(param2) = param2 else {
            return;
        };
        let name = self.peers.get(&from).map(|p| p.display_name().to_string()).unwrap_or_default();
        info!("{name} placed {} at {under:?}", content::name_of(id));
        self.edit_event(&name, "placed", id);
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.inventory.take_one(slot);
        }
        self.inventory_changed(from);
        self.world.set_node(pos, id, param2);
        self.broadcast_node(under, &build_addnode(under, id, param2));
        let (sound, gain) = content::place_sound(id);
        self.play_sound_near(-1, sound, gain, under.map(|v| v as f32 * 10.0), HEARING);
    }

    /// minetest_game's apple_mark timer: an apple grows again if apple
    /// leaves are still next to the mark and it is light; in the dark it
    /// waits; with no leaves left the mark goes.
    fn regrow_apple(&mut self, pos: NodePos) {
        if !content::is_mark(self.world.node(pos)) {
            return; // something was placed there
        }
        let daylight = (6000..18000).contains(&self.time_of_day());
        let world = &self.world;
        let node = |p: NodePos| world.node(p);
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        match nature::regrow(pos, &node, daylight) {
            nature::Regrow::Gone => {
                self.world.set_node(pos, world::CONTENT_AIR, 0);
                self.broadcast_node(p16, &build_removenode(p16));
            }
            nature::Regrow::Wait => self.timers.start(pos, Job::Regrow, nature::quick(nature::REGROW_RETRY)),
            nature::Regrow::Apple => {
                self.world.set_node(pos, content::CONTENT_APPLE, 0);
                self.broadcast_node(p16, &build_addnode(p16, content::CONTENT_APPLE, 0));
            }
        }
    }

    /// Left-click with a usable item: eating an apple restores 2 HP
    /// (minetest_game's item_eat(2): one is used up even at full health).
    /// An empty bucket used on a water source (the bucket mod's on_use):
    /// the water goes into the bucket, leaving air. With a stack of empty
    /// buckets, the full one goes into the inventory (or is dropped if it
    /// doesn't fit). Returns whether the wielded item was an empty bucket.
    fn scoop_water(&mut self, addr: SocketAddr, at: BucketUse) -> bool {
        let BucketUse { slot, under, body, .. } = at;
        let Some(peer) = self.peers.get(&addr) else {
            return false;
        };
        let Some(bucket) = peer.inventory.get(slot).filter(|s| slot < HOTBAR_SIZE && s.id == content::ITEM_BUCKET_EMPTY) else {
            return false;
        };
        let name = peer.display_name().to_string();
        let pos = (under[0] as i32, under[1] as i32, under[2] as i32);
        let near = (0..3).map(|i| (under[i] as f32 - body[i]).powi(2)).sum::<f32>().sqrt() <= MAX_INTERACT_DISTANCE;
        // A water or lava source (the bucket mod registers both).
        let full = match self.world.node(pos) {
            content::CONTENT_WATER => content::ITEM_BUCKET_WATER,
            content::CONTENT_LAVA => content::ITEM_BUCKET_LAVA,
            other => {
                // Say why, as the client shows nothing: a bucket fills only
                // from a source, never from flowing water or lava.
                info!("{name}: can't fill a bucket from {} at {under:?}", content::name_of(other));
                return true;
            }
        };
        if !near {
            info!("{name}: {under:?} is too far away to fill a bucket from");
            return true;
        }
        info!("{name} scooped up {} at {under:?}", content::name_of(self.world.node(pos)));
        let was = self.world.node(pos);
        self.world.set_node(pos, world::CONTENT_AIR, 0);
        self.show_node(under, was, world::CONTENT_AIR, 0);
        let full = Stack::new(full, 1);
        let mut dropped = false;
        if let Some(peer) = self.peers.get_mut(&addr) {
            if bucket.count == 1 {
                peer.inventory.set(slot, Some(full));
            } else {
                peer.inventory.take_one(slot);
                dropped = peer.inventory.add(full) > 0;
            }
        }
        if dropped {
            self.spawn_item(full, (body, [0.0; 3]));
        }
        self.wake_items(pos);
        self.inventory_changed(addr);
        true
    }

    /// A water bucket placed (the bucket mod's on_place): a water source
    /// in the node pointed at if it can be built into (such as water),
    /// otherwise in the one in front of it, which must be. The bucket is
    /// left empty.
    fn pour_water(&mut self, addr: SocketAddr, at: BucketUse) {
        let BucketUse { slot, under, above, .. } = at;
        let Some(source) = self.peers.get(&addr).and_then(|p| p.inventory.get(slot)).and_then(|s| match s.id {
            content::ITEM_BUCKET_WATER => Some(content::CONTENT_WATER),
            content::ITEM_BUCKET_LAVA => Some(content::CONTENT_LAVA),
            _ => None,
        }) else {
            return;
        };
        let node_pos = |p: [i16; 3]| (p[0] as i32, p[1] as i32, p[2] as i32);
        let pos = if content::buildable_to(self.world.node(node_pos(under))) { under } else { above };
        if !content::buildable_to(self.world.node(node_pos(pos))) {
            self.resend_node(addr, pos);
            return;
        }
        let name = self.peers.get(&addr).map(|p| p.display_name().to_string()).unwrap_or_default();
        info!("{name} poured {} at {pos:?}", content::name_of(source));
        self.edit_event(&name, "placed", source);
        let was = self.world.node(node_pos(pos));
        self.world.set_node(node_pos(pos), source, 0);
        self.show_node(pos, was, source, 0);
        if let Some(peer) = self.peers.get_mut(&addr) {
            peer.inventory.set(slot, Some(Stack::new(content::ITEM_BUCKET_EMPTY, 1)));
        }
        self.inventory_changed(addr);
    }

    fn use_item(&mut self, addr: SocketAddr, slot: usize) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        // A book opens to be written in, or read (book_on_use).
        if let Some(book) = peer.inventory.get(slot).filter(|s| slot < HOTBAR_SIZE && books::is_book(s.id)) {
            let form = books::use_form(&book, peer.display_name(), peer.book_tab);
            info!("{} opened {} (slot {slot})", peer.display_name(), content::description_of(book.id));
            peer.book_slot = Some(slot);
            self.send_command(addr, 0, &build_show_formspec(&form, books::FORM));
            return;
        }
        // Food: an apple heals 2, bread 5 (minetest_game's item_eat); a
        // red mushroom hurts.
        let food = peer.inventory.get(slot).filter(|_| slot < HOTBAR_SIZE).and_then(|s| match s.id {
            id if content::is_apple(id) => Some((id, content::APPLE_HP as i32)),
            content::ITEM_BREAD => Some((content::ITEM_BREAD, content::BREAD_HP as i32)),
            content::ITEM_BLUEBERRIES => Some((content::ITEM_BLUEBERRIES, plants::BLUEBERRY_HP as i32)),
            id => colours::mushroom_hp(id).map(|hp| (id, hp)),
        });
        let Some((food, heals)) = food else {
            return;
        };
        peer.inventory.take_one(slot);
        if heals < 0 {
            info!("{} ate {}", peer.display_name(), content::description_of(food));
            self.inventory_changed(addr);
            self.hurt(addr, (-heals) as u16, "was poisoned");
            return;
        }
        peer.hp = (peer.hp + heals as u16).min(PLAYER_MAX_HP);
        let (hp, name) = (peer.hp, peer.display_name().to_string());
        peer.save(&self.players);
        info!("{name} ate {}: {hp} HP", content::description_of(food));
        self.send_command(addr, 0, &build_hp(hp, false));
        self.send_command(addr, 0, &build_health_bar_change(hp));
        self.inventory_changed(addr);
    }

    /// The secret of the key in a player's hand, if they hold one.
    fn wielded_key(&self, addr: SocketAddr) -> Option<String> {
        let peer = self.peers.get(&addr)?;
        keys::secret_of(peer.inventory.get(peer.wield).filter(|_| peer.wield < HOTBAR_SIZE))
    }

    /// binoculars.update_player_property: the zoom, if it has changed,
    /// in the player's properties, which everyone is sent.
    fn update_zoom(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get_mut(&addr).filter(|p| p.ready) else {
            return;
        };
        let zoom = binoculars::zoom_fov(&peer.inventory.main);
        if zoom == peer.zoom_fov {
            return;
        }
        peer.zoom_fov = zoom;
        let (id, skin, lying) = (peer.id, peer.skin.clone(), peer.posture());
        info!("{}: zoom {zoom}", peer.display_name());
        let texture = self.skin_texture(&skin);
        let packet = build_object_messages(&[(id, player_properties_message(&texture, lying, zoom))]);
        self.send_to_players(&packet);
    }

    /// map.update_hud_flags: the minimap allowed with a mapping kit, its
    /// radar never; the client told only of a change.
    /// Running water and lava near a player, for them alone.
    fn play_env_sounds(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get(&addr).filter(|p| p.ready) else {
            return;
        };
        let eye_height = if peer.in_bed.is_some() { 0.3 } else { 1.47 };
        let eye = [peer.pos_bs[0] / 10.0, peer.pos_bs[1] / 10.0 + eye_height, peer.pos_bs[2] / 10.0];
        // Above LAVA_LEVEL nothing flows, and there is no lava, but what
        // has changed: only the deep is looked at in full.
        let world = &self.world;
        let heard = env_sounds::around(eye, |pos| {
            if pos.1 <= crate::terrain::LAVA_LEVEL {
                world.node(pos)
            } else {
                world.changed_node(pos).unwrap_or(crate::world::CONTENT_AIR)
            }
        });
        for h in heard {
            let packet = build_play_sound(-1, h.sound, h.gain, h.pos.map(|v| v * 10.0));
            self.send_command(addr, 0, &packet);
        }
    }

    /// A player's clouds and shadows, for the weather where they are.
    fn set_weather(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get(&addr).filter(|p| p.ready) else {
            return;
        };
        let (x, z) = ((peer.pos_bs[0] / 10.0).round() as i32, (peer.pos_bs[2] / 10.0).round() as i32);
        let w = weather::at(self.clock.game_time(), crate::terrain::humidity(x, z));
        self.send_command(addr, 0, &build_cloud_params(&w));
        self.send_command(addr, 0, &build_set_lighting(w.shadow_intensity));
    }

    fn update_minimap(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get_mut(&addr).filter(|p| p.ready) else {
            return;
        };
        let allowed = binoculars::minimap(&peer.inventory.main);
        if peer.minimap == Some(allowed) {
            return;
        }
        let had = peer.minimap == Some(true);
        peer.minimap = Some(allowed);
        let (flags, mask) = binoculars::minimap_flags(allowed);
        self.send_command(addr, 0, &build_hud_set_flags(flags, mask));
        // builtin's minimap HUD element, added and removed with the flag
        // (the client shows the minimap only through it).
        if allowed {
            self.send_command(addr, 0, &build_minimap_hud());
        } else if had {
            self.send_command(addr, 0, &build_hud_remove(HUD_MINIMAP));
        }
        info!("{}: minimap {}", self.peers.get(&addr).map_or("", |p| p.display_name()), if allowed { "allowed" } else { "off" });
    }

    /// keys' skeleton key, used on a locked chest, steel door or steel
    /// trapdoor (its on_skeleton_key_use): for the owner, the lock gets a
    /// secret if it has none, and one skeleton key becomes a key to it.
    fn cut_key(&mut self, addr: SocketAddr, slot: usize, under: [i16; 3]) {
        let pos = (under[0] as i32, under[1] as i32, under[2] as i32);
        let node = self.world.node(pos);
        let Some(name) = self.peers.get(&addr).map(|p| p.display_name().to_string()) else {
            return;
        };
        let new = keys::new_secret(|| self.rng.next() as u16);
        let mine = |owner: Option<&str>| owner.is_some_and(|o| o.eq_ignore_ascii_case(&name));
        let (secret, owner) = if content::is_locked_chest(node) {
            let record = self.chests.get_or_add(pos);
            if !mine(record.owner.as_deref()) {
                self.tell(addr, keys::not_yours(node));
                return;
            }
            let owner = record.owner.clone().unwrap_or_default();
            let secret = record.secret.get_or_insert(new).clone();
            self.chests.save();
            (secret, owner)
        } else if doors::is_locked(node) {
            let owner = self.door_owners.owner(pos).map(str::to_string);
            if !mine(owner.as_deref()) {
                self.tell(addr, keys::not_yours(node));
                return;
            }
            let Some(secret) = self.door_owners.lock_secret(pos, new) else {
                return;
            };
            (secret, owner.unwrap_or_default())
        } else {
            return;
        };
        let key = keys::key_for(&secret, &owner, node);
        info!("{name} cut a key to the {} at {under:?}", content::description_of(node));
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        let Some(skeleton) = peer.inventory.get(slot).filter(|s| s.id == content::ITEM_SKELETON_KEY) else {
            return;
        };
        if skeleton.count == 1 {
            peer.inventory.set(slot, Some(key));
        } else {
            peer.inventory.take_one(slot);
            if peer.inventory.add(key) > 0 {
                let at = peer.pos_bs.map(|v| v / 10.0);
                self.spawn_item(key, (at, [0.0; 3]));
            }
        }
        self.inventory_changed(addr);
    }

    /// The screwdriver (screwdriver.handler): the node at `under` turned
    /// about its face or axis, as its param2 allows, unless it refuses or
    /// couldn't be dug just now; then the screwdriver wears.
    fn screwdrive(&mut self, addr: SocketAddr, slot: usize, under: [i16; 3], mode: screwdriver::Mode) {
        let pos = (under[0] as i32, under[1] as i32, under[2] as i32);
        let (node, param2) = (self.world.node(pos), self.world.param2(pos));
        let Some(name) = self.peers.get(&addr).map(|p| p.display_name().to_string()) else {
            return;
        };
        let Some(turned) = screwdriver::turns(node) else {
            return;
        };
        let new = match turned {
            content::Turned::Facedir => screwdriver::facedir(param2, mode),
            content::Turned::Wallmounted => {
                let attached = content::group_of(node, "attached_node") != 0;
                let world = &self.world;
                screwdriver::wallmounted(param2, mode, attached, |r| {
                    let d = content::wallmounted_support(r);
                    content::walkable(world.node((pos.0 + d.0, pos.1 + d.1, pos.2 + d.2)))
                })
            }
        };
        if beds::is_bed_bottom(node) {
            // The bed's own on_rotate: both halves, or not at all.
            if !self.turn_bed(pos, node, param2, new) {
                return;
            }
        } else {
            // can_dig: an emptied furnace, chest or shelf; someone else's
            // locked chest or steel trapdoor not at all.
            let can_dig = if content::is_furnace(node) {
                self.furnaces.get(pos).map_or(true, |f| f.is_empty())
            } else if content::is_chest(node) || node == bones::BONES || content::shelf_of(node).is_some() {
                self.chests.get(pos).map_or(true, |c| c.is_empty() && c.allows(&name))
            } else if doors::is_locked(node) {
                self.door_owners.allows(pos, &name)
            } else {
                true
            };
            if !can_dig {
                return;
            }
            if new != param2 {
                info!("{name} turned {} at {under:?} ({param2} to {new})", content::name_of(node));
                self.world.set_node(pos, node, new);
                self.broadcast_node(under, &build_addnode_keeping_meta(under, node, new));
            }
        }
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        let Some(mut tool) = peer.inventory.get(slot) else {
            return;
        };
        let intact = tool.add_wear(content::use_wear(screwdriver::USES, tool.wear));
        peer.inventory.set(slot, intact.then_some(tool));
        if !intact {
            info!("{name}'s screwdriver broke");
            self.play_sound_near(-1, content::TOOL_BREAKS, 0.5, under.map(|v| v as f32 * 10.0), HEARING);
        }
        self.inventory_changed(addr);
    }

    /// beds' on_rotate: the bed turns on its bottom half, if its top is
    /// where it should be, the new turn is a level one, and there is room
    /// for the top where it goes. Returns whether it turned.
    fn turn_bed(&mut self, pos: NodePos, bottom: u16, param2: u8, new: u8) -> bool {
        let old_top = beds::other_half(pos, param2, false);
        let top = self.world.node(old_top);
        if !beds::bed_of(top).is_some_and(|(_, is_top)| is_top) || self.world.param2(old_top) != param2 {
            return false;
        }
        if new % 32 > 3 {
            return false;
        }
        let new_top = beds::other_half(pos, new, false);
        if !content::buildable_to(self.world.node(new_top)) {
            return false;
        }
        let p16 = |p: NodePos| [p.0 as i16, p.1 as i16, p.2 as i16];
        self.world.set_node(old_top, world::CONTENT_AIR, 0);
        self.broadcast_node(p16(old_top), &build_removenode(p16(old_top)));
        self.world.set_node(pos, bottom, new);
        self.broadcast_node(p16(pos), &build_addnode(p16(pos), bottom, new));
        self.world.set_node(new_top, top, new);
        self.broadcast_node(p16(new_top), &build_addnode(p16(new_top), top, new));
        true
    }

    /// minetest_game's grow_sapling: on soil and in daylight it becomes a
    /// tree; otherwise it tries again in 5 minutes.
    fn grow_sapling(&mut self, pos: NodePos) {
        let sapling = self.world.node(pos);
        if !content::is_sapling(sapling) {
            return; // dug meanwhile
        }
        let daytime = (6000..18000).contains(&self.time_of_day());
        let world = &self.world;
        let node = |p: NodePos| world.node(p);
        let sky = world.highest_solid(pos.0, pos.2) < pos.1;
        if !nature::can_grow(pos, &node, sky, daytime) {
            self.timers.start(pos, Job::Grow, nature::quick(nature::GROW_RETRY));
            return;
        }
        let height = nature::tree_height(sapling, &mut self.rng);
        let tree = nature::grow(pos, sapling, height, &node);
        info!("The {} at {pos:?} grew into a tree ({} nodes)", content::name_of(sapling), tree.len());
        for (p, id) in tree {
            let p16 = [p.0 as i16, p.1 as i16, p.2 as i16];
            self.world.set_node(p, id, 0);
            self.broadcast_node(p16, &build_addnode(p16, id, 0));
        }
    }

    /// A line in a player's chat area.
    fn tell(&mut self, addr: SocketAddr, text: &str) {
        self.send_command(addr, 0, &build_chat_message(CHAT_RAW, "", text));
    }

    /// A line in everyone's chat area.
    fn tell_all(&mut self, text: &str) {
        info!("CHAT: {text}");
        self.send_to_players(&build_chat_message(CHAT_RAW, "", text));
    }

    /// Something a player typed in chat: a line for everyone, or a command.
    /// As Luanti does: trimmed, at most CHAT_MAX_CHARS characters, one
    /// line, and at most CHAT_MAX_PER_10S messages in ten seconds.
    fn on_chat(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(text) = parse_chat_message(payload) else {
            return;
        };
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if !peer.ready {
            return;
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let now = Instant::now();
        while peer.chat_times.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(10)) {
            peer.chat_times.pop_front();
        }
        let name = peer.display_name().to_string();
        if peer.chat_times.len() >= CHAT_MAX_PER_10S {
            let reply = format!("You cannot send more messages. You are limited to {CHAT_MAX_PER_10S} messages per 10 seconds.");
            self.tell(from, &reply);
            return;
        }
        peer.chat_times.push_back(now);
        if text.chars().count() > CHAT_MAX_CHARS {
            self.tell(from, "Your message exceeds the maximum chat message length. It was refused. Send a shorter message.");
            return;
        }
        if text.contains(['\r', '\n']) {
            self.tell(from, "Newlines are not permitted in chat messages");
            return;
        }
        if let Some(command) = text.strip_prefix('/') {
            self.chat_command(from, &name, command);
            return;
        }
        if !self.peers.get(&from).is_some_and(|p| p.privs.has("shout")) {
            self.tell(from, "-!- You don't have permission to shout.");
            return;
        }
        self.tell_all(&format!("<{name}> {text}"));
        self.event(format!("{name}: {text}"));
        status::lock(&self.status).chat_line(status::ChatLine { name, text, action: false });
    }

    /// Like Luanti's status line: version, uptime, time of day, players.
    fn status_line(&self) -> String {
        let up = status::lock(&self.status).started.elapsed().as_secs();
        let players: Vec<&str> = self.peers.values().filter(|p| p.ready).map(|p| p.display_name()).collect();
        format!(
            "# Server: {SERVER_NAME}, uptime: {}h {:02}m, time of day: {}, players ({}/{}): {}",
            up / 3600,
            up / 60 % 60,
            self.clock.hh_mm(),
            players.len(),
            self.max_players,
            players.join(", ")
        )
    }

    /// The game's time of day in Luanti's units (0..24000, 6000 = 06:00),
    /// carried on from before a restart (clock.rs).
    fn time_of_day(&self) -> u16 {
        self.clock.time_of_day()
    }

    /// The welcome message, and who else is here, in the chat area.
    fn send_welcome(&mut self, addr: SocketAddr) {
        let mut lines = self.welcome.clone();
        let others: Vec<&str> = self
            .peers
            .iter()
            .filter(|(a, p)| **a != addr && p.ready)
            .map(|(_, p)| p.display_name())
            .collect();
        lines.push(if others.is_empty() {
            "Nobody else is playing right now.".to_string()
        } else {
            format!("Also playing: {}", others.join(", "))
        });
        for line in lines {
            self.send_command(addr, 0, &build_chat_message(CHAT_SYSTEM, "", &line));
        }
    }

    /// Whether the game is full for `name`. A player reconnecting (whose old
    /// connection is still counted until it is replaced) is let in.
    fn user_limit_reached(&self, name: &str) -> bool {
        let others = self
            .peers
            .values()
            .filter(|p| p.ready && !p.display_name().eq_ignore_ascii_case(name))
            .count();
        others >= self.max_players
    }

    /// The texture for a skin key.
    fn skin_texture(&self, key: &str) -> String {
        self.skins.get(self.skins.index_of(key)).texture.clone()
    }

    /// The inventory window, with the skin chooser showing this player's
    /// skin. While the window is open the client redraws it when this
    /// changes.
    fn send_inventory_formspec(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get(&addr) else {
            return;
        };
        let packet = if peer.guide.open {
            build_recipes_formspec(&peer.guide)
        } else {
            let skin = self.skins.get(self.skins.index_of(&peer.skin));
            build_inventory_formspec(&SkinChoice { label: &skin.label, texture: &skin.texture })
        };
        self.send_command(addr, 0, &packet);
    }

    /// Buttons in the inventory window: its tabs, the crafting guide on the
    /// Recipes tab, and the skin chooser's < and > on the Crafting tab.
    fn on_inventory_fields(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some((form, fields)) = parse_inventory_fields(payload) else {
            return;
        };
        if form == beds::FORM {
            self.on_bed_fields(from, &fields);
            return;
        }
        if form == books::FORM {
            self.on_book_fields(from, &fields);
            return;
        }
        let pressed = |name: &str| fields.iter().any(|(f, _)| f == name);
        if form == CHEST_FORM {
            if pressed("quit") {
                self.close_chest_for(from);
            }
            return;
        }
        if form == FURNACE_FORM {
            // The furnace window was closed.
            if pressed("quit") {
                let Some(peer) = self.peers.get_mut(&from) else {
                    return;
                };
                peer.furnace = None;
                // An update sent just before the window closed would open
                // it again: close that too (an empty formspec closes the
                // window only if it is ours, so this is safe).
                if peer.furnace_form_sent.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)) {
                    self.send_command(from, 0, &build_show_formspec("", FURNACE_FORM));
                }
            }
            return;
        }
        if !form.is_empty() {
            return; // someone else's form
        }
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if !peer.ready {
            return;
        }
        // A tab: Crafting or Recipes.
        if let Some((_, tab)) = fields.iter().find(|(f, _)| f == craftguide::TABS) {
            let open = tab == "2";
            if open != peer.guide.open {
                peer.guide.open = open;
                self.send_inventory_formspec(from);
            }
            return;
        }
        if peer.guide.open {
            if craftguide::on_fields(&mut peer.guide, &fields) {
                self.send_inventory_formspec(from);
            }
            return;
        }
        let forward = match (pressed(FIELD_SKIN_PREVIOUS), pressed(FIELD_SKIN_NEXT)) {
            (true, false) => false,
            (false, true) => true,
            _ => return, // closing the window
        };
        let next = self.skins.step(self.skins.index_of(&peer.skin), forward);
        let skin = self.skins.get(next);
        peer.skin = skin.key.clone();
        peer.save(&self.players);
        let (id, name, texture, label, lying, zoom) = (peer.id, peer.display_name().to_string(), skin.texture.clone(), skin.label.clone(), peer.posture(), peer.zoom_fov);
        info!("{name} now wears the {label} skin");
        self.send_inventory_formspec(from);
        // Everyone (the player too, for the third-person view) redraws
        // the model with the new texture.
        let packet = build_object_messages(&[(id, player_properties_message(&texture, lying, zoom))]);
        self.send_to_players(&packet);
    }

    /// Play a sound at a position (BS units) for everyone within `range`
    /// nodes. `id` -1 for a one-off sound; otherwise one that can be faded.
    fn play_sound_near(&mut self, id: i32, sound: &str, gain: f32, pos_bs: [f32; 3], range: f32) {
        let packet = build_play_sound(id, sound, gain, pos_bs);
        let reach = range * 10.0;
        let near: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, p)| p.ready && (0..3).map(|i| (p.pos_bs[i] - pos_bs[i]).powi(2)).sum::<f32>() <= reach * reach)
            .map(|(a, _)| *a)
            .collect();
        for addr in near {
            self.send_command(addr, 0, &packet);
        }
    }

    /// Fade sounds out for everyone (those who never heard them ignore it).
    fn fade_sounds(&mut self, ids: &[i32]) {
        for &id in ids {
            self.send_to_players(&build_fade_sound(id, -1.0, 0.0));
        }
    }

    /// Put an item stack into the world and show it to everyone.
    /// `motion` is position and velocity, in nodes.
    fn spawn_item(&mut self, stack: Stack, motion: ([f32; 3], [f32; 3])) {
        let world = &self.world;
        let solid = |x: i32, y: i32, z: i32| content::walkable(world.node((x, y, z)));
        let (id, evicted) = self.items.add(stack, motion, &solid);
        let Some(d) = self.items.get(id) else {
            return;
        };
        let added = [(id, item_init(d))];
        let removed: Vec<u16> = evicted.into_iter().collect();
        self.send_to_players(&build_remove_add(&removed, &added));
    }

    /// Punching a dropped item: into the inventory, as much as fits.
    fn pick_up(&mut self, from: SocketAddr, id: u16, body: [f32; 3]) {
        let Some(d) = self.items.get(id).cloned() else {
            return; // a player, or an item that has just gone
        };
        let dist: f32 = (0..3).map(|i| (d.pos[i] - body[i]).powi(2)).sum::<f32>().sqrt();
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if dist > MAX_INTERACT_DISTANCE {
            return;
        }
        let left = peer.inventory.add(d.stack());
        let name = peer.display_name().to_string();
        if left == d.count {
            return; // inventory full
        }
        info!("{name} picked up {} {}", d.count - left, content::name_of(d.item));
        if left == 0 {
            self.items.remove(id);
            self.send_to_players(&build_remove_add(&[id], &[]));
        } else {
            self.items.set_count(id, left);
            self.send_item_properties(id);
        }
        self.inventory_changed(from);
    }

    // -------------------------------------------------------------------
    // Boats (boats.rs)
    // -------------------------------------------------------------------

    /// The boat item's on_place: pointed at water, a boat goes on it,
    /// facing the way the player looks, and one boat leaves their hand.
    fn place_boat(&mut self, from: SocketAddr, slot: usize, under: [i16; 3], look_yaw: f32) {
        let water = (under[0] as i32, under[1] as i32, under[2] as i32);
        if content::group_of(self.world.node(water), "water") == 0 {
            self.send_inventory(from);
            return;
        }
        let pos = [under[0] as f32, under[1] as f32 + 0.5, under[2] as f32];
        let name = self.peers.get(&from).map(|p| p.display_name().to_string()).unwrap_or_default();
        let Some(id) = self.boats.add(pos, look_yaw.to_radians()) else {
            self.tell(from, "There are too many boats in the world already.");
            self.send_inventory(from);
            return;
        };
        info!("{name} put a boat on the water at {under:?} ({} boats)", self.boats.len());
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.inventory.take_one(slot);
        }
        self.inventory_changed(from);
        if let Some(b) = self.boats.get(id) {
            let added = build_remove_add(&[], &[(id, boats::init_data(b))]);
            self.send_to_players(&added);
        }
    }

    /// Right-clicking a boat (its on_rightclick): the driver gets out; if
    /// nobody is in it, the player gets in.
    fn board_boat(&mut self, from: SocketAddr, id: u16, body: [f32; 3]) {
        let Some(b) = self.boats.get(id) else {
            return;
        };
        let near = (0..3).map(|i| (b.pos[i] - body[i]).powi(2)).sum::<f32>().sqrt() <= MAX_INTERACT_DISTANCE + 1.0;
        let (driver, boat_yaw) = (b.driver, b.yaw);
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if driver == Some(peer.id) {
            self.leave_boat(from, true);
            return;
        }
        if driver.is_some() || !near || peer.in_bed.is_some() || peer.in_boat.is_some() {
            return;
        }
        let sit = Animation::new(ANIM_SIT, ANIM_FPS);
        peer.in_boat = Some(id);
        peer.anim = sit;
        peer.yaw = boat_yaw.to_degrees();
        let (pid, name, pitch, pos_bs, skin, zoom) = (peer.id, peer.display_name().to_string(), peer.pitch, peer.pos_bs, peer.skin.clone(), peer.zoom_fov);
        if let Some(b) = self.boats.get_mut(id) {
            b.driver = Some(pid);
        }
        info!("{name} gets into boat {id}");
        let texture = self.skin_texture(&skin);
        // Attached, sitting, looking the way the boat faces.
        self.send_to_players(&build_object_messages(&[
            (pid, attach_message(id, boats::SEAT)),
            (pid, set_animation_message(sit)),
            (pid, player_properties_message(&texture, Posture::Sit, zoom)),
        ]));
        self.send_command(from, 0, &build_local_player_animations(false));
        self.send_command(from, 0, &build_move_player(pos_bs, pitch, boat_yaw.to_degrees()));
    }

    /// Out of the boat (right-clicked again, or punched): standing, and,
    /// if `step_off`, a little above where it is. (Dead, or gone, they
    /// stay where they are.) Returns whether they were in one.
    fn leave_boat(&mut self, from: SocketAddr, step_off: bool) -> bool {
        let Some(peer) = self.peers.get_mut(&from) else {
            return false;
        };
        let Some(id) = peer.in_boat.take() else {
            return false;
        };
        let stand = Animation::new(ANIM_STAND, ANIM_FPS);
        peer.anim = stand;
        let (pid, name, pitch, yaw, skin, zoom) = (peer.id, peer.display_name().to_string(), peer.pitch, peer.yaw, peer.skin.clone(), peer.zoom_fov);
        let at = self.boats.get_mut(id).map(|b| {
            b.driver = None;
            b.auto = false;
            b.pos_bs()
        });
        info!("{name} gets out of boat {id}");
        let texture = self.skin_texture(&skin);
        self.send_to_players(&build_object_messages(&[
            (pid, not_attached_message()),
            (pid, set_animation_message(stand)),
            (pid, player_properties_message(&texture, Posture::Stand, zoom)),
        ]));
        self.send_command(from, 0, &build_local_player_animations(true));
        if let (true, Some(at)) = (step_off, at) {
            let pos_bs = [at[0], at[1] + 2.0, at[2]];
            if let Some(peer) = self.peers.get_mut(&from) {
                peer.pos_bs = pos_bs;
            }
            self.send_command(from, 0, &build_move_player(pos_bs, pitch, yaw));
            self.announce_position(from);
        }
        true
    }

    /// Punching a boat (its on_punch): its driver punching it gets out;
    /// with nobody in it, the boat goes into the puncher's inventory (or,
    /// if that is full, is dropped where it was).
    fn punch_boat(&mut self, from: SocketAddr, id: u16, body: [f32; 3]) {
        let Some(b) = self.boats.get(id) else {
            return;
        };
        let near = (0..3).map(|i| (b.pos[i] - body[i]).powi(2)).sum::<f32>().sqrt() <= MAX_INTERACT_DISTANCE + 1.0;
        let (driver, pos) = (b.driver, b.pos);
        let Some(peer) = self.peers.get(&from) else {
            return;
        };
        if driver == Some(peer.id) {
            self.leave_boat(from, false);
        } else if driver.is_some() || !near {
            return;
        }
        self.boats.remove(id);
        self.send_to_players(&build_remove_add(&[id], &[]));
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        let name = peer.display_name().to_string();
        let left = peer.inventory.add(Stack::new(content::ITEM_BOAT, 1));
        info!("{name} took boat {id} ({} boats left)", self.boats.len());
        if left > 0 {
            self.spawn_item(Stack::new(content::ITEM_BOAT, left), (pos, [0.0; 3]));
        }
        self.inventory_changed(from);
    }

    /// Move the boats on, steered by their drivers' keys, and tell clients.
    fn step_boats(&mut self, dt: f32) {
        if self.boats.len() == 0 {
            return;
        }
        let keys: Vec<(u16, u32)> = self.peers.values().filter(|p| p.in_boat.is_some()).map(|p| (p.id, p.keys)).collect();
        let controls = |driver: u16| keys.iter().find(|(id, _)| *id == driver).map(|&(_, k)| boats::Controls::from_keys(k));
        let terrain = MobTerrain(&self.world);
        let events = self.boats.step(dt, &terrain, &controls);
        let mut moved = Vec::new();
        for e in events {
            match e {
                boats::Event::Moved(id) => {
                    if let Some(b) = self.boats.get(id) {
                        moved.push((id, boats::position(b)));
                    }
                }
                boats::Event::Cruise { driver, on } => {
                    let addr = self.peers.iter().find(|(_, p)| p.id == driver).map(|(a, _)| *a);
                    if let Some(addr) = addr {
                        self.tell(addr, if on { "Boat cruise mode on" } else { "Boat cruise mode off" });
                    }
                }
            }
        }
        if !moved.is_empty() {
            self.send_to_players(&build_object_messages(&moved));
        }
    }

    /// The node at `node` changed: items on it may fall.
    fn wake_items(&mut self, node: (i32, i32, i32)) {
        for id in self.items.wake_near([node.0, node.1, node.2]) {
            self.send_item_position(id);
        }
    }

    /// builtin's check_for_falling: sand or gravel at `pos` with nothing
    /// under it comes loose, and so in turn does any stacked on it. The
    /// world loses it at once; players keep seeing it as it falls.
    /// xpanes' on_placenode and on_dignode: the pane at `pos` and those on
    /// its four sides become flat or joined, as their neighbours now say.
    fn update_panes_near(&mut self, pos: NodePos) {
        for (dx, dz) in [(0, 0)].into_iter().chain(panes::SIDES) {
            let p = (pos.0 + dx, pos.1, pos.2 + dz);
            let id = self.world.node(p);
            if !panes::is_pane(id) {
                continue;
            }
            let sides = panes::SIDES.map(|(sx, sz)| self.world.node((p.0 + sx, p.1, p.2 + sz)));
            if let Some((node, param2)) = panes::update(id, self.world.param2(p), sides) {
                self.world.set_node(p, node, param2);
                let p16 = [p.0 as i16, p.1 as i16, p.2 as i16];
                self.broadcast_node(p16, &build_addnode(p16, node, param2));
            }
        }
    }

    fn check_falling(&mut self, mut pos: NodePos) {
        loop {
            let id = self.world.node(pos);
            if !falling::falls(id) || !falling::falls_into(self.world.node((pos.0, pos.1 - 1, pos.2))) {
                return;
            }
            let param2 = self.world.param2(pos);
            self.world.set_node(pos, world::CONTENT_AIR, 0);
            self.falling.push(Falling::new(id, param2, pos));
            let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
            self.wake_items(pos);
            self.drop_attached(p16);
            pos.1 += 1;
        }
    }

    /// Move falling sand and gravel on: shown a node lower as it goes, set
    /// in the world where it lands (or dropped as an item where it can't
    /// be).
    fn step_falling(&mut self, dt: f32) {
        let mut falling = std::mem::take(&mut self.falling);
        falling.retain_mut(|f| {
            let world = &self.world;
            let step = falling::step(f, dt, &|p| world.node(p));
            let shown = |p: NodePos| [p.0 as i16, p.1 as i16, p.2 as i16];
            match step {
                falling::Step::Same => true,
                falling::Step::Moved { from } => {
                    self.show_world_node(from);
                    let at = shown(f.pos);
                    self.broadcast_node(at, &build_addnode(at, f.id, f.param2));
                    true
                }
                falling::Step::Landed => {
                    let at = shown(f.pos);
                    self.world.set_node(f.pos, f.id, f.param2);
                    self.broadcast_node(at, &build_addnode(at, f.id, f.param2));
                    let (sound, gain) = content::place_sound(f.id);
                    self.play_sound_near(-1, sound, gain, at.map(|v| v as f32 * 10.0), HEARING);
                    false
                }
                falling::Step::Dropped => {
                    self.show_world_node(f.pos);
                    let at = shown(f.pos);
                    self.spawn_item(Stack::new(f.id, 1), (at.map(|v| v as f32), [0.0; 3]));
                    false
                }
                falling::Step::Lost => {
                    self.show_world_node(f.pos);
                    false
                }
            }
        });
        // Anything that came loose meanwhile was added to self.falling.
        falling.append(&mut self.falling);
        self.falling = falling;
    }

    /// Show players what the world holds at a node again.
    fn show_world_node(&mut self, pos: NodePos) {
        let at = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        let id = self.world.node(pos);
        if id == world::CONTENT_AIR {
            self.broadcast_node(at, &build_removenode(at));
        } else {
            self.broadcast_node(at, &build_addnode(at, id, self.world.param2(pos)));
        }
    }

    fn step_items(&mut self, dt: f32) {
        let world = &self.world;
        let solid = |x: i32, y: i32, z: i32| content::walkable(world.node((x, y, z)));
        for change in self.items.step(dt, &solid) {
            match change {
                Change::Moved(id) => self.send_item_position(id),
                Change::Resized(id) => self.send_item_properties(id),
                Change::Removed(id) => self.send_to_players(&build_remove_add(&[id], &[])),
            }
        }
    }

    fn send_item_position(&mut self, id: u16) {
        if let Some(d) = self.items.get(id) {
            let msg = build_object_messages(&[(id, item_position_message(d.pos_bs(), d.vel_bs()))]);
            self.send_to_players(&msg);
        }
    }

    fn send_item_properties(&mut self, id: u16) {
        if let Some(d) = self.items.get(id) {
            let init = item_init_of(d);
            let msg = build_object_messages(&[(id, item_properties_message(&init.as_init(id)))]);
            self.send_to_players(&msg);
        }
    }

    // -------------------------------------------------------------------
    // Butterflies and fireflies (insects.rs)
    // -------------------------------------------------------------------

    /// The butterflies' and fireflies' node timers: each near a player
    /// (within INSECT_RANGE map blocks) looks at the light where it is, and
    /// hides or comes out.
    fn tend_insects(&mut self) {
        let players: Vec<BlockPos> = self
            .peers
            .values()
            .filter(|p| p.ready)
            .map(|p| world::block_of_node(((p.pos_bs[0] / 10.0).round() as i32, (p.pos_bs[1] / 10.0).round() as i32, (p.pos_bs[2] / 10.0).round() as i32)))
            .collect();
        let mut columns = BTreeSet::new();
        for b in &players {
            for bx in b.0 - INSECT_RANGE..=b.0 + INSECT_RANGE {
                for bz in b.2 - INSECT_RANGE..=b.2 + INSECT_RANGE {
                    columns.insert((bx, bz));
                }
            }
        }
        let time = self.time_of_day();
        let near = |p: NodePos| {
            let b = world::block_of_node(p);
            players.iter().any(|c| (b.0 - c.0).abs() <= INSECT_RANGE && (b.1 - c.1).abs() <= INSECT_RANGE && (b.2 - c.2).abs() <= INSECT_RANGE)
        };
        let mut turned = Vec::new();
        for (bx, bz) in columns {
            for p in self.world.insects_in(bx, bz) {
                if !near(p) {
                    continue;
                }
                let light = insects::light(self.world.sunlight_at(p), self.world.torch_light_at(p), time);
                if let Some(to) = insects::turns_into(self.world.node(p), light) {
                    turned.push((p, to));
                }
            }
        }
        for (p, to) in turned {
            self.world.set_node(p, to, 0);
            let p16 = [p.0 as i16, p.1 as i16, p.2 as i16];
            self.broadcast_node(p16, &build_addnode(p16, to, 0));
        }
    }

    /// A firefly in a bottle at `pos` is let out above it, and the bottle
    /// is an empty glass bottle again.
    fn release_firefly(&mut self, pos: NodePos) {
        let world = &self.world;
        let Some(to) = insects::release_to(pos, &|p| world.node(p)) else {
            return;
        };
        for (p, id) in [(pos, content::VESSELS_GLASS_BOTTLE), (to, insects::FIREFLY)] {
            self.world.set_node(p, id, 0);
            let p16 = [p.0 as i16, p.1 as i16, p.2 as i16];
            self.broadcast_node(p16, &build_addnode(p16, id, 0));
        }
    }

    // -------------------------------------------------------------------
    // Mobs (mobs.rs)
    // -------------------------------------------------------------------

    /// The players as the mobs see them.
    fn mob_players(&self) -> Vec<mobs::Player> {
        self.peers
            .values()
            .filter(|p| p.ready)
            .map(|p| mobs::Player { id: p.id, pos: p.pos_bs.map(|v| v / 10.0), alive: p.hp > 0 })
            .collect()
    }

    fn step_mobs(&mut self, dt: f32) {
        if self.mobs.is_empty() {
            return;
        }
        let players = self.mob_players();
        let day = mobs::daytime(self.time_of_day());
        let terrain = MobTerrain(&self.world);
        let events = self.mobs.step(dt, &terrain, &players, day, &mut self.rng);
        self.mob_events(events);
    }

    /// Now and then, at night, packs come near players.
    fn spawn_mobs(&mut self) {
        if !self.settings.mobs() || !mobs::dark(self.time_of_day()) {
            return;
        }
        let players = self.mob_players();
        if players.is_empty() {
            return;
        }
        let terrain = MobTerrain(&self.world);
        let events = self.mobs.spawn(&terrain, &players, &mut self.rng);
        if !events.is_empty() {
            info!("{} mobs about", self.mobs.len());
        }
        self.mob_events(events);
    }

    /// Tell clients what the mobs did, and hurt the players they hit.
    fn mob_events(&mut self, events: Vec<mobs::Event>) {
        use mobs::Event;
        let mut messages: Vec<(u16, Vec<u8>)> = Vec::new();
        let mut added = Vec::new();
        let mut removed = Vec::new();
        for e in events {
            match e {
                Event::Added(id) => {
                    if let Some(m) = self.mobs.get(id) {
                        added.push((id, mobs::init_data(m)));
                    }
                }
                Event::Removed(id) => {
                    removed.push(id);
                    self.stop_mob_sounds(id);
                }
                Event::Moved(id) => {
                    if let Some(m) = self.mobs.get(id) {
                        messages.push((id, mobs::position(m)));
                    }
                }
                Event::Animated(id) => {
                    if let Some(m) = self.mobs.get(id) {
                        messages.push((id, mobs::animation_message(m)));
                    }
                }
                Event::Sinking(id) => {
                    // A mossling sinks in a spray of the ground under it.
                    if let Some(feet) = self.mobs.get(id).filter(|m| !m.def().flies).map(|m| m.pos) {
                        let below = (feet[0].round() as i32, (feet[1] - 0.5).round() as i32, feet[2].round() as i32);
                        let ground = self.world.node(below);
                        let p = Particles {
                            amount: 14,
                            time: 1.5,
                            pos: ([feet[0] - 0.3, feet[1], feet[2] - 0.3], [feet[0] + 0.3, feet[1] + 0.1, feet[2] + 0.3]),
                            vel: ([-0.6, 0.8, -0.6], [0.6, 2.0, 0.6]),
                            acc: ([0.0, -GRAVITY, 0.0], [0.0, -GRAVITY, 0.0]),
                            life: (0.4, 0.9),
                            size: (0.0, 0.0),
                            node: content::walkable(ground).then_some((ground, 0)),
                            texture: "tnt_smoke.png",
                            ..Particles::default()
                        };
                        self.particles(feet, &p, None);
                    }
                    if let Some(m) = self.mobs.get(id) {
                        messages.push((id, mobs::properties(m, false)));
                        messages.push((id, mobs::animation_message(m)));
                        if m.def().flies {
                            // A wisp fades as it rises.
                            messages.push((id, texture_mod_message("^[opacity:110")));
                        }
                        messages.push((id, mobs::position(m)));
                    }
                    if let Some(hum) = self.mob_sounds.get_mut(&id).map(|s| std::mem::take(&mut s.0)) {
                        if hum != 0 {
                            self.send_to_players(&build_fade_sound(hum, -0.5, 0.0));
                        }
                    }
                }
                Event::Sound(id, sound, gain, range, pos) => self.mob_sound(id, sound, gain, range, pos),
                Event::Fuse(id, lit) => self.mob_fuse(id, lit),
                Event::Drain(feet) => self.drain_land(feet),
                Event::Head(id) => {
                    if let Some(m) = self.mobs.get(id) {
                        messages.push((id, mobs::head_message(m)));
                    }
                }
                Event::Blink(id, from) => {
                    let to = self.mobs.get(id).map(|m| {
                        messages.push((id, mobs::position(m)));
                        m.pos
                    });
                    // A puff of smoke where it went from, and where it is.
                    for at in std::iter::once(from).chain(to) {
                        self.puff(at);
                    }
                }
                // A puff of smoke: the client's own, for an object punched
                // down to 0 HP. Removed follows.
                Event::Vanish(id) => messages.push((id, punched_message(0))),
                Event::Burst { mob, pos, radius } => {
                    let kills = self.mobs.get(mob).map_or("was burnt up by a wisp", |m| m.def().kills);
                    self.mob_burst(pos, radius, kills);
                }
                Event::Hit { mob: _, player, damage, push, kills } => {
                    let Some(addr) = self.peers.iter().find(|(_, p)| p.id == player && p.ready).map(|(a, _)| *a) else {
                        continue;
                    };
                    self.hurt(addr, damage, kills);
                    // No knockback in a boat (player_api's
                    // calculate_knockback for attached players).
                    if self.peers.get(&addr).is_some_and(|p| p.in_boat.is_none()) {
                        self.send_command(addr, 0, &build_player_speed(push.map(|v| v * 10.0)));
                    }
                }
            }
        }
        // New ones first, then what they all do, then those gone (a hexer's
        // last puff of smoke must come before it goes).
        if !added.is_empty() {
            self.send_to_players(&build_remove_add(&[], &added));
        }
        if !messages.is_empty() {
            self.send_to_players(&build_object_messages(&messages));
        }
        if !removed.is_empty() {
            self.send_to_players(&build_remove_add(&removed, &[]));
        }
        // Hex orbs leave a faint trail.
        for (id, _) in &added {
            if let Some(at) = self.mobs.get(*id).filter(|m| m.kind == mobs::Kind::Orb).map(|m| m.pos) {
                let p = Particles {
                    amount: 40,
                    time: mobs::ORB_LIFE,
                    pos: ([-0.1; 3], [0.1; 3]),
                    vel: ([-0.2; 3], [0.2; 3]),
                    life: (0.3, 0.6),
                    size: (1.0, 1.6),
                    texture: "mobs_hexer_orb.png",
                    glow: 12,
                    attached: *id,
                    ..Particles::default()
                };
                self.particles(at, &p, None);
            }
        }
        // New wisps hum, as long as they are about.
        for (id, _) in &added {
            let Some(m) = self.mobs.get(*id) else {
                continue;
            };
            let (sound, pos_bs) = (m.def().hum, m.pos_bs());
            if sound.is_empty() {
                continue;
            }
            let hum = self.new_sound_id();
            self.mob_sounds.insert(*id, (hum, 0));
            self.send_to_players(&build_play_sound_object(hum, sound, mobs::HUM_GAIN, pos_bs, *id, true));
        }
    }

    /// A sound from a mob, following it (or at `pos`, nodes, where it
    /// was), for players within `range`.
    fn mob_sound(&mut self, id: u16, sound: &str, gain: f32, range: f32, pos: [f32; 3]) {
        if sound.is_empty() {
            return;
        }
        let pos_bs = self.mobs.get(id).map_or(pos.map(|v| v * 10.0), |m| m.pos_bs());
        let packet = build_play_sound_object(-1, sound, gain, pos_bs, id, false);
        let reach = range * 10.0;
        let near: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, p)| p.ready && (0..3).map(|i| (p.pos_bs[i] - pos_bs[i]).powi(2)).sum::<f32>() <= reach * reach)
            .map(|(a, _)| *a)
            .collect();
        for addr in near {
            self.send_command(addr, 0, &packet);
        }
    }

    /// A player punches a mob, with what is in hand: damage as Luanti
    /// works it out from the tool, wear on the tool, and at 0 HP the mob
    /// is gone and leaves its drops.
    fn punch_mob(&mut self, from: SocketAddr, id: u16, slot: usize, body: [f32; 3]) {
        let Some(pos) = self.mobs.get(id).map(|m| m.pos) else {
            return;
        };
        let dist: f32 = (0..3).map(|i| (pos[i] - body[i]).powi(2)).sum::<f32>().sqrt();
        let Some(peer) = self.peers.get(&from) else {
            return;
        };
        if dist > MAX_INTERACT_DISTANCE + 1.0 {
            return;
        }
        let peer_id = peer.id;
        let feet = peer.pos_bs.map(|v| v / 10.0);
        let wielded = peer.inventory.get(slot).filter(|_| slot < HOTBAR_SIZE);
        let caps = content::caps_of(wielded.map(|s| s.id));
        let since = self.mobs.since_punch(peer_id);
        let (damage, wear) = mobs::hit_params(caps, since, wielded.map_or(0, |s| s.wear));
        if wear > 0 {
            if let Some(mut tool) = wielded {
                let intact = tool.add_wear(wear);
                if let Some(peer) = self.peers.get_mut(&from) {
                    peer.inventory.set(slot, intact.then_some(tool));
                }
                if !intact {
                    self.play_sound_near(-1, content::TOOL_BREAKS, 0.5, body.map(|v| v * 10.0), HEARING);
                }
                self.inventory_changed(from);
            }
        }
        if damage == 0 {
            return;
        }
        let Some(hit) = self.mobs.punch(id, peer_id, feet, damage) else {
            return;
        };
        let d = mobs::def(hit.kind);
        if self.mob_hit(id, hit) {
            if let Some(name) = self.peers.get(&from).map(|p| p.display_name().to_string()) {
                info!("{name} killed a {}", d.description);
            }
        }
    }

    /// Tell clients a mob was hit: its HP, and where it is thrown; or,
    /// dead, it is gone and leaves its drops. Returns whether it died.
    fn mob_hit(&mut self, id: u16, hit: mobs::Punched) -> bool {
        let d = mobs::def(hit.kind);
        self.send_to_players(&build_object_messages(&[(id, punched_message(hit.hp))]));
        if hit.hp > 0 {
            self.mob_sound(id, d.hurt, 0.8, 16.0, hit.pos);
            if let Some(m) = self.mobs.get(id) {
                let msg = build_object_messages(&[(id, mobs::position(m))]);
                self.send_to_players(&msg);
            }
            return false;
        }
        let at = hit.pos.map(|v| v * 10.0);
        self.play_sound_near(-1, d.death, 0.8, at, 16.0);
        self.send_to_players(&build_remove_add(&[id], &[]));
        self.stop_mob_sounds(id);
        let (item, lo, hi) = d.drops;
        if item == 0 {
            return true;
        }
        let count = self.rng.range(lo, hi) as u16;
        let drop_at = [hit.pos[0], hit.pos[1] + 0.5, hit.pos[2]];
        let vel = [(self.rng.range(0, 20) as f32 - 10.0) / 10.0, 2.0, (self.rng.range(0, 20) as f32 - 10.0) / 10.0];
        self.spawn_item(Stack::new(item, count), (drop_at, vel));
        true
    }

    fn new_sound_id(&mut self) -> i32 {
        let id = self.next_sound_id;
        self.next_sound_id = if id == i32::MAX { 1 } else { id + 1 };
        id
    }

    /// A mob has gone: its hum and whine stop.
    fn stop_mob_sounds(&mut self, id: u16) {
        self.mob_sparks.remove(&id);
        if let Some((hum, whine)) = self.mob_sounds.remove(&id) {
            for s in [hum, whine] {
                if s != 0 {
                    self.send_to_players(&build_stop_sound(s));
                }
            }
        }
    }

    /// A wisp's fuse is lit: the whine (as long as the fuse) starts; or
    /// put out: it stops.
    fn mob_fuse(&mut self, id: u16, lit: bool) {
        let old = self.mob_sounds.get_mut(&id).map_or(0, |s| std::mem::take(&mut s.1));
        if old != 0 {
            self.send_to_players(&build_stop_sound(old));
        }
        if let Some(sparks) = self.mob_sparks.remove(&id) {
            self.send_to_players(&build_delete_particle_spawner(sparks));
        }
        if !lit {
            return;
        }
        // Sparks fly off it as the fuse burns.
        if let Some(at) = self.mobs.get(id).map(|m| m.pos) {
            let p = Particles {
                amount: 36,
                time: mobs::def(mobs::Kind::Wisp).fuse,
                pos: ([-0.2; 3], [0.2; 3]),
                vel: ([-1.5, -0.5, -1.5], [1.5, 2.0, 1.5]),
                acc: ([0.0, -2.0, 0.0], [0.0, -2.0, 0.0]),
                life: (0.2, 0.5),
                size: (0.4, 0.9),
                texture: "mobs_wisp_spark.png",
                glow: 14,
                attached: id,
                ..Particles::default()
            };
            self.mob_sparks.insert(id, self.next_spawner);
            self.particles(at, &p, None);
        }
        let Some((sound, pos_bs)) = self.mobs.get(id).map(|m| (m.def().whine, m.pos_bs())) else {
            return;
        };
        let whine = self.new_sound_id();
        self.mob_sounds.entry(id).or_insert((0, 0)).1 = whine;
        let packet = build_play_sound_object(whine, sound, 1.0, pos_bs, id, false);
        let near: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, p)| p.ready && (0..3).map(|i| (p.pos_bs[i] - pos_bs[i]).powi(2)).sum::<f32>() <= 320.0 * 320.0)
            .map(|(a, _)| *a)
            .collect();
        for addr in near {
            self.send_command(addr, 0, &packet);
        }
    }

    /// A wisp bursts: a small blast, like TNT's. With TNT off, or another
    /// wisp's blast less than a second ago (for the board's sake), it
    /// only hurts and throws.
    fn mob_burst(&mut self, pos: [f32; 3], radius: i32, kills: &str) {
        let center = ((pos[0]).round() as i32, (pos[1]).round() as i32, (pos[2]).round() as i32);
        let recent = self.mob_blasted.is_some_and(|t| t.elapsed() < Duration::from_secs(1));
        if self.settings.tnt() && !recent {
            self.mob_blasted = Some(Instant::now());
            self.blast(center, radius, 0, &format!("A wisp burst at {center:?}"), kills);
        } else {
            let at = pos.map(|v| v * 10.0);
            self.play_sound_near(-1, "tnt_explode", 1.5, at, 64.0);
            self.blast_effects(center, radius, None);
            self.blast_physics(center, radius, kills);
        }
    }

    /// Remove every mob (mobs turned off).
    fn clear_mobs(&mut self) {
        let ids = self.mobs.clear();
        if !ids.is_empty() {
            self.send_to_players(&build_remove_add(&ids, &[]));
        }
    }

    /// A hexer draws the life from the ground about its feet: up to
    /// DRAIN_NODES nodes within 2 of `feet` (at its level, a node up or
    /// down) die: grass to dirt, plants on it to dry shrubs.
    fn drain_land(&mut self, feet: NodePos) {
        let mut left = DRAIN_NODES;
        for _ in 0..16 {
            if left == 0 {
                break;
            }
            let (dx, dz) = (self.rng.range(0, 4) as i32 - 2, self.rng.range(0, 4) as i32 - 2);
            for dy in [0, 1, -1, 2] {
                let p = (feet.0 + dx, feet.1 + dy, feet.2 + dz);
                let id = self.world.node(p);
                let Some((to, param2)) = mobs::drained(id) else {
                    continue;
                };
                self.world.set_node(p, to, param2);
                let p16 = [p.0 as i16, p.1 as i16, p.2 as i16];
                self.broadcast_node(p16, &build_addnode(p16, to, param2));
                left -= 1;
                break;
            }
        }
    }

    /// A pack in front of a player (/mobs spawn). Returns how many came.
    fn spawn_mobs_near(&mut self, name: &str, kind: mobs::Kind) -> usize {
        let Some(peer) = self.peers.values().find(|p| p.ready && p.display_name() == name) else {
            return 0;
        };
        let (pos, yaw) = (peer.pos_bs.map(|v| v / 10.0), peer.yaw.to_radians());
        let dir = [-yaw.sin(), yaw.cos()];
        let terrain = MobTerrain(&self.world);
        let events = self.mobs.spawn_near(kind, &terrain, pos, dir, &mut self.rng);
        let count = events.len();
        self.mob_events(events);
        count
    }

    // -------------------------------------------------------------------
    // Particles
    // -------------------------------------------------------------------

    /// Particles at `at` (nodes), for players within PARTICLE_RANGE nodes
    /// of it (as Luanti's server sends short-lived ones to players within
    /// its block send distance), but not `except`. Only clients that know
    /// the 5.6 form (protocol 42) get them.
    fn particles(&mut self, at: [f32; 3], p: &Particles, except: Option<SocketAddr>) {
        let id = self.next_spawner;
        self.next_spawner = self.next_spawner.wrapping_add(1).max(1);
        let packet = build_particle_spawner(id, p);
        let reach = PARTICLE_RANGE * 10.0;
        let at_bs = at.map(|v| v * 10.0);
        let near: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(a, p)| p.ready && p.proto >= 42 && Some(**a) != except)
            .filter(|(_, p)| (0..3).map(|i| (p.pos_bs[i] - at_bs[i]).powi(2)).sum::<f32>() <= reach * reach)
            .map(|(a, _)| *a)
            .collect();
        for addr in near {
            self.send_command(addr, 0, &packet);
        }
    }

    /// A puff of smoke round a body (a hexer blinking), feet at `at`.
    fn puff(&mut self, at: [f32; 3]) {
        let p = Particles {
            amount: 16,
            time: 0.1,
            pos: ([at[0] - 0.4, at[1], at[2] - 0.4], [at[0] + 0.4, at[1] + 1.8, at[2] + 0.4]),
            vel: ([-0.5, 0.3, -0.5], [0.5, 1.2, 0.5]),
            life: (0.8, 1.5),
            size: (2.0, 4.0),
            texture: "tnt_smoke.png",
            ..Particles::default()
        };
        self.particles(at, &p, None);
    }

    /// Bits of a node flying off as it goes: as builtin's handle_node_drops
    /// shows other players a node being dug (`dug`), or as leaves decay.
    fn node_bits(&mut self, pos: NodePos, node: u16, param2: u8, dug: Option<SocketAddr>) {
        let c = [pos.0 as f32, pos.1 as f32, pos.2 as f32];
        let p = if dug.is_some() {
            Particles {
                amount: 16,
                pos: (c.map(|v| v - 0.25), c.map(|v| v + 0.25)),
                vel: ([-1.5, 0.0, -1.5], [1.5, 3.0, 1.5]),
                acc: ([0.0, -GRAVITY, 0.0], [0.0, -GRAVITY, 0.0]),
                life: (0.0, 1.0),
                size: (0.0, 0.0), // random
                node: Some((node, param2)),
                ..Particles::default()
            }
        } else {
            Particles {
                amount: 8,
                pos: (c.map(|v| v - 0.5), c.map(|v| v + 0.5)),
                vel: ([-0.5, -1.0, -0.5], [0.5, 0.0, 0.5]),
                acc: ([0.0, -GRAVITY, 0.0], [0.0, -GRAVITY, 0.0]),
                size: (0.0, 0.0),
                node: Some((node, param2)),
                ..Particles::default()
            }
        };
        self.particles(c, &p, dug);
    }

    /// minetest_game's item_entity burn_up: a dropped item burning up goes
    /// in a puff of smoke.
    fn item_smoke(&mut self, at: [f32; 3]) {
        let p = Particles {
            amount: 3,
            time: 0.1,
            pos: (at.map(|v| v - 0.1), at.map(|v| v + 0.1)),
            vel: ([0.0, 2.5, 0.0], [0.0, 2.5, 0.0]),
            acc: ([-0.15, -0.02, -0.15], [0.15, -0.01, 0.15]),
            life: (4.0, 6.0),
            size: (5.0, 5.0),
            collide: true,
            texture: "default_item_smoke.png",
            ..Particles::default()
        };
        self.particles(at, &p, None);
    }

    /// The tnt mod's add_effects: a flash, a cloud of smoke, and bits of
    /// what was blown up (`node`, the most of it) flying out and falling.
    fn blast_effects(&mut self, center: NodePos, radius: i32, node: Option<u16>) {
        let c = [center.0 as f32, center.1 as f32, center.2 as f32];
        let r = radius as f32;
        let flash = Particles { life: (0.4, 0.4), size: (r * 10.0, r * 10.0), texture: "tnt_boom.png", glow: 15, pos: (c, c), ..Particles::default() };
        self.particles(c, &flash, None);
        let half = r / 2.0;
        let smoke = Particles {
            amount: 64,
            time: 0.5,
            pos: (c.map(|v| v - half), c.map(|v| v + half)),
            vel: ([-10.0; 3], [10.0; 3]),
            life: (1.0, 2.5),
            size: (r * 3.0, r * 5.0),
            texture: "tnt_smoke.png",
            ..Particles::default()
        };
        self.particles(c, &smoke, None);
        let debris = Particles {
            amount: 64,
            time: 0.1,
            pos: (c.map(|v| v - half), c.map(|v| v + half)),
            vel: ([-3.0, 0.0, -3.0], [3.0, 5.0, 3.0]),
            acc: ([0.0, -10.0, 0.0], [0.0, -10.0, 0.0]),
            life: (0.8, 2.0),
            size: (r * 0.33, r),
            collide: true,
            texture: "tnt_blast.png",
            node: node.map(|n| (n, 0)),
            ..Particles::default()
        };
        self.particles(c, &debris, None);
    }

    /// Send a command reliably to every player in the game.
    fn send_to_players(&mut self, packet: &[u8]) {
        let ready: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.ready).map(|(a, _)| *a).collect();
        for addr in ready {
            self.send_command(addr, 0, packet);
        }
    }

    /// Where a new (or respawning) player appears: just above the ground
    /// near node (8, ?, 8), a few nodes apart so players don't arrive inside
    /// each other. A position is the player's feet; a node's top face is 0.5
    /// above its centre.
    fn spawn_point(&self, peer_id: u16) -> [f32; 3] {
        let slot = (peer_id as i32 - 2).rem_euclid(4);
        let (sx, sz) = (8 + slot * 3, 8);
        let ground = self.world.highest_solid(sx, sz);
        [sx as f32, ground as f32 + 0.6, sz as f32]
    }

    /// A saved position, moved up onto the ground if it is now inside solid
    /// nodes (the world may have changed since). The feet of a player
    /// standing on a node are exactly on its top face, half a node above
    /// its centre, so the node checked is the one just above that point:
    /// rounding the feet themselves would pick the floor below ground
    /// level (-9.5 rounds to -10), and send anyone underground back to the
    /// surface.
    fn safe_position(&self, pos: [f32; 3]) -> [f32; 3] {
        let (x, y, z) = (pos[0].round() as i32, (pos[1] + 0.1).round() as i32, pos[2].round() as i32);
        let free = |y: i32| !content::walkable(self.world.node((x, y, z)));
        if free(y) && free(y + 1) {
            pos
        } else {
            [pos[0], self.world.highest_solid(x, z) as f32 + 0.6, pos[2]]
        }
    }

    /// Fall damage, worked out by the client.
    fn on_damage(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(damage) = parse_damage(payload) else {
            return;
        };
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if !peer.ready || peer.hp == 0 || damage == 0 {
            return;
        }
        self.hurt(from, damage, "fell to their death");
    }

    /// A player loses `damage` HP; at 0 they die, and the event says how
    /// (`death`, after their name).
    fn hurt(&mut self, from: SocketAddr, damage: u16, death: &str) {
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if peer.hp == 0 {
            return;
        }
        peer.hp = peer.hp.saturating_sub(damage);
        let (hp, name) = (peer.hp, peer.display_name().to_string());
        info!("{name} took {damage} damage, {hp} HP left");
        self.send_command(from, 0, &build_hp(hp, true));
        self.send_command(from, 0, &build_health_bar_change(hp));
        if hp == 0 {
            info!("{name} died");
            self.leave_bones(from);
            // Dead, they get up where they lie (beds' on_dieplayer), and
            // out of any boat (Luanti detaches the dead).
            self.get_up(from, false);
            self.leave_boat(from, false);
            self.event(format!("{name} {death}"));
            self.send_command(from, 0, &build_deathscreen());
            if let Some(peer) = self.peers.get_mut(&from) {
                peer.save(&self.players);
            }
        }
    }

    /// The Respawn button on the death screen.
    fn on_respawn(&mut self, from: SocketAddr) {
        let Some(peer) = self.peers.get(&from) else {
            return;
        };
        if !peer.ready || peer.hp != 0 {
            return; // only the dead respawn
        }
        let name = peer.display_name().to_string();
        // At the bed they last used, standing on it, if it is still there
        // (beds' register_on_spawn).
        let spawn = match self.bed_spawns.get(&name) {
            Some(b) => [b.0 as f32, b.1 as f32 + 0.07, b.2 as f32],
            None => self.spawn_point(peer.id),
        };
        let spawn_bs = spawn.map(|v| v * 10.0);
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.hp = PLAYER_MAX_HP;
            peer.breath = PLAYER_MAX_BREATH;
            peer.pos_bs = spawn_bs;
            peer.save(&self.players);
        }
        info!("{name} respawns at ({:.1}, {:.1}, {:.1})", spawn[0], spawn[1], spawn[2]);
        self.send_command(from, 0, &build_hp(PLAYER_MAX_HP, false));
        self.send_command(from, 0, &build_health_bar_change(PLAYER_MAX_HP));
        self.send_breath(from);
        self.send_command(from, 0, &build_move_player(spawn_bs, 0.0, 0.0));
        self.plan_blocks(from, spawn, MAX_VIEW_RADIUS);
        self.announce_position(from);
    }

    /// Rearranging items in the inventory window. The client has already
    /// shown the move; whatever we decide, we send the real inventory back.
    fn on_inventory_action(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if !peer.ready {
            return;
        }
        if !peer.privs.has("interact") {
            // Luanti ignores inventory actions without "interact"; the
            // client is sent its inventory as it really is.
            info!("{} can't move items without 'interact' privilege", peer.display_name());
            self.send_inventory(from);
            return;
        }
        let text = String::from_utf8_lossy(&payload[2..]).into_owned();
        let name = peer.display_name().to_string();
        let open_furnace = peer.furnace;
        let open_chest = peer.chest;
        // A locked chest's contents only for its owner, or while its key is
        // in hand (the chest's allow_metadata_inventory_* callbacks).
        let key = keys::secret_of(peer.inventory.get(peer.wield).filter(|_| peer.wield < HOTBAR_SIZE));
        let throw = drop_motion(peer.pos_bs, peer.yaw, peer.pitch);
        // Which list an action names: the player's own (not "craftpreview":
        // taking from it is a Craft action, and nothing can be put into it,
        // as Luanti's serverpackethandler has it), or a list of the furnace
        // they have open.
        let furnaces = &self.furnaces;
        let chests = &self.chests;
        let place = |loc: &str, list: &str| -> Option<Place> {
            if inventory::is_own(loc, &name) {
                return List::from_name(list).map(Place::Own);
            }
            if let Some(pos) = chest::pos_of_location(loc) {
                let ok = open_chest == Some(pos) && chests.get(pos).is_some_and(|c| c.allows_with(&name, key.as_deref())) && list == "main";
                return ok.then_some(Place::Chest(pos));
            }
            let pos = furnace::pos_of_location(loc).filter(|&p| open_furnace == Some(p) && furnaces.get(p).is_some())?;
            FurnaceList::from_name(list).map(|l| Place::Furnace(pos, l))
        };
        let action = inventory::parse_action(&text);
        let places = match &action {
            Some(Action::Move { from: (fi, fl, _), to: (ti, tl, _), .. }) => (place(fi, fl), place(ti, tl)),
            Some(Action::MoveSomewhere { from: (fi, fl, _), to: (ti, tl), .. }) => (place(fi, fl), place(ti, tl)),
            Some(Action::Drop { from: (fi, fl, _), .. }) => (place(fi, fl), None),
            Some(Action::Craft { inv, .. }) => (inventory::is_own(inv, &name).then_some(Place::Own(List::Craft)), None),
            _ => (None, None),
        };
        let Self { peers, furnaces, chests, .. } = self;
        let Some(peer) = peers.get_mut(&from) else {
            return;
        };
        let mut lists = Lists { player: &mut peer.inventory, furnaces, chests };
        let mut dropped = None;
        let ok = match (action, places) {
            (Some(Action::Move { count, from: (_, _, fs), to: (_, _, ts) }), (Some(f), Some(t))) => {
                lists.move_items((f, fs), (t, ts), count)
            }
            (Some(Action::MoveSomewhere { count, from: (_, _, fs), .. }), (Some(f), Some(t))) => {
                lists.move_somewhere((f, fs), t, count)
            }
            (Some(Action::Craft { count, .. }), (Some(_), _)) => match lists.player.craft(count) {
                Some(made) => {
                    info!("{name} crafted {}", made.item_string());
                    true
                }
                None => false,
            },
            (Some(Action::Drop { count, from: (_, _, fs), .. }), (Some(f), _)) => {
                let stack = lists.take((f, fs), count);
                if let Some(stack) = stack {
                    info!("{name} dropped {}", stack.item_string());
                    dropped = Some((stack, throw));
                }
                stack.is_some()
            }
            (Some(Action::Other(kind)), _) => {
                info!("{name}: inventory action {kind} isn't supported");
                false
            }
            _ => {
                info!("{name}: refused inventory action \"{text}\"");
                false
            }
        };
        let furnace_used = [places.0, places.1].into_iter().flatten().find_map(|p| match p {
            Place::Furnace(pos, _) => Some(pos),
            _ => None,
        });
        let chest_used = [places.0, places.1].into_iter().flatten().find_map(|p| match p {
            Place::Chest(pos) => Some(pos),
            _ => None,
        });
        if let Some(pos) = chest_used {
            if ok {
                self.chests.save();
            }
            self.send_chest(pos);
            if ok && self.chests.get(pos).is_some_and(|c| c.shelf.is_some()) {
                self.update_shelf(pos);
            }
            // Bones vanish when the last thing is taken out, and the taker
            // gets the bones block (on_metadata_inventory_take).
            if ok && self.chests.get(pos).is_some_and(|c| c.bones && c.is_empty()) && self.world.node(pos) == bones::BONES {
                self.remove_bones(pos, from);
            }
        }
        if let Some((stack, motion)) = dropped {
            self.spawn_item(stack, motion);
        }
        if ok {
            self.inventory_changed(from);
        } else {
            self.send_inventory(from); // undo the client's prediction
        }
        // The client doesn't predict moves into or out of a detached
        // inventory, so it always needs the furnace's contents back.
        if let Some(pos) = furnace_used {
            if ok {
                // minetest_game starts the furnace's timer on any change.
                if let Some(f) = self.furnaces.get_mut(pos) {
                    f.running = true;
                }
                self.furnaces.save();
            }
            self.send_furnace(pos);
        }
    }

    /// Open a chest for a player (minetest_game's on_rightclick): a locked
    /// chest only for its owner. The open sound plays and, unless a block
    /// sits on it, the lid goes up while anyone has it open.
    fn open_chest(&mut self, addr: SocketAddr, pos: NodePos) {
        let node = self.world.node(pos);
        let Some((closed, open)) = content::chest_pair(node) else {
            return;
        };
        let Some(name) = self.peers.get(&addr).map(|p| p.display_name().to_string()) else {
            return;
        };
        self.fill_dungeon_chest(pos);
        let key = self.wielded_key(addr);
        let record = self.chests.get_or_add(pos);
        if !record.allows_with(&name, key.as_deref()) {
            let owner = record.owner.clone().unwrap_or_default();
            self.tell(addr, &format!("This chest is locked; it belongs to {owner}."));
            return;
        }
        // One chest at a time: a chest they had open closes.
        if let Some(old) = self.peers.get_mut(&addr).and_then(|p| p.chest.take()) {
            self.close_unwatched_lids_at(old);
        }
        if let Some(peer) = self.peers.get_mut(&addr) {
            peer.chest = Some(pos);
        }
        info!("{name} opened the chest at {pos:?}");
        let at = [pos.0 as f32 * 10.0, pos.1 as f32 * 10.0, pos.2 as f32 * 10.0];
        self.play_sound_near(-1, "default_chest_open", 0.3, at, CHEST_HEARING);
        let above = self.world.node((pos.0, pos.1 + 1, pos.2));
        if node == closed && !content::lid_obstructed(above) {
            let param2 = self.world.param2(pos);
            self.swap_node(pos, open, param2);
            self.open_chests.insert(pos);
        }
        if let Some(packet) = self.chest_packet(pos) {
            self.send_command(addr, 0, &packet);
        }
        let form = chest_formspec(&chest::detached_name(pos));
        self.send_command(addr, 0, &build_show_formspec(&form, CHEST_FORM));
    }

    /// A dungeon's chest gets its loot the first time it is opened or
    /// dug (dungeon_loot fills it as the map is generated).
    fn fill_dungeon_chest(&mut self, pos: NodePos) {
        if self.chests.get(pos).is_some() || !self.world.is_dungeon_chest(pos) {
            return;
        }
        self.chests.get_or_add(pos).main = dungeons::loot_for(pos, dungeons::kind_at(pos));
        self.chests.save();
        info!("Filled the dungeon chest at {pos:?}");
    }

    /// A player closed their chest window.
    fn close_chest_for(&mut self, addr: SocketAddr) {
        if let Some(pos) = self.peers.get_mut(&addr).and_then(|p| p.chest.take()) {
            self.close_unwatched_lids_at(pos);
        }
    }

    /// Lower the lid of the chest at `pos` if nobody has it open any more,
    /// with the closing sound (minetest_game's chest_lid_close).
    fn close_unwatched_lids_at(&mut self, pos: NodePos) {
        if self.peers.values().any(|p| p.chest == Some(pos)) || !self.open_chests.remove(&pos) {
            return;
        }
        let node = self.world.node(pos);
        if let Some((closed, open)) = content::chest_pair(node) {
            if node == open {
                let param2 = self.world.param2(pos);
                self.swap_node(pos, closed, param2);
                let at = [pos.0 as f32 * 10.0, pos.1 as f32 * 10.0, pos.2 as f32 * 10.0];
                self.play_sound_near(-1, "default_chest_close", 0.3, at, CHEST_HEARING);
            }
        }
    }

    /// Players left: lower the lids nobody is watching any more.
    fn close_unwatched_lids(&mut self) {
        let open: Vec<NodePos> = self.open_chests.iter().copied().collect();
        for pos in open {
            self.close_unwatched_lids_at(pos);
        }
    }

    /// A player died: what they carried goes into bones where they fell
    /// (bones' register_on_dieplayer), or onto the ground if there is
    /// nowhere to put them.
    fn leave_bones(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        let stacks: Vec<Stack> = peer.inventory.main.iter().chain(peer.inventory.craft.iter()).flatten().copied().collect();
        let name = peer.display_name().to_string();
        let feet = peer.pos_bs.map(|v| (v / 10.0).round() as i32);
        let feet = (feet[0], feet[1], feet[2]);
        let facing = doors::facedir_of(look_dir(0.0, peer.yaw));
        if stacks.is_empty() {
            info!("{name} dies at {feet:?}. No bones placed");
            return;
        }
        for list in [List::Main, List::Craft, List::CraftResult] {
            peer.inventory.list_mut(list).fill(None);
        }
        self.inventory_changed(addr);
        let world = &self.world;
        let chests = &self.chests;
        // Not into a chest, bones or furnace that happens to be there.
        let spot = bones::spot(feet, &|p| if chests.get(p).is_some() { content::CONTENT_STONE } else { world.node(p) });
        let at = |p: NodePos| [p.0 as f32, p.1 as f32, p.2 as f32];
        let Some(pos) = spot else {
            info!("{name} dies at {feet:?}. Inventory dropped");
            for stack in stacks.into_iter().chain([Stack::new(bones::BONES, 1)]) {
                self.drop_from_bones(at(feet), stack);
            }
            return;
        };
        let (main, left) = bones::pack(stacks);
        let share_at = self.clock.game_time() + bones::SHARE_TIME;
        self.chests.insert(pos, chest::Chest { main, owner: Some(name.clone()), bones: true, share_at: Some(share_at), shelf: None, secret: None });
        self.world.set_node(pos, bones::BONES, facing);
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.broadcast_node(p16, &build_addnode(p16, bones::BONES, facing));
        self.wake_items(pos);
        info!("{name} dies at {feet:?}. Bones placed at {pos:?}");
        self.event(format!("{name}'s bones"));
        for stack in left {
            self.drop_from_bones(at(pos), stack);
        }
    }

    /// An item thrown up out of bones (bones' drop): straight up at 5
    /// nodes a second, and a little to one side.
    fn drop_from_bones(&mut self, pos: [f32; 3], stack: Stack) {
        let mut side = || (self.rng.next() % 21) as f32 / 9.0 - 10.0 / 9.0;
        let vel = [side(), 5.0, side()];
        self.spawn_item(stack, (pos, vel));
    }

    /// Right-clicking bones: their window (like a chest's, without the
    /// lid). Fresh bones only for whoever died there.
    fn open_bones(&mut self, addr: SocketAddr, pos: NodePos) {
        let Some(name) = self.peers.get(&addr).map(|p| p.display_name().to_string()) else {
            return;
        };
        let Some(record) = self.chests.get(pos).filter(|c| c.bones) else {
            return; // bones placed by hand hold nothing
        };
        if !record.allows(&name) {
            let owner = record.owner.clone().unwrap_or_default();
            self.tell(addr, &format!("These are {owner}'s fresh bones."));
            return;
        }
        if let Some(old) = self.peers.get_mut(&addr).and_then(|p| p.chest.replace(pos)) {
            self.close_unwatched_lids_at(old);
        }
        info!("{name} opened the bones at {pos:?}");
        if let Some(packet) = self.chest_packet(pos) {
            self.send_command(addr, 0, &packet);
        }
        let form = chest_formspec(&chest::detached_name(pos));
        self.send_command(addr, 0, &build_show_formspec(&form, CHEST_FORM));
    }

    /// The book window sent its fields: save what was written, turn a
    /// page, or change tab.
    fn on_book_fields(&mut self, from: SocketAddr, fields: &[(String, String)]) {
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        let Some(slot) = peer.book_slot else {
            return;
        };
        let name = peer.display_name().to_string();
        let reply = books::on_fields(peer.inventory.get(slot), &name, fields, peer.book_tab);
        if let Some(tab) = reply.tab {
            peer.book_tab = tab;
        }
        if reply.done {
            peer.book_slot = None;
        }
        if let Some(stack) = reply.stack {
            peer.inventory.main[slot] = stack;
        }
        let mut dropped = None;
        if let Some(book) = reply.extra {
            if peer.inventory.add(book) > 0 {
                dropped = Some(book);
            }
        }
        if let Some(stack) = reply.stack.flatten().filter(|s| s.id == content::ITEM_BOOK_WRITTEN).or(reply.extra) {
            if fields.iter().any(|(k, _)| k == "save") {
                info!("{name} wrote the book {:?}", stack.meta.get("title").unwrap_or_default());
            }
        }
        if let Some(book) = dropped {
            let at = self.position(from);
            self.spawn_item(book, ([at[0], at[1] + 1.0, at[2]], [0.0; 3]));
        }
        if let Some(message) = reply.message {
            self.tell(from, message);
        }
        if reply.stack.is_some() || reply.extra.is_some() {
            self.inventory_changed(from);
        }
        if let Some(form) = reply.show {
            self.send_command(from, 0, &build_show_formspec(&form, books::FORM));
        }
    }

    /// Right-clicking a bookshelf or vessels shelf: its window.
    fn open_shelf(&mut self, addr: SocketAddr, pos: NodePos) {
        let Some(name) = self.peers.get(&addr).map(|p| p.display_name().to_string()) else {
            return;
        };
        let kind = content::shelf_of(self.world.node(pos));
        let record = self.chests.get_or_add(pos);
        if record.shelf != kind {
            record.shelf = kind;
            self.chests.save();
        }
        if let Some(old) = self.peers.get_mut(&addr).and_then(|p| p.chest.replace(pos)) {
            self.close_unwatched_lids_at(old);
        }
        info!("{name} opened the {} at {pos:?}", content::name_of(self.world.node(pos)));
        if let Some(packet) = self.chest_packet(pos) {
            self.send_command(addr, 0, &packet);
        }
        let form = self.shelf_form(pos);
        self.send_command(addr, 0, &build_show_formspec(&form, CHEST_FORM));
    }

    fn shelf_form(&self, pos: NodePos) -> String {
        let empty: Vec<bool> = self.chests.get(pos).map_or(vec![true; chest::SHELF_SLOTS], |c| c.slots().iter().map(Option::is_none).collect());
        let kind = self.chests.get(pos).and_then(|c| c.shelf).unwrap_or(chest::Shelf::Books);
        shelf_formspec(&chest::detached_name(pos), &empty, kind)
    }

    /// After a shelf's contents change (update_bookshelf,
    /// update_vessels_shelf): what it holds, shown when it is pointed at,
    /// and the outlines of its empty slots.
    fn update_shelf(&mut self, pos: NodePos) {
        let (mut written, mut empty) = (0, 0);
        let record = self.chests.get(pos);
        for s in record.map(|c| c.slots()).unwrap_or(&[]).iter().flatten() {
            if s.meta.get("text").is_some() {
                written += s.count;
            } else {
                empty += s.count;
            }
        }
        let infotext = match record.and_then(|c| c.shelf) {
            Some(chest::Shelf::Vessels) if written + empty == 0 => "Empty Vessels Shelf".to_string(),
            Some(chest::Shelf::Vessels) => format!("Vessels Shelf ({} items)", written + empty),
            _ if written + empty == 0 => "Empty Bookshelf".to_string(),
            _ => format!("Bookshelf ({written} written, {empty} empty books)"),
        };
        self.world.set_meta(pos, Some(meta::Fields::from([("infotext".to_string(), infotext)])));
        self.broadcast_meta(pos);
        let form = self.shelf_form(pos);
        let viewers: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.chest == Some(pos)).map(|(a, _)| *a).collect();
        for addr in viewers {
            self.send_command(addr, 0, &build_show_formspec(&form, CHEST_FORM));
        }
    }

    /// Punching bones (their on_punch): everything that fits goes into the
    /// player's inventory, stack by stack until one doesn't; if they are
    /// emptied, they go too.
    fn punch_bones(&mut self, addr: SocketAddr, pos: NodePos) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        let name = peer.display_name().to_string();
        let Some(record) = self.chests.get_mut(pos).filter(|c| c.bones && c.allows(&name)) else {
            return;
        };
        let mut moved = false;
        for slot in record.main.iter_mut() {
            let Some(stack) = *slot else {
                continue;
            };
            let mut trial = peer.inventory.main;
            if inventory::add_to(&mut trial, stack) > 0 {
                break; // no room for this one
            }
            peer.inventory.main = trial;
            *slot = None;
            moved = true;
        }
        let emptied = record.is_empty();
        if moved {
            info!("{name} took from the bones at {pos:?}");
            self.chests.save();
            self.inventory_changed(addr);
            self.send_chest(pos);
        }
        if emptied {
            self.remove_bones(pos, addr);
        }
    }

    /// Empty bones go, and whoever emptied them gets the bones block (or it
    /// drops, if their inventory is full).
    fn remove_bones(&mut self, pos: NodePos, taker: SocketAddr) {
        self.close_chest_windows(pos);
        self.chests.remove(pos);
        self.world.set_node(pos, world::CONTENT_AIR, 0);
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.broadcast_node(p16, &build_removenode(p16));
        let left = self.peers.get_mut(&taker).map_or(1, |p| p.inventory.add(Stack::new(bones::BONES, 1)));
        if left > 0 {
            self.drop_from_bones([pos.0 as f32, pos.1 as f32, pos.2 as f32], Stack::new(bones::BONES, 1));
        } else {
            self.inventory_changed(taker);
        }
        self.wake_items(pos);
    }

    /// Close a chest's window for everyone who has it open (it was dug).
    fn close_chest_windows(&mut self, pos: NodePos) {
        let viewers: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.chest == Some(pos)).map(|(a, _)| *a).collect();
        for addr in viewers {
            if let Some(peer) = self.peers.get_mut(&addr) {
                peer.chest = None;
            }
            self.send_command(addr, 0, &build_show_formspec("", CHEST_FORM));
        }
        self.open_chests.remove(&pos);
    }

    /// TOCLIENT_DETACHED_INVENTORY with a chest's contents.
    fn chest_packet(&self, pos: NodePos) -> Option<Vec<u8>> {
        let c = self.chests.get(pos)?;
        let slots = c.slots().iter().map(|s| s.map(|s| s.item_string())).collect();
        Some(build_detached_inventory(&chest::detached_name(pos), &[("main", 0, slots)]))
    }

    /// Send a chest's contents to everyone who has it open.
    fn send_chest(&mut self, pos: NodePos) {
        let Some(packet) = self.chest_packet(pos) else {
            return;
        };
        let viewers: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.chest == Some(pos)).map(|(a, _)| *a).collect();
        for addr in viewers {
            self.send_command(addr, 0, &packet);
        }
    }

    /// Change a node's look and nothing else (minetest_game's swap_node).
    fn swap_node(&mut self, pos: NodePos, id: u16, param2: u8) {
        let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
        self.world.set_node(pos, id, param2);
        self.broadcast_node(p16, &build_addnode(p16, id, param2));
    }

    /// Open a furnace's window for a player: its contents, then the form.
    fn open_furnace(&mut self, addr: SocketAddr, pos: NodePos) {
        self.furnaces.get_or_add(pos);
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        peer.furnace = Some(pos);
        info!("{} opened the furnace at {pos:?}", peer.display_name());
        if let Some(packet) = self.furnace_packet(pos) {
            self.send_command(addr, 0, &packet);
        }
        self.send_furnace_form(addr, pos);
    }

    /// Send (or update) the furnace window, showing the fire and progress.
    fn send_furnace_form(&mut self, addr: SocketAddr, pos: NodePos) {
        let display = self.furnaces.get(pos).and_then(|f| f.display);
        let form = furnace_formspec(&furnace::detached_name(pos), display);
        if let Some(peer) = self.peers.get_mut(&addr) {
            peer.furnace_form_sent = Some(Instant::now());
        }
        self.send_command(addr, 0, &build_show_formspec(&form, FURNACE_FORM));
    }

    /// Run the timers of the furnaces that are burning (or were just
    /// filled): smelt, swap between the lit and unlit furnace nodes, keep
    /// open windows up to date, and play the fire and cooling sounds.
    fn step_furnaces(&mut self, dt: f32) {
        let mut save = false;
        for pos in self.furnaces.running() {
            let node = self.world.node(pos);
            if !content::is_furnace(node) {
                warn!("No furnace at {pos:?} any more; forgetting its contents");
                self.furnaces.remove(pos);
                continue;
            }
            let Some(f) = self.furnaces.get_mut(pos) else {
                continue;
            };
            let shown = f.display;
            let step = f.step(dt);
            let display = f.display;
            save |= step.changed;
            let p16 = [pos.0 as i16, pos.1 as i16, pos.2 as i16];
            let at = [pos.0 as f32 * 10.0, pos.1 as f32 * 10.0, pos.2 as f32 * 10.0];
            // The lit or unlit node (minetest_game's swap_node).
            let want = if step.active { content::CONTENT_FURNACE_ACTIVE } else { content::CONTENT_FURNACE };
            if want != node {
                let param2 = self.world.param2(pos);
                self.world.set_node(pos, want, param2);
                self.broadcast_node(p16, &build_addnode(p16, want, param2));
            }
            if step.smelted > 0 {
                let gain = 0.07 * step.smelted.min(7) as f32;
                self.play_sound_near(-1, "default_cool_lava", gain, at, FURNACE_HEARING);
            }
            if step.fire_sound {
                let id = self.next_sound_id;
                self.next_sound_id = if id == i32::MAX { 1 } else { id + 1 };
                self.play_sound_near(id, "default_furnace_active", 0.25, at, FURNACE_HEARING);
                if let Some(f) = self.furnaces.get_mut(pos) {
                    f.sounds.push(id);
                    // Only the last three can still be playing.
                    if f.sounds.len() > 3 {
                        f.sounds.remove(0);
                    }
                }
            }
            if !step.active {
                let sounds = self.furnaces.get_mut(pos).map(|f| std::mem::take(&mut f.sounds)).unwrap_or_default();
                self.fade_sounds(&sounds);
            }
            if step.changed {
                self.send_furnace(pos);
            }
            if display != shown {
                let viewers: Vec<SocketAddr> =
                    self.peers.iter().filter(|(_, p)| p.furnace == Some(pos)).map(|(a, _)| *a).collect();
                for addr in viewers {
                    self.send_furnace_form(addr, pos);
                }
            }
        }
        if save {
            self.furnaces.save();
        }
    }

    /// TOCLIENT_DETACHED_INVENTORY with a furnace's contents.
    fn furnace_packet(&self, pos: NodePos) -> Option<Vec<u8>> {
        let f = self.furnaces.get(pos)?;
        let lists: Vec<_> = f
            .lists()
            .into_iter()
            .map(|(name, width, slots)| (name, width, slots.iter().map(|s| s.map(|s| s.item_string())).collect()))
            .collect();
        Some(build_detached_inventory(&furnace::detached_name(pos), &lists))
    }

    /// Send a furnace's contents to everyone who has it open.
    fn send_furnace(&mut self, pos: NodePos) {
        let Some(packet) = self.furnace_packet(pos) else {
            return;
        };
        let viewers: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.furnace == Some(pos)).map(|(a, _)| *a).collect();
        for addr in viewers {
            self.send_command(addr, 0, &packet);
        }
    }

    /// Close a furnace's window for everyone who has it open (it was dug).
    fn close_furnace(&mut self, pos: NodePos) {
        let viewers: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.furnace == Some(pos)).map(|(a, _)| *a).collect();
        for addr in viewers {
            if let Some(peer) = self.peers.get_mut(&addr) {
                peer.furnace = None;
            }
            self.send_command(addr, 0, &build_show_formspec("", FURNACE_FORM));
        }
    }

    /// Save a player's inventory and send it to them.
    /// The inventory changed: it goes to the client once this turn of the
    /// loop is over, and to the SD card within INVENTORY_SAVE (as Luanti
    /// sends an inventory once per server step, however often it
    /// changed). A dig used to send and save it twice (tool wear, then the
    /// drop), each save a full rewrite of the file on the card.
    fn inventory_changed(&mut self, addr: SocketAddr) {
        if let Some(peer) = self.peers.get_mut(&addr) {
            peer.inventory_unsent = true;
            peer.inventory_unsaved.get_or_insert_with(Instant::now);
        }
    }

    /// Send changed inventories, and save those changed a while ago.
    fn flush_inventories(&mut self) {
        let unsent: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| p.inventory_unsent).map(|(a, _)| *a).collect();
        for addr in unsent {
            if let Some(peer) = self.peers.get_mut(&addr) {
                peer.inventory_unsent = false;
            }
            self.send_inventory(addr);
        }
        let store = &self.inventories;
        for peer in self.peers.values_mut() {
            if peer.inventory_unsaved.is_some_and(|t| t.elapsed() >= INVENTORY_SAVE) {
                peer.save_inventory(store);
            }
        }
    }

    fn send_inventory(&mut self, addr: SocketAddr) {
        let Some(peer) = self.peers.get(&addr) else {
            return;
        };
        let lists: Vec<_> = peer
            .inventory
            .lists()
            .into_iter()
            .map(|(name, width, slots)| (name, width, slots.iter().map(|s| s.map(|s| s.item_string())).collect()))
            .collect();
        let packet = build_inventory(&lists);
        self.send_command(addr, 0, &packet);
    }

    /// Send a node change to every player whose client has that map block.
    /// Others will get the changed block when they come near. The change
    /// goes on the block channel, so it can't overtake (and be undone by)
    /// a copy of the block that is still on its way.
    fn broadcast_node(&mut self, pos: [i16; 3], packet: &[u8]) {
        let block = world::block_of_node((pos[0] as i32, pos[1] as i32, pos[2] as i32));
        let targets: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, p)| p.ready && p.sent_blocks.contains(&block))
            .map(|(a, _)| *a)
            .collect();
        for addr in targets {
            self.send_command(addr, BLOCK_CHANNEL, packet);
        }
    }

    /// Tell one client what a node really is, undoing its prediction.
    fn resend_node(&mut self, addr: SocketAddr, pos: [i16; 3]) {
        let id = self.world.node((pos[0] as i32, pos[1] as i32, pos[2] as i32));
        let p2 = self.world.param2((pos[0] as i32, pos[1] as i32, pos[2] as i32));
        // The node is as the client last had it, metadata and all.
        let packet = if id == world::CONTENT_AIR { build_removenode(pos) } else { build_addnode_keeping_meta(pos, id, p2) };
        self.send_command(addr, BLOCK_CHANNEL, &packet);
    }

    /// Send a node's metadata, as it now is, to everyone who has its block
    /// (Luanti's sendMetadataChanged).
    fn broadcast_meta(&mut self, pos: NodePos) {
        let fields = self.world.meta(pos).map(|f| (*f).clone()).unwrap_or_default();
        let packet = meta::build_changed(&[(pos, &fields)]);
        self.broadcast_node([pos.0 as i16, pos.1 as i16, pos.2 as i16], &packet);
    }

    /// TOSERVER_NODEMETA_FIELDS: a node's own form was sent: a sign's
    /// new text (minetest_game's on_receive_fields). Only from a player
    /// who may interact, within reach.
    fn on_nodemeta_fields(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some((p, fields)) = parse_nodemeta_fields(payload) else {
            return;
        };
        let pos = (p[0] as i32, p[1] as i32, p[2] as i32);
        let Some(peer) = self.peers.get(&from) else {
            return;
        };
        let name = peer.display_name().to_string();
        let body = [peer.pos_bs[0] / 10.0, peer.pos_bs[1] / 10.0 + 1.0, peer.pos_bs[2] / 10.0];
        let near = (0..3).map(|i| (p[i] as f32 - body[i]).powi(2)).sum::<f32>().sqrt() <= MAX_INTERACT_DISTANCE;
        if !peer.ready || !peer.privs.has("interact") || !near || !content::is_sign(self.world.node(pos)) {
            info!("{name}: ignored fields for {pos:?}");
            return;
        }
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        // Only when the form closes (minetest_game's workaround for
        // Luanti issue 16187).
        let (Some(_), Some(text)) = (get("quit"), get("text")) else {
            return;
        };
        let current = self.world.meta(pos).map(|f| (*f).clone()).unwrap_or_else(meta::new_sign);
        let Some(updated) = meta::write_sign(&current, text) else {
            self.tell(from, "Text too long");
            return;
        };
        info!("{name} wrote {:?} to the sign at {pos:?}", updated["text"]);
        self.world.set_meta(pos, Some(updated));
        self.broadcast_meta(pos);
    }

    fn on_deleted_blocks(&mut self, from: SocketAddr, payload: &[u8]) {
        let Some(list) = parse_block_list(payload) else {
            return;
        };
        if let Some(peer) = self.peers.get_mut(&from) {
            for pos in &list {
                peer.sent_blocks.remove(pos);
            }
            debug!("Client dropped {} blocks", list.len());
            // Re-plan so blocks come back if the player returns.
            peer.view_center = None;
        }
    }

    /// Work out which blocks around `pos` (in nodes) the client still needs,
    /// nearest first.
    fn plan_blocks(&mut self, addr: SocketAddr, pos: [f32; 3], radius: i16) {
        let center = node_to_block(pos);
        let Some(peer) = self.peers.get(&addr) else {
            return;
        };

        // In each column of blocks: the blocks holding the surface there,
        // from one node below its lowest point to one above its highest,
        // plus the blocks around the player (who may be flying or in a
        // cave).
        let mut wanted = Vec::new();
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx * dx + dz * dz > radius * radius + radius {
                    continue; // roughly circular
                }
                let (bx, bz) = (center.0 + dx, center.2 + dz);
                if !world::block_in_world((bx, 0, bz)) {
                    continue;
                }
                let (low, high) = self.world.height_range(bx, bz);
                let surface = world::block_of(low - 1)..=world::block_of(high + 1);
                let around = center.1 - 1..=center.1 + 1;
                let ys = surface.clone().chain(around.filter(|y| !surface.contains(y)));
                for y in ys {
                    let p = (bx, y, bz);
                    // Nothing beyond the edge of the world: the client
                    // treats missing blocks as solid, so that is the wall.
                    if world::block_in_world(p) && !peer.sent_blocks.contains(&p) {
                        wanted.push(p);
                    }
                }
            }
        }
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        wanted.sort_by_key(|p| {
            let (dx, dy, dz) = (
                (p.0 - center.0) as i32,
                (p.1 - center.1) as i32,
                (p.2 - center.2) as i32,
            );
            dx * dx + dy * dy * 2 + dz * dz
        });

        if !wanted.is_empty() {
            debug!(
                "{}: {} blocks to send around block {center:?}",
                peer.display_name(),
                wanted.len()
            );
        }
        peer.pending_blocks = wanted.into();
        peer.view_center = Some(center);
        peer.view_radius = radius;
    }

    /// Send players the blocks they still need: from the cache of built
    /// blocks when possible, otherwise asking the builder thread for them
    /// (nearest first) and sending them as they are finished.
    fn send_blocks(&mut self) {
        self.collect_built();
        let started = Instant::now();
        // Take turns between players, so one player loading a lot doesn't
        // hold up the others.
        let mut addrs: Vec<SocketAddr> = self.peers.keys().copied().collect();
        if !addrs.is_empty() {
            let n = self.block_turn % addrs.len();
            addrs.rotate_left(n);
            self.block_turn = self.block_turn.wrapping_add(1);
        }
        for &addr in &addrs {
            loop {
                if self.congested() {
                    return;
                }
                let Some(peer) = self.peers.get_mut(&addr) else {
                    break;
                };
                if !peer.ready {
                    break;
                }
                // Keep a few blocks queued or being built per player, no more:
                // the plan changes as the player moves.
                if peer.channels[BLOCK_CHANNEL].queue.len() + peer.awaiting.len() >= BLOCK_QUEUE_LIMIT {
                    break;
                }
                let Some(pos) = peer.pending_blocks.pop_front() else {
                    break;
                };
                if peer.sent_blocks.contains(&pos) || peer.awaiting.contains(&pos) {
                    continue;
                }
                if let Some(payload) = self.built_block(pos) {
                    self.deliver(addr, pos, &payload);
                } else if self.builder.is_some() {
                    if let Some(peer) = self.peers.get_mut(&addr) {
                        peer.awaiting.push_back(pos);
                    }
                } else {
                    // No builder thread: build here, a little at a time.
                    if started.elapsed() >= BLOCK_TIME_BUDGET {
                        if let Some(peer) = self.peers.get_mut(&addr) {
                            peer.pending_blocks.push_front(pos);
                        }
                        return;
                    }
                    let building = Instant::now();
                    let payload = Arc::new(build_blockdata(&self.world.block_payload(pos)));
                    {
                        let mut st = status::lock(&self.status);
                        st.blocks_built += 1;
                        st.block_us += building.elapsed().as_micros() as u64;
                    }
                    self.keep_built(pos, self.world.version(), payload.clone());
                    self.deliver(addr, pos, &payload);
                }
            }
        }
        self.submit_builds(&addrs);
    }

    /// A built copy of a block that is still up to date, if there is one.
    fn built_block(&mut self, pos: BlockPos) -> Option<Arc<Vec<u8>>> {
        let (version, payload) = self.built.get(&pos)?;
        if self.world.changed_near_since(pos, *version) {
            self.built.remove(&pos);
            return None;
        }
        Some(payload.clone())
    }

    fn keep_built(&mut self, pos: BlockPos, version: u64, payload: Arc<Vec<u8>>) {
        if self.built.len() >= BUILT_CACHE_MAX {
            self.built.clear();
        }
        self.built.insert(pos, (version, payload));
    }

    /// Take finished blocks from the builder and send them to whoever is
    /// waiting. A block the world has changed around since it was started is
    /// dropped; it gets built again.
    fn collect_built(&mut self) {
        let Some(builder) = &self.builder else {
            return;
        };
        let finished: Vec<Built> = builder.done.try_iter().collect();
        for b in finished {
            self.building.remove(&b.pos);
            if self.world.changed_near_since(b.pos, b.version) {
                continue;
            }
            let payload = Arc::new(b.payload);
            self.keep_built(b.pos, b.version, payload.clone());
            let waiting: Vec<SocketAddr> = self
                .peers
                .iter()
                .filter(|(_, p)| p.awaiting.contains(&b.pos))
                .map(|(a, _)| *a)
                .collect();
            for addr in waiting {
                if let Some(peer) = self.peers.get_mut(&addr) {
                    peer.awaiting.retain(|p| *p != b.pos);
                }
                self.deliver(addr, b.pos, &payload);
            }
        }
    }

    /// Hand the builder the next blocks players are waiting for, taking
    /// turns between players, keeping at most BUILDS_IN_FLIGHT in hand.
    fn submit_builds(&mut self, addrs: &[SocketAddr]) {
        let Some(builder) = &self.builder else {
            return;
        };
        let mut round = 0;
        while self.building.len() < BUILDS_IN_FLIGHT {
            let mut submitted = false;
            for addr in addrs {
                if self.building.len() >= BUILDS_IN_FLIGHT {
                    break;
                }
                let Some(peer) = self.peers.get(addr) else {
                    continue;
                };
                // This player's next block not already being built.
                let Some(&pos) = peer.awaiting.iter().filter(|p| !self.building.contains(p)).nth(round) else {
                    continue;
                };
                if builder.jobs.send(BuildJob { pos, state: self.world.snapshot() }).is_err() {
                    warn!("The block builder has stopped; building blocks in the server loop instead");
                    status::problem();
                    self.builder = None;
                    // Hand the waiting blocks back to be built here.
                    for peer in self.peers.values_mut() {
                        for p in peer.awaiting.drain(..).rev() {
                            peer.pending_blocks.push_front(p);
                        }
                    }
                    self.building.clear();
                    return;
                }
                self.building.insert(pos);
                submitted = true;
            }
            if !submitted {
                break;
            }
            round += 1;
        }
    }

    /// Send a block to a player and note that they have it.
    fn deliver(&mut self, addr: SocketAddr, pos: BlockPos, payload: &[u8]) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        if !peer.sent_blocks.insert(pos) {
            return;
        }
        let finished = peer.pending_blocks.is_empty() && peer.awaiting.is_empty();
        let (total, name) = (peer.sent_blocks.len(), peer.display_name().to_string());
        status::lock(&self.status).blocks_sent += 1;
        self.send_command(addr, BLOCK_CHANNEL, payload);
        if finished {
            info!("{name}: all nearby blocks sent ({total} in view)");
        }
    }

    // -----------------------------------------------------------------------
    // Peers and the reliability layer
    // -----------------------------------------------------------------------

    fn add_peer(&mut self, addr: SocketAddr) {
        let in_use: HashSet<u16> = self.peers.values().map(|p| p.id).collect();
        let Some(id) = free_peer_id(self.next_peer_id, &in_use) else {
            warn!("No peer id free for {addr} ({} peers); ignored", self.peers.len());
            return;
        };
        self.next_peer_id = if id >= LAST_PEER_ID { FIRST_PEER_ID } else { id + 1 };

        self.peers.insert(addr, Peer::new(id));
        info!("New peer {id} from {addr}");
        self.refresh_players();
        self.send_reliable(addr, 0, set_peer_id_body(id));
    }

    /// Send a command reliably, splitting it if it doesn't fit in one datagram.
    fn send_command(&mut self, addr: SocketAddr, channel: usize, payload: &[u8]) {
        if 1 + payload.len() <= MAX_RELIABLE_BODY {
            self.send_reliable(addr, channel, original_body(payload));
            return;
        }
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        let ch = &mut peer.channels[channel];
        let split_seqnum = ch.next_split;
        ch.next_split = split_seqnum.wrapping_add(1);
        let bodies = split_bodies(payload, split_seqnum);
        debug!(
            "Splitting {} byte command into {} packets on ch{channel}",
            payload.len(),
            bodies.len()
        );
        ch.queue.extend(bodies);
        self.flush(addr, channel);
    }

    fn send_reliable(&mut self, addr: SocketAddr, channel: usize, body: Vec<u8>) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        peer.channels[channel].queue.push_back(body);
        self.flush(addr, channel);
    }

    /// Send queued reliable packets while there is room in the window and
    /// the radio is keeping up. What isn't sent stays queued; flush is
    /// called again from tick().
    fn flush(&mut self, addr: SocketAddr, channel: usize) {
        if self.congested() {
            return;
        }
        let mut failed = false;
        let mut count = 0;
        if let Some(peer) = self.peers.get_mut(&addr) {
            let timeout = peer.resend_timeout;
            let ch = &mut peer.channels[channel];
            while ch.unacked.len() < RELIABLE_WINDOW {
                let Some(body) = ch.queue.pop_front() else {
                    break;
                };
                let seqnum = ch.next_out;
                ch.next_out = seqnum.wrapping_add(1);
                let data = reliable_datagram(channel as u8, seqnum, &body);
                let now = Instant::now();
                let sent = send_datagram(&self.socket, addr, &data);
                // Unsent, it is in the window all the same (it has its
                // seqnum), and is tried again soon.
                let (first_sent, due) = if sent { (Some(now), now + timeout) } else { (None, now + RETRY_UNSENT) };
                ch.unacked.push(Unacked { seqnum, data, first_sent, due, resends: 0 });
                count += 1;
                if !sent {
                    failed = true;
                    break;
                }
            }
        }
        status::lock(&self.status).packets_out += count;
        if failed {
            self.note_send_failure();
        }
    }

    /// Send one datagram now. Returns whether it went.
    fn send_raw(&mut self, addr: SocketAddr, data: &[u8]) -> bool {
        status::lock(&self.status).packets_out += 1;
        let sent = send_datagram(&self.socket, addr, data);
        if !sent {
            self.note_send_failure();
        }
        sent
    }

    /// The radio's transmit buffers are full (or sending failed some other
    /// way): back off, and report it now and then rather than per packet.
    fn note_send_failure(&mut self) {
        self.congested_until = Instant::now() + CONGESTION_BACKOFF;
        status::lock(&self.status).send_failures += 1;
        self.send_failures += 1;
        if self.send_failures_reported.elapsed() >= SEND_FAILURE_REPORT_INTERVAL {
            use std::sync::atomic::Ordering::Relaxed;
            let full = SEND_ERRORS_FULL.swap(0, Relaxed);
            let other = SEND_ERRORS_OTHER.swap(0, Relaxed);
            let last = LAST_OTHER_ERROR.load(Relaxed);
            let others = if other > 0 {
                format!(", {other} other (last: {})", std::io::Error::from_raw_os_error(last))
            } else {
                String::new()
            };
            let rtts: Vec<String> = self
                .peers
                .values()
                .filter_map(|p| p.avg_rtt.map(|r| format!("{} {:.0} ms", p.display_name(), r * 1000.0)))
                .collect();
            warn!(
                "{} packet(s) couldn't be sent in the last {}s: {full} with Wi-Fi transmit buffers full{others}; \
                 {} resent, {} ACKs; round trip {}; slowing down",
                self.send_failures,
                self.send_failures_reported.elapsed().as_secs(),
                self.resends,
                self.acks,
                if rtts.is_empty() { "unknown".to_string() } else { rtts.join(", ") },
            );
            self.resends = 0;
            self.acks = 0;
            self.send_failures = 0;
            self.send_failures_reported = Instant::now();
        }
    }

    /// Someone is doing something: Wi-Fi fully awake, at once.
    /// Remember something the server did, for the heap check's report.
    fn note(&mut self, what: &'static str, n: u16) {
        if self.recent.len() >= RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back((Instant::now(), what, n));
    }

    fn note_command(&mut self, cmd: u16, peer: u16) {
        // The command and the peer it came from, in one number.
        self.note("command (id << 4 | peer)", cmd << 4 | (peer & 0xf));
    }

    /// Every HEAP_CHECK_INTERVAL, check the heaps (and, with heap
    /// poisoning on, every block's guard words). Damage is reported with
    /// what the server did just before, so it can be traced to its cause
    /// rather than crashing later somewhere unrelated.
    fn check_heap(&mut self, now: Instant) {
        if self.heap_damaged || now.duration_since(self.heap_checked) < HEAP_CHECK_INTERVAL {
            return;
        }
        let last_good = self.heap_checked;
        self.heap_checked = now;
        let started = Instant::now();
        let ok = heap_intact();
        let took = started.elapsed();
        if !self.heap_check_timed {
            self.heap_check_timed = true;
            info!("Heap check: {} us", took.as_micros());
        }
        if ok {
            return;
        }
        self.heap_damaged = true;
        log::error!("HEAP DAMAGED: found now, intact {} ms ago. What the server did lately, oldest first:", now.duration_since(last_good).as_millis());
        for (at, what, n) in &self.recent {
            log::error!("  {:>6} ms ago: {what} {n:#06x}", now.duration_since(*at).as_millis());
        }
        crate::status::problem();
    }

    fn wake(&mut self) {
        self.last_activity = Instant::now();
        if self.power_saving {
            self.power_saving = false;
            set_wifi_power_save(false);
            status::lock(&self.status).wifi_dozing = false;
            info!("Wi-Fi power saving off");
        }
    }

    /// Let the Wi-Fi radio doze between the router's beacons once nobody
    /// has done anything for IDLE_POWER_SAVE: no one connected, or every
    /// player idle with their map loaded. It saves power and heat (the
    /// board's 3.3 V regulator gets hot), at the cost of delaying packets
    /// by up to a few hundred milliseconds, which only the first move after
    /// a pause will notice.
    fn update_power_saving(&mut self) {
        if self.hotspot || self.power_saving || self.last_activity.elapsed() < IDLE_POWER_SAVE {
            return;
        }
        // Not while a player is still being sent their map: it would load
        // slowly, the client's acknowledgements being delayed.
        let loading = self.peers.values().any(|p| !p.ready || !p.pending_blocks.is_empty() || !p.awaiting.is_empty());
        if loading {
            return;
        }
        self.power_saving = true;
        set_wifi_power_save(true);
        status::lock(&self.status).wifi_dozing = true;
        info!("Wi-Fi power saving on: nobody has done anything for {}s", IDLE_POWER_SAVE.as_secs());
    }

    fn congested(&self) -> bool {
        Instant::now() < self.congested_until
    }

    /// Periodic housekeeping: drop silent peers, resend unacknowledged
    /// packets, send map blocks.
    fn tick(&mut self) {
        let now = Instant::now();
        self.finish_srp();
        self.send_blocks();
        if now.duration_since(self.items_stepped) >= ITEM_STEP_INTERVAL {
            let dt = now.duration_since(self.items_stepped).as_secs_f32();
            self.items_stepped = now;
            self.step_items(dt);
            self.step_falling(dt);
            self.step_mobs(dt);
            self.step_boats(dt);
        }
        if now.duration_since(self.mobs_spawned) >= mobs::SPAWN_EVERY {
            self.mobs_spawned = now;
            self.spawn_mobs();
        }
        if self.items.save_if_due() {
            self.note("items saved", 0);
        }
        if self.boats.save_if_due() {
            self.note("boats saved", 0);
        }
        if self.clock.save_if_due() {
            self.note("time saved", 0);
        }
        self.check_heap(now);
        if now.duration_since(self.fields_checked) >= farming::soil_check().0 {
            self.fields_checked = now;
            self.tend_fields();
        }
        if now.duration_since(self.climbers_checked) >= plants::CLIMB_INTERVAL {
            self.climbers_checked = now;
            self.tend_climbers();
        }
        self.update_power_saving();
        self.run_timers();
        self.tend_beds();
        self.step_breath(now);
        self.watch_links();
        self.flush_inventories();
        if now.duration_since(self.liquids_flowed) >= LIQUID_UPDATE {
            self.liquids_flowed = now;
            self.flow_liquids();
            // Lava burns once a second too (PlayerSAO's node damage).
            self.burn_players();
            // Node changes go to the SD card once a second.
            self.world.flush_log();
        }
        if now.duration_since(self.grass_spread) >= nature::grass_interval(nature::GRASS_SPREAD_INTERVAL) {
            self.grass_spread = now;
            self.grow_grass(true);
        }
        if now.duration_since(self.grass_covered) >= nature::grass_interval(nature::GRASS_COVERED_INTERVAL) {
            self.grass_covered = now;
            self.grow_grass(false);
        }
        if now.duration_since(self.moss_checked) >= nature::moss_interval() {
            self.moss_checked = now;
            self.grow_moss();
        }
        // binoculars' and map's cyclic_update.
        if now.duration_since(self.zoom_checked) >= binoculars::BINOCULARS_EVERY {
            self.zoom_checked = now;
            let all: Vec<SocketAddr> = self.peers.keys().copied().collect();
            for addr in all {
                self.update_zoom(addr);
            }
        }
        if now.duration_since(self.map_checked) >= binoculars::MAP_EVERY {
            self.map_checked = now;
            let all: Vec<SocketAddr> = self.peers.keys().copied().collect();
            for addr in all {
                self.update_minimap(addr);
            }
        }
        // The fire mod's ABMs and crackle, and burning dropped items.
        let (every, chance) = fire::ignite_every();
        if now.duration_since(self.fire_ignited) >= every {
            self.fire_ignited = now;
            self.ignite(chance);
        }
        let (every, chance) = fire::burn_every();
        if now.duration_since(self.fire_burned) >= every {
            self.fire_burned = now;
            self.burn_flammables(chance);
        }
        if now.duration_since(self.insects_tended) >= insects::every() {
            self.insects_tended = now;
            self.tend_insects();
        }
        if now.duration_since(self.tnt_ignited) >= tnt::IGNITION_EVERY {
            self.tnt_ignited = now;
            self.ignite_tnt();
        }
        if now.duration_since(self.fire_sounded) >= fire::SOUND_EVERY {
            self.fire_sounded = now;
            self.crackle();
        }
        if now.duration_since(self.items_burned) >= fire::ITEMS_EVERY {
            self.items_burned = now;
            self.burn_items();
        }
        // env_sounds' cyclic_update.
        if now.duration_since(self.sounds_played) >= env_sounds::EVERY {
            self.sounds_played = now;
            let all: Vec<SocketAddr> = self.peers.keys().copied().collect();
            for addr in all {
                self.play_env_sounds(addr);
            }
        }
        // The weather's cyclic_update.
        if now.duration_since(self.weather_set) >= weather::CYCLE {
            self.weather_set = now;
            let all: Vec<SocketAddr> = self.peers.keys().copied().collect();
            for addr in all {
                self.set_weather(addr);
            }
        }
        if now.duration_since(self.lava_cooled) >= LAVA_COOLING {
            self.lava_cooled = now;
            self.cool_lava();
        }
        if now.duration_since(self.furnaces_stepped) >= FURNACE_STEP_INTERVAL {
            let dt = now.duration_since(self.furnaces_stepped).as_secs_f32();
            self.furnaces_stepped = now;
            self.step_furnaces(dt);
        }
        if now.duration_since(self.bones_checked) >= bones::CHECK_INTERVAL {
            self.bones_checked = now;
            for owner in self.chests.share_old_bones(self.clock.game_time()) {
                info!("{owner}'s bones are old: anyone may take from them now");
            }
        }

        let mut gone = Vec::new();
        let mut saved = Vec::new();
        let before = self.peers.len();
        let players = &self.players;
        let inventories = &self.inventories;
        self.peers.retain(|addr, peer| {
            let alive = now.duration_since(peer.last_seen) < PEER_TIMEOUT;
            if !alive {
                info!("Peer {} ({}) at {addr} timed out", peer.id, peer.display_name());
                peer.save(players);
                peer.save_inventory(inventories);
                saved.push(peer.id);
                if peer.ready {
                    gone.push((peer.id, peer.display_name().to_string()));
                }
            } else if peer.ready && now.duration_since(peer.saved_at) >= SAVE_INTERVAL {
                peer.save(players);
                saved.push(peer.id);
            }
            alive
        });
        for id in saved {
            self.note("player saved", id);
        }
        for (id, name) in gone {
            self.remove_player_object(id);
            self.event(format!("{name} timed out"));
            status::lock(&self.status).leaves += 1;
        }
        if self.peers.len() != before {
            self.close_unwatched_lids();
            self.refresh_players();
        }

        // Resend what hasn't been acknowledged, then anything still
        // queued, unless the radio is behind.
        if self.congested() {
            return;
        }
        let mut failed = false;
        let mut status_out = 0;
        let mut resent = 0;
        'peers: for (addr, peer) in self.peers.iter_mut() {
            let timeout = peer.resend_timeout;
            for (n, ch) in peer.channels.iter_mut().enumerate() {
                for u in ch.unacked.iter_mut() {
                    if now < u.due {
                        continue;
                    }
                    if !send_datagram(&self.socket, *addr, &u.data) {
                        u.due = now + RETRY_UNSENT;
                        failed = true;
                        break 'peers;
                    }
                    status_out += 1;
                    if u.first_sent.is_none() {
                        // Its first sending (the radio was full before).
                        u.first_sent = Some(now);
                        u.due = now + timeout;
                    } else {
                        // Each resend of a packet waits 1.5 times longer
                        // (Luanti's RESEND_SCALE_BASE), so a slow link isn't
                        // swamped with copies of what is still on its way.
                        u.resends += 1;
                        resent += 1;
                        u.due = now + timeout.mul_f32(RESEND_SCALE_BASE.powi(u.resends as i32));
                        debug!("Resending reliable {} on ch{n} (resend {})", u.seqnum, u.resends);
                    }
                }
            }
        }
        self.resends += resent;
        status::lock(&self.status).packets_out += status_out;
        if failed {
            self.note_send_failure();
            return;
        }
        let queued: Vec<(SocketAddr, usize)> = self
            .peers
            .iter()
            .flat_map(|(a, p)| p.channels.iter().enumerate().filter(|(_, c)| !c.queue.is_empty()).map(move |(n, _)| (*a, n)))
            .collect();
        for (addr, channel) in queued {
            self.flush(addr, channel);
        }
    }
}

pub fn hexdump(data: &[u8]) {
    for (i, chunk) in data.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
            .collect();
        info!("  {:04x}  {:<48} {}", i * 16, hex.join(" "), ascii);
    }
}

/// The way a player with this pitch and yaw (degrees, as the client sends
/// them) looks (ObjectRef::get_look_dir).
fn look_dir(pitch: f32, yaw: f32) -> [f32; 3] {
    let p = (-pitch).to_radians();
    let y = (yaw + 90.0).to_radians();
    [p.cos() * y.cos(), p.sin(), p.cos() * y.sin()]
}

/// The world as the mobs see it.
struct MobTerrain<'a>(&'a World);

impl mobs::Terrain for MobTerrain<'_> {
    fn solid(&self, p: NodePos) -> bool {
        content::walkable(self.0.node(p)) || content::is_tall(self.0.node((p.0, p.1 - 1, p.2)))
    }
    fn liquid(&self, p: NodePos) -> bool {
        content::is_liquid(self.0.node(p))
    }
    fn opaque(&self, p: NodePos) -> bool {
        let id = self.0.node(p);
        content::walkable(id) && content::group_of(id, "leaves") == 0 && !content::passes_light(id)
    }
    fn node(&self, p: NodePos) -> u16 {
        self.0.node(p)
    }
    fn top(&self, x: i32, z: i32) -> i32 {
        self.0.highest_solid(x, z)
    }
    fn light(&self, p: NodePos) -> u8 {
        self.0.torch_light_at(p)
    }
}

/// A bucket used or placed: the hotbar slot holding it, what it pointed at,
/// and where the player's body is (nodes).
#[derive(Clone, Copy)]
struct BucketUse {
    slot: usize,
    under: [i16; 3],
    above: [i16; 3],
    body: [f32; 3],
}

/// Where a player puts a door: what they pointed at, where they look, and
/// the hotbar slot holding it.
struct DoorPlace {
    under: [i16; 3],
    above: [i16; 3],
    look: [f32; 3],
    slot: usize,
}

/// A player lying in a bed: the bed's bottom, and where they were before
/// (beds.pos), to get up there again.
#[derive(Clone, Copy)]
struct InBed {
    bed: NodePos,
    from_bs: [f32; 3],
}

/// Where builtin's item_drop puts a dropped item, and how it throws it:
/// position and velocity in nodes.
fn drop_motion(pos_bs: [f32; 3], yaw: f32, pitch: f32) -> ([f32; 3], [f32; 3]) {
    let dir = look_dir(pitch, yaw);
    let pos = [pos_bs[0] / 10.0, pos_bs[1] / 10.0 + DROP_HEIGHT, pos_bs[2] / 10.0];
    let vel = [dir[0] * DROP_SPEED, dir[1] * DROP_SPEED + DROP_LIFT, dir[2] * DROP_SPEED];
    (pos, vel)
}

/// The strings a dropped item's object needs, which ItemInit borrows.
struct ItemStrings {
    itemstring: String,
    item: u16,
    count: u16,
    pos_bs: [f32; 3],
    vel_bs: [f32; 3],
}

impl ItemStrings {
    fn as_init(&self, id: u16) -> ItemInit<'_> {
        ItemInit {
            id,
            itemstring: &self.itemstring,
            item_name: content::name_of(self.item),
            description: content::description_of(self.item),
            count: self.count,
            light_source: content::light_source(self.item),
            pos_bs: self.pos_bs,
            vel_bs: self.vel_bs,
        }
    }
}

fn item_init_of(d: &crate::items::Dropped) -> ItemStrings {
    ItemStrings {
        itemstring: format!("{} {}", content::name_of(d.item), d.count),
        item: d.item,
        count: d.count,
        pos_bs: d.pos_bs(),
        vel_bs: d.vel_bs(),
    }
}

/// GenericCAO initialisation data for a dropped item, followed by its
/// current motion (so a falling item keeps falling for a newcomer).
fn item_init(d: &crate::items::Dropped) -> Vec<u8> {
    item_init_data(&item_init_of(d).as_init(d.id))
}

/// Send a datagram, saying whether it went. ENOMEM ("Not enough space")
/// means the Wi-Fi transmit buffers are full.
fn send_datagram(socket: &UdpSocket, addr: SocketAddr, data: &[u8]) -> bool {
    if test_send_failure() {
        return false;
    }
    match socket.send_to(data, addr) {
        Ok(_) => true,
        Err(e) => {
            use std::sync::atomic::Ordering::Relaxed;
            debug!("send_to {addr} failed: {e}");
            // ENOMEM: lwIP and the Wi-Fi driver out of transmit buffers.
            if e.raw_os_error() == Some(ENOMEM) {
                SEND_ERRORS_FULL.fetch_add(1, Relaxed);
            } else {
                SEND_ERRORS_OTHER.fetch_add(1, Relaxed);
                LAST_OTHER_ERROR.store(e.raw_os_error().unwrap_or(0), Relaxed);
            }
            false
        }
    }
}

/// Send errors by kind since the last report (see note_send_failure).
static SEND_ERRORS_FULL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SEND_ERRORS_OTHER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_OTHER_ERROR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// errno "Not enough space" (the same number in lwIP and on Linux).
const ENOMEM: i32 = 12;

/// Wi-Fi modem power saving (WIFI_PS_MIN_MODEM): the radio sleeps between
/// the router's beacons, which holds incoming packets until it wakes.
/// Off (WIFI_PS_NONE) keeps it listening all the time.
#[cfg(target_os = "espidf")]
fn set_wifi_power_save(on: bool) {
    use esp_idf_svc::sys::{esp_wifi_set_ps, wifi_ps_type_t_WIFI_PS_MIN_MODEM, wifi_ps_type_t_WIFI_PS_NONE};
    let mode = if on { wifi_ps_type_t_WIFI_PS_MIN_MODEM } else { wifi_ps_type_t_WIFI_PS_NONE };
    if let Err(e) = esp_idf_svc::sys::esp!(unsafe { esp_wifi_set_ps(mode) }) {
        warn!("Setting Wi-Fi power saving: {e}");
    }
}

#[cfg(not(target_os = "espidf"))]
fn set_wifi_power_save(_on: bool) {}

/// The board's side of a link: internal RAM (free, and the largest piece,
/// which the Wi-Fi driver needs for each frame) and the Wi-Fi connection.
#[cfg(target_os = "espidf")]
fn link_state() -> String {
    use esp_idf_svc::sys::*;
    let caps = MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT;
    let (free, largest) = unsafe { (heap_caps_get_free_size(caps) / 1024, heap_caps_get_largest_free_block(caps) / 1024) };
    let mut ap: wifi_ap_record_t = Default::default();
    let wifi = if unsafe { esp_wifi_sta_get_ap_info(&mut ap) } == 0 {
        format!("Wi-Fi connected, {} dBm, channel {}", ap.rssi, ap.primary)
    } else {
        "Wi-Fi NOT connected".to_string()
    };
    format!("internal RAM {free} KB free, largest block {largest} KB; {wifi}")
}

#[cfg(not(target_os = "espidf"))]
fn link_state() -> String {
    "(no board state on the PC)".to_string()
}

/// Whether ESP-IDF's heaps are intact (heap_caps_check_integrity_all,
/// which prints what it finds wrong).
#[cfg(target_os = "espidf")]
fn heap_intact() -> bool {
    unsafe { esp_idf_svc::sys::heap_caps_check_integrity_all(true) }
}

#[cfg(not(target_os = "espidf"))]
fn heap_intact() -> bool {
    true
}

/// For testing on a PC: with LUANTI_FAIL_SENDS=n set, every nth send
/// fails, as sends do on the ESP32 when its Wi-Fi buffers are full.
#[cfg(not(target_os = "espidf"))]
fn test_send_failure() -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    static EVERY: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let every = *EVERY.get_or_init(|| std::env::var("LUANTI_FAIL_SENDS").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
    every > 0 && COUNT.fetch_add(1, Ordering::Relaxed) % every == every - 1
}

#[cfg(target_os = "espidf")]
fn test_send_failure() -> bool {
    false
}

fn node_to_block(pos: [f32; 3]) -> BlockPos {
    let b = |v: f32| world::block_of(v.round() as i32);
    (b(pos[0]), b(pos[1]), b(pos[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_ids() {
        let none = HashSet::new();
        assert_eq!(free_peer_id(FIRST_PEER_ID, &none), Some(2));
        // In use ones are skipped; past the last it goes round to the start,
        // never into the mobs' and items' object ids.
        let busy: HashSet<u16> = [5, 6].into();
        assert_eq!(free_peer_id(5, &busy), Some(7));
        assert_eq!(free_peer_id(LAST_PEER_ID, &[LAST_PEER_ID].into()), Some(FIRST_PEER_ID));
        assert_eq!(free_peer_id(LAST_PEER_ID + 1, &none), Some(LAST_PEER_ID));
        assert!(LAST_PEER_ID < crate::boats::FIRST_ID);
        // A long run of connections, each gone again: ids stay in range.
        let mut next = FIRST_PEER_ID;
        for _ in 0..100_000 {
            let id = free_peer_id(next, &none).unwrap();
            assert!((FIRST_PEER_ID..=LAST_PEER_ID).contains(&id));
            next = if id >= LAST_PEER_ID { FIRST_PEER_ID } else { id + 1 };
        }
        let all: HashSet<u16> = (FIRST_PEER_ID..=LAST_PEER_ID).collect();
        assert_eq!(free_peer_id(9, &all), None);
    }
}
