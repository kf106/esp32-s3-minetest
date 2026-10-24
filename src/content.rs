//! The game content sent to clients: item definitions, node definitions and
//! media (textures).
//!
//! The serialisation follows Luanti 5.17 (protocol 53): ItemDefinition and
//! ContentFeatures in src/itemdef.cpp and src/nodedef.cpp. Both formats end
//! with optional sections that the client only reads if present, so we write
//! up to the last section we need and let the client fill in defaults.

use std::borrow::Cow;
use std::fs;

use log::{info, warn};

use crate::messages::Writer;
use crate::sha1::sha1;
use crate::beds;
use crate::bones;
use crate::doors;
use crate::fire;
use crate::tnt;
use crate::dungeons;
use crate::colours;
use crate::farming;
use crate::fences;
use crate::panes;
use crate::plants;
use crate::stairs;

// ---------------------------------------------------------------------------
// Definitions
// ---------------------------------------------------------------------------

pub struct Node {
    pub id: u16,
    pub name: &'static str,
    pub description: &'static str,
    /// Textures in Luanti order: +Y (top), -Y (bottom), +X, -X, +Z, -Z.
    pub tiles: [&'static str; 6],
    pub groups: &'static [(&'static str, i16)],
    /// What digging it gives (minetest_game: grass gives dirt), and how many.
    pub drop: u16,
    pub drop_count: u16,
    pub kind: Kind,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// An ordinary full cube.
    Solid,
    /// A torch: a small 3D model mounted on the floor, ceiling or a wall
    /// (param2 says which way), gives light, can be walked through, and
    /// falls off when what it hangs on is dug. As in minetest_game there
    /// is one node per model; the item is always the floor one.
    Torch,
    /// Leaves and needles: solid to walk on, drawn with see-through gaps
    /// (Luanti's allfaces_optional), letting light through but casting shade
    /// (light_propagates, not sunlight_propagates, as in minetest_game).
    Leaves,
    /// A furnace: a full cube turned to face the player who placed it
    /// (facedir), with its own inventory window on right-click.
    Furnace,
    /// A sapling: a small plant (plantlike, two crossed pictures) that can
    /// be walked through, stands on the node below and grows into a tree.
    Sapling,
    /// An apple hanging in a tree (plantlike, walked through, eaten).
    Apple,
    /// Where an apple was picked: invisible, and an apple grows there again
    /// (minetest_game's apple_mark: airlike, can't be pointed at, and
    /// anything placed there replaces it).
    Mark,
    /// A chest: a cube turned to face whoever placed it, opened with a
    /// right-click.
    Chest,
    /// A chest while someone has it open: minetest_game's open-lid model.
    ChestOpen,
    /// Glass: see-through, framed (minetest_game's glasslike_framed_optional:
    /// connected frames if the player has "connected glass" on).
    Glass,
    /// A ladder: a flat picture against a wall (signlike, wallmounted),
    /// walked through and climbed.
    Ladder,
    /// A stair or slab (stairs.rs): boxes (nodebox) turned by facedir, with
    /// light passing into it as minetest_game's paramtype "light" lets it.
    Shaped,
    /// The bottom of a door (doors.rs): a model two nodes tall, turned by
    /// facedir, opened and closed with a right-click.
    Door,
    /// The invisible node above a door's bottom, keeping its top half free.
    DoorHidden,
    /// A trapdoor (doors.rs), closed or open: a panel (nodebox) turned by
    /// facedir, opened and closed with a right-click.
    Trapdoor,
    /// Water: see-through, swum in, built into, not pointed at (except
    /// with a bucket). Light passes, dimming, as through leaves.
    Liquid,
    /// Tall grass, wild cotton and crops (farming.rs): plantlike, walked
    /// through, built over, and standing on the node below.
    Plant,
    /// A planted seed: a flat picture on the soil (signlike), standing on
    /// it.
    Seed,
    /// A bed's bottom half (beds.rs), the item: boxes turned by facedir,
    /// slept in with a right-click.
    Bed,
    /// A bed's top half: it drops nothing, and goes with the bottom.
    BedTop,
    /// A fence or fence rail (fences.rs): a post, joined by rails to its
    /// neighbours, or just the rails (a "connected" nodebox the client
    /// joins up).
    Fence,
    /// A fence gate, closed or open: a model turned by facedir, opened and
    /// closed with a right-click.
    Gate,
    /// Papyrus: plantlike, walked through, not built over.
    Stalk,
    /// A bush's stem: plantlike and larger than its node, but solid.
    Stem,
    /// Kelp and corals: a block with a plant growing out of its top
    /// (plantlike_rooted), drawn through the water above.
    Rooted,
    /// A mese lamp: a see-through block (glasslike) that gives light.
    Lamp,
    /// A layer of snow: a quarter of a node thick, built over, washed away.
    SnowLayer,
    /// A waterlily: a thin pad lying on water, built over, washed away.
    Lily,
    /// A glass pane or steel bars (panes.rs): flat, or joining its
    /// neighbours.
    Pane,
    /// Bones (bones.rs): a cube turned to face the way its player looked,
    /// opened with a right-click.
    Bones,
    /// A wall (fences.rs): a post joining its neighbours, like a fence,
    /// but casting shade.
    Wall,
    /// A mese post light: a thin wooden post (nodebox) with mese windows
    /// down its sides, giving light.
    Post,
    /// Mist (our own, not minetest_game's): a half-see-through cube that
    /// is walked through, built into and not pointed at, and veils the
    /// view of a player inside it.
    Mist,
    /// A flame (the fire mod's): drawn as crossed and boxed pictures
    /// (firelike), walked through, built over, washed out by water, and
    /// burning whoever stands in it. It glows, but lights nothing round it.
    Fire,
    /// Gunpowder (the tnt mod's): a trail on the ground that joins up with
    /// its neighbours, as rails do (raillike), walked over.
    Powder,
    /// A bookshelf: a cube turned to face whoever placed it, opened with a
    /// right-click, holding books.
    Shelf,
    /// A sign: a thin board (a wallmounted nodebox) on a wall, floor or
    /// ceiling, walked through, with text (meta.rs) its right-click
    /// edits.
    Sign,
    /// A bottle or glass (vessels): plantlike, walked through, standing on
    /// the node below.
    Vessel,
}

pub const CONTENT_STONE: u16 = 0;
pub const CONTENT_DIRT: u16 = 1;
pub const CONTENT_GRASS: u16 = 2;
pub const CONTENT_SAND: u16 = 3;
pub const CONTENT_SANDSTONE: u16 = 4;
pub const CONTENT_SNOW: u16 = 5;
pub const CONTENT_COAL: u16 = 6;
pub const CONTENT_IRON: u16 = 7;
pub const CONTENT_TORCH: u16 = 8;
pub const CONTENT_TORCH_WALL: u16 = 9;
pub const CONTENT_TORCH_CEILING: u16 = 10;
pub const CONTENT_BIRCH_TREE: u16 = 11;
pub const CONTENT_BIRCH_LEAVES: u16 = 12;
pub const CONTENT_FIR_TREE: u16 = 13;
pub const CONTENT_FIR_NEEDLES: u16 = 14;
pub const CONTENT_BIRCH_WOOD: u16 = 15;
pub const CONTENT_FIR_WOOD: u16 = 16;
pub const CONTENT_COBBLE: u16 = 17;
pub const CONTENT_FURNACE: u16 = 18;
pub const CONTENT_FURNACE_ACTIVE: u16 = 19;
pub const CONTENT_BIRCH_SAPLING: u16 = 20;
pub const CONTENT_FIR_SAPLING: u16 = 21;
pub const CONTENT_APPLE_TREE: u16 = 22;
pub const CONTENT_APPLE_LEAVES: u16 = 23;
pub const CONTENT_APPLE: u16 = 24;
pub const CONTENT_APPLE_MARK: u16 = 25;
pub const CONTENT_APPLE_SAPLING: u16 = 26;
pub const CONTENT_APPLE_WOOD: u16 = 27;
pub const CONTENT_CHEST: u16 = 28;
pub const CONTENT_CHEST_OPEN: u16 = 29;
pub const CONTENT_CHEST_LOCKED: u16 = 30;
pub const CONTENT_CHEST_LOCKED_OPEN: u16 = 31;
pub const CONTENT_STONE_BRICK: u16 = 32;
pub const CONTENT_STONE_BLOCK: u16 = 33;
pub const CONTENT_SANDSTONE_BRICK: u16 = 34;
pub const CONTENT_SANDSTONE_BLOCK: u16 = 35;
pub const CONTENT_COAL_BLOCK: u16 = 36;
pub const CONTENT_STEEL_BLOCK: u16 = 37;
pub const CONTENT_GLASS: u16 = 38;
pub const CONTENT_LADDER_WOOD: u16 = 39;
pub const CONTENT_LADDER_STEEL: u16 = 40;
/// Water (minetest_game's water_source and water_flowing), after the
/// stairs, doors and trapdoors.
pub const CONTENT_WATER: u16 = 98;
pub const CONTENT_WATER_FLOWING: u16 = 99;
/// minetest_game's lava, and the obsidian it cools into, after the beds.
pub const CONTENT_LAVA: u16 = 135;
pub const CONTENT_LAVA_FLOWING: u16 = 136;
pub const CONTENT_OBSIDIAN: u16 = 137;
/// minetest_game's jungle and acacia trees, and the ground of the
/// rainforest and savanna, after the fences.
pub const CONTENT_JUNGLE_TREE: u16 = 150;
pub const CONTENT_JUNGLE_LEAVES: u16 = 151;
pub const CONTENT_JUNGLE_WOOD: u16 = 152;
pub const CONTENT_JUNGLE_SAPLING: u16 = 153;
pub const CONTENT_ACACIA_TREE: u16 = 154;
pub const CONTENT_ACACIA_LEAVES: u16 = 155;
pub const CONTENT_ACACIA_WOOD: u16 = 156;
pub const CONTENT_ACACIA_SAPLING: u16 = 157;
pub const CONTENT_DRY_DIRT: u16 = 158;
pub const CONTENT_DRY_GRASS: u16 = 159;
pub const CONTENT_LITTER: u16 = 160;
/// minetest_game's other ores and their blocks, after the plants.
pub const CONTENT_COPPER: u16 = 211;
pub const CONTENT_TIN: u16 = 212;
pub const CONTENT_GOLD: u16 = 213;
pub const CONTENT_MESE_ORE: u16 = 214;
pub const CONTENT_DIAMOND_ORE: u16 = 215;
pub const CONTENT_COPPER_BLOCK: u16 = 216;
pub const CONTENT_TIN_BLOCK: u16 = 217;
pub const CONTENT_BRONZE_BLOCK: u16 = 218;
pub const CONTENT_GOLD_BLOCK: u16 = 219;
pub const CONTENT_MESE_BLOCK: u16 = 220;
pub const CONTENT_DIAMOND_BLOCK: u16 = 221;
pub const CONTENT_MESE_LAMP: u16 = 222;
/// minetest_game's other ground and building blocks, after the mese lamp.
pub const CONTENT_GRAVEL: u16 = 223;
pub const CONTENT_CLAY: u16 = 224;
pub const CONTENT_BRICK: u16 = 225;
pub const CONTENT_DESERT_SAND: u16 = 226;
pub const CONTENT_DESERT_STONE: u16 = 227;
pub const CONTENT_DESERT_COBBLE: u16 = 228;
pub const CONTENT_DESERT_STONE_BRICK: u16 = 229;
pub const CONTENT_DESERT_STONE_BLOCK: u16 = 230;
pub const CONTENT_DESERT_SANDSTONE: u16 = 231;
pub const CONTENT_DESERT_SANDSTONE_BRICK: u16 = 232;
pub const CONTENT_DESERT_SANDSTONE_BLOCK: u16 = 233;
pub const CONTENT_SILVER_SAND: u16 = 234;
pub const CONTENT_SILVER_SANDSTONE: u16 = 235;
pub const CONTENT_SILVER_SANDSTONE_BRICK: u16 = 236;
pub const CONTENT_SILVER_SANDSTONE_BLOCK: u16 = 237;
pub const CONTENT_SNOW_LAYER: u16 = 238;
pub const CONTENT_SNOW_BLOCK: u16 = 239;
pub const CONTENT_ICE: u16 = 240;
pub const CONTENT_PERMAFROST: u16 = 241;
pub const CONTENT_PERMAFROST_MOSS: u16 = 242;
pub const CONTENT_PERMAFROST_STONES: u16 = 243;
pub const CONTENT_CONIFEROUS_LITTER: u16 = 244;
pub const CONTENT_OBSIDIAN_BRICK: u16 = 245;
pub const CONTENT_OBSIDIAN_BLOCK: u16 = 246;
pub const CONTENT_OBSIDIAN_GLASS: u16 = 247;
/// Light a mese lamp gives (minetest_game's LIGHT_MAX, 14, less one as
/// for torches: the lit area, and the work, stay smaller).
pub const MESE_LAMP_LIGHT: u8 = 13;
/// minetest_game's mese post lights, one per wood: apple, birch, fir,
/// jungle and acacia (its default, aspen, pine, junglewood and acacia).
pub const MESE_POST_APPLE: u16 = stairs::MOSSY.end();
pub const MESE_POST_BIRCH: u16 = MESE_POST_APPLE + 1;
pub const MESE_POST_FIR: u16 = MESE_POST_APPLE + 2;
pub const MESE_POST_JUNGLE: u16 = MESE_POST_APPLE + 3;
pub const MESE_POST_ACACIA: u16 = MESE_POST_APPLE + 4;
/// Each post and the planks it's made with (for the tests).
#[cfg(test)]
pub const MESE_POSTS: [(u16, u16); 5] = [
    (MESE_POST_APPLE, CONTENT_APPLE_WOOD),
    (MESE_POST_BIRCH, CONTENT_BIRCH_WOOD),
    (MESE_POST_FIR, CONTENT_FIR_WOOD),
    (MESE_POST_JUNGLE, CONTENT_JUNGLE_WOOD),
    (MESE_POST_ACACIA, CONTENT_ACACIA_WOOD),
];
/// minetest_game's bookshelf: holds 16 books.
pub const CONTENT_BOOKSHELF: u16 = SIGN_STEEL + 1;
/// minetest_game's cloud: a white block nobody can dig, for admins and
/// map makers (given with /give).
pub const CONTENT_CLOUD: u16 = CONTENT_BOOKSHELF + 1;
/// minetest_game's dry shrub: a small dead bush in the deserts.
pub const DRY_SHRUB: u16 = CONTENT_CLOUD + 1;
/// Mist, our own: see Kind::Mist. For admins and map makers, like the
/// cloud.
pub const CONTENT_MIST: u16 = DRY_SHRUB + 1;
/// The vessels mod's bottles and glass, and its shelf.
pub const VESSELS_GLASS_BOTTLE: u16 = CONTENT_MIST + 1;
pub const VESSELS_DRINKING_GLASS: u16 = VESSELS_GLASS_BOTTLE + 1;
pub const VESSELS_STEEL_BOTTLE: u16 = VESSELS_GLASS_BOTTLE + 2;
pub const VESSELS_SHELF: u16 = VESSELS_GLASS_BOTTLE + 3;
/// minetest_game's cave ice: the icesheet's ice underground, which gives
/// ordinary ice when dug.
pub const CONTENT_CAVE_ICE: u16 = VESSELS_SHELF + 1;
/// minetest_game's emergent jungle tree sapling.
pub const EMERGENT_JUNGLE_SAPLING: u16 = CONTENT_CAVE_ICE + 1;
/// A vessel's selection box (vessels' {-0.25, -0.5, -0.25, 0.25, 0.3, 0.25}).
const VESSEL_BOX: [f32; 6] = [-0.25, -0.5, -0.25, 0.25, 0.3, 0.25];
/// The grey veil over the view of a player in mist (ARGB).
const MIST_POST_EFFECT: u32 = 0xA0D8DCE0;
/// Its param2: drawn as minetest_game places it (meshoptions 4).
pub const DRY_SHRUB_PARAM2: u8 = 4;
/// minetest_game's wooden and steel signs.
pub const SIGN_WOOD: u16 = MESE_POST_ACACIA + 1;
pub const SIGN_STEEL: u16 = SIGN_WOOD + 1;
/// A sign's board: on the ceiling, on the floor, and on a wall (the -X
/// side; the client turns it).
const SIGN_BOXES: [[f32; 6]; 3] = [
    [-0.4375, 0.4375, -0.3125, 0.4375, 0.5, 0.3125],
    [-0.4375, -0.5, -0.3125, 0.4375, -0.4375, 0.3125],
    [-0.5, -0.3125, -0.4375, -0.4375, 0.3125, 0.4375],
];
/// The post: 4/16 of a node square, full height.
const MESE_POST_BOX: [f32; 6] = [-2.0 / 16.0, -0.5, -2.0 / 16.0, 2.0 / 16.0, 0.5, 2.0 / 16.0];
/// Light a mese block gives (minetest_game's light_source 3).
pub const MESE_LIGHT: u8 = 3;
/// Light lava glows with (minetest_game's LIGHT_MAX - 1). Only the client
/// uses it, to draw lava bright: the server spreads no light from lava.
const LAVA_GLOW: u8 = 13;
/// Damage per second in lava (minetest_game's 4 * 2).
pub const LAVA_DAMAGE: u16 = 8;
/// Health an apple gives (minetest_game's item_eat(2)).
pub const APPLE_HP: u16 = 2;
/// Leaves give a sapling one time in this many (minetest_game's rarity).
pub const SAPLING_RARITY: u32 = 20;
/// Light a burning furnace gives (minetest_game's furnace_active).
pub const FURNACE_LIGHT: u8 = 8;

/// Items that aren't nodes (minetest_game's craftitems) have ids from
/// 0x1000 up, well clear of node content ids.
pub const ITEM_STICK: u16 = 0x1000;
pub const ITEM_COAL_LUMP: u16 = 0x1001;
pub const ITEM_IRON_LUMP: u16 = 0x1002;
pub const ITEM_STEEL_INGOT: u16 = 0x1003;
/// Door items (doors.rs): minetest_game's are craftitems that place the
/// door's two nodes.
pub const ITEM_DOOR_WOOD: u16 = 0x1004;
pub const ITEM_DOOR_STEEL: u16 = 0x1005;
/// xpanes' steel bar door, after the dyes.
pub const ITEM_DOOR_STEEL_BAR: u16 = 0x102D;
/// minetest_game's paper, book, and book with text.
pub const ITEM_PAPER: u16 = 0x102E;
pub const ITEM_BOOK: u16 = 0x102F;
pub const ITEM_BOOK_WRITTEN: u16 = 0x1030;
/// vessels' glass fragments; keys' skeleton key and key; binoculars; map's
/// mapping kit.
pub const ITEM_GLASS_FRAGMENTS: u16 = 0x1031;
pub const ITEM_SKELETON_KEY: u16 = 0x1032;
pub const ITEM_KEY: u16 = 0x1033;
pub const ITEM_BINOCULARS: u16 = 0x1034;
pub const ITEM_MAPPING_KIT: u16 = 0x1035;
/// The tnt mod's TNT stick.
pub const ITEM_TNT_STICK: u16 = 0x1036;
pub const ITEM_DOOR_GLASS: u16 = 0x1006;
/// minetest_game's buckets.
pub const ITEM_BUCKET_EMPTY: u16 = 0x1007;
pub const ITEM_BUCKET_WATER: u16 = 0x1008;
/// Farming's harvests and what is made of them (farming.rs).
pub const ITEM_WHEAT: u16 = 0x1009;
pub const ITEM_FLOUR: u16 = 0x100A;
pub const ITEM_BREAD: u16 = 0x100B;
pub const ITEM_COTTON: u16 = 0x100C;
pub const ITEM_STRING: u16 = 0x100D;
pub const ITEM_BUCKET_LAVA: u16 = 0x100E;
pub const ITEM_BLUEBERRIES: u16 = 0x100F;
pub const ITEM_COPPER_LUMP: u16 = 0x1010;
pub const ITEM_TIN_LUMP: u16 = 0x1011;
pub const ITEM_GOLD_LUMP: u16 = 0x1012;
pub const ITEM_MESE_CRYSTAL: u16 = 0x1013;
pub const ITEM_MESE_FRAGMENT: u16 = 0x1014;
pub const ITEM_DIAMOND: u16 = 0x1015;
pub const ITEM_COPPER_INGOT: u16 = 0x1016;
pub const ITEM_TIN_INGOT: u16 = 0x1017;
pub const ITEM_BRONZE_INGOT: u16 = 0x1018;
pub const ITEM_GOLD_INGOT: u16 = 0x1019;
pub const ITEM_FLINT: u16 = 0x101A;
pub const ITEM_CLAY_LUMP: u16 = 0x101B;
pub const ITEM_CLAY_BRICK: u16 = 0x101C;
pub const ITEM_OBSIDIAN_SHARD: u16 = 0x101D;
/// Health bread gives (minetest_game's item_eat(5)).
pub const BREAD_HP: u16 = 5;
/// Tools have ids from 0x2000 up.
pub const TOOL_PICK_WOOD: u16 = 0x2000;
pub const TOOL_PICK_STONE: u16 = 0x2001;
pub const TOOL_SHOVEL_WOOD: u16 = 0x2002;
pub const TOOL_SHOVEL_STONE: u16 = 0x2003;
pub const TOOL_AXE_WOOD: u16 = 0x2004;
pub const TOOL_AXE_STONE: u16 = 0x2005;
pub const TOOL_SWORD_WOOD: u16 = 0x2006;
pub const TOOL_SWORD_STONE: u16 = 0x2007;
pub const TOOL_PICK_STEEL: u16 = 0x2008;
pub const TOOL_SHOVEL_STEEL: u16 = 0x2009;
pub const TOOL_AXE_STEEL: u16 = 0x200A;
pub const TOOL_SWORD_STEEL: u16 = 0x200B;
pub const TOOL_HOE_WOOD: u16 = 0x200C;
pub const TOOL_HOE_STONE: u16 = 0x200D;
pub const TOOL_HOE_STEEL: u16 = 0x200E;
pub const TOOL_PICK_BRONZE: u16 = 0x200F;
pub const TOOL_SHOVEL_BRONZE: u16 = 0x2010;
pub const TOOL_AXE_BRONZE: u16 = 0x2011;
pub const TOOL_SWORD_BRONZE: u16 = 0x2012;
pub const TOOL_PICK_MESE: u16 = 0x2013;
pub const TOOL_SHOVEL_MESE: u16 = 0x2014;
pub const TOOL_AXE_MESE: u16 = 0x2015;
pub const TOOL_SWORD_MESE: u16 = 0x2016;
pub const TOOL_PICK_DIAMOND: u16 = 0x2017;
pub const TOOL_SHOVEL_DIAMOND: u16 = 0x2018;
pub const TOOL_AXE_DIAMOND: u16 = 0x2019;
pub const TOOL_SWORD_DIAMOND: u16 = 0x201A;
/// The screwdriver mod's screwdriver.
pub const TOOL_SCREWDRIVER: u16 = 0x201B;
/// The fire mod's flint and steel.
pub const TOOL_FLINT_AND_STEEL: u16 = 0x201C;
/// Light given off by a torch (minetest_game: LIGHT_MAX - 1 = 13; 12 is
/// slightly gentler and keeps the lit area, and the work, smaller).
pub const TORCH_LIGHT: u8 = 12;

/// A stair or slab: it drops itself.
macro_rules! shaped {
    ($id:expr, $name:literal, $description:literal, $tile:literal, $groups:expr) => {
        Node {
            id: $id,
            name: concat!("esp:", $name),
            description: $description,
            tiles: [$tile; 6],
            groups: $groups,
            drop: $id,
            drop_count: 1,
            kind: Kind::Shaped,
        }
    };
}

/// A door's bottom node: it drops the door item.
macro_rules! door {
    ($id:expr, $name:literal, $description:literal, $tile:literal, $groups:expr, $item:expr) => {
        Node {
            id: $id,
            name: concat!("esp:", $name),
            description: $description,
            tiles: [$tile; 6],
            groups: $groups,
            drop: $item,
            drop_count: 1,
            kind: Kind::Door,
        }
    };
}

/// A plant (Kind::Plant): what it drops is farming::drops.
macro_rules! plant {
    ($id:expr, $name:literal, $description:literal, $tile:literal, $groups:expr) => {
        Node {
            id: $id,
            name: concat!("esp:", $name),
            description: $description,
            tiles: [$tile; 6],
            groups: $groups,
            drop: $id,
            drop_count: 0,
            kind: Kind::Plant,
        }
    };
}

/// A planted seed: it gives itself back.
macro_rules! seed {
    ($id:expr, $name:literal, $description:literal, $tile:literal) => {
        Node {
            id: $id,
            name: concat!("esp:", $name),
            description: $description,
            tiles: [$tile; 6],
            groups: &[("seed", 1), ("snappy", 3), ("attached_node", 1), ("flammable", 2), ("grassland", 1)],
            drop: $id,
            drop_count: 1,
            kind: Kind::Seed,
        }
    };
}

/// Content IDs 125, 126 and 127 are reserved for unknown, air and ignore.
/// Listed in id order, without gaps except those three, so that an id
/// finds its node at once.
pub const NODES: &[Node] = &[
    Node {
        id: CONTENT_STONE,
        name: "esp:stone",
        description: "Stone",
        tiles: ["esp_stone.png"; 6],
        groups: &[("cracky", 3), ("stone", 1)],
        // minetest_game: stone gives cobblestone.
        drop: CONTENT_COBBLE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DIRT,
        name: "esp:dirt",
        description: "Dirt",
        tiles: ["esp_dirt.png"; 6],
        groups: &[("crumbly", 3), ("soil", 1)],
        drop: CONTENT_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_GRASS,
        name: "esp:dirt_with_grass",
        description: "Dirt with Grass",
        tiles: [
            "esp_grass.png",
            "esp_dirt.png",
            "esp_grass_side.png",
            "esp_grass_side.png",
            "esp_grass_side.png",
            "esp_grass_side.png",
        ],
        groups: &[("crumbly", 3), ("soil", 1), ("spreading_dirt_type", 1)],
        drop: CONTENT_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SAND,
        name: "esp:sand",
        description: "Sand",
        tiles: ["esp_sand.png"; 6],
        groups: &[("crumbly", 3), ("sand", 1), ("falling_node", 1)],
        drop: CONTENT_SAND,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SANDSTONE,
        name: "esp:sandstone",
        description: "Sandstone",
        tiles: ["esp_sandstone.png"; 6],
        groups: &[("crumbly", 1), ("cracky", 3)],
        drop: CONTENT_SANDSTONE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SNOW,
        name: "esp:dirt_with_snow",
        description: "Dirt with Snow",
        tiles: [
            "esp_snow.png",
            "esp_dirt.png",
            "esp_snow_side.png",
            "esp_snow_side.png",
            "esp_snow_side.png",
            "esp_snow_side.png",
        ],
        groups: &[("crumbly", 3), ("soil", 1), ("spreading_dirt_type", 1)],
        drop: CONTENT_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // Ores give lumps, as in minetest_game.
    Node {
        id: CONTENT_COAL,
        name: "esp:stone_with_coal",
        description: "Coal Ore",
        tiles: ["esp_stone_with_coal.png"; 6],
        groups: &[("cracky", 3)],
        drop: ITEM_COAL_LUMP,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_IRON,
        name: "esp:stone_with_iron",
        description: "Iron Ore",
        tiles: ["esp_stone_with_iron.png"; 6],
        // Needs a stone pickaxe or better.
        groups: &[("cracky", 2)],
        drop: ITEM_IRON_LUMP,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_TORCH,
        name: "esp:torch",
        description: "Torch",
        tiles: [TORCH_TILE; 6],
        groups: &[("dig_immediate", 3), ("attached_node", 1), ("torch", 1)],
        drop: CONTENT_TORCH,
        drop_count: 1,
        kind: Kind::Torch,
    },
    Node {
        id: CONTENT_TORCH_WALL,
        name: "esp:torch_wall",
        description: "Torch",
        tiles: [TORCH_TILE; 6],
        groups: &[("dig_immediate", 3), ("not_in_creative_inventory", 1), ("attached_node", 1), ("torch", 1)],
        drop: CONTENT_TORCH,
        drop_count: 1,
        kind: Kind::Torch,
    },
    Node {
        id: CONTENT_TORCH_CEILING,
        name: "esp:torch_ceiling",
        description: "Torch",
        tiles: [TORCH_TILE; 6],
        groups: &[("dig_immediate", 3), ("not_in_creative_inventory", 1), ("attached_node", 1), ("torch", 1)],
        drop: CONTENT_TORCH,
        drop_count: 1,
        kind: Kind::Torch,
    },
    // Trees, as minetest_game's: trunks are dug (slowly) by hand through
    // oddly_breakable_by_hand, leaves quickly through snappy.
    Node {
        id: CONTENT_BIRCH_TREE,
        name: "esp:birch_tree",
        description: "Birch Tree",
        tiles: [
            "esp_birch_tree_top.png",
            "esp_birch_tree_top.png",
            "esp_birch_tree.png",
            "esp_birch_tree.png",
            "esp_birch_tree.png",
            "esp_birch_tree.png",
        ],
        groups: &[("tree", 1), ("choppy", 3), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: CONTENT_BIRCH_TREE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_BIRCH_LEAVES,
        name: "esp:birch_leaves",
        description: "Birch Leaves",
        tiles: ["esp_birch_leaves.png"; 6],
        groups: &[("snappy", 3), ("leaves", 1), ("flammable", 2)],
        drop: CONTENT_BIRCH_LEAVES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: CONTENT_FIR_TREE,
        name: "esp:fir_tree",
        description: "Fir Tree",
        tiles: [
            "esp_fir_tree_top.png",
            "esp_fir_tree_top.png",
            "esp_fir_tree.png",
            "esp_fir_tree.png",
            "esp_fir_tree.png",
            "esp_fir_tree.png",
        ],
        groups: &[("tree", 1), ("choppy", 3), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: CONTENT_FIR_TREE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_FIR_NEEDLES,
        name: "esp:fir_needles",
        description: "Fir Needles",
        tiles: ["esp_fir_needles.png"; 6],
        groups: &[("snappy", 3), ("leaves", 1), ("flammable", 2)],
        drop: CONTENT_FIR_NEEDLES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    // Planks, as minetest_game's aspen and pine wood: four from a trunk.
    Node {
        id: CONTENT_BIRCH_WOOD,
        name: "esp:birch_wood",
        description: "Birch Wood Planks",
        tiles: ["esp_birch_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("wood", 1)],
        drop: CONTENT_BIRCH_WOOD,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_FIR_WOOD,
        name: "esp:fir_wood",
        description: "Fir Wood Planks",
        tiles: ["esp_fir_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("wood", 1)],
        drop: CONTENT_FIR_WOOD,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_COBBLE,
        name: "esp:cobble",
        description: "Cobblestone",
        tiles: ["esp_cobble.png"; 6],
        groups: &[("cracky", 3), ("stone", 2)],
        drop: CONTENT_COBBLE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // minetest_game's furnace and its textures (CC BY-SA 3.0, see
    // media/LICENSE-items.txt). Cracky 2: a stone pickaxe digs it.
    Node {
        id: CONTENT_FURNACE,
        name: "esp:furnace",
        description: "Furnace",
        tiles: [
            "default_furnace_top.png",
            "default_furnace_bottom.png",
            "default_furnace_side.png",
            "default_furnace_side.png",
            "default_furnace_side.png",
            "default_furnace_front.png",
        ],
        groups: &[("cracky", 2)],
        drop: CONTENT_FURNACE,
        drop_count: 1,
        kind: Kind::Furnace,
    },
    // A furnace while it burns: the fire shows at the front, it glows,
    // and digging it gives an ordinary furnace.
    Node {
        id: CONTENT_FURNACE_ACTIVE,
        name: "esp:furnace_active",
        description: "Furnace",
        tiles: [
            "default_furnace_top.png",
            "default_furnace_bottom.png",
            "default_furnace_side.png",
            "default_furnace_side.png",
            "default_furnace_side.png",
            FURNACE_ACTIVE_TILE,
        ],
        groups: &[("cracky", 2), ("not_in_creative_inventory", 1)],
        drop: CONTENT_FURNACE,
        drop_count: 1,
        kind: Kind::Furnace,
    },
    Node {
        id: CONTENT_BIRCH_SAPLING,
        name: "esp:birch_sapling",
        description: "Birch Sapling",
        tiles: ["default_aspen_sapling.png"; 6],
        groups: SAPLING_GROUPS,
        drop: CONTENT_BIRCH_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: CONTENT_FIR_SAPLING,
        name: "esp:fir_sapling",
        description: "Fir Sapling",
        tiles: ["default_pine_sapling.png"; 6],
        groups: SAPLING_GROUPS,
        drop: CONTENT_FIR_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: CONTENT_APPLE_TREE,
        name: "esp:apple_tree",
        description: "Apple Tree",
        tiles: [
            "esp_apple_tree_top.png",
            "esp_apple_tree_top.png",
            "esp_apple_tree.png",
            "esp_apple_tree.png",
            "esp_apple_tree.png",
            "esp_apple_tree.png",
        ],
        groups: &[("tree", 1), ("choppy", 2), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: CONTENT_APPLE_TREE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_APPLE_LEAVES,
        name: "esp:apple_leaves",
        description: "Apple Tree Leaves",
        tiles: ["esp_apple_leaves.png"; 6],
        groups: &[("snappy", 3), ("leafdecay", 3), ("flammable", 2), ("leaves", 1)],
        drop: CONTENT_APPLE_LEAVES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    // minetest_game's apple and its picture (CC BY-SA 3.0).
    Node {
        id: CONTENT_APPLE,
        name: "esp:apple",
        description: "Apple",
        tiles: ["default_apple.png"; 6],
        groups: &[("fleshy", 3), ("dig_immediate", 3), ("flammable", 2), ("leafdecay", 3), ("leafdecay_drop", 1), ("food_apple", 1)],
        drop: CONTENT_APPLE,
        drop_count: 1,
        kind: Kind::Apple,
    },
    Node {
        id: CONTENT_APPLE_MARK,
        name: "esp:apple_mark",
        description: "Apple Marker",
        tiles: [""; 6],
        groups: &[("not_in_creative_inventory", 1)],
        drop: CONTENT_APPLE_MARK,
        drop_count: 0,
        kind: Kind::Mark,
    },
    Node {
        id: CONTENT_APPLE_SAPLING,
        name: "esp:apple_sapling",
        description: "Apple Tree Sapling",
        tiles: ["default_sapling.png"; 6],
        groups: SAPLING_GROUPS,
        drop: CONTENT_APPLE_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: CONTENT_APPLE_WOOD,
        name: "esp:apple_wood",
        description: "Apple Wood Planks",
        tiles: ["esp_apple_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("wood", 1)],
        drop: CONTENT_APPLE_WOOD,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // minetest_game's chests, with its textures and open-lid model
    // (CC BY-SA 3.0, see media/LICENSE-items.txt). Closed, the faces are
    // arranged as minetest_game does for its plain cube: the front at -Z
    // (towards whoever placed it), the side texture mirrored at +X.
    Node {
        id: CONTENT_CHEST,
        name: "esp:chest",
        description: "Chest",
        tiles: CHEST_CLOSED_TILES,
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2)],
        drop: CONTENT_CHEST,
        drop_count: 1,
        kind: Kind::Chest,
    },
    Node {
        id: CONTENT_CHEST_OPEN,
        name: "esp:chest_open",
        description: "Chest",
        tiles: CHEST_OPEN_TILES,
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("not_in_creative_inventory", 1)],
        drop: CONTENT_CHEST,
        drop_count: 1,
        kind: Kind::ChestOpen,
    },
    Node {
        id: CONTENT_CHEST_LOCKED,
        name: "esp:chest_locked",
        description: "Locked Chest",
        tiles: CHEST_LOCKED_CLOSED_TILES,
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2)],
        drop: CONTENT_CHEST_LOCKED,
        drop_count: 1,
        kind: Kind::Chest,
    },
    Node {
        id: CONTENT_CHEST_LOCKED_OPEN,
        name: "esp:chest_locked_open",
        description: "Locked Chest",
        tiles: CHEST_LOCKED_OPEN_TILES,
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("not_in_creative_inventory", 1)],
        drop: CONTENT_CHEST_LOCKED,
        drop_count: 1,
        kind: Kind::ChestOpen,
    },
    // Building blocks from minetest_game's recipes. The stone and sandstone
    // ones are drawn to match our stone and sandstone; the others use
    // minetest_game's textures (CC BY-SA 3.0, see media/LICENSE-items.txt).
    Node {
        id: CONTENT_STONE_BRICK,
        name: "esp:stonebrick",
        description: "Stone Brick",
        tiles: ["esp_stone_brick.png"; 6],
        groups: &[("cracky", 2), ("stone", 1)],
        drop: CONTENT_STONE_BRICK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_STONE_BLOCK,
        name: "esp:stone_block",
        description: "Stone Block",
        tiles: ["esp_stone_block.png"; 6],
        groups: &[("cracky", 2), ("stone", 1)],
        drop: CONTENT_STONE_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SANDSTONE_BRICK,
        name: "esp:sandstonebrick",
        description: "Sandstone Brick",
        tiles: ["esp_sandstone_brick.png"; 6],
        groups: &[("cracky", 2)],
        drop: CONTENT_SANDSTONE_BRICK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SANDSTONE_BLOCK,
        name: "esp:sandstone_block",
        description: "Sandstone Block",
        tiles: ["esp_sandstone_block.png"; 6],
        groups: &[("cracky", 2)],
        drop: CONTENT_SANDSTONE_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_COAL_BLOCK,
        name: "esp:coalblock",
        description: "Coal Block",
        tiles: ["default_coal_block.png"; 6],
        groups: &[("cracky", 3)],
        drop: CONTENT_COAL_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // Level 2: only a steel pickaxe digs it.
    Node {
        id: CONTENT_STEEL_BLOCK,
        name: "esp:steelblock",
        description: "Steel Block",
        tiles: ["default_steel_block.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_STEEL_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // The frame picture, then the detail, as minetest_game's two tiles
    // fill the six.
    Node {
        id: CONTENT_GLASS,
        name: "esp:glass",
        description: "Glass",
        tiles: [
            "default_glass.png",
            "default_glass_detail.png",
            "default_glass_detail.png",
            "default_glass_detail.png",
            "default_glass_detail.png",
            "default_glass_detail.png",
        ],
        groups: &[("cracky", 3), ("oddly_breakable_by_hand", 3)],
        drop: CONTENT_GLASS,
        drop_count: 1,
        kind: Kind::Glass,
    },
    Node {
        id: CONTENT_LADDER_WOOD,
        name: "esp:ladder_wood",
        description: "Wooden Ladder",
        tiles: ["default_ladder_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 2)],
        drop: CONTENT_LADDER_WOOD,
        drop_count: 1,
        kind: Kind::Ladder,
    },
    Node {
        id: CONTENT_LADDER_STEEL,
        name: "esp:ladder_steel",
        description: "Steel Ladder",
        tiles: ["default_ladder_steel.png"; 6],
        groups: &[("cracky", 2)],
        drop: CONTENT_LADDER_STEEL,
        drop_count: 1,
        kind: Kind::Ladder,
    },
    // Stairs, slabs and corner stairs (stairs.rs), in its MATERIALS order: minetest_game's
    // groups for each, without "wood" or "stone" (so they don't craft as
    // planks or stone), and the block's own picture on every side.
    shaped!(stairs::FIRST_STAIR + 0, "stair_birch_wood", "Birch Wood Stair", "esp_birch_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 1, "stair_fir_wood", "Fir Wood Stair", "esp_fir_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 2, "stair_apple_wood", "Apple Wood Stair", "esp_apple_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 3, "stair_stone", "Stone Stair", "esp_stone.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 4, "stair_cobble", "Cobblestone Stair", "esp_cobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 5, "stair_stonebrick", "Stone Brick Stair", "esp_stone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 6, "stair_stone_block", "Stone Block Stair", "esp_stone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 7, "stair_sandstone", "Sandstone Stair", "esp_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 8, "stair_sandstonebrick", "Sandstone Brick Stair", "esp_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_STAIR + 9, "stair_sandstone_block", "Sandstone Block Stair", "esp_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_SLAB + 0, "slab_birch_wood", "Birch Wood Slab", "esp_birch_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 1, "slab_fir_wood", "Fir Wood Slab", "esp_fir_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 2, "slab_apple_wood", "Apple Wood Slab", "esp_apple_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 3, "slab_stone", "Stone Slab", "esp_stone.png", &[("cracky", 3), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 4, "slab_cobble", "Cobblestone Slab", "esp_cobble.png", &[("cracky", 3), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 5, "slab_stonebrick", "Stone Brick Slab", "esp_stone_brick.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 6, "slab_stone_block", "Stone Block Slab", "esp_stone_block.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 7, "slab_sandstone", "Sandstone Slab", "esp_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 8, "slab_sandstonebrick", "Sandstone Brick Slab", "esp_sandstone_brick.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::FIRST_SLAB + 9, "slab_sandstone_block", "Sandstone Block Slab", "esp_sandstone_block.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::FIRST_INNER + 0, "stair_inner_birch_wood", "Inner Birch Wood Stair", "esp_birch_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 1, "stair_inner_fir_wood", "Inner Fir Wood Stair", "esp_fir_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 2, "stair_inner_apple_wood", "Inner Apple Wood Stair", "esp_apple_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 3, "stair_inner_stone", "Inner Stone Stair", "esp_stone.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 4, "stair_inner_cobble", "Inner Cobblestone Stair", "esp_cobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 5, "stair_inner_stonebrick", "Inner Stone Brick Stair", "esp_stone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 6, "stair_inner_stone_block", "Inner Stone Block Stair", "esp_stone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 7, "stair_inner_sandstone", "Inner Sandstone Stair", "esp_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 8, "stair_inner_sandstonebrick", "Inner Sandstone Brick Stair", "esp_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_INNER + 9, "stair_inner_sandstone_block", "Inner Sandstone Block Stair", "esp_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 0, "stair_outer_birch_wood", "Outer Birch Wood Stair", "esp_birch_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 1, "stair_outer_fir_wood", "Outer Fir Wood Stair", "esp_fir_wood.png", &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 2, "stair_outer_apple_wood", "Outer Apple Wood Stair", "esp_apple_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 3, "stair_outer_stone", "Outer Stone Stair", "esp_stone.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 4, "stair_outer_cobble", "Outer Cobblestone Stair", "esp_cobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 5, "stair_outer_stonebrick", "Outer Stone Brick Stair", "esp_stone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 6, "stair_outer_stone_block", "Outer Stone Block Stair", "esp_stone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 7, "stair_outer_sandstone", "Outer Sandstone Stair", "esp_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 8, "stair_outer_sandstonebrick", "Outer Sandstone Brick Stair", "esp_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::FIRST_OUTER + 9, "stair_outer_sandstone_block", "Outer Sandstone Block Stair", "esp_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    // Doors (doors.rs): wooden, steel and glass, each as nodes a, b, c and d,
    // then the hidden node above them all.
    door!(doors::FIRST_DOOR + 0, "door_wood_a", "Wooden Door", "doors_door_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("door", 1)], ITEM_DOOR_WOOD),
    door!(doors::FIRST_DOOR + 1, "door_wood_b", "Wooden Door", "doors_door_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("door", 1)], ITEM_DOOR_WOOD),
    door!(doors::FIRST_DOOR + 2, "door_wood_c", "Wooden Door", "doors_door_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("door", 1)], ITEM_DOOR_WOOD),
    door!(doors::FIRST_DOOR + 3, "door_wood_d", "Wooden Door", "doors_door_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("door", 1)], ITEM_DOOR_WOOD),
    door!(doors::FIRST_DOOR + 4, "door_steel_a", "Steel Door", "doors_door_steel.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL),
    door!(doors::FIRST_DOOR + 5, "door_steel_b", "Steel Door", "doors_door_steel.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL),
    door!(doors::FIRST_DOOR + 6, "door_steel_c", "Steel Door", "doors_door_steel.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL),
    door!(doors::FIRST_DOOR + 7, "door_steel_d", "Steel Door", "doors_door_steel.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL),
    door!(doors::FIRST_DOOR + 8, "door_glass_a", "Glass Door", "doors_door_glass.png", &[("cracky", 3), ("oddly_breakable_by_hand", 3), ("door", 1)], ITEM_DOOR_GLASS),
    door!(doors::FIRST_DOOR + 9, "door_glass_b", "Glass Door", "doors_door_glass.png", &[("cracky", 3), ("oddly_breakable_by_hand", 3), ("door", 1)], ITEM_DOOR_GLASS),
    door!(doors::FIRST_DOOR + 10, "door_glass_c", "Glass Door", "doors_door_glass.png", &[("cracky", 3), ("oddly_breakable_by_hand", 3), ("door", 1)], ITEM_DOOR_GLASS),
    door!(doors::FIRST_DOOR + 11, "door_glass_d", "Glass Door", "doors_door_glass.png", &[("cracky", 3), ("oddly_breakable_by_hand", 3), ("door", 1)], ITEM_DOOR_GLASS),
    Node {
        id: doors::HIDDEN,
        name: "esp:door_hidden",
        description: "Hidden Door Segment",
        tiles: [""; 6],
        groups: &[("not_in_creative_inventory", 1)],
        drop: doors::HIDDEN,
        drop_count: 0,
        kind: Kind::DoorHidden,
    },
    // Trapdoors, closed then open, with minetest_game's pictures turned
    // as it turns them for each side (CC BY-SA 3.0, see
    // media/LICENSE-doors.txt). The open one gives the closed one.
    Node {
        id: doors::FIRST_TRAPDOOR,
        name: "esp:trapdoor",
        description: "Wooden Trapdoor",
        tiles: TRAPDOOR_WOOD_CLOSED,
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("door", 1)],
        drop: doors::FIRST_TRAPDOOR,
        drop_count: 1,
        kind: Kind::Trapdoor,
    },
    Node {
        id: doors::FIRST_TRAPDOOR + 1,
        name: "esp:trapdoor_open",
        description: "Wooden Trapdoor",
        tiles: TRAPDOOR_WOOD_OPEN,
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("door", 1), ("not_in_creative_inventory", 1)],
        drop: doors::FIRST_TRAPDOOR,
        drop_count: 1,
        kind: Kind::Trapdoor,
    },
    Node {
        id: doors::FIRST_TRAPDOOR + 2,
        name: "esp:trapdoor_steel",
        description: "Steel Trapdoor",
        tiles: TRAPDOOR_STEEL_CLOSED,
        groups: &[("cracky", 1), ("level", 2), ("door", 1)],
        drop: doors::FIRST_TRAPDOOR + 2,
        drop_count: 1,
        kind: Kind::Trapdoor,
    },
    Node {
        id: doors::FIRST_TRAPDOOR + 3,
        name: "esp:trapdoor_steel_open",
        description: "Steel Trapdoor",
        tiles: TRAPDOOR_STEEL_OPEN,
        groups: &[("cracky", 1), ("level", 2), ("door", 1), ("not_in_creative_inventory", 1)],
        drop: doors::FIRST_TRAPDOOR + 2,
        drop_count: 1,
        kind: Kind::Trapdoor,
    },
    // minetest_game's water (CC BY-SA 3.0, see media/LICENSE-items.txt).
    // Still water only, so far: the flowing node is there for the client,
    // which draws a source's edges by it.
    Node {
        id: CONTENT_WATER,
        name: "esp:water_source",
        description: "Water Source",
        tiles: [WATER_SOURCE_TILE; 6],
        groups: &[("water", 3), ("liquid", 3), ("cools_lava", 1)],
        drop: CONTENT_WATER,
        drop_count: 0,
        kind: Kind::Liquid,
    },
    Node {
        id: CONTENT_WATER_FLOWING,
        name: "esp:water_flowing",
        description: "Flowing Water",
        tiles: ["default_water.png"; 6],
        groups: &[("water", 3), ("liquid", 3), ("not_in_creative_inventory", 1), ("cools_lava", 1)],
        drop: CONTENT_WATER_FLOWING,
        drop_count: 0,
        kind: Kind::Liquid,
    },
    // Farming (farming.rs), with minetest_game's pictures (see
    // media/LICENSE-farming.txt). What digging gives is farming::drops.
    plant!(farming::GRASS[0], "grass_1", "Grass", "default_grass_1.png", GRASS_GROUPS),
    plant!(farming::GRASS[1], "grass_2", "Grass", "default_grass_2.png", GRASS_GROUPS),
    plant!(farming::GRASS[2], "grass_3", "Grass", "default_grass_3.png", GRASS_GROUPS),
    plant!(farming::GRASS[3], "grass_4", "Grass", "default_grass_4.png", GRASS_GROUPS),
    plant!(farming::GRASS[4], "grass_5", "Grass", "default_grass_5.png", GRASS_GROUPS),
    plant!(farming::COTTON_WILD, "cotton_wild", "Wild Cotton", "farming_cotton_wild.png", &[("snappy", 3), ("attached_node", 1), ("flammable", 4)]),
    Node {
        id: farming::SOIL,
        name: "esp:soil",
        description: "Soil",
        tiles: ["esp_dirt.png^farming_soil.png", "esp_dirt.png", "esp_dirt.png", "esp_dirt.png", "esp_dirt.png", "esp_dirt.png"],
        groups: &[("crumbly", 3), ("not_in_creative_inventory", 1), ("soil", 2), ("grassland", 1), ("field", 1)],
        drop: CONTENT_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: farming::SOIL_WET,
        name: "esp:soil_wet",
        description: "Wet Soil",
        tiles: [
            "esp_dirt.png^farming_soil_wet.png",
            "esp_dirt.png",
            "esp_dirt.png^farming_soil_wet_side.png",
            "esp_dirt.png^farming_soil_wet_side.png",
            "esp_dirt.png^farming_soil_wet_side.png",
            "esp_dirt.png^farming_soil_wet_side.png",
        ],
        groups: &[("crumbly", 3), ("not_in_creative_inventory", 1), ("soil", 3), ("wet", 1), ("grassland", 1), ("field", 1)],
        drop: CONTENT_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    seed!(farming::SEED_WHEAT, "seed_wheat", "Wheat Seed", "farming_wheat_seed.png"),
    seed!(farming::SEED_COTTON, "seed_cotton", "Cotton Seed", "farming_cotton_seed.png"),
    plant!(farming::WHEAT_STAGES[0], "wheat_1", "Wheat", "farming_wheat_1.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 1)]),
    plant!(farming::WHEAT_STAGES[1], "wheat_2", "Wheat", "farming_wheat_2.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 2)]),
    plant!(farming::WHEAT_STAGES[2], "wheat_3", "Wheat", "farming_wheat_3.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 3)]),
    plant!(farming::WHEAT_STAGES[3], "wheat_4", "Wheat", "farming_wheat_4.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 4)]),
    plant!(farming::WHEAT_STAGES[4], "wheat_5", "Wheat", "farming_wheat_5.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 5)]),
    plant!(farming::WHEAT_STAGES[5], "wheat_6", "Wheat", "farming_wheat_6.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 6)]),
    plant!(farming::WHEAT_STAGES[6], "wheat_7", "Wheat", "farming_wheat_7.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 7)]),
    plant!(farming::WHEAT_STAGES[7], "wheat_8", "Wheat", "farming_wheat_8.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("wheat", 8)]),
    Node {
        id: farming::STRAW,
        name: "esp:straw",
        description: "Straw",
        tiles: ["farming_straw.png"; 6],
        groups: &[("snappy", 3), ("flammable", 4), ("fall_damage_add_percent", -30)],
        drop: farming::STRAW,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: farming::WOOL_WHITE,
        name: "esp:wool_white",
        description: "White Wool",
        tiles: ["wool_white.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_white", 1)],
        drop: farming::WOOL_WHITE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    plant!(farming::COTTON_STAGES[0], "cotton_1", "Cotton", "farming_cotton_1.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 1)]),
    plant!(farming::COTTON_STAGES[1], "cotton_2", "Cotton", "farming_cotton_2.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 2)]),
    plant!(farming::COTTON_STAGES[2], "cotton_3", "Cotton", "farming_cotton_3.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 3)]),
    plant!(farming::COTTON_STAGES[3], "cotton_4", "Cotton", "farming_cotton_4.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 4)]),
    plant!(farming::COTTON_STAGES[4], "cotton_5", "Cotton", "farming_cotton_5.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 5)]),
    // Luanti's own content ids 125 to 127 come here.
    plant!(farming::COTTON_STAGES[5], "cotton_6", "Cotton", "farming_cotton_6.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 6)]),
    plant!(farming::COTTON_STAGES[6], "cotton_7", "Cotton", "farming_cotton_7.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 7)]),
    plant!(farming::COTTON_STAGES[7], "cotton_8", "Cotton", "farming_cotton_8.png", &[("snappy", 3), ("flammable", 2), ("plant", 1), ("not_in_creative_inventory", 1), ("attached_node", 1), ("cotton", 8)]),
    // Beds (beds.rs), with minetest_game's textures (CC BY-SA 3.0, see
    // media/LICENSE-beds.txt). The top drops nothing.
    Node {
        id: beds::BED_BOTTOM,
        name: "esp:bed_bottom",
        description: "Simple Bed",
        tiles: [
            "beds_bed_top_bottom.png^[transformR90",
            "beds_bed_under.png",
            "beds_bed_side_bottom_r.png",
            "beds_bed_side_bottom_r.png^[transformFX",
            "blank.png",
            "beds_bed_side_bottom.png",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("bed", 1)],
        drop: beds::BED_BOTTOM,
        drop_count: 1,
        kind: Kind::Bed,
    },
    Node {
        id: beds::BED_TOP,
        name: "esp:bed_top",
        description: "Simple Bed",
        tiles: [
            "beds_bed_top_top.png^[transformR90",
            "beds_bed_under.png",
            "beds_bed_side_top_r.png",
            "beds_bed_side_top_r.png^[transformFX",
            "beds_bed_side_top.png",
            "blank.png",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("bed", 2), ("not_in_creative_inventory", 1)],
        drop: beds::BED_TOP,
        drop_count: 0,
        kind: Kind::BedTop,
    },
    Node {
        id: beds::FANCY_BOTTOM,
        name: "esp:fancy_bed_bottom",
        description: "Fancy Bed",
        tiles: [
            "beds_bed_top1.png",
            "beds_bed_under.png",
            "beds_bed_side1.png",
            "beds_bed_side1.png^[transformFX",
            "beds_bed_foot.png",
            "beds_bed_foot.png",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("bed", 1)],
        drop: beds::FANCY_BOTTOM,
        drop_count: 1,
        kind: Kind::Bed,
    },
    Node {
        id: beds::FANCY_TOP,
        name: "esp:fancy_bed_top",
        description: "Fancy Bed",
        tiles: [
            "beds_bed_top2.png",
            "beds_bed_under.png",
            "beds_bed_side2.png",
            "beds_bed_side2.png^[transformFX",
            "beds_bed_head.png",
            "beds_bed_head.png",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("bed", 2), ("not_in_creative_inventory", 1)],
        drop: beds::FANCY_TOP,
        drop_count: 0,
        kind: Kind::BedTop,
    },
    // minetest_game's lava and obsidian (CC BY-SA 3.0, see
    // media/LICENSE-items.txt).
    Node {
        id: CONTENT_LAVA,
        name: "esp:lava_source",
        description: "Lava Source",
        tiles: [LAVA_SOURCE_TILE; 6],
        groups: &[("lava", 3), ("liquid", 2), ("igniter", 1)],
        drop: CONTENT_LAVA,
        drop_count: 0,
        kind: Kind::Liquid,
    },
    Node {
        id: CONTENT_LAVA_FLOWING,
        name: "esp:lava_flowing",
        description: "Flowing Lava",
        tiles: ["default_lava.png"; 6],
        groups: &[("lava", 3), ("liquid", 2), ("igniter", 1), ("not_in_creative_inventory", 1)],
        drop: CONTENT_LAVA_FLOWING,
        drop_count: 0,
        kind: Kind::Liquid,
    },
    Node {
        id: CONTENT_OBSIDIAN,
        name: "esp:obsidian",
        description: "Obsidian",
        tiles: ["default_obsidian.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_OBSIDIAN,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // minetest_game's fences and fence gates (fences.rs), of our planks.
    Node {
        id: fences::FENCE_APPLE,
        name: "esp:fence_apple_wood",
        description: "Apple Wood Fence",
        tiles: ["esp_apple_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::FENCE_APPLE,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::FENCE_FIR,
        name: "esp:fence_fir_wood",
        description: "Fir Wood Fence",
        tiles: ["esp_fir_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1)],
        drop: fences::FENCE_FIR,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::FENCE_BIRCH,
        name: "esp:fence_birch_wood",
        description: "Birch Wood Fence",
        tiles: ["esp_birch_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1)],
        drop: fences::FENCE_BIRCH,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::GATE_APPLE,
        name: "esp:gate_apple_wood_closed",
        description: "Apple Wood Fence Gate",
        tiles: ["esp_apple_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::GATE_APPLE,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_APPLE + 1,
        name: "esp:gate_apple_wood_open",
        description: "Apple Wood Fence Gate",
        tiles: ["esp_apple_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1), ("not_in_creative_inventory", 1)],
        drop: fences::GATE_APPLE,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_FIR,
        name: "esp:gate_fir_wood_closed",
        description: "Fir Wood Fence Gate",
        tiles: ["esp_fir_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1)],
        drop: fences::GATE_FIR,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_FIR + 1,
        name: "esp:gate_fir_wood_open",
        description: "Fir Wood Fence Gate",
        tiles: ["esp_fir_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1), ("not_in_creative_inventory", 1)],
        drop: fences::GATE_FIR,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_BIRCH,
        name: "esp:gate_birch_wood_closed",
        description: "Birch Wood Fence Gate",
        tiles: ["esp_birch_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1)],
        drop: fences::GATE_BIRCH,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_BIRCH + 1,
        name: "esp:gate_birch_wood_open",
        description: "Birch Wood Fence Gate",
        tiles: ["esp_birch_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1), ("not_in_creative_inventory", 1)],
        drop: fences::GATE_BIRCH,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::RAIL_APPLE,
        name: "esp:fence_rail_apple_wood",
        description: "Apple Wood Fence Rail",
        tiles: ["esp_apple_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::RAIL_APPLE,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::RAIL_FIR,
        name: "esp:fence_rail_fir_wood",
        description: "Fir Wood Fence Rail",
        tiles: ["esp_fir_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1)],
        drop: fences::RAIL_FIR,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::RAIL_BIRCH,
        name: "esp:fence_rail_birch_wood",
        description: "Birch Wood Fence Rail",
        tiles: ["esp_birch_wood.png"; 6],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3), ("fence", 1)],
        drop: fences::RAIL_BIRCH,
        drop_count: 1,
        kind: Kind::Fence,
    },
    // minetest_game's jungle and acacia trees (CC BY-SA 3.0, see
    // media/LICENSE-items.txt), with the rainforest and savanna ground and
    // grasses they grow among.
    Node {
        id: CONTENT_JUNGLE_TREE,
        name: "esp:jungle_tree",
        description: "Jungle Tree",
        tiles: [
            "default_jungletree_top.png",
            "default_jungletree_top.png",
            "default_jungletree.png",
            "default_jungletree.png",
            "default_jungletree.png",
            "default_jungletree.png",
        ],
        groups: &[("tree", 1), ("choppy", 2), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: CONTENT_JUNGLE_TREE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_JUNGLE_LEAVES,
        name: "esp:jungle_leaves",
        description: "Jungle Tree Leaves",
        tiles: ["default_jungleleaves.png"; 6],
        groups: &[("snappy", 3), ("leaves", 1), ("flammable", 2)],
        drop: CONTENT_JUNGLE_LEAVES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: CONTENT_JUNGLE_WOOD,
        name: "esp:jungle_wood",
        description: "Jungle Wood Planks",
        tiles: ["default_junglewood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("wood", 1)],
        drop: CONTENT_JUNGLE_WOOD,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_JUNGLE_SAPLING,
        name: "esp:jungle_sapling",
        description: "Jungle Tree Sapling",
        tiles: ["default_junglesapling.png"; 6],
        groups: SAPLING_GROUPS,
        drop: CONTENT_JUNGLE_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: CONTENT_ACACIA_TREE,
        name: "esp:acacia_tree",
        description: "Acacia Tree",
        tiles: [
            "default_acacia_tree_top.png",
            "default_acacia_tree_top.png",
            "default_acacia_tree.png",
            "default_acacia_tree.png",
            "default_acacia_tree.png",
            "default_acacia_tree.png",
        ],
        groups: &[("tree", 1), ("choppy", 2), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: CONTENT_ACACIA_TREE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_ACACIA_LEAVES,
        name: "esp:acacia_leaves",
        description: "Acacia Tree Leaves",
        tiles: ["default_acacia_leaves.png"; 6],
        groups: &[("snappy", 3), ("leaves", 1), ("flammable", 2)],
        drop: CONTENT_ACACIA_LEAVES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: CONTENT_ACACIA_WOOD,
        name: "esp:acacia_wood",
        description: "Acacia Wood Planks",
        tiles: ["default_acacia_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("wood", 1)],
        drop: CONTENT_ACACIA_WOOD,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_ACACIA_SAPLING,
        name: "esp:acacia_sapling",
        description: "Acacia Tree Sapling",
        tiles: ["default_acacia_sapling.png"; 6],
        groups: SAPLING_GROUPS,
        drop: CONTENT_ACACIA_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: CONTENT_DRY_DIRT,
        name: "esp:dry_dirt",
        description: "Savanna Dirt",
        tiles: ["default_dry_dirt.png"; 6],
        groups: &[("crumbly", 3), ("soil", 1)],
        drop: CONTENT_DRY_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DRY_GRASS,
        name: "esp:dry_dirt_with_dry_grass",
        description: "Savanna Dirt with Savanna Grass",
        tiles: [
            "default_dry_grass.png",
            "default_dry_dirt.png",
            "default_dry_dirt.png^default_dry_grass_side.png",
            "default_dry_dirt.png^default_dry_grass_side.png",
            "default_dry_dirt.png^default_dry_grass_side.png",
            "default_dry_dirt.png^default_dry_grass_side.png",
        ],
        groups: &[("crumbly", 3), ("soil", 1)],
        drop: CONTENT_DRY_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_LITTER,
        name: "esp:dirt_with_rainforest_litter",
        description: "Dirt with Rainforest Litter",
        tiles: [
            "default_rainforest_litter.png",
            "esp_dirt.png",
            "esp_dirt.png^default_rainforest_litter_side.png",
            "esp_dirt.png^default_rainforest_litter_side.png",
            "esp_dirt.png^default_rainforest_litter_side.png",
            "esp_dirt.png^default_rainforest_litter_side.png",
        ],
        groups: &[("crumbly", 3), ("soil", 1), ("spreading_dirt_type", 1)],
        drop: CONTENT_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    plant!(farming::DRY_GRASS[0], "dry_grass_1", "Savanna Grass", "default_dry_grass_1.png", DRY_GRASS_GROUPS),
    plant!(farming::DRY_GRASS[1], "dry_grass_2", "Savanna Grass", "default_dry_grass_2.png", DRY_GRASS_GROUPS),
    plant!(farming::DRY_GRASS[2], "dry_grass_3", "Savanna Grass", "default_dry_grass_3.png", DRY_GRASS_GROUPS),
    plant!(farming::DRY_GRASS[3], "dry_grass_4", "Savanna Grass", "default_dry_grass_4.png", DRY_GRASS_GROUPS),
    plant!(farming::DRY_GRASS[4], "dry_grass_5", "Savanna Grass", "default_dry_grass_5.png", DRY_GRASS_GROUPS),
    plant!(farming::JUNGLEGRASS, "junglegrass", "Jungle Grass", "default_junglegrass.png", &[("snappy", 3), ("flora", 1), ("attached_node", 1), ("grass", 1), ("junglegrass", 1), ("flammable", 1)]),
    Node {
        id: fences::FENCE_JUNGLE,
        name: "esp:fence_jungle_wood",
        description: "Jungle Wood Fence",
        tiles: ["default_junglewood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::FENCE_JUNGLE,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::FENCE_ACACIA,
        name: "esp:fence_acacia_wood",
        description: "Acacia Wood Fence",
        tiles: ["default_acacia_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::FENCE_ACACIA,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::GATE_JUNGLE,
        name: "esp:gate_jungle_wood_closed",
        description: "Jungle Wood Fence Gate",
        tiles: ["default_junglewood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::GATE_JUNGLE,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_JUNGLE + 1,
        name: "esp:gate_jungle_wood_open",
        description: "Jungle Wood Fence Gate",
        tiles: ["default_junglewood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1), ("not_in_creative_inventory", 1)],
        drop: fences::GATE_JUNGLE,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_ACACIA,
        name: "esp:gate_acacia_wood_closed",
        description: "Acacia Wood Fence Gate",
        tiles: ["default_acacia_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::GATE_ACACIA,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::GATE_ACACIA + 1,
        name: "esp:gate_acacia_wood_open",
        description: "Acacia Wood Fence Gate",
        tiles: ["default_acacia_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1), ("not_in_creative_inventory", 1)],
        drop: fences::GATE_ACACIA,
        drop_count: 1,
        kind: Kind::Gate,
    },
    Node {
        id: fences::RAIL_JUNGLE,
        name: "esp:fence_rail_jungle_wood",
        description: "Jungle Wood Fence Rail",
        tiles: ["default_junglewood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::RAIL_JUNGLE,
        drop_count: 1,
        kind: Kind::Fence,
    },
    Node {
        id: fences::RAIL_ACACIA,
        name: "esp:fence_rail_acacia_wood",
        description: "Acacia Wood Fence Rail",
        tiles: ["default_acacia_wood.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("fence", 1)],
        drop: fences::RAIL_ACACIA,
        drop_count: 1,
        kind: Kind::Fence,
    },
    shaped!(stairs::LATER.first_stair() + 0, "stair_jungle_wood", "Jungle Wood Stair", "default_junglewood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::LATER.first_stair() + 1, "stair_acacia_wood", "Acacia Wood Stair", "default_acacia_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::LATER.first_slab() + 0, "slab_jungle_wood", "Jungle Wood Slab", "default_junglewood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("slab", 1)]),
    shaped!(stairs::LATER.first_slab() + 1, "slab_acacia_wood", "Acacia Wood Slab", "default_acacia_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("slab", 1)]),
    shaped!(stairs::LATER.first_inner() + 0, "stair_inner_jungle_wood", "Inner Jungle Wood Stair", "default_junglewood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::LATER.first_inner() + 1, "stair_inner_acacia_wood", "Inner Acacia Wood Stair", "default_acacia_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::LATER.first_outer() + 0, "stair_outer_jungle_wood", "Outer Jungle Wood Stair", "default_junglewood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    shaped!(stairs::LATER.first_outer() + 1, "stair_outer_acacia_wood", "Outer Acacia Wood Stair", "default_acacia_wood.png", &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2), ("stair", 1)]),
    // minetest_game's cactus, papyrus, bushes, ferns, marram grass, kelp
    // and corals (plants.rs; CC BY-SA 3.0, see media/LICENSE-items.txt).
    Node {
        id: plants::CACTUS,
        name: "esp:cactus",
        description: "Cactus",
        tiles: [
            "default_cactus_top.png",
            "default_cactus_top.png",
            "default_cactus_side.png",
            "default_cactus_side.png",
            "default_cactus_side.png",
            "default_cactus_side.png",
        ],
        groups: &[("choppy", 3)],
        drop: plants::CACTUS,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: plants::PAPYRUS,
        name: "esp:papyrus",
        description: "Papyrus",
        tiles: ["default_papyrus.png"; 6],
        groups: &[("snappy", 3), ("flammable", 2)],
        drop: plants::PAPYRUS,
        drop_count: 1,
        kind: Kind::Stalk,
    },
    Node {
        id: plants::BUSH_STEM,
        name: "esp:bush_stem",
        description: "Bush Stem",
        tiles: ["default_bush_stem.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: plants::BUSH_STEM,
        drop_count: 1,
        kind: Kind::Stem,
    },
    Node {
        id: plants::BUSH_LEAVES,
        name: "esp:bush_leaves",
        description: "Bush Leaves",
        tiles: ["default_leaves_simple.png"; 6],
        groups: &[("snappy", 3), ("flammable", 2), ("leaves", 1)],
        drop: plants::BUSH_LEAVES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: plants::BUSH_SAPLING,
        name: "esp:bush_sapling",
        description: "Bush Sapling",
        tiles: ["default_bush_sapling.png"; 6],
        groups: &[("snappy", 2), ("dig_immediate", 3), ("flammable", 2), ("attached_node", 1), ("sapling", 1)],
        drop: plants::BUSH_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: plants::BLUEBERRY_LEAVES_WITH_BERRIES,
        name: "esp:blueberry_bush_leaves_with_berries",
        description: "Blueberry Bush Leaves with Berries",
        tiles: ["default_blueberry_bush_leaves.png^default_blueberry_overlay.png"; 6],
        groups: &[("snappy", 3), ("flammable", 2), ("leaves", 1), ("dig_immediate", 3)],
        drop: ITEM_BLUEBERRIES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: plants::BLUEBERRY_LEAVES,
        name: "esp:blueberry_bush_leaves",
        description: "Blueberry Bush Leaves",
        tiles: ["default_blueberry_bush_leaves.png"; 6],
        groups: &[("snappy", 3), ("flammable", 2), ("leaves", 1)],
        drop: plants::BLUEBERRY_LEAVES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: plants::BLUEBERRY_SAPLING,
        name: "esp:blueberry_bush_sapling",
        description: "Blueberry Bush Sapling",
        tiles: ["default_blueberry_bush_sapling.png"; 6],
        groups: &[("snappy", 2), ("dig_immediate", 3), ("flammable", 2), ("attached_node", 1), ("sapling", 1)],
        drop: plants::BLUEBERRY_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: plants::ACACIA_BUSH_STEM,
        name: "esp:acacia_bush_stem",
        description: "Acacia Bush Stem",
        tiles: ["default_acacia_bush_stem.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: plants::ACACIA_BUSH_STEM,
        drop_count: 1,
        kind: Kind::Stem,
    },
    Node {
        id: plants::ACACIA_BUSH_LEAVES,
        name: "esp:acacia_bush_leaves",
        description: "Acacia Bush Leaves",
        tiles: ["default_acacia_leaves_simple.png"; 6],
        groups: &[("snappy", 3), ("flammable", 2), ("leaves", 1)],
        drop: plants::ACACIA_BUSH_LEAVES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: plants::ACACIA_BUSH_SAPLING,
        name: "esp:acacia_bush_sapling",
        description: "Acacia Bush Sapling",
        tiles: ["default_acacia_bush_sapling.png"; 6],
        groups: &[("snappy", 2), ("dig_immediate", 3), ("flammable", 2), ("attached_node", 1), ("sapling", 1)],
        drop: plants::ACACIA_BUSH_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: plants::FIR_BUSH_STEM,
        name: "esp:fir_bush_stem",
        description: "Fir Bush Stem",
        tiles: ["default_pine_bush_stem.png"; 6],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 1), ("flammable", 2)],
        drop: plants::FIR_BUSH_STEM,
        drop_count: 1,
        kind: Kind::Stem,
    },
    Node {
        id: plants::FIR_BUSH_NEEDLES,
        name: "esp:fir_bush_needles",
        description: "Fir Bush Needles",
        tiles: ["esp_fir_needles.png"; 6],
        groups: &[("snappy", 3), ("flammable", 2), ("leaves", 1)],
        drop: plants::FIR_BUSH_NEEDLES,
        drop_count: 1,
        kind: Kind::Leaves,
    },
    Node {
        id: plants::FIR_BUSH_SAPLING,
        name: "esp:fir_bush_sapling",
        description: "Fir Bush Sapling",
        tiles: ["default_pine_bush_sapling.png"; 6],
        groups: &[("snappy", 2), ("dig_immediate", 3), ("flammable", 2), ("attached_node", 1), ("sapling", 1)],
        drop: plants::FIR_BUSH_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    Node {
        id: plants::FERN[0],
        name: "esp:fern_1",
        description: "Fern",
        tiles: ["default_fern_1.png"; 6],
        groups: &[("snappy", 3), ("flammable", 3), ("flora", 1), ("grass", 1), ("fern", 1), ("attached_node", 1)],
        drop: plants::FERN[0],
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: plants::FERN[1],
        name: "esp:fern_2",
        description: "Fern",
        tiles: ["default_fern_2.png"; 6],
        groups: &[("snappy", 3), ("flammable", 3), ("flora", 1), ("grass", 1), ("fern", 1), ("attached_node", 1), ("not_in_creative_inventory", 1)],
        drop: plants::FERN[0],
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: plants::FERN[2],
        name: "esp:fern_3",
        description: "Fern",
        tiles: ["default_fern_3.png"; 6],
        groups: &[("snappy", 3), ("flammable", 3), ("flora", 1), ("grass", 1), ("fern", 1), ("attached_node", 1), ("not_in_creative_inventory", 1)],
        drop: plants::FERN[0],
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: plants::MARRAM[0],
        name: "esp:marram_grass_1",
        description: "Marram Grass",
        tiles: ["default_marram_grass_1.png"; 6],
        groups: &[("snappy", 3), ("flammable", 3), ("flora", 1), ("grass", 1), ("marram_grass", 1), ("attached_node", 1)],
        drop: plants::MARRAM[0],
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: plants::MARRAM[1],
        name: "esp:marram_grass_2",
        description: "Marram Grass",
        tiles: ["default_marram_grass_2.png"; 6],
        groups: &[("snappy", 3), ("flammable", 3), ("flora", 1), ("grass", 1), ("marram_grass", 1), ("attached_node", 1), ("not_in_creative_inventory", 1)],
        drop: plants::MARRAM[0],
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: plants::MARRAM[2],
        name: "esp:marram_grass_3",
        description: "Marram Grass",
        tiles: ["default_marram_grass_3.png"; 6],
        groups: &[("snappy", 3), ("flammable", 3), ("flora", 1), ("grass", 1), ("marram_grass", 1), ("attached_node", 1), ("not_in_creative_inventory", 1)],
        drop: plants::MARRAM[0],
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: plants::KELP,
        name: "esp:sand_with_kelp",
        description: "Kelp",
        tiles: ["esp_sand.png"; 6],
        groups: &[("snappy", 3)],
        drop: plants::KELP,
        drop_count: 1,
        kind: Kind::Rooted,
    },
    Node {
        id: plants::CORAL_GREEN,
        name: "esp:coral_green",
        description: "Green Coral",
        tiles: ["default_coral_skeleton.png"; 6],
        groups: &[("snappy", 3)],
        drop: plants::CORAL_GREEN,
        drop_count: 1,
        kind: Kind::Rooted,
    },
    Node {
        id: plants::CORAL_PINK,
        name: "esp:coral_pink",
        description: "Pink Coral",
        tiles: ["default_coral_skeleton.png"; 6],
        groups: &[("snappy", 3)],
        drop: plants::CORAL_PINK,
        drop_count: 1,
        kind: Kind::Rooted,
    },
    Node {
        id: plants::CORAL_CYAN,
        name: "esp:coral_cyan",
        description: "Cyan Coral",
        tiles: ["default_coral_skeleton.png"; 6],
        groups: &[("snappy", 3)],
        drop: plants::CORAL_CYAN,
        drop_count: 1,
        kind: Kind::Rooted,
    },
    Node {
        id: plants::CORAL_BROWN,
        name: "esp:coral_brown",
        description: "Brown Coral",
        tiles: ["default_coral_brown.png"; 6],
        groups: &[("cracky", 3)],
        drop: plants::CORAL_SKELETON,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: plants::CORAL_ORANGE,
        name: "esp:coral_orange",
        description: "Orange Coral",
        tiles: ["default_coral_orange.png"; 6],
        groups: &[("cracky", 3)],
        drop: plants::CORAL_SKELETON,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: plants::CORAL_SKELETON,
        name: "esp:coral_skeleton",
        description: "Coral Skeleton",
        tiles: ["default_coral_skeleton.png"; 6],
        groups: &[("cracky", 3)],
        drop: plants::CORAL_SKELETON,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: plants::LARGE_CACTUS_SEEDLING,
        name: "esp:large_cactus_seedling",
        description: "Large Cactus Seedling",
        tiles: ["default_large_cactus_seedling.png"; 6],
        groups: &[("choppy", 3), ("dig_immediate", 3), ("attached_node", 1)],
        drop: plants::LARGE_CACTUS_SEEDLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    // minetest_game's copper, tin, gold, mese and diamond ores, and the
    // metal and mineral blocks (CC BY-SA 3.0, see media/LICENSE-items.txt).
    Node {
        id: CONTENT_COPPER,
        name: "esp:stone_with_copper",
        description: "Copper Ore",
        tiles: ["esp_stone.png^default_mineral_copper.png"; 6],
        groups: &[("cracky", 2)],
        drop: ITEM_COPPER_LUMP,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_TIN,
        name: "esp:stone_with_tin",
        description: "Tin Ore",
        tiles: ["esp_stone.png^default_mineral_tin.png"; 6],
        groups: &[("cracky", 2)],
        drop: ITEM_TIN_LUMP,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_GOLD,
        name: "esp:stone_with_gold",
        description: "Gold Ore",
        tiles: ["esp_stone.png^default_mineral_gold.png"; 6],
        groups: &[("cracky", 2)],
        drop: ITEM_GOLD_LUMP,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_MESE_ORE,
        name: "esp:stone_with_mese",
        description: "Mese Ore",
        tiles: ["esp_stone.png^default_mineral_mese.png"; 6],
        groups: &[("cracky", 1)],
        drop: ITEM_MESE_CRYSTAL,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DIAMOND_ORE,
        name: "esp:stone_with_diamond",
        description: "Diamond Ore",
        tiles: ["esp_stone.png^default_mineral_diamond.png"; 6],
        groups: &[("cracky", 1)],
        drop: ITEM_DIAMOND,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_COPPER_BLOCK,
        name: "esp:copperblock",
        description: "Copper Block",
        tiles: ["default_copper_block.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_COPPER_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_TIN_BLOCK,
        name: "esp:tinblock",
        description: "Tin Block",
        tiles: ["default_tin_block.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_TIN_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_BRONZE_BLOCK,
        name: "esp:bronzeblock",
        description: "Bronze Block",
        tiles: ["default_bronze_block.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_BRONZE_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_GOLD_BLOCK,
        name: "esp:goldblock",
        description: "Gold Block",
        tiles: ["default_gold_block.png"; 6],
        groups: &[("cracky", 1)],
        drop: CONTENT_GOLD_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_MESE_BLOCK,
        name: "esp:mese",
        description: "Mese Block",
        tiles: ["default_mese_block.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_MESE_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DIAMOND_BLOCK,
        name: "esp:diamondblock",
        description: "Diamond Block",
        tiles: ["default_diamond_block.png"; 6],
        groups: &[("cracky", 1), ("level", 3)],
        drop: CONTENT_DIAMOND_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_MESE_LAMP,
        name: "esp:meselamp",
        description: "Mese Lamp",
        tiles: ["default_meselamp.png"; 6],
        groups: &[("cracky", 3), ("oddly_breakable_by_hand", 3)],
        drop: CONTENT_MESE_LAMP,
        drop_count: 1,
        kind: Kind::Lamp,
    },
    // minetest_game's gravel, clay, brick, desert and silver sands and
    // stones, snow, ice, permafrost, coniferous litter and obsidian
    // building blocks (CC BY-SA 3.0, see media/LICENSE-items.txt).
    Node {
        id: CONTENT_GRAVEL,
        name: "esp:gravel",
        description: "Gravel",
        tiles: ["default_gravel.png"; 6],
        groups: &[("crumbly", 2), ("falling_node", 1)],
        drop: CONTENT_GRAVEL,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_CLAY,
        name: "esp:clay",
        description: "Clay",
        tiles: ["default_clay.png"; 6],
        groups: &[("crumbly", 3)],
        drop: ITEM_CLAY_LUMP,
        drop_count: 4,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_BRICK,
        name: "esp:brick",
        description: "Brick Block",
        tiles: ["default_brick.png"; 6],
        groups: &[("cracky", 3)],
        drop: CONTENT_BRICK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_SAND,
        name: "esp:desert_sand",
        description: "Desert Sand",
        tiles: ["default_desert_sand.png"; 6],
        groups: &[("crumbly", 3), ("falling_node", 1), ("sand", 1)],
        drop: CONTENT_DESERT_SAND,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_STONE,
        name: "esp:desert_stone",
        description: "Desert Stone",
        tiles: ["default_desert_stone.png"; 6],
        groups: &[("cracky", 3), ("stone", 1)],
        drop: CONTENT_DESERT_COBBLE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_COBBLE,
        name: "esp:desert_cobble",
        description: "Desert Cobblestone",
        tiles: ["default_desert_cobble.png"; 6],
        groups: &[("cracky", 3), ("stone", 2)],
        drop: CONTENT_DESERT_COBBLE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_STONE_BRICK,
        name: "esp:desert_stonebrick",
        description: "Desert Stone Brick",
        tiles: ["default_desert_stone_brick.png"; 6],
        groups: &[("cracky", 2), ("stone", 1)],
        drop: CONTENT_DESERT_STONE_BRICK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_STONE_BLOCK,
        name: "esp:desert_stone_block",
        description: "Desert Stone Block",
        tiles: ["default_desert_stone_block.png"; 6],
        groups: &[("cracky", 2), ("stone", 1)],
        drop: CONTENT_DESERT_STONE_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_SANDSTONE,
        name: "esp:desert_sandstone",
        description: "Desert Sandstone",
        tiles: ["default_desert_sandstone.png"; 6],
        groups: &[("crumbly", 1), ("cracky", 3)],
        drop: CONTENT_DESERT_SANDSTONE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_SANDSTONE_BRICK,
        name: "esp:desert_sandstone_brick",
        description: "Desert Sandstone Brick",
        tiles: ["default_desert_sandstone_brick.png"; 6],
        groups: &[("cracky", 2)],
        drop: CONTENT_DESERT_SANDSTONE_BRICK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_DESERT_SANDSTONE_BLOCK,
        name: "esp:desert_sandstone_block",
        description: "Desert Sandstone Block",
        tiles: ["default_desert_sandstone_block.png"; 6],
        groups: &[("cracky", 2)],
        drop: CONTENT_DESERT_SANDSTONE_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SILVER_SAND,
        name: "esp:silver_sand",
        description: "Silver Sand",
        tiles: ["default_silver_sand.png"; 6],
        groups: &[("crumbly", 3), ("falling_node", 1), ("sand", 1)],
        drop: CONTENT_SILVER_SAND,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SILVER_SANDSTONE,
        name: "esp:silver_sandstone",
        description: "Silver Sandstone",
        tiles: ["default_silver_sandstone.png"; 6],
        groups: &[("crumbly", 1), ("cracky", 3)],
        drop: CONTENT_SILVER_SANDSTONE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SILVER_SANDSTONE_BRICK,
        name: "esp:silver_sandstone_brick",
        description: "Silver Sandstone Brick",
        tiles: ["default_silver_sandstone_brick.png"; 6],
        groups: &[("cracky", 2)],
        drop: CONTENT_SILVER_SANDSTONE_BRICK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SILVER_SANDSTONE_BLOCK,
        name: "esp:silver_sandstone_block",
        description: "Silver Sandstone Block",
        tiles: ["default_silver_sandstone_block.png"; 6],
        groups: &[("cracky", 2)],
        drop: CONTENT_SILVER_SANDSTONE_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_SNOW_LAYER,
        name: "esp:snow",
        description: "Snow",
        tiles: ["esp_snow.png"; 6],
        groups: &[("crumbly", 3), ("falling_node", 1), ("snowy", 1), ("cools_lava", 1)],
        drop: CONTENT_SNOW_LAYER,
        drop_count: 1,
        kind: Kind::SnowLayer,
    },
    Node {
        id: CONTENT_SNOW_BLOCK,
        name: "esp:snowblock",
        description: "Snow Block",
        tiles: ["esp_snow.png"; 6],
        groups: &[("crumbly", 3), ("cools_lava", 1), ("snowy", 1)],
        drop: CONTENT_SNOW_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_ICE,
        name: "esp:ice",
        description: "Ice",
        tiles: ["default_ice.png"; 6],
        groups: &[("cracky", 3), ("cools_lava", 1), ("slippery", 3)],
        drop: CONTENT_ICE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_PERMAFROST,
        name: "esp:permafrost",
        description: "Permafrost",
        tiles: ["default_permafrost.png"; 6],
        groups: &[("cracky", 3)],
        drop: CONTENT_PERMAFROST,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_PERMAFROST_MOSS,
        name: "esp:permafrost_with_moss",
        description: "Permafrost with Moss",
        tiles: [
            "default_moss.png",
            "default_permafrost.png",
            "default_permafrost.png^default_moss_side.png",
            "default_permafrost.png^default_moss_side.png",
            "default_permafrost.png^default_moss_side.png",
            "default_permafrost.png^default_moss_side.png",
        ],
        groups: &[("cracky", 3)],
        drop: CONTENT_PERMAFROST_MOSS,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_PERMAFROST_STONES,
        name: "esp:permafrost_with_stones",
        description: "Permafrost with Stones",
        tiles: [
            "default_permafrost.png^default_stones.png",
            "default_permafrost.png",
            "default_permafrost.png^default_stones_side.png",
            "default_permafrost.png^default_stones_side.png",
            "default_permafrost.png^default_stones_side.png",
            "default_permafrost.png^default_stones_side.png",
        ],
        groups: &[("cracky", 3)],
        drop: CONTENT_PERMAFROST_STONES,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_CONIFEROUS_LITTER,
        name: "esp:dirt_with_coniferous_litter",
        description: "Dirt with Coniferous Litter",
        tiles: [
            "default_coniferous_litter.png",
            "esp_dirt.png",
            "esp_dirt.png^default_coniferous_litter_side.png",
            "esp_dirt.png^default_coniferous_litter_side.png",
            "esp_dirt.png^default_coniferous_litter_side.png",
            "esp_dirt.png^default_coniferous_litter_side.png",
        ],
        groups: &[("crumbly", 3), ("soil", 1), ("spreading_dirt_type", 1)],
        drop: CONTENT_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_OBSIDIAN_BRICK,
        name: "esp:obsidianbrick",
        description: "Obsidian Brick",
        tiles: ["default_obsidian_brick.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_OBSIDIAN_BRICK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_OBSIDIAN_BLOCK,
        name: "esp:obsidian_block",
        description: "Obsidian Block",
        tiles: ["default_obsidian_block.png"; 6],
        groups: &[("cracky", 1), ("level", 2)],
        drop: CONTENT_OBSIDIAN_BLOCK,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: CONTENT_OBSIDIAN_GLASS,
        name: "esp:obsidian_glass",
        description: "Obsidian Glass",
        tiles: [
            "default_obsidian_glass.png",
            "default_obsidian_glass_detail.png",
            "default_obsidian_glass_detail.png",
            "default_obsidian_glass_detail.png",
            "default_obsidian_glass_detail.png",
            "default_obsidian_glass_detail.png",
        ],
        groups: &[("cracky", 3)],
        drop: CONTENT_OBSIDIAN_GLASS,
        drop_count: 1,
        kind: Kind::Glass,
    },
    shaped!(stairs::BUILDING.first_stair() + 0, "stair_desert_stone", "Desert Stone Stair", "default_desert_stone.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 1, "stair_desert_cobble", "Desert Cobblestone Stair", "default_desert_cobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 2, "stair_desert_stonebrick", "Desert Stone Brick Stair", "default_desert_stone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 3, "stair_desert_stone_block", "Desert Stone Block Stair", "default_desert_stone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 4, "stair_desert_sandstone", "Desert Sandstone Stair", "default_desert_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 5, "stair_desert_sandstone_brick", "Desert Sandstone Brick Stair", "default_desert_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 6, "stair_desert_sandstone_block", "Desert Sandstone Block Stair", "default_desert_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 7, "stair_silver_sandstone", "Silver Sandstone Stair", "default_silver_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 8, "stair_silver_sandstone_brick", "Silver Sandstone Brick Stair", "default_silver_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 9, "stair_silver_sandstone_block", "Silver Sandstone Block Stair", "default_silver_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 10, "stair_brick", "Brick Stair", "default_brick.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 11, "stair_obsidian", "Obsidian Stair", "default_obsidian.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 12, "stair_obsidianbrick", "Obsidian Brick Stair", "default_obsidian_brick.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 13, "stair_obsidian_block", "Obsidian Block Stair", "default_obsidian_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 14, "stair_ice", "Ice Stair", "default_ice.png", &[("cracky", 3), ("cools_lava", 1), ("slippery", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 15, "stair_snowblock", "Snow Block Stair", "esp_snow.png", &[("crumbly", 3), ("cools_lava", 1), ("snowy", 1), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 16, "stair_steelblock", "Steel Block Stair", "default_steel_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 17, "stair_copperblock", "Copper Block Stair", "default_copper_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 18, "stair_tinblock", "Tin Block Stair", "default_tin_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 19, "stair_bronzeblock", "Bronze Block Stair", "default_bronze_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_stair() + 20, "stair_goldblock", "Gold Block Stair", "default_gold_block.png", &[("cracky", 1), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 0, "slab_desert_stone", "Desert Stone Slab", "default_desert_stone.png", &[("cracky", 3), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 1, "slab_desert_cobble", "Desert Cobblestone Slab", "default_desert_cobble.png", &[("cracky", 3), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 2, "slab_desert_stonebrick", "Desert Stone Brick Slab", "default_desert_stone_brick.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 3, "slab_desert_stone_block", "Desert Stone Block Slab", "default_desert_stone_block.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 4, "slab_desert_sandstone", "Desert Sandstone Slab", "default_desert_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 5, "slab_desert_sandstone_brick", "Desert Sandstone Brick Slab", "default_desert_sandstone_brick.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 6, "slab_desert_sandstone_block", "Desert Sandstone Block Slab", "default_desert_sandstone_block.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 7, "slab_silver_sandstone", "Silver Sandstone Slab", "default_silver_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 8, "slab_silver_sandstone_brick", "Silver Sandstone Brick Slab", "default_silver_sandstone_brick.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 9, "slab_silver_sandstone_block", "Silver Sandstone Block Slab", "default_silver_sandstone_block.png", &[("cracky", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 10, "slab_brick", "Brick Slab", "default_brick.png", &[("cracky", 3), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 11, "slab_obsidian", "Obsidian Slab", "default_obsidian.png", &[("cracky", 1), ("level", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 12, "slab_obsidianbrick", "Obsidian Brick Slab", "default_obsidian_brick.png", &[("cracky", 1), ("level", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 13, "slab_obsidian_block", "Obsidian Block Slab", "default_obsidian_block.png", &[("cracky", 1), ("level", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 14, "slab_ice", "Ice Slab", "default_ice.png", &[("cracky", 3), ("cools_lava", 1), ("slippery", 3), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 15, "slab_snowblock", "Snow Block Slab", "esp_snow.png", &[("crumbly", 3), ("cools_lava", 1), ("snowy", 1), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 16, "slab_steelblock", "Steel Block Slab", "default_steel_block.png", &[("cracky", 1), ("level", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 17, "slab_copperblock", "Copper Block Slab", "default_copper_block.png", &[("cracky", 1), ("level", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 18, "slab_tinblock", "Tin Block Slab", "default_tin_block.png", &[("cracky", 1), ("level", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 19, "slab_bronzeblock", "Bronze Block Slab", "default_bronze_block.png", &[("cracky", 1), ("level", 2), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_slab() + 20, "slab_goldblock", "Gold Block Slab", "default_gold_block.png", &[("cracky", 1), ("slab", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 0, "stair_inner_desert_stone", "Inner Desert Stone Stair", "default_desert_stone.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 1, "stair_inner_desert_cobble", "Inner Desert Cobblestone Stair", "default_desert_cobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 2, "stair_inner_desert_stonebrick", "Inner Desert Stone Brick Stair", "default_desert_stone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 3, "stair_inner_desert_stone_block", "Inner Desert Stone Block Stair", "default_desert_stone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 4, "stair_inner_desert_sandstone", "Inner Desert Sandstone Stair", "default_desert_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 5, "stair_inner_desert_sandstone_brick", "Inner Desert Sandstone Brick Stair", "default_desert_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 6, "stair_inner_desert_sandstone_block", "Inner Desert Sandstone Block Stair", "default_desert_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 7, "stair_inner_silver_sandstone", "Inner Silver Sandstone Stair", "default_silver_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 8, "stair_inner_silver_sandstone_brick", "Inner Silver Sandstone Brick Stair", "default_silver_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 9, "stair_inner_silver_sandstone_block", "Inner Silver Sandstone Block Stair", "default_silver_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 10, "stair_inner_brick", "Inner Brick Stair", "default_brick.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 11, "stair_inner_obsidian", "Inner Obsidian Stair", "default_obsidian.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 12, "stair_inner_obsidianbrick", "Inner Obsidian Brick Stair", "default_obsidian_brick.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 13, "stair_inner_obsidian_block", "Inner Obsidian Block Stair", "default_obsidian_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 14, "stair_inner_ice", "Inner Ice Stair", "default_ice.png", &[("cracky", 3), ("cools_lava", 1), ("slippery", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 15, "stair_inner_snowblock", "Inner Snow Block Stair", "esp_snow.png", &[("crumbly", 3), ("cools_lava", 1), ("snowy", 1), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 16, "stair_inner_steelblock", "Inner Steel Block Stair", "default_steel_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 17, "stair_inner_copperblock", "Inner Copper Block Stair", "default_copper_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 18, "stair_inner_tinblock", "Inner Tin Block Stair", "default_tin_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 19, "stair_inner_bronzeblock", "Inner Bronze Block Stair", "default_bronze_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_inner() + 20, "stair_inner_goldblock", "Inner Gold Block Stair", "default_gold_block.png", &[("cracky", 1), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 0, "stair_outer_desert_stone", "Outer Desert Stone Stair", "default_desert_stone.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 1, "stair_outer_desert_cobble", "Outer Desert Cobblestone Stair", "default_desert_cobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 2, "stair_outer_desert_stonebrick", "Outer Desert Stone Brick Stair", "default_desert_stone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 3, "stair_outer_desert_stone_block", "Outer Desert Stone Block Stair", "default_desert_stone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 4, "stair_outer_desert_sandstone", "Outer Desert Sandstone Stair", "default_desert_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 5, "stair_outer_desert_sandstone_brick", "Outer Desert Sandstone Brick Stair", "default_desert_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 6, "stair_outer_desert_sandstone_block", "Outer Desert Sandstone Block Stair", "default_desert_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 7, "stair_outer_silver_sandstone", "Outer Silver Sandstone Stair", "default_silver_sandstone.png", &[("crumbly", 1), ("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 8, "stair_outer_silver_sandstone_brick", "Outer Silver Sandstone Brick Stair", "default_silver_sandstone_brick.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 9, "stair_outer_silver_sandstone_block", "Outer Silver Sandstone Block Stair", "default_silver_sandstone_block.png", &[("cracky", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 10, "stair_outer_brick", "Outer Brick Stair", "default_brick.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 11, "stair_outer_obsidian", "Outer Obsidian Stair", "default_obsidian.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 12, "stair_outer_obsidianbrick", "Outer Obsidian Brick Stair", "default_obsidian_brick.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 13, "stair_outer_obsidian_block", "Outer Obsidian Block Stair", "default_obsidian_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 14, "stair_outer_ice", "Outer Ice Stair", "default_ice.png", &[("cracky", 3), ("cools_lava", 1), ("slippery", 3), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 15, "stair_outer_snowblock", "Outer Snow Block Stair", "esp_snow.png", &[("crumbly", 3), ("cools_lava", 1), ("snowy", 1), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 16, "stair_outer_steelblock", "Outer Steel Block Stair", "default_steel_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 17, "stair_outer_copperblock", "Outer Copper Block Stair", "default_copper_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 18, "stair_outer_tinblock", "Outer Tin Block Stair", "default_tin_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 19, "stair_outer_bronzeblock", "Outer Bronze Block Stair", "default_bronze_block.png", &[("cracky", 1), ("level", 2), ("stair", 1)]),
    shaped!(stairs::BUILDING.first_outer() + 20, "stair_outer_goldblock", "Outer Gold Block Stair", "default_gold_block.png", &[("cracky", 1), ("stair", 1)]),
    // minetest_game's flowers, mushrooms and waterlily (colours.rs; CC BY-SA
    // 3.0, see media/LICENSE-flowers.txt).
    Node {
        id: colours::FIRST_FLOWER + 0,
        name: "esp:rose",
        description: "Red Rose",
        tiles: ["flowers_rose.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_red", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 0,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::FIRST_FLOWER + 1,
        name: "esp:tulip",
        description: "Orange Tulip",
        tiles: ["flowers_tulip.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_orange", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 1,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::FIRST_FLOWER + 2,
        name: "esp:dandelion_yellow",
        description: "Yellow Dandelion",
        tiles: ["flowers_dandelion_yellow.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_yellow", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 2,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::FIRST_FLOWER + 3,
        name: "esp:chrysanthemum_green",
        description: "Green Chrysanthemum",
        tiles: ["flowers_chrysanthemum_green.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_green", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 3,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::FIRST_FLOWER + 4,
        name: "esp:geranium",
        description: "Blue Geranium",
        tiles: ["flowers_geranium.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_blue", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 4,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::FIRST_FLOWER + 5,
        name: "esp:viola",
        description: "Viola",
        tiles: ["flowers_viola.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_violet", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 5,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::FIRST_FLOWER + 6,
        name: "esp:dandelion_white",
        description: "White Dandelion",
        tiles: ["flowers_dandelion_white.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_white", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 6,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::FIRST_FLOWER + 7,
        name: "esp:tulip_black",
        description: "Black Tulip",
        tiles: ["flowers_tulip_black.png"; 6],
        groups: &[("snappy", 3), ("flower", 1), ("flora", 1), ("attached_node", 1), ("color_black", 1), ("flammable", 1)],
        drop: colours::FIRST_FLOWER + 7,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::MUSHROOM_RED,
        name: "esp:mushroom_red",
        description: "Red Mushroom",
        tiles: ["flowers_mushroom_red.png"; 6],
        groups: &[("mushroom", 1), ("snappy", 3), ("attached_node", 1), ("flammable", 1)],
        drop: colours::MUSHROOM_RED,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::MUSHROOM_BROWN,
        name: "esp:mushroom_brown",
        description: "Brown Mushroom",
        tiles: ["flowers_mushroom_brown.png"; 6],
        groups: &[("mushroom", 1), ("food_mushroom", 1), ("snappy", 3), ("attached_node", 1), ("flammable", 1)],
        drop: colours::MUSHROOM_BROWN,
        drop_count: 1,
        kind: Kind::Plant,
    },
    Node {
        id: colours::WATERLILY,
        name: "esp:waterlily",
        description: "Waterlily",
        tiles: [
            "flowers_waterlily.png",
            "flowers_waterlily_bottom.png",
            "flowers_waterlily_bottom.png",
            "flowers_waterlily_bottom.png",
            "flowers_waterlily_bottom.png",
            "flowers_waterlily_bottom.png",
        ],
        groups: &[("snappy", 3), ("flower", 1), ("flammable", 1)],
        drop: colours::WATERLILY,
        drop_count: 1,
        kind: Kind::Lily,
    },
    // minetest_game's wool in its other 14 colours (see
    // media/LICENSE-farming.txt; white is farming's, above).
    Node {
        id: colours::wool(1),
        name: "esp:wool_grey",
        description: "Grey Wool",
        tiles: ["wool_grey.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_grey", 1)],
        drop: colours::wool(1),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(2),
        name: "esp:wool_dark_grey",
        description: "Dark Grey Wool",
        tiles: ["wool_dark_grey.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_dark_grey", 1)],
        drop: colours::wool(2),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(3),
        name: "esp:wool_black",
        description: "Black Wool",
        tiles: ["wool_black.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_black", 1)],
        drop: colours::wool(3),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(4),
        name: "esp:wool_violet",
        description: "Violet Wool",
        tiles: ["wool_violet.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_violet", 1)],
        drop: colours::wool(4),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(5),
        name: "esp:wool_blue",
        description: "Blue Wool",
        tiles: ["wool_blue.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_blue", 1)],
        drop: colours::wool(5),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(6),
        name: "esp:wool_cyan",
        description: "Cyan Wool",
        tiles: ["wool_cyan.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_cyan", 1)],
        drop: colours::wool(6),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(7),
        name: "esp:wool_dark_green",
        description: "Dark Green Wool",
        tiles: ["wool_dark_green.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_dark_green", 1)],
        drop: colours::wool(7),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(8),
        name: "esp:wool_green",
        description: "Green Wool",
        tiles: ["wool_green.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_green", 1)],
        drop: colours::wool(8),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(9),
        name: "esp:wool_yellow",
        description: "Yellow Wool",
        tiles: ["wool_yellow.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_yellow", 1)],
        drop: colours::wool(9),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(10),
        name: "esp:wool_brown",
        description: "Brown Wool",
        tiles: ["wool_brown.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_brown", 1)],
        drop: colours::wool(10),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(11),
        name: "esp:wool_orange",
        description: "Orange Wool",
        tiles: ["wool_orange.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_orange", 1)],
        drop: colours::wool(11),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(12),
        name: "esp:wool_red",
        description: "Red Wool",
        tiles: ["wool_red.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_red", 1)],
        drop: colours::wool(12),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(13),
        name: "esp:wool_magenta",
        description: "Magenta Wool",
        tiles: ["wool_magenta.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_magenta", 1)],
        drop: colours::wool(13),
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: colours::wool(14),
        name: "esp:wool_pink",
        description: "Pink Wool",
        tiles: ["wool_pink.png"; 6],
        groups: &[("snappy", 2), ("choppy", 2), ("oddly_breakable_by_hand", 3), ("flammable", 3), ("wool", 1), ("color_pink", 1)],
        drop: colours::wool(14),
        drop_count: 1,
        kind: Kind::Solid,
    },
    // minetest_game's xpanes (panes.rs; CC BY-SA 3.0, see
    // media/LICENSE-xpanes.txt): each pane flat, then connected.
    Node {
        id: panes::PANE_FLAT,
        name: "esp:pane_flat",
        description: "Glass Pane",
        tiles: ["xpanes_edge.png", "xpanes_edge.png", "xpanes_edge.png", "xpanes_edge.png", "default_glass.png", "default_glass.png"],
        groups: &[("snappy", 2), ("cracky", 3), ("oddly_breakable_by_hand", 3), ("pane", 1)],
        drop: panes::PANE_FLAT,
        drop_count: 1,
        kind: Kind::Pane,
    },
    Node {
        id: panes::PANE,
        name: "esp:pane",
        description: "Glass Pane",
        tiles: ["xpanes_edge.png", "xpanes_edge.png", "default_glass.png", "default_glass.png", "default_glass.png", "default_glass.png"],
        groups: &[("snappy", 2), ("cracky", 3), ("oddly_breakable_by_hand", 3), ("pane", 1), ("not_in_creative_inventory", 1)],
        drop: panes::PANE_FLAT,
        drop_count: 1,
        kind: Kind::Pane,
    },
    Node {
        id: panes::OBSIDIAN_PANE_FLAT,
        name: "esp:obsidian_pane_flat",
        description: "Obsidian Glass Pane",
        tiles: ["xpanes_edge_obsidian.png", "xpanes_edge_obsidian.png", "xpanes_edge_obsidian.png", "xpanes_edge_obsidian.png", "default_obsidian_glass.png", "default_obsidian_glass.png"],
        groups: &[("snappy", 2), ("cracky", 3), ("pane", 1)],
        drop: panes::OBSIDIAN_PANE_FLAT,
        drop_count: 1,
        kind: Kind::Pane,
    },
    Node {
        id: panes::OBSIDIAN_PANE,
        name: "esp:obsidian_pane",
        description: "Obsidian Glass Pane",
        tiles: ["xpanes_edge_obsidian.png", "xpanes_edge_obsidian.png", "default_obsidian_glass.png", "default_obsidian_glass.png", "default_obsidian_glass.png", "default_obsidian_glass.png"],
        groups: &[("snappy", 2), ("cracky", 3), ("pane", 1), ("not_in_creative_inventory", 1)],
        drop: panes::OBSIDIAN_PANE_FLAT,
        drop_count: 1,
        kind: Kind::Pane,
    },
    Node {
        id: panes::BAR_FLAT,
        name: "esp:bar_flat",
        description: "Steel Bars",
        tiles: ["xpanes_bar_top.png", "xpanes_bar_top.png", "xpanes_bar_top.png", "xpanes_bar_top.png", "xpanes_bar.png", "xpanes_bar.png"],
        groups: &[("cracky", 2), ("pane", 1)],
        drop: panes::BAR_FLAT,
        drop_count: 1,
        kind: Kind::Pane,
    },
    Node {
        id: panes::BAR,
        name: "esp:bar",
        description: "Steel Bars",
        tiles: ["xpanes_bar_top.png", "xpanes_bar_top.png", "xpanes_bar.png", "xpanes_bar.png", "xpanes_bar.png", "xpanes_bar.png"],
        groups: &[("cracky", 2), ("pane", 1), ("not_in_creative_inventory", 1)],
        drop: panes::BAR_FLAT,
        drop_count: 1,
        kind: Kind::Pane,
    },
    // The steel bar door and trapdoor (xpanes, with the doors mod).
    door!(panes::BAR_DOOR + 0, "door_steel_bar_a", "Steel Bar Door", "xpanes_door_steel_bar.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL_BAR),
    door!(panes::BAR_DOOR + 1, "door_steel_bar_b", "Steel Bar Door", "xpanes_door_steel_bar.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL_BAR),
    door!(panes::BAR_DOOR + 2, "door_steel_bar_c", "Steel Bar Door", "xpanes_door_steel_bar.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL_BAR),
    door!(panes::BAR_DOOR + 3, "door_steel_bar_d", "Steel Bar Door", "xpanes_door_steel_bar.png", &[("cracky", 1), ("level", 2), ("door", 1)], ITEM_DOOR_STEEL_BAR),
    Node {
        id: panes::BAR_TRAPDOOR,
        name: "esp:trapdoor_steel_bar",
        description: "Steel Bar Trapdoor",
        tiles: TRAPDOOR_BAR_CLOSED,
        groups: &[("cracky", 1), ("level", 2), ("door", 1)],
        drop: panes::BAR_TRAPDOOR,
        drop_count: 1,
        kind: Kind::Trapdoor,
    },
    Node {
        id: panes::BAR_TRAPDOOR + 1,
        name: "esp:trapdoor_steel_bar_open",
        description: "Steel Bar Trapdoor",
        tiles: TRAPDOOR_BAR_OPEN,
        groups: &[("cracky", 1), ("level", 2), ("door", 1), ("not_in_creative_inventory", 1)],
        drop: panes::BAR_TRAPDOOR,
        drop_count: 1,
        kind: Kind::Trapdoor,
    },
    // minetest_game's bones (bones.rs; CC BY-SA 3.0, see
    // media/LICENSE-bones.txt).
    Node {
        id: bones::BONES,
        name: "esp:bones",
        description: "Bones",
        tiles: ["bones_top.png^[transform2", "bones_bottom.png", "bones_side.png", "bones_side.png", "bones_rear.png", "bones_front.png"],
        groups: &[("dig_immediate", 2)],
        drop: bones::BONES,
        drop_count: 1,
        kind: Kind::Bones,
    },
    // minetest_game's walls (fences.rs).
    Node {
        id: fences::WALL_COBBLE,
        name: "esp:wall_cobble",
        description: "Cobblestone Wall",
        tiles: ["esp_cobble.png"; 6],
        groups: &[("cracky", 3), ("wall", 1), ("stone", 2)],
        drop: fences::WALL_COBBLE,
        drop_count: 1,
        kind: Kind::Wall,
    },
    Node {
        id: fences::WALL_DESERT_COBBLE,
        name: "esp:wall_desert_cobble",
        description: "Desert Cobblestone Wall",
        tiles: ["default_desert_cobble.png"; 6],
        groups: &[("cracky", 3), ("wall", 1), ("stone", 2)],
        drop: fences::WALL_DESERT_COBBLE,
        drop_count: 1,
        kind: Kind::Wall,
    },
    // minetest_game's mossy cobblestone, which dungeons are partly built
    // of (dungeons.rs), and its wall, stairs and slab.
    Node {
        id: dungeons::MOSSYCOBBLE,
        name: "esp:mossycobble",
        description: "Mossy Cobblestone",
        tiles: ["default_mossycobble.png"; 6],
        groups: &[("cracky", 3), ("stone", 1)],
        drop: dungeons::MOSSYCOBBLE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: dungeons::WALL_MOSSYCOBBLE,
        name: "esp:wall_mossycobble",
        description: "Mossy Cobblestone Wall",
        tiles: ["default_mossycobble.png"; 6],
        groups: &[("cracky", 3), ("wall", 1), ("stone", 2)],
        drop: dungeons::WALL_MOSSYCOBBLE,
        drop_count: 1,
        kind: Kind::Wall,
    },
    shaped!(stairs::MOSSY.first_stair(), "stair_mossycobble", "Mossy Cobblestone Stair", "default_mossycobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::MOSSY.first_slab(), "slab_mossycobble", "Mossy Cobblestone Slab", "default_mossycobble.png", &[("cracky", 3), ("slab", 1)]),
    shaped!(stairs::MOSSY.first_inner(), "stair_inner_mossycobble", "Inner Mossy Cobblestone Stair", "default_mossycobble.png", &[("cracky", 3), ("stair", 1)]),
    shaped!(stairs::MOSSY.first_outer(), "stair_outer_mossycobble", "Outer Mossy Cobblestone Stair", "default_mossycobble.png", &[("cracky", 3), ("stair", 1)]),
    // minetest_game's mese post lights (default.register_mesepost).
    Node {
        id: MESE_POST_APPLE,
        name: "esp:mese_post_light",
        description: "Apple Wood Mese Post Light",
        tiles: [
            "esp_apple_wood.png",
            "esp_apple_wood.png",
            "esp_apple_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "esp_apple_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "esp_apple_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
            "esp_apple_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2)],
        drop: MESE_POST_APPLE,
        drop_count: 1,
        kind: Kind::Post,
    },
    Node {
        id: MESE_POST_BIRCH,
        name: "esp:mese_post_light_birch_wood",
        description: "Birch Wood Mese Post Light",
        tiles: [
            "esp_birch_wood.png",
            "esp_birch_wood.png",
            "esp_birch_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "esp_birch_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "esp_birch_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
            "esp_birch_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2)],
        drop: MESE_POST_BIRCH,
        drop_count: 1,
        kind: Kind::Post,
    },
    Node {
        id: MESE_POST_FIR,
        name: "esp:mese_post_light_fir_wood",
        description: "Fir Wood Mese Post Light",
        tiles: [
            "esp_fir_wood.png",
            "esp_fir_wood.png",
            "esp_fir_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "esp_fir_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "esp_fir_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
            "esp_fir_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2)],
        drop: MESE_POST_FIR,
        drop_count: 1,
        kind: Kind::Post,
    },
    Node {
        id: MESE_POST_JUNGLE,
        name: "esp:mese_post_light_junglewood",
        description: "Jungle Wood Mese Post Light",
        tiles: [
            "default_junglewood.png",
            "default_junglewood.png",
            "default_junglewood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "default_junglewood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "default_junglewood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
            "default_junglewood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2)],
        drop: MESE_POST_JUNGLE,
        drop_count: 1,
        kind: Kind::Post,
    },
    Node {
        id: MESE_POST_ACACIA,
        name: "esp:mese_post_light_acacia_wood",
        description: "Acacia Wood Mese Post Light",
        tiles: [
            "default_acacia_wood.png",
            "default_acacia_wood.png",
            "default_acacia_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "default_acacia_wood.png^default_mese_post_light_side_dark.png^[makealpha:0,0,0",
            "default_acacia_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
            "default_acacia_wood.png^default_mese_post_light_side.png^[makealpha:0,0,0",
        ],
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2)],
        drop: MESE_POST_ACACIA,
        drop_count: 1,
        kind: Kind::Post,
    },
    // minetest_game's signs (default.register_sign).
    Node {
        id: SIGN_WOOD,
        name: "esp:sign_wall_wood",
        description: "Wooden Sign",
        tiles: ["default_sign_wall_wood.png"; 6],
        groups: &[("choppy", 2), ("attached_node", 1), ("flammable", 2), ("oddly_breakable_by_hand", 3)],
        drop: SIGN_WOOD,
        drop_count: 1,
        kind: Kind::Sign,
    },
    Node {
        id: SIGN_STEEL,
        name: "esp:sign_wall_steel",
        description: "Steel Sign",
        tiles: ["default_sign_wall_steel.png"; 6],
        groups: &[("cracky", 2), ("attached_node", 1)],
        drop: SIGN_STEEL,
        drop_count: 1,
        kind: Kind::Sign,
    },
    // minetest_game's bookshelf: wood, with books on its front and back.
    Node {
        id: CONTENT_BOOKSHELF,
        name: "esp:bookshelf",
        description: "Bookshelf",
        tiles: ["esp_apple_wood.png", "esp_apple_wood.png", "esp_apple_wood.png", "esp_apple_wood.png", "default_bookshelf.png", "default_bookshelf.png"],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3)],
        drop: CONTENT_BOOKSHELF,
        drop_count: 1,
        kind: Kind::Shelf,
    },
    // minetest_game's cloud and dry shrub.
    Node {
        id: CONTENT_CLOUD,
        name: "esp:cloud",
        description: "Cloud",
        tiles: ["default_cloud.png"; 6],
        groups: &[("not_in_creative_inventory", 1)],
        drop: CONTENT_CLOUD,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: DRY_SHRUB,
        name: "esp:dry_shrub",
        description: "Dry Shrub",
        tiles: ["default_dry_shrub.png"; 6],
        groups: &[("snappy", 3), ("flammable", 3), ("attached_node", 1)],
        drop: DRY_SHRUB,
        drop_count: 1,
        kind: Kind::Plant,
    },
    // Mist (ours): for filling hollows and dungeons with haze.
    Node {
        id: CONTENT_MIST,
        name: "esp:mist",
        description: "Mist",
        tiles: ["esp_mist.png"; 6],
        groups: &[("not_in_creative_inventory", 1)],
        drop: CONTENT_MIST,
        drop_count: 1,
        kind: Kind::Mist,
    },
    // The vessels mod (its pictures CC BY-SA 3.0, see
    // media/LICENSE-vessels.txt).
    Node {
        id: VESSELS_GLASS_BOTTLE,
        name: "esp:glass_bottle",
        description: "Empty Glass Bottle",
        tiles: ["vessels_glass_bottle.png"; 6],
        groups: &[("vessel", 1), ("dig_immediate", 3), ("attached_node", 1)],
        drop: VESSELS_GLASS_BOTTLE,
        drop_count: 1,
        kind: Kind::Vessel,
    },
    Node {
        id: VESSELS_DRINKING_GLASS,
        name: "esp:drinking_glass",
        description: "Empty Drinking Glass",
        tiles: ["vessels_drinking_glass.png"; 6],
        groups: &[("vessel", 1), ("dig_immediate", 3), ("attached_node", 1)],
        drop: VESSELS_DRINKING_GLASS,
        drop_count: 1,
        kind: Kind::Vessel,
    },
    Node {
        id: VESSELS_STEEL_BOTTLE,
        name: "esp:steel_bottle",
        description: "Empty Heavy Steel Bottle",
        tiles: ["vessels_steel_bottle.png"; 6],
        groups: &[("vessel", 1), ("dig_immediate", 3), ("attached_node", 1)],
        drop: VESSELS_STEEL_BOTTLE,
        drop_count: 1,
        kind: Kind::Vessel,
    },
    Node {
        id: VESSELS_SHELF,
        name: "esp:vessels_shelf",
        description: "Vessels Shelf",
        tiles: ["esp_apple_wood.png", "esp_apple_wood.png", "esp_apple_wood.png", "esp_apple_wood.png", "vessels_shelf.png", "vessels_shelf.png"],
        groups: &[("choppy", 3), ("oddly_breakable_by_hand", 2), ("flammable", 3)],
        drop: VESSELS_SHELF,
        drop_count: 1,
        kind: Kind::Shelf,
    },
    Node {
        id: CONTENT_CAVE_ICE,
        name: "esp:cave_ice",
        description: "Cave Ice",
        tiles: ["default_ice.png"; 6],
        groups: &[("cracky", 3), ("cools_lava", 1), ("slippery", 3), ("not_in_creative_inventory", 1)],
        drop: CONTENT_ICE,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: EMERGENT_JUNGLE_SAPLING,
        name: "esp:emergent_jungle_sapling",
        description: "Emergent Jungle Tree Sapling",
        tiles: ["default_emergent_jungle_sapling.png"; 6],
        groups: &[("snappy", 2), ("dig_immediate", 3), ("flammable", 2), ("attached_node", 1), ("sapling", 1)],
        drop: EMERGENT_JUNGLE_SAPLING,
        drop_count: 1,
        kind: Kind::Sapling,
    },
    // farming's savanna soil and desert sand soil (farming/nodes.lua).
    Node {
        id: farming::DRY_SOIL,
        name: "esp:dry_soil",
        description: "Savanna Soil",
        tiles: ["default_dry_dirt.png^farming_soil.png", "default_dry_dirt.png", "default_dry_dirt.png", "default_dry_dirt.png", "default_dry_dirt.png", "default_dry_dirt.png"],
        groups: &[("crumbly", 3), ("not_in_creative_inventory", 1), ("soil", 2), ("grassland", 1), ("field", 1)],
        drop: CONTENT_DRY_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: farming::DRY_SOIL_WET,
        name: "esp:dry_soil_wet",
        description: "Wet Savanna Soil",
        tiles: [
            "default_dry_dirt.png^farming_soil_wet.png",
            "default_dry_dirt.png",
            "default_dry_dirt.png^farming_soil_wet_side.png",
            "default_dry_dirt.png^farming_soil_wet_side.png",
            "default_dry_dirt.png^farming_soil_wet_side.png",
            "default_dry_dirt.png^farming_soil_wet_side.png",
        ],
        groups: &[("crumbly", 3), ("not_in_creative_inventory", 1), ("soil", 3), ("wet", 1), ("grassland", 1), ("field", 1)],
        drop: CONTENT_DRY_DIRT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: farming::DESERT_SAND_SOIL,
        name: "esp:desert_sand_soil",
        description: "Desert Sand Soil",
        tiles: ["farming_desert_sand_soil.png", "default_desert_sand.png", "default_desert_sand.png", "default_desert_sand.png", "default_desert_sand.png", "default_desert_sand.png"],
        groups: &[("crumbly", 3), ("not_in_creative_inventory", 1), ("falling_node", 1), ("sand", 1), ("soil", 2), ("desert", 1), ("field", 1)],
        drop: CONTENT_DESERT_SAND,
        drop_count: 1,
        kind: Kind::Solid,
    },
    Node {
        id: farming::DESERT_SAND_SOIL_WET,
        name: "esp:desert_sand_soil_wet",
        description: "Wet Desert Sand Soil",
        tiles: [
            "farming_desert_sand_soil_wet.png",
            "default_desert_sand.png",
            "farming_desert_sand_soil_wet_side.png",
            "farming_desert_sand_soil_wet_side.png",
            "farming_desert_sand_soil_wet_side.png",
            "farming_desert_sand_soil_wet_side.png",
        ],
        groups: &[("crumbly", 3), ("falling_node", 1), ("sand", 1), ("not_in_creative_inventory", 1), ("soil", 3), ("wet", 1), ("desert", 1), ("field", 1)],
        drop: CONTENT_DESERT_SAND,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // The fire mod's flames (fire.rs; pictures CC BY-SA 3.0, see
    // media/LICENSE-fire.txt). They drop nothing.
    Node {
        id: fire::BASIC_FLAME,
        name: "esp:basic_flame",
        description: "Fire",
        tiles: [fire::FLAME_TILE; 6],
        groups: &[("igniter", 2), ("dig_immediate", 3), ("fire", 1), ("not_in_creative_inventory", 1)],
        drop: 0,
        drop_count: 0,
        kind: Kind::Fire,
    },
    Node {
        id: fire::PERMANENT_FLAME,
        name: "esp:permanent_flame",
        description: "Permanent Fire",
        tiles: [fire::FLAME_TILE; 6],
        groups: &[("igniter", 2), ("dig_immediate", 3), ("fire", 1)],
        drop: 0,
        drop_count: 0,
        kind: Kind::Fire,
    },
    // The tnt mod's gunpowder and TNT (tnt.rs; pictures CC BY-SA 3.0, see
    // media/LICENSE-tnt.txt).
    Node {
        id: tnt::GUNPOWDER,
        name: "esp:gunpowder",
        description: "Gun Powder",
        tiles: [
            "tnt_gunpowder_straight.png",
            "tnt_gunpowder_curved.png",
            "tnt_gunpowder_t_junction.png",
            "tnt_gunpowder_crossing.png",
            "tnt_gunpowder_straight.png",
            "tnt_gunpowder_straight.png",
        ],
        groups: &[("dig_immediate", 2), ("attached_node", 1), ("flammable", 5), ("connect_to_raillike", tnt::RAILLIKE_GROUP)],
        drop: tnt::GUNPOWDER,
        drop_count: 1,
        kind: Kind::Powder,
    },
    Node {
        id: tnt::GUNPOWDER_BURNING,
        name: "esp:gunpowder_burning",
        description: "",
        tiles: [
            "tnt_gunpowder_burning_straight_animated.png",
            "tnt_gunpowder_burning_curved_animated.png",
            "tnt_gunpowder_burning_t_junction_animated.png",
            "tnt_gunpowder_burning_crossing_animated.png",
            "tnt_gunpowder_burning_straight_animated.png",
            "tnt_gunpowder_burning_straight_animated.png",
        ],
        groups: &[("dig_immediate", 2), ("attached_node", 1), ("connect_to_raillike", tnt::RAILLIKE_GROUP), ("not_in_creative_inventory", 1)],
        drop: 0,
        drop_count: 0,
        kind: Kind::Powder,
    },
    Node {
        id: tnt::TNT,
        name: "esp:tnt",
        description: "TNT",
        tiles: ["tnt_top.png", "tnt_bottom.png", "tnt_side.png", "tnt_side.png", "tnt_side.png", "tnt_side.png"],
        groups: &[("dig_immediate", 2), ("tnt", 1), ("flammable", 5)],
        drop: tnt::TNT,
        drop_count: 1,
        kind: Kind::Solid,
    },
    // Lit: it can't be dug, and goes off in 4 s.
    Node {
        id: tnt::TNT_BURNING,
        name: "esp:tnt_burning",
        description: "",
        tiles: ["tnt_top_burning_animated.png", "tnt_bottom.png", "tnt_side.png", "tnt_side.png", "tnt_side.png", "tnt_side.png"],
        groups: &[("not_in_creative_inventory", 1)],
        drop: 0,
        drop_count: 0,
        kind: Kind::Solid,
    },
];

/// minetest_game's savanna grass groups.
const DRY_GRASS_GROUPS: &[(&str, i16)] = &[("snappy", 3), ("flora", 1), ("attached_node", 1), ("grass", 1), ("dry_grass", 1), ("flammable", 1)];

/// minetest_game's tall grass groups.
const GRASS_GROUPS: &[(&str, i16)] = &[("snappy", 3), ("flora", 1), ("attached_node", 1), ("grass", 1), ("normal_grass", 1), ("flammable", 1)];

/// minetest_game's water textures: 16 frames one above another.
const WATER_SOURCE_TILE: &str = "default_water_source_animated.png";
const WATER_FLOWING_TILE: &str = "default_water_flowing_animated.png";
const LAVA_SOURCE_TILE: &str = "default_lava_source_animated.png";
const LAVA_FLOWING_TILE: &str = "default_lava_flowing_animated.png";

// Saplings, from leaves (one time in SAPLING_RARITY), as minetest_game's
// aspen and pine saplings, with their pictures (CC BY-SA 3.0, see
// media/LICENSE-items.txt).
const SAPLING_GROUPS: &[(&str, i16)] =
    &[("snappy", 2), ("dig_immediate", 3), ("flammable", 3), ("attached_node", 1), ("sapling", 1)];

const CHEST_CLOSED_TILES: [&str; 6] = [
    "default_chest_top.png",
    "default_chest_top.png",
    "default_chest_side.png^[transformFX",
    "default_chest_side.png",
    "default_chest_side.png",
    "default_chest_front.png",
];
const CHEST_LOCKED_CLOSED_TILES: [&str; 6] = [
    "default_chest_top.png",
    "default_chest_top.png",
    "default_chest_side.png^[transformFX",
    "default_chest_side.png",
    "default_chest_side.png",
    "default_chest_lock.png",
];
/// The open model's materials, in minetest_game's order.
const CHEST_OPEN_TILES: [&str; 6] = [
    "default_chest_top.png",
    "default_chest_top.png",
    "default_chest_side.png",
    "default_chest_side.png",
    "default_chest_front.png",
    "default_chest_inside.png",
];
const CHEST_LOCKED_OPEN_TILES: [&str; 6] = [
    "default_chest_top.png",
    "default_chest_top.png",
    "default_chest_side.png",
    "default_chest_side.png",
    "default_chest_lock.png",
    "default_chest_inside.png",
];

/// minetest_game's trapdoor tiles: closed, the picture on top and flipped
/// underneath, the edge picture round the sides; open, turned to stand up.
const TRAPDOOR_WOOD_CLOSED: [&str; 6] = [
    "doors_trapdoor.png",
    "doors_trapdoor.png^[transformFY",
    "doors_trapdoor_side.png",
    "doors_trapdoor_side.png",
    "doors_trapdoor_side.png",
    "doors_trapdoor_side.png",
];
const TRAPDOOR_WOOD_OPEN: [&str; 6] = [
    "doors_trapdoor_side.png",
    "doors_trapdoor_side.png^[transform2",
    "doors_trapdoor_side.png^[transform3",
    "doors_trapdoor_side.png^[transform1",
    "doors_trapdoor.png^[transform46",
    "doors_trapdoor.png^[transform6",
];
const TRAPDOOR_STEEL_CLOSED: [&str; 6] = [
    "doors_trapdoor_steel.png",
    "doors_trapdoor_steel.png^[transformFY",
    "doors_trapdoor_steel_side.png",
    "doors_trapdoor_steel_side.png",
    "doors_trapdoor_steel_side.png",
    "doors_trapdoor_steel_side.png",
];
const TRAPDOOR_BAR_CLOSED: [&str; 6] = [
    "xpanes_trapdoor_steel_bar.png",
    "xpanes_trapdoor_steel_bar.png^[transformFY",
    "xpanes_trapdoor_steel_bar_side.png",
    "xpanes_trapdoor_steel_bar_side.png",
    "xpanes_trapdoor_steel_bar_side.png",
    "xpanes_trapdoor_steel_bar_side.png",
];
const TRAPDOOR_BAR_OPEN: [&str; 6] = [
    "xpanes_trapdoor_steel_bar_side.png",
    "xpanes_trapdoor_steel_bar_side.png^[transform2",
    "xpanes_trapdoor_steel_bar_side.png^[transform3",
    "xpanes_trapdoor_steel_bar_side.png^[transform1",
    "xpanes_trapdoor_steel_bar.png^[transform46",
    "xpanes_trapdoor_steel_bar.png^[transform6",
];
const TRAPDOOR_STEEL_OPEN: [&str; 6] = [
    "doors_trapdoor_steel_side.png",
    "doors_trapdoor_steel_side.png^[transform2",
    "doors_trapdoor_steel_side.png^[transform3",
    "doors_trapdoor_steel_side.png^[transform1",
    "doors_trapdoor_steel.png^[transform46",
    "doors_trapdoor_steel.png^[transform6",
];

/// The open and closed nodes of a chest: (closed, open).
pub fn chest_pair(id: u16) -> Option<(u16, u16)> {
    match id {
        CONTENT_CHEST | CONTENT_CHEST_OPEN => Some((CONTENT_CHEST, CONTENT_CHEST_OPEN)),
        CONTENT_CHEST_LOCKED | CONTENT_CHEST_LOCKED_OPEN => Some((CONTENT_CHEST_LOCKED, CONTENT_CHEST_LOCKED_OPEN)),
        _ => None,
    }
}

pub fn is_chest(id: u16) -> bool {
    chest_pair(id).is_some()
}

pub fn is_locked_chest(id: u16) -> bool {
    matches!(id, CONTENT_CHEST_LOCKED | CONTENT_CHEST_LOCKED_OPEN)
}

/// minetest_game's chest_lid_obstructed: the lid can't open under a block
/// (air, torches and the like are fine).
pub fn lid_obstructed(above: u16) -> bool {
    !(above == crate::world::CONTENT_AIR || is_torch(above) || def(above).is_some_and(|n| n.kind == Kind::Mark))
}

/// minetest_game's burning furnace front: 8 frames of fire, one above
/// another, 1.5 seconds for the cycle.
const FURNACE_ACTIVE_TILE: &str = "default_furnace_front_active.png";

/// An item that is only an item, never a node: crafting material.
pub struct CraftItem {
    pub id: u16,
    pub name: &'static str,
    pub description: &'static str,
    pub image: &'static str,
    pub groups: &'static [(&'static str, i16)],
}

/// minetest_game's craftitems, with its pictures (CC BY-SA 3.0, see
/// media/LICENSE-items.txt).
pub const CRAFTITEMS: &[CraftItem] = &[
    CraftItem {
        id: ITEM_STICK,
        name: "esp:stick",
        description: "Stick",
        image: "default_stick.png",
        groups: &[("stick", 1), ("flammable", 2)],
    },
    CraftItem {
        id: ITEM_COAL_LUMP,
        name: "esp:coal_lump",
        description: "Coal Lump",
        image: "default_coal_lump.png",
        groups: &[("coal", 1), ("flammable", 1)],
    },
    CraftItem {
        id: ITEM_IRON_LUMP,
        name: "esp:iron_lump",
        description: "Iron Lump",
        image: "default_iron_lump.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_STEEL_INGOT,
        name: "esp:steel_ingot",
        description: "Steel Ingot",
        image: "default_steel_ingot.png",
        groups: &[],
    },
    // minetest_game's doors, with their pictures (CC BY-SA 3.0, see
    // media/LICENSE-doors.txt).
    CraftItem {
        id: ITEM_DOOR_WOOD,
        name: "esp:door_wood",
        description: "Wooden Door",
        image: "doors_item_wood.png",
        groups: &[("choppy", 2), ("oddly_breakable_by_hand", 2), ("flammable", 2)],
    },
    CraftItem {
        id: ITEM_DOOR_STEEL,
        name: "esp:door_steel",
        description: "Steel Door",
        image: "doors_item_steel.png",
        groups: &[("cracky", 1), ("level", 2)],
    },
    CraftItem {
        id: ITEM_DOOR_GLASS,
        name: "esp:door_glass",
        description: "Glass Door",
        image: "doors_item_glass.png",
        groups: &[("cracky", 3), ("oddly_breakable_by_hand", 3)],
    },
    // minetest_game's buckets (bucket mod; CC BY-SA 3.0, see
    // media/LICENSE-items.txt).
    CraftItem {
        id: ITEM_BUCKET_EMPTY,
        name: "esp:bucket_empty",
        description: "Empty Bucket",
        image: "bucket.png",
        groups: &[("tool", 1)],
    },
    CraftItem {
        id: ITEM_BUCKET_WATER,
        name: "esp:bucket_water",
        description: "Water Bucket",
        image: "bucket_water.png",
        groups: &[("tool", 1), ("water_bucket", 1)],
    },
    CraftItem {
        id: ITEM_BUCKET_LAVA,
        name: "esp:bucket_lava",
        description: "Lava Bucket",
        image: "bucket_lava.png",
        groups: &[("tool", 1)],
    },
    // minetest_game's blueberries, eaten for 2 HP.
    CraftItem {
        id: ITEM_BLUEBERRIES,
        name: "esp:blueberries",
        description: "Blueberries",
        image: "default_blueberries.png",
        groups: &[("food_blueberries", 1), ("food_berry", 1)],
    },
    // minetest_game's dyes (mods/dye).
    CraftItem {
        id: colours::dye(0),
        name: "esp:dye_white",
        description: "White Dye",
        image: "dye_white.png",
        groups: &[("dye", 1), ("color_white", 1)],
    },
    CraftItem {
        id: colours::dye(1),
        name: "esp:dye_grey",
        description: "Grey Dye",
        image: "dye_grey.png",
        groups: &[("dye", 1), ("color_grey", 1)],
    },
    CraftItem {
        id: colours::dye(2),
        name: "esp:dye_dark_grey",
        description: "Dark Grey Dye",
        image: "dye_dark_grey.png",
        groups: &[("dye", 1), ("color_dark_grey", 1)],
    },
    CraftItem {
        id: colours::dye(3),
        name: "esp:dye_black",
        description: "Black Dye",
        image: "dye_black.png",
        groups: &[("dye", 1), ("color_black", 1)],
    },
    CraftItem {
        id: colours::dye(4),
        name: "esp:dye_violet",
        description: "Violet Dye",
        image: "dye_violet.png",
        groups: &[("dye", 1), ("color_violet", 1)],
    },
    CraftItem {
        id: colours::dye(5),
        name: "esp:dye_blue",
        description: "Blue Dye",
        image: "dye_blue.png",
        groups: &[("dye", 1), ("color_blue", 1)],
    },
    CraftItem {
        id: colours::dye(6),
        name: "esp:dye_cyan",
        description: "Cyan Dye",
        image: "dye_cyan.png",
        groups: &[("dye", 1), ("color_cyan", 1)],
    },
    CraftItem {
        id: colours::dye(7),
        name: "esp:dye_dark_green",
        description: "Dark Green Dye",
        image: "dye_dark_green.png",
        groups: &[("dye", 1), ("color_dark_green", 1)],
    },
    CraftItem {
        id: colours::dye(8),
        name: "esp:dye_green",
        description: "Green Dye",
        image: "dye_green.png",
        groups: &[("dye", 1), ("color_green", 1)],
    },
    CraftItem {
        id: colours::dye(9),
        name: "esp:dye_yellow",
        description: "Yellow Dye",
        image: "dye_yellow.png",
        groups: &[("dye", 1), ("color_yellow", 1)],
    },
    CraftItem {
        id: colours::dye(10),
        name: "esp:dye_brown",
        description: "Brown Dye",
        image: "dye_brown.png",
        groups: &[("dye", 1), ("color_brown", 1)],
    },
    CraftItem {
        id: colours::dye(11),
        name: "esp:dye_orange",
        description: "Orange Dye",
        image: "dye_orange.png",
        groups: &[("dye", 1), ("color_orange", 1)],
    },
    CraftItem {
        id: colours::dye(12),
        name: "esp:dye_red",
        description: "Red Dye",
        image: "dye_red.png",
        groups: &[("dye", 1), ("color_red", 1)],
    },
    CraftItem {
        id: colours::dye(13),
        name: "esp:dye_magenta",
        description: "Magenta Dye",
        image: "dye_magenta.png",
        groups: &[("dye", 1), ("color_magenta", 1)],
    },
    CraftItem {
        id: colours::dye(14),
        name: "esp:dye_pink",
        description: "Pink Dye",
        image: "dye_pink.png",
        groups: &[("dye", 1), ("color_pink", 1)],
    },
    // xpanes' steel bar door (CC BY-SA 3.0, see media/LICENSE-xpanes.txt).
    CraftItem {
        id: ITEM_DOOR_STEEL_BAR,
        name: "esp:door_steel_bar",
        description: "Steel Bar Door",
        image: "xpanes_item_steel_bar.png",
        groups: &[("cracky", 1), ("level", 2)],
    },
    // minetest_game's paper and books (craftitems.lua).
    CraftItem {
        id: ITEM_PAPER,
        name: "esp:paper",
        description: "Paper",
        image: "default_paper.png",
        groups: &[("flammable", 3)],
    },
    CraftItem {
        id: ITEM_BOOK,
        name: "esp:book",
        description: "Book",
        image: "default_book.png",
        groups: &[("book", 1), ("flammable", 3)],
    },
    CraftItem {
        id: ITEM_BOOK_WRITTEN,
        name: "esp:book_written",
        description: "Book with Text",
        image: "default_book_written.png",
        groups: &[("book", 1), ("not_in_creative_inventory", 1), ("flammable", 3)],
    },
    // vessels' glass fragments (CC BY-SA 3.0, media/LICENSE-vessels.txt).
    CraftItem {
        id: ITEM_GLASS_FRAGMENTS,
        name: "esp:glass_fragments",
        description: "Glass Fragments",
        image: "vessels_glass_fragments.png",
        groups: &[],
    },
    // The keys mod (CC BY-SA 3.0, media/LICENSE-keys.txt): a skeleton key
    // makes a key to its owner's locked chest or door (keys.rs).
    CraftItem {
        id: ITEM_SKELETON_KEY,
        name: "esp:skeleton_key",
        description: "Skeleton Key",
        image: "keys_key_skeleton.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_KEY,
        name: "esp:key",
        description: "Key",
        image: "keys_key.png",
        groups: &[("key", 1), ("not_in_creative_inventory", 1)],
    },
    // binoculars and map (CC BY-SA 3.0, media/LICENSE-keys.txt): carried,
    // they let the player zoom, or see the minimap (binoculars.rs).
    CraftItem {
        id: ITEM_BINOCULARS,
        name: "esp:binoculars",
        description: "Binoculars\nUse with 'Zoom' key",
        image: "binoculars_binoculars.png",
        groups: &[("tool", 1)],
    },
    CraftItem {
        id: ITEM_MAPPING_KIT,
        name: "esp:mapping_kit",
        description: "Mapping Kit\nUse with 'Minimap' key",
        image: "map_mapping_kit.png",
        groups: &[("flammable", 3), ("tool", 1)],
    },
    // The tnt mod's TNT stick (its picture CC BY-SA 3.0, see
    // media/LICENSE-tnt.txt).
    CraftItem {
        id: ITEM_TNT_STICK,
        name: "esp:tnt_stick",
        description: "TNT Stick",
        image: "tnt_tnt_stick.png",
        groups: &[("flammable", 5)],
    },
    // minetest_game's lumps, crystals and ingots (craftitems.lua).
    CraftItem {
        id: ITEM_FLINT,
        name: "esp:flint",
        description: "Flint",
        image: "default_flint.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_CLAY_LUMP,
        name: "esp:clay_lump",
        description: "Clay Lump",
        image: "default_clay_lump.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_CLAY_BRICK,
        name: "esp:clay_brick",
        description: "Clay Brick",
        image: "default_clay_brick.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_OBSIDIAN_SHARD,
        name: "esp:obsidian_shard",
        description: "Obsidian Shard",
        image: "default_obsidian_shard.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_COPPER_LUMP,
        name: "esp:copper_lump",
        description: "Copper Lump",
        image: "default_copper_lump.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_TIN_LUMP,
        name: "esp:tin_lump",
        description: "Tin Lump",
        image: "default_tin_lump.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_GOLD_LUMP,
        name: "esp:gold_lump",
        description: "Gold Lump",
        image: "default_gold_lump.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_MESE_CRYSTAL,
        name: "esp:mese_crystal",
        description: "Mese Crystal",
        image: "default_mese_crystal.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_MESE_FRAGMENT,
        name: "esp:mese_crystal_fragment",
        description: "Mese Crystal Fragment",
        image: "default_mese_crystal_fragment.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_DIAMOND,
        name: "esp:diamond",
        description: "Diamond",
        image: "default_diamond.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_COPPER_INGOT,
        name: "esp:copper_ingot",
        description: "Copper Ingot",
        image: "default_copper_ingot.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_TIN_INGOT,
        name: "esp:tin_ingot",
        description: "Tin Ingot",
        image: "default_tin_ingot.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_BRONZE_INGOT,
        name: "esp:bronze_ingot",
        description: "Bronze Ingot",
        image: "default_bronze_ingot.png",
        groups: &[],
    },
    CraftItem {
        id: ITEM_GOLD_INGOT,
        name: "esp:gold_ingot",
        description: "Gold Ingot",
        image: "default_gold_ingot.png",
        groups: &[],
    },
    // farming's (see media/LICENSE-farming.txt).
    CraftItem {
        id: ITEM_WHEAT,
        name: "esp:wheat",
        description: "Wheat",
        image: "farming_wheat.png",
        groups: &[("food_wheat", 1), ("flammable", 4)],
    },
    CraftItem {
        id: ITEM_FLOUR,
        name: "esp:flour",
        description: "Flour",
        image: "farming_flour.png",
        groups: &[("food_flour", 1), ("flammable", 1)],
    },
    CraftItem {
        id: ITEM_BREAD,
        name: "esp:bread",
        description: "Bread",
        image: "farming_bread.png",
        groups: &[("food_bread", 1), ("flammable", 2)],
    },
    CraftItem {
        id: ITEM_COTTON,
        name: "esp:cotton",
        description: "Cotton",
        image: "farming_cotton.png",
        groups: &[("flammable", 4)],
    },
    CraftItem {
        id: ITEM_STRING,
        name: "esp:string",
        description: "String",
        image: "farming_string.png",
        groups: &[("flammable", 2)],
    },
];

fn craftitem(id: u16) -> Option<&'static CraftItem> {
    CRAFTITEMS.iter().find(|c| c.id == id)
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

/// What a tool (or the hand) can dig in one group: the time for each
/// rating of the group, how many digs it lasts (0: for ever), and the
/// highest node "level" it can dig (Luanti's ToolGroupCap).
pub struct GroupCap {
    pub group: &'static str,
    pub times: &'static [(i16, f32)],
    pub uses: u32,
    pub maxlevel: i16,
}

/// Luanti's ToolCapabilities.
pub struct ToolCaps {
    pub full_punch_interval: f32,
    pub max_drop_level: i16,
    pub groupcaps: &'static [GroupCap],
    /// Damage to the "fleshy" armour group when punching.
    pub fleshy: i16,
}

/// minetest_game's hand (tools.lua): dirt and sand, leaves, and trunks and
/// planks slowly, but no stone. It never wears out.
pub const HAND: ToolCaps = ToolCaps {
    full_punch_interval: 0.9,
    max_drop_level: 0,
    groupcaps: &[
        GroupCap { group: "crumbly", times: &[(2, 3.0), (3, 0.7)], uses: 0, maxlevel: 1 },
        GroupCap { group: "snappy", times: &[(3, 0.4)], uses: 0, maxlevel: 1 },
        GroupCap { group: "oddly_breakable_by_hand", times: &[(1, 3.5), (2, 2.0), (3, 0.7)], uses: 0, maxlevel: 1 },
    ],
    fleshy: 1,
};

pub struct Tool {
    pub id: u16,
    pub name: &'static str,
    pub description: &'static str,
    pub image: &'static str,
    pub wield_image: &'static str,
    pub caps: ToolCaps,
    pub groups: &'static [(&'static str, i16)],
}

/// minetest_game's wood and stone tools (tools.lua), with its pictures
/// (CC BY-SA 3.0, see media/LICENSE-items.txt). They break with
/// "default_tool_breaks".
pub const TOOLS: &[Tool] = &[
    Tool {
        id: TOOL_PICK_WOOD,
        name: "esp:pick_wood",
        description: "Wooden Pickaxe",
        image: "default_tool_woodpick.png",
        wield_image: "default_tool_woodpick.png",
        caps: ToolCaps {
            full_punch_interval: 1.2,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "cracky", times: &[(3, 1.6)], uses: 10, maxlevel: 1 }],
            fleshy: 2,
        },
        groups: &[("pickaxe", 1), ("flammable", 2)],
    },
    Tool {
        id: TOOL_PICK_STONE,
        name: "esp:pick_stone",
        description: "Stone Pickaxe",
        image: "default_tool_stonepick.png",
        wield_image: "default_tool_stonepick.png",
        caps: ToolCaps {
            full_punch_interval: 1.3,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "cracky", times: &[(2, 2.0), (3, 1.0)], uses: 20, maxlevel: 1 }],
            fleshy: 3,
        },
        groups: &[("pickaxe", 1)],
    },
    Tool {
        id: TOOL_SHOVEL_WOOD,
        name: "esp:shovel_wood",
        description: "Wooden Shovel",
        image: "default_tool_woodshovel.png",
        wield_image: "default_tool_woodshovel.png^[transformR90",
        caps: ToolCaps {
            full_punch_interval: 1.2,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "crumbly", times: &[(1, 3.0), (2, 1.6), (3, 0.6)], uses: 10, maxlevel: 1 }],
            fleshy: 2,
        },
        groups: &[("shovel", 1), ("flammable", 2)],
    },
    Tool {
        id: TOOL_SHOVEL_STONE,
        name: "esp:shovel_stone",
        description: "Stone Shovel",
        image: "default_tool_stoneshovel.png",
        wield_image: "default_tool_stoneshovel.png^[transformR90",
        caps: ToolCaps {
            full_punch_interval: 1.4,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "crumbly", times: &[(1, 1.8), (2, 1.2), (3, 0.5)], uses: 20, maxlevel: 1 }],
            fleshy: 2,
        },
        groups: &[("shovel", 1)],
    },
    Tool {
        id: TOOL_AXE_WOOD,
        name: "esp:axe_wood",
        description: "Wooden Axe",
        image: "default_tool_woodaxe.png",
        wield_image: "default_tool_woodaxe.png",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "choppy", times: &[(2, 3.0), (3, 1.6)], uses: 10, maxlevel: 1 }],
            fleshy: 2,
        },
        groups: &[("axe", 1), ("flammable", 2)],
    },
    Tool {
        id: TOOL_AXE_STONE,
        name: "esp:axe_stone",
        description: "Stone Axe",
        image: "default_tool_stoneaxe.png",
        wield_image: "default_tool_stoneaxe.png",
        caps: ToolCaps {
            full_punch_interval: 1.2,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "choppy", times: &[(1, 3.0), (2, 2.0), (3, 1.3)], uses: 20, maxlevel: 1 }],
            fleshy: 3,
        },
        groups: &[("axe", 1)],
    },
    Tool {
        id: TOOL_SWORD_WOOD,
        name: "esp:sword_wood",
        description: "Wooden Sword",
        image: "default_tool_woodsword.png",
        wield_image: "default_tool_woodsword.png",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "snappy", times: &[(2, 1.6), (3, 0.4)], uses: 10, maxlevel: 1 }],
            fleshy: 2,
        },
        groups: &[("sword", 1), ("flammable", 2)],
    },
    Tool {
        id: TOOL_SWORD_STONE,
        name: "esp:sword_stone",
        description: "Stone Sword",
        image: "default_tool_stonesword.png",
        wield_image: "default_tool_stonesword.png",
        caps: ToolCaps {
            full_punch_interval: 1.2,
            max_drop_level: 0,
            groupcaps: &[GroupCap { group: "snappy", times: &[(2, 1.4), (3, 0.4)], uses: 20, maxlevel: 1 }],
            fleshy: 4,
        },
        groups: &[("sword", 1)],
    },
    Tool {
        id: TOOL_PICK_STEEL,
        name: "esp:pick_steel",
        description: "Steel Pickaxe",
        image: "default_tool_steelpick.png",
        wield_image: "default_tool_steelpick.png",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "cracky", times: &[(1, 4.0), (2, 1.6), (3, 0.8)], uses: 20, maxlevel: 2 }],
            fleshy: 4,
        },
        groups: &[("pickaxe", 1)],
    },
    Tool {
        id: TOOL_SHOVEL_STEEL,
        name: "esp:shovel_steel",
        description: "Steel Shovel",
        image: "default_tool_steelshovel.png",
        wield_image: "default_tool_steelshovel.png^[transformR90",
        caps: ToolCaps {
            full_punch_interval: 1.1,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "crumbly", times: &[(1, 1.5), (2, 0.9), (3, 0.4)], uses: 30, maxlevel: 2 }],
            fleshy: 3,
        },
        groups: &[("shovel", 1)],
    },
    Tool {
        id: TOOL_AXE_STEEL,
        name: "esp:axe_steel",
        description: "Steel Axe",
        image: "default_tool_steelaxe.png",
        wield_image: "default_tool_steelaxe.png",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "choppy", times: &[(1, 2.5), (2, 1.4), (3, 1.0)], uses: 20, maxlevel: 2 }],
            fleshy: 4,
        },
        groups: &[("axe", 1)],
    },
    Tool {
        id: TOOL_SWORD_STEEL,
        name: "esp:sword_steel",
        description: "Steel Sword",
        image: "default_tool_steelsword.png",
        wield_image: "default_tool_steelsword.png",
        caps: ToolCaps {
            full_punch_interval: 0.8,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "snappy", times: &[(1, 2.5), (2, 1.2), (3, 0.35)], uses: 30, maxlevel: 2 }],
            fleshy: 6,
        },
        groups: &[("sword", 1)],
    },
    // minetest_game's bronze, mese and diamond tools.
    Tool {
        id: TOOL_PICK_BRONZE,
        name: "esp:pick_bronze",
        description: "Bronze Pickaxe",
        image: "default_tool_bronzepick.png",
        wield_image: "default_tool_bronzepick.png",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "cracky", times: &[(1, 4.5), (2, 1.8), (3, 0.9)], uses: 20, maxlevel: 2 }],
            fleshy: 4,
        },
        groups: &[("pickaxe", 1)],
    },
    Tool {
        id: TOOL_SHOVEL_BRONZE,
        name: "esp:shovel_bronze",
        description: "Bronze Shovel",
        image: "default_tool_bronzeshovel.png",
        wield_image: "default_tool_bronzeshovel.png^[transformR90",
        caps: ToolCaps {
            full_punch_interval: 1.1,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "crumbly", times: &[(1, 1.65), (2, 1.05), (3, 0.45)], uses: 25, maxlevel: 2 }],
            fleshy: 3,
        },
        groups: &[("shovel", 1)],
    },
    Tool {
        id: TOOL_AXE_BRONZE,
        name: "esp:axe_bronze",
        description: "Bronze Axe",
        image: "default_tool_bronzeaxe.png",
        wield_image: "default_tool_bronzeaxe.png",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "choppy", times: &[(1, 2.75), (2, 1.7), (3, 1.15)], uses: 20, maxlevel: 2 }],
            fleshy: 4,
        },
        groups: &[("axe", 1)],
    },
    Tool {
        id: TOOL_SWORD_BRONZE,
        name: "esp:sword_bronze",
        description: "Bronze Sword",
        image: "default_tool_bronzesword.png",
        wield_image: "default_tool_bronzesword.png",
        caps: ToolCaps {
            full_punch_interval: 0.8,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "snappy", times: &[(1, 2.75), (2, 1.3), (3, 0.375)], uses: 25, maxlevel: 2 }],
            fleshy: 6,
        },
        groups: &[("sword", 1)],
    },
    Tool {
        id: TOOL_PICK_MESE,
        name: "esp:pick_mese",
        description: "Mese Pickaxe",
        image: "default_tool_mesepick.png",
        wield_image: "default_tool_mesepick.png",
        caps: ToolCaps {
            full_punch_interval: 0.9,
            max_drop_level: 3,
            groupcaps: &[GroupCap { group: "cracky", times: &[(1, 2.4), (2, 1.2), (3, 0.6)], uses: 20, maxlevel: 3 }],
            fleshy: 5,
        },
        groups: &[("pickaxe", 1)],
    },
    Tool {
        id: TOOL_SHOVEL_MESE,
        name: "esp:shovel_mese",
        description: "Mese Shovel",
        image: "default_tool_meseshovel.png",
        wield_image: "default_tool_meseshovel.png^[transformR90",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 3,
            groupcaps: &[GroupCap { group: "crumbly", times: &[(1, 1.2), (2, 0.6), (3, 0.3)], uses: 20, maxlevel: 3 }],
            fleshy: 4,
        },
        groups: &[("shovel", 1)],
    },
    Tool {
        id: TOOL_AXE_MESE,
        name: "esp:axe_mese",
        description: "Mese Axe",
        image: "default_tool_meseaxe.png",
        wield_image: "default_tool_meseaxe.png",
        caps: ToolCaps {
            full_punch_interval: 0.9,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "choppy", times: &[(1, 2.2), (2, 1.0), (3, 0.6)], uses: 20, maxlevel: 3 }],
            fleshy: 6,
        },
        groups: &[("axe", 1)],
    },
    Tool {
        id: TOOL_SWORD_MESE,
        name: "esp:sword_mese",
        description: "Mese Sword",
        image: "default_tool_mesesword.png",
        wield_image: "default_tool_mesesword.png",
        caps: ToolCaps {
            full_punch_interval: 0.7,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "snappy", times: &[(1, 2.0), (2, 1.0), (3, 0.35)], uses: 30, maxlevel: 3 }],
            fleshy: 7,
        },
        groups: &[("sword", 1)],
    },
    Tool {
        id: TOOL_PICK_DIAMOND,
        name: "esp:pick_diamond",
        description: "Diamond Pickaxe",
        image: "default_tool_diamondpick.png",
        wield_image: "default_tool_diamondpick.png",
        caps: ToolCaps {
            full_punch_interval: 0.9,
            max_drop_level: 3,
            groupcaps: &[GroupCap { group: "cracky", times: &[(1, 2.0), (2, 1.0), (3, 0.5)], uses: 30, maxlevel: 3 }],
            fleshy: 5,
        },
        groups: &[("pickaxe", 1)],
    },
    Tool {
        id: TOOL_SHOVEL_DIAMOND,
        name: "esp:shovel_diamond",
        description: "Diamond Shovel",
        image: "default_tool_diamondshovel.png",
        wield_image: "default_tool_diamondshovel.png^[transformR90",
        caps: ToolCaps {
            full_punch_interval: 1.0,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "crumbly", times: &[(1, 1.1), (2, 0.5), (3, 0.3)], uses: 30, maxlevel: 3 }],
            fleshy: 4,
        },
        groups: &[("shovel", 1)],
    },
    Tool {
        id: TOOL_AXE_DIAMOND,
        name: "esp:axe_diamond",
        description: "Diamond Axe",
        image: "default_tool_diamondaxe.png",
        wield_image: "default_tool_diamondaxe.png",
        caps: ToolCaps {
            full_punch_interval: 0.9,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "choppy", times: &[(1, 2.1), (2, 0.9), (3, 0.5)], uses: 30, maxlevel: 3 }],
            fleshy: 7,
        },
        groups: &[("axe", 1)],
    },
    Tool {
        id: TOOL_SWORD_DIAMOND,
        name: "esp:sword_diamond",
        description: "Diamond Sword",
        image: "default_tool_diamondsword.png",
        wield_image: "default_tool_diamondsword.png",
        caps: ToolCaps {
            full_punch_interval: 0.7,
            max_drop_level: 1,
            groupcaps: &[GroupCap { group: "snappy", times: &[(1, 1.9), (2, 0.9), (3, 0.3)], uses: 40, maxlevel: 3 }],
            fleshy: 8,
        },
        groups: &[("sword", 1)],
    },
    // farming's hoes (hoes.lua). They have no digging of their own: a
    // left-click uses them, to turn dirt into soil (farming.rs).
    Tool {
        id: TOOL_HOE_WOOD,
        name: "esp:hoe_wood",
        description: "Wooden Hoe",
        image: "farming_tool_woodhoe.png",
        wield_image: "farming_tool_woodhoe.png",
        caps: HAND,
        groups: &[("hoe", 1), ("flammable", 2)],
    },
    Tool {
        id: TOOL_HOE_STONE,
        name: "esp:hoe_stone",
        description: "Stone Hoe",
        image: "farming_tool_stonehoe.png",
        wield_image: "farming_tool_stonehoe.png",
        caps: HAND,
        groups: &[("hoe", 1)],
    },
    Tool {
        id: TOOL_HOE_STEEL,
        name: "esp:hoe_steel",
        description: "Steel Hoe",
        image: "farming_tool_steelhoe.png",
        wield_image: "farming_tool_steelhoe.png",
        caps: HAND,
        groups: &[("hoe", 1)],
    },
    // The screwdriver mod (screwdriver.rs; its picture CC BY-SA 3.0, see
    // media/LICENSE-keys.txt). It digs as the hand does.
    Tool {
        id: TOOL_SCREWDRIVER,
        name: "esp:screwdriver",
        description: "Screwdriver\n(left-click rotates face, right-click rotates axis)",
        image: "screwdriver.png",
        wield_image: "screwdriver.png",
        caps: HAND,
        groups: &[("tool", 1)],
    },
    // The fire mod's flint and steel (fire.rs; its picture CC BY-SA 3.0,
    // see media/LICENSE-fire.txt). It digs as the hand does.
    Tool {
        id: TOOL_FLINT_AND_STEEL,
        name: "esp:flint_and_steel",
        description: "Flint and Steel",
        image: "fire_flint_steel.png",
        wield_image: "fire_flint_steel.png",
        caps: HAND,
        groups: &[],
    },
];

/// The sound a tool makes when it wears out (minetest_game's sound.breaks).
/// The steel tools are minetest_game's too: maxlevel 2 against our level 0
/// blocks, so Luanti halves their times and gives them nine times the uses.
pub const TOOL_BREAKS: &str = "default_tool_breaks";

fn tool(id: u16) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.id == id)
}

pub fn is_tool(id: u16) -> bool {
    tool(id).is_some()
}

/// The capabilities an item digs with: a tool's own, otherwise the
/// hand's (ItemStack::getToolCapabilities).
pub fn caps_of(id: Option<u16>) -> &'static ToolCaps {
    id.and_then(tool).map_or(&HAND, |t| &t.caps)
}

/// The wear one use adds to a tool that lasts `uses` uses, spread so that
/// it breaks after exactly that many (Luanti's calculateResultWear).
pub fn use_wear(uses: u32, wear: u16) -> u32 {
    if uses == 0 {
        return 0;
    }
    let normal = 65536 / uses;
    let oversize = 65536 % uses;
    let extra = if oversize > 0 && wear as u32 >= (uses - oversize) * normal { 1 } else { 0 };
    normal + extra
}

/// Whether `caps` can dig a node, and if so how long it takes and how much
/// wear it adds (Luanti's getDigParams).
pub fn dig_params(node: u16, caps: &ToolCaps, wear: u16) -> Option<(f32, u32)> {
    // dig_immediate: fixed times, no wear, whatever the tool.
    if !caps.groupcaps.iter().any(|c| c.group == "dig_immediate") {
        match group_of(node, "dig_immediate") {
            2 => return Some((0.5, 0)),
            3 => return Some((0.0, 0)),
            _ => {}
        }
    }
    let level = group_of(node, "level");
    let mut best: Option<(f32, u32)> = None;
    for cap in caps.groupcaps {
        let leveldiff = cap.maxlevel - level;
        if leveldiff < 0 {
            continue;
        }
        let rating = group_of(node, cap.group);
        let Some(&(_, mut time)) = cap.times.iter().find(|(r, _)| *r == rating) else {
            continue;
        };
        if leveldiff > 1 {
            time /= leveldiff as f32;
        }
        if !matches!(best, Some((t, _)) if time >= t) {
            let uses = (cap.uses as f64 * 3f64.powi(leveldiff as i32)).min(65535.0) as u32;
            best = Some((time, use_wear(uses, wear)));
        }
    }
    best
}

/// minetest_game's torch texture: 16 frames of flame, one above another.
const TORCH_TILE: &str = "esp_torch_animated.png";
const TORCH_FRAMES_LENGTH: f32 = 3.3;
const TORCH_IMAGE: &str = "esp_torch_on_floor.png";

/// The model each torch node uses.
fn torch_mesh(id: u16) -> &'static str {
    match id {
        CONTENT_TORCH_WALL => "esp_torch_wall.obj",
        CONTENT_TORCH_CEILING => "esp_torch_ceiling.obj",
        _ => "esp_torch_floor.obj",
    }
}

/// The torch node for a placement direction (param2: 0 ceiling, 1 floor,
/// otherwise a wall), as minetest_game's torch on_place picks it. Other
/// nodes are returned unchanged.
pub fn torch_variant(id: u16, param2: u8) -> u16 {
    if !is_torch(id) {
        return id;
    }
    match param2 & 7 {
        0 => CONTENT_TORCH_CEILING,
        1 => CONTENT_TORCH,
        _ => CONTENT_TORCH_WALL,
    }
}

fn def(id: u16) -> Option<&'static Node> {
    NODES.get(node_index(id)).filter(|n| n.id == id).or_else(|| NODES.iter().find(|n| n.id == id))
}

/// Where a node is in NODES: its id, less three past Luanti's reserved ids.
fn node_index(id: u16) -> usize {
    if id >= 128 { id as usize - 3 } else { id as usize }
}

/// Light a node gives off (0 for most).
pub fn light_source(id: u16) -> u8 {
    match def(id) {
        Some(n) if n.kind == Kind::Torch => TORCH_LIGHT,
        Some(n) if n.id == CONTENT_FURNACE_ACTIVE => FURNACE_LIGHT,
        Some(n) if n.id == CONTENT_MESE_BLOCK => MESE_LIGHT,
        Some(n) if n.id == CONTENT_MESE_LAMP || n.kind == Kind::Post => MESE_LAMP_LIGHT,
        _ => 0,
    }
}

/// Whether light passes through a node (fading by one level per node).
pub fn passes_light(id: u16) -> bool {
    id == crate::world::CONTENT_AIR || def(id).is_some_and(|n| matches!(n.kind, Kind::Torch | Kind::Leaves | Kind::Sapling | Kind::Vessel | Kind::Apple | Kind::Mark | Kind::Chest | Kind::ChestOpen | Kind::Glass | Kind::Ladder | Kind::Shaped | Kind::Door | Kind::DoorHidden | Kind::Trapdoor | Kind::Liquid | Kind::Plant | Kind::Seed | Kind::Bed | Kind::BedTop | Kind::Fence | Kind::Gate | Kind::Stalk | Kind::Stem | Kind::Lamp | Kind::SnowLayer | Kind::Lily | Kind::Pane | Kind::Wall | Kind::Post | Kind::Sign | Kind::Mist | Kind::Fire | Kind::Powder))
}

/// Whether a node is a liquid (water or lava).
pub fn is_liquid(id: u16) -> bool {
    def(id).is_some_and(|n| n.kind == Kind::Liquid)
}

pub fn is_lava(id: u16) -> bool {
    matches!(id, CONTENT_LAVA | CONTENT_LAVA_FLOWING)
}

/// The light a node is drawn with, whatever light reaches it: torches and
/// a burning furnace (which also light their surroundings), and lava
/// (which doesn't, as far as the server is concerned).
pub fn glow(id: u16) -> u8 {
    if is_lava(id) {
        LAVA_GLOW
    } else if fire::is_flame(id) {
        fire::FLAME_GLOW
    } else if tnt::is_lit(id) {
        tnt::BURNING_GLOW
    } else {
        light_source(id)
    }
}

/// Breath a node takes from a player whose head is in it, every 2 s once
/// their breath is gone in HP (minetest_game's water and lava: 1).
pub fn drowning(id: u16) -> u16 {
    is_liquid(id) as u16
}

/// Damage a node does each second to a player in it.
pub fn damage_per_second(id: u16) -> u16 {
    if is_lava(id) {
        LAVA_DAMAGE
    } else if fire::is_flame(id) {
        fire::FLAME_DAMAGE
    } else {
        0
    }
}

/// Whether water washes a node away (minetest_game's floodable): torches
/// and snow.
pub fn floodable(id: u16) -> bool {
    is_torch(id) || id == CONTENT_SNOW_LAYER || id == colours::WATERLILY || fire::is_flame(id)
}

/// Whether placing a node can replace this one (builtin's buildable_to):
/// air, an apple mark, or water.
pub fn buildable_to(id: u16) -> bool {
    id == crate::world::CONTENT_AIR || def(id).is_some_and(|n| matches!(n.kind, Kind::Mark | Kind::Liquid | Kind::Plant | Kind::SnowLayer | Kind::Lily | Kind::Mist | Kind::Fire))
}

/// Whether direct sunlight carries on down through a node at full
/// strength. Leaves let light through but cast shade.
pub fn passes_sunlight(id: u16) -> bool {
    id == crate::world::CONTENT_AIR || def(id).is_some_and(|n| matches!(n.kind, Kind::Torch | Kind::Sapling | Kind::Vessel | Kind::Apple | Kind::Mark | Kind::Glass | Kind::Ladder | Kind::Door | Kind::DoorHidden | Kind::Plant | Kind::Seed | Kind::Fence | Kind::Gate | Kind::Stalk | Kind::Stem | Kind::Lamp | Kind::Lily | Kind::Pane | Kind::Post | Kind::Sign | Kind::Mist | Kind::Fire | Kind::Powder))
}

/// Whether a node's collision box reaches half a node above it (fences,
/// walls, closed gates): what a mob can't jump over.
pub fn is_tall(id: u16) -> bool {
    def(id).is_some_and(|n| matches!(n.kind, Kind::Fence | Kind::Wall)) || crate::fences::gate_of(id).is_some_and(|(_, open)| !open)
}

/// Whether players stand on a node (and so whether it can hold a torch).
pub fn walkable(id: u16) -> bool {
    def(id).is_some_and(|n| matches!(n.kind, Kind::Solid | Kind::Leaves | Kind::Furnace | Kind::Chest | Kind::ChestOpen | Kind::Glass | Kind::Shaped | Kind::Door | Kind::Trapdoor | Kind::Bed | Kind::BedTop | Kind::Fence | Kind::Gate | Kind::Stem | Kind::Rooted | Kind::Lamp | Kind::SnowLayer | Kind::Pane | Kind::Bones | Kind::Wall | Kind::Post | Kind::Shelf))
}

/// Whether right-clicking a node opens it (and so doesn't place against
/// it, unless the player is sneaking).
pub fn is_furnace(id: u16) -> bool {
    def(id).is_some_and(|n| n.kind == Kind::Furnace)
}

/// The facedir param2 for a node placed at `above` by a player standing at
/// `player` (nodes): the front turned towards the player
/// (builtin's item_place_node with core.dir_to_facedir).
pub fn facedir_param2(above: [i16; 3], player: [f32; 3]) -> u8 {
    let dx = above[0] as f32 - player[0];
    let dz = above[2] as f32 - player[2];
    if dx.abs() > dz.abs() {
        if dx < 0.0 { 3 } else { 1 }
    } else if dz < 0.0 {
        2
    } else {
        0
    }
}

pub fn is_torch(id: u16) -> bool {
    def(id).is_some_and(|n| n.kind == Kind::Torch)
}

/// Nodes that hang on what they were placed against (param2 says which
/// way): torches and ladders.
pub fn is_wallmounted(id: u16) -> bool {
    def(id).is_some_and(|n| matches!(n.kind, Kind::Torch | Kind::Ladder | Kind::Sign))
}

/// How param2 turns a node (its paramtype2), where it does: facedir or
/// wallmounted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turned {
    Facedir,
    Wallmounted,
}

pub fn turned_by(id: u16) -> Option<Turned> {
    let n = def(id)?;
    match n.kind {
        Kind::Torch | Kind::Sign | Kind::Ladder | Kind::Seed => Some(Turned::Wallmounted),
        Kind::Furnace | Kind::Bones | Kind::Shelf | Kind::Chest | Kind::ChestOpen | Kind::Shaped | Kind::Door | Kind::DoorHidden
        | Kind::Trapdoor | Kind::Bed | Kind::BedTop | Kind::Gate | Kind::Lily => Some(Turned::Facedir),
        Kind::Pane if panes::is_flat(id) => Some(Turned::Facedir),
        _ => None,
    }
}

pub fn is_sign(id: u16) -> bool {
    def(id).is_some_and(|n| n.kind == Kind::Sign)
}

/// Nodes that keep metadata (meta.rs): signs, and bookshelves (what they
/// hold, as shown when pointed at).
pub fn takes_meta(id: u16) -> bool {
    is_sign(id) || shelf_of(id).is_some()
}

/// The kind of shelf a node is, if it is one.
pub fn shelf_of(id: u16) -> Option<crate::chest::Shelf> {
    match id {
        CONTENT_BOOKSHELF => Some(crate::chest::Shelf::Books),
        VESSELS_SHELF => Some(crate::chest::Shelf::Vessels),
        _ => None,
    }
}

/// Bottles and glasses (the vessels mod's nodes).
pub fn is_vessel(id: u16) -> bool {
    def(id).is_some_and(|n| n.kind == Kind::Vessel)
}

/// Nodes standing on the node below, which must be walkable, and which
/// fall off (and drop) when it goes: saplings, plants and vessels
/// (attached_node on a floor).
pub fn stands_on_floor(id: u16) -> bool {
    is_sapling(id) || is_vessel(id) || crate::farming::is_attached_plant(id) || tnt::is_powder(id)
}

pub fn is_sapling(id: u16) -> bool {
    def(id).is_some_and(|n| n.kind == Kind::Sapling)
}

pub fn is_leaves(id: u16) -> bool {
    def(id).is_some_and(|n| n.kind == Kind::Leaves)
}

/// The sapling that a kind of leaves gives, and the trunk that keeps them
/// from decaying (minetest_game's register_leafdecay: leaves stay while a
/// trunk of their own tree is within LEAF_DECAY_RADIUS).
pub fn tree_of_leaves(id: u16) -> Option<(u16, u16)> {
    match id {
        CONTENT_BIRCH_LEAVES => Some((CONTENT_BIRCH_SAPLING, CONTENT_BIRCH_TREE)),
        CONTENT_FIR_NEEDLES => Some((CONTENT_FIR_SAPLING, CONTENT_FIR_TREE)),
        CONTENT_APPLE_LEAVES => Some((CONTENT_APPLE_SAPLING, CONTENT_APPLE_TREE)),
        CONTENT_JUNGLE_LEAVES => Some((CONTENT_JUNGLE_SAPLING, CONTENT_JUNGLE_TREE)),
        CONTENT_ACACIA_LEAVES => Some((CONTENT_ACACIA_SAPLING, CONTENT_ACACIA_TREE)),
        plants::BUSH_LEAVES => Some((plants::BUSH_SAPLING, plants::BUSH_STEM)),
        plants::ACACIA_BUSH_LEAVES => Some((plants::ACACIA_BUSH_SAPLING, plants::ACACIA_BUSH_STEM)),
        plants::FIR_BUSH_NEEDLES => Some((plants::FIR_BUSH_SAPLING, plants::FIR_BUSH_STEM)),
        _ => None,
    }
}

/// The trunk that keeps a node from decaying: leaves' own tree, and apples
/// on apple trees (minetest_game registers both for leafdecay).
pub fn decay_trunk(id: u16) -> Option<u16> {
    match id {
        CONTENT_APPLE => Some(CONTENT_APPLE_TREE),
        _ => tree_of_leaves(id).map(|(_, trunk)| trunk),
    }
}

/// Whether a node is a trunk whose digging may make leaves decay.
pub fn is_trunk(id: u16) -> bool {
    matches!(id, CONTENT_BIRCH_TREE | CONTENT_FIR_TREE | CONTENT_APPLE_TREE | CONTENT_JUNGLE_TREE | CONTENT_ACACIA_TREE)
        || matches!(id, plants::BUSH_STEM | plants::ACACIA_BUSH_STEM | plants::FIR_BUSH_STEM)
}

/// The sapling leaves give now and then, and how rarely (one time in
/// so many): SAPLING_RARITY for trees, 5 for bushes (minetest_game's
/// drop tables). Blueberry bush leaves have no stem to decay without,
/// but give saplings all the same.
pub fn sapling_of_leaves(id: u16) -> Option<(u16, u32)> {
    match id {
        plants::BUSH_LEAVES | plants::ACACIA_BUSH_LEAVES | plants::FIR_BUSH_NEEDLES => tree_of_leaves(id).map(|(s, _)| (s, 5)),
        plants::BLUEBERRY_LEAVES => Some((plants::BLUEBERRY_SAPLING, 5)),
        _ => tree_of_leaves(id).map(|(s, _)| (s, SAPLING_RARITY)),
    }
}

pub fn is_apple(id: u16) -> bool {
    id == CONTENT_APPLE
}

pub fn is_mark(id: u16) -> bool {
    id == CONTENT_APPLE_MARK
}

/// minetest_game's leafdecay radius for pine and aspen.
pub const LEAF_DECAY_RADIUS: i32 = 3;

/// For a wall-mounted node, the offset to the node it hangs on, from its
/// param2: 0 ceiling, 1 floor, 2 +X, 3 -X, 4 +Z, 5 -Z (Luanti's order).
pub fn wallmounted_support(param2: u8) -> (i32, i32, i32) {
    match param2 & 7 {
        0 => (0, 1, 0),
        1 => (0, -1, 0),
        2 => (1, 0, 0),
        3 => (-1, 0, 0),
        4 => (0, 0, 1),
        _ => (0, 0, -1),
    }
}

/// The param2 the client predicts when placing a wall-mounted node
/// against `under` from `above` (Game::nodePlacement): which way the
/// support lies.
pub fn wallmounted_param2(under: [i16; 3], above: [i16; 3]) -> u8 {
    let d = [under[0] - above[0], under[1] - above[1], under[2] - above[2]];
    if d[1].abs() > d[0].abs().max(d[2].abs()) {
        if d[1] < 0 { 1 } else { 0 }
    } else if d[0].abs() > d[2].abs() {
        if d[0] < 0 { 3 } else { 2 }
    } else if d[2] < 0 {
        5
    } else {
        4
    }
}

/// Content id for a node name, including the built-in "air".
pub fn node_id_of(name: &str) -> Option<u16> {
    if name == "air" {
        return Some(crate::world::CONTENT_AIR);
    }
    NODES.iter().find(|n| n.name == name).map(|n| n.id)
}

/// Id of any item (node or craftitem) by name.
pub fn id_of(name: &str) -> Option<u16> {
    node_id_of(name)
        .or_else(|| CRAFTITEMS.iter().find(|c| c.name == name).map(|c| c.id))
        .or_else(|| TOOLS.iter().find(|t| t.name == name).map(|t| t.id))
}

pub fn name_of(id: u16) -> &'static str {
    match (def(id), craftitem(id), tool(id)) {
        (Some(n), _, _) => n.name,
        (_, Some(c), _) => c.name,
        (_, _, Some(t)) => t.name,
        _ => "air",
    }
}

pub fn description_of(id: u16) -> &'static str {
    match (def(id), craftitem(id), tool(id)) {
        (Some(n), _, _) => n.description,
        (_, Some(c), _) => c.description,
        (_, _, Some(t)) => t.description,
        _ => "",
    }
}

/// Whether an id is an item a player can hold: any of our nodes,
/// craftitems or tools.
pub fn is_item(id: u16) -> bool {
    def(id).is_some() || craftitem(id).is_some() || tool(id).is_some()
}

/// How many of an item fit in one inventory slot: 99 (Luanti's default),
/// or 1 for tools.
pub fn stack_max(id: u16) -> u16 {
    // A full bucket doesn't stack (minetest_game's stack_max = 1).
    // So doesn't a bed.
    // Nor does a book with text.
    if is_tool(id) || matches!(id, ITEM_BUCKET_WATER | ITEM_BUCKET_LAVA | ITEM_BOOK_WRITTEN | ITEM_BINOCULARS | ITEM_MAPPING_KIT) || beds::is_bed(id) { 1 } else { 99 }
}

/// Whether an item points at liquids, as buckets do: then the client
/// points at water rather than through it.
fn liquids_pointable(id: u16) -> bool {
    matches!(id, ITEM_BUCKET_EMPTY | ITEM_BUCKET_WATER | ITEM_BUCKET_LAVA)
}

/// Whether an item can be placed as a node.
pub fn is_node(id: u16) -> bool {
    def(id).is_some()
}

/// An item's rating in a group (0: not in it), as for "group:wood" in
/// recipes.
pub fn group_of(id: u16, group: &str) -> i16 {
    let groups = match (def(id), craftitem(id), tool(id)) {
        (Some(n), _, _) => n.groups,
        (_, Some(c), _) => c.groups,
        (_, _, Some(t)) => t.groups,
        _ => &[],
    };
    groups.iter().find(|(g, _)| *g == group).map_or(0, |(_, r)| *r)
}

/// The item digging a node gives, if any, and how many.
pub fn drop_of(id: u16) -> Option<(u16, u16)> {
    def(id).filter(|n| n.drop_count > 0).map(|n| (n.drop, n.drop_count))
}

/// Default textures compiled into the firmware. Files in the SD card's media
/// folder with the same name replace them.
const EMBEDDED_MEDIA: &[(&str, &[u8])] = &[
    ("esp_stone.png", include_bytes!("../media/esp_stone.png")),
    ("esp_dirt.png", include_bytes!("../media/esp_dirt.png")),
    ("esp_grass.png", include_bytes!("../media/esp_grass.png")),
    ("esp_grass_side.png", include_bytes!("../media/esp_grass_side.png")),
    ("esp_hand.png", include_bytes!("../media/esp_hand.png")),
    ("esp_sand.png", include_bytes!("../media/esp_sand.png")),
    // Torch models and textures from minetest_game's default mod (CC BY-SA
    // 3.0, see media/LICENSE-torch.txt).
    ("esp_torch_on_floor.png", include_bytes!("../media/esp_torch_on_floor.png")),
    ("esp_torch_animated.png", include_bytes!("../media/esp_torch_animated.png")),
    ("esp_torch_floor.obj", include_bytes!("../media/esp_torch_floor.obj")),
    ("esp_torch_wall.obj", include_bytes!("../media/esp_torch_wall.obj")),
    ("esp_torch_ceiling.obj", include_bytes!("../media/esp_torch_ceiling.obj")),
    ("esp_sandstone.png", include_bytes!("../media/esp_sandstone.png")),
    ("esp_snow.png", include_bytes!("../media/esp_snow.png")),
    ("esp_snow_side.png", include_bytes!("../media/esp_snow_side.png")),
    ("esp_stone_with_coal.png", include_bytes!("../media/esp_stone_with_coal.png")),
    ("esp_stone_with_iron.png", include_bytes!("../media/esp_stone_with_iron.png")),
    // Player model and skin from Minetest Game's player_api (CC BY-SA 3.0,
    // see media/LICENSE-character.txt).
    ("character.b3d", include_bytes!("../media/character.b3d")),
    ("character.png", include_bytes!("../media/character.png")),
    ("esp_birch_tree.png", include_bytes!("../media/esp_birch_tree.png")),
    ("esp_birch_tree_top.png", include_bytes!("../media/esp_birch_tree_top.png")),
    ("esp_birch_leaves.png", include_bytes!("../media/esp_birch_leaves.png")),
    ("esp_fir_tree.png", include_bytes!("../media/esp_fir_tree.png")),
    ("esp_fir_tree_top.png", include_bytes!("../media/esp_fir_tree_top.png")),
    ("esp_fir_needles.png", include_bytes!("../media/esp_fir_needles.png")),
    // Sounds from minetest_game's default mod (see media/LICENSE-sounds.txt),
    // under their original names: the client asks for some by name
    // ("default_dig_<group>" while digging, "player_damage").
    ("default_dig_choppy.1.ogg", include_bytes!("../media/default_dig_choppy.1.ogg")),
    ("default_dig_choppy.2.ogg", include_bytes!("../media/default_dig_choppy.2.ogg")),
    ("default_dig_choppy.3.ogg", include_bytes!("../media/default_dig_choppy.3.ogg")),
    ("default_dig_cracky.1.ogg", include_bytes!("../media/default_dig_cracky.1.ogg")),
    ("default_dig_cracky.2.ogg", include_bytes!("../media/default_dig_cracky.2.ogg")),
    ("default_dig_cracky.3.ogg", include_bytes!("../media/default_dig_cracky.3.ogg")),
    ("default_dig_crumbly.ogg", include_bytes!("../media/default_dig_crumbly.ogg")),
    ("default_dig_dig_immediate.ogg", include_bytes!("../media/default_dig_dig_immediate.ogg")),
    ("default_dig_oddly_breakable_by_hand.ogg", include_bytes!("../media/default_dig_oddly_breakable_by_hand.ogg")),
    ("default_dig_snappy.ogg", include_bytes!("../media/default_dig_snappy.ogg")),
    ("default_dirt_footstep.1.ogg", include_bytes!("../media/default_dirt_footstep.1.ogg")),
    ("default_dirt_footstep.2.ogg", include_bytes!("../media/default_dirt_footstep.2.ogg")),
    ("default_dug_node.1.ogg", include_bytes!("../media/default_dug_node.1.ogg")),
    ("default_dug_node.2.ogg", include_bytes!("../media/default_dug_node.2.ogg")),
    ("default_grass_footstep.1.ogg", include_bytes!("../media/default_grass_footstep.1.ogg")),
    ("default_grass_footstep.2.ogg", include_bytes!("../media/default_grass_footstep.2.ogg")),
    ("default_grass_footstep.3.ogg", include_bytes!("../media/default_grass_footstep.3.ogg")),
    ("default_hard_footstep.1.ogg", include_bytes!("../media/default_hard_footstep.1.ogg")),
    ("default_hard_footstep.2.ogg", include_bytes!("../media/default_hard_footstep.2.ogg")),
    ("default_hard_footstep.3.ogg", include_bytes!("../media/default_hard_footstep.3.ogg")),
    ("default_place_node.1.ogg", include_bytes!("../media/default_place_node.1.ogg")),
    ("default_place_node.2.ogg", include_bytes!("../media/default_place_node.2.ogg")),
    ("default_place_node.3.ogg", include_bytes!("../media/default_place_node.3.ogg")),
    ("default_place_node_hard.1.ogg", include_bytes!("../media/default_place_node_hard.1.ogg")),
    ("default_place_node_hard.2.ogg", include_bytes!("../media/default_place_node_hard.2.ogg")),
    ("default_sand_footstep.1.ogg", include_bytes!("../media/default_sand_footstep.1.ogg")),
    ("default_sand_footstep.2.ogg", include_bytes!("../media/default_sand_footstep.2.ogg")),
    ("default_sand_footstep.3.ogg", include_bytes!("../media/default_sand_footstep.3.ogg")),
    ("default_snow_footstep.1.ogg", include_bytes!("../media/default_snow_footstep.1.ogg")),
    ("default_snow_footstep.2.ogg", include_bytes!("../media/default_snow_footstep.2.ogg")),
    ("default_snow_footstep.3.ogg", include_bytes!("../media/default_snow_footstep.3.ogg")),
    ("default_snow_footstep.4.ogg", include_bytes!("../media/default_snow_footstep.4.ogg")),
    ("default_snow_footstep.5.ogg", include_bytes!("../media/default_snow_footstep.5.ogg")),
    ("default_wood_footstep.1.ogg", include_bytes!("../media/default_wood_footstep.1.ogg")),
    ("default_wood_footstep.2.ogg", include_bytes!("../media/default_wood_footstep.2.ogg")),
    ("player_damage.ogg", include_bytes!("../media/player_damage.ogg")),
    // Extra skins to choose from in the inventory (see skins.rs).
    ("esp_skin_wizard.png", include_bytes!("../media/esp_skin_wizard.png")),
    ("esp_skin_elf.png", include_bytes!("../media/esp_skin_elf.png")),
    ("esp_skin_borgo.png", include_bytes!("../media/esp_skin_borgo.png")),
    ("esp_skin_strider.png", include_bytes!("../media/esp_skin_strider.png")),
    ("esp_birch_wood.png", include_bytes!("../media/esp_birch_wood.png")),
    ("esp_fir_wood.png", include_bytes!("../media/esp_fir_wood.png")),
    ("esp_cobble.png", include_bytes!("../media/esp_cobble.png")),
    // Item pictures and the crafting arrow from minetest_game's default mod
    // (CC BY-SA 3.0, see media/LICENSE-items.txt), under their own names.
    ("default_stick.png", include_bytes!("../media/default_stick.png")),
    ("default_coal_lump.png", include_bytes!("../media/default_coal_lump.png")),
    ("default_iron_lump.png", include_bytes!("../media/default_iron_lump.png")),
    ("gui_furnace_arrow_bg.png", include_bytes!("../media/gui_furnace_arrow_bg.png")),
    ("default_tool_woodpick.png", include_bytes!("../media/default_tool_woodpick.png")),
    ("default_tool_stonepick.png", include_bytes!("../media/default_tool_stonepick.png")),
    ("default_tool_woodshovel.png", include_bytes!("../media/default_tool_woodshovel.png")),
    ("default_tool_stoneshovel.png", include_bytes!("../media/default_tool_stoneshovel.png")),
    ("default_tool_woodaxe.png", include_bytes!("../media/default_tool_woodaxe.png")),
    ("default_tool_stoneaxe.png", include_bytes!("../media/default_tool_stoneaxe.png")),
    ("default_tool_woodsword.png", include_bytes!("../media/default_tool_woodsword.png")),
    ("default_tool_stonesword.png", include_bytes!("../media/default_tool_stonesword.png")),
    ("default_furnace_top.png", include_bytes!("../media/default_furnace_top.png")),
    ("default_furnace_bottom.png", include_bytes!("../media/default_furnace_bottom.png")),
    ("default_furnace_side.png", include_bytes!("../media/default_furnace_side.png")),
    ("default_furnace_front.png", include_bytes!("../media/default_furnace_front.png")),
    ("default_furnace_fire_bg.png", include_bytes!("../media/default_furnace_fire_bg.png")),
    ("default_furnace_front_active.png", include_bytes!("../media/default_furnace_front_active.png")),
    ("default_furnace_fire_fg.png", include_bytes!("../media/default_furnace_fire_fg.png")),
    ("gui_furnace_arrow_fg.png", include_bytes!("../media/gui_furnace_arrow_fg.png")),
    ("default_steel_ingot.png", include_bytes!("../media/default_steel_ingot.png")),
    ("default_tool_steelpick.png", include_bytes!("../media/default_tool_steelpick.png")),
    ("default_tool_steelshovel.png", include_bytes!("../media/default_tool_steelshovel.png")),
    ("default_tool_steelaxe.png", include_bytes!("../media/default_tool_steelaxe.png")),
    ("default_tool_steelsword.png", include_bytes!("../media/default_tool_steelsword.png")),
    ("default_furnace_active.ogg", include_bytes!("../media/default_furnace_active.ogg")),
    ("default_cool_lava.1.ogg", include_bytes!("../media/default_cool_lava.1.ogg")),
    ("default_cool_lava.2.ogg", include_bytes!("../media/default_cool_lava.2.ogg")),
    ("default_cool_lava.3.ogg", include_bytes!("../media/default_cool_lava.3.ogg")),
    ("esp_stone_brick.png", include_bytes!("../media/esp_stone_brick.png")),
    ("esp_stone_block.png", include_bytes!("../media/esp_stone_block.png")),
    ("esp_sandstone_brick.png", include_bytes!("../media/esp_sandstone_brick.png")),
    ("esp_sandstone_block.png", include_bytes!("../media/esp_sandstone_block.png")),
    ("default_coal_block.png", include_bytes!("../media/default_coal_block.png")),
    ("default_steel_block.png", include_bytes!("../media/default_steel_block.png")),
    ("default_glass.png", include_bytes!("../media/default_glass.png")),
    ("default_glass_detail.png", include_bytes!("../media/default_glass_detail.png")),
    ("default_ladder_wood.png", include_bytes!("../media/default_ladder_wood.png")),
    ("default_ladder_steel.png", include_bytes!("../media/default_ladder_steel.png")),
    ("default_glass_footstep.ogg", include_bytes!("../media/default_glass_footstep.ogg")),
    ("default_break_glass.1.ogg", include_bytes!("../media/default_break_glass.1.ogg")),
    ("default_break_glass.2.ogg", include_bytes!("../media/default_break_glass.2.ogg")),
    ("default_break_glass.3.ogg", include_bytes!("../media/default_break_glass.3.ogg")),
    ("default_metal_footstep.1.ogg", include_bytes!("../media/default_metal_footstep.1.ogg")),
    ("default_metal_footstep.2.ogg", include_bytes!("../media/default_metal_footstep.2.ogg")),
    ("default_metal_footstep.3.ogg", include_bytes!("../media/default_metal_footstep.3.ogg")),
    ("default_dig_metal.ogg", include_bytes!("../media/default_dig_metal.ogg")),
    ("default_dug_metal.1.ogg", include_bytes!("../media/default_dug_metal.1.ogg")),
    ("default_dug_metal.2.ogg", include_bytes!("../media/default_dug_metal.2.ogg")),
    ("default_place_node_metal.1.ogg", include_bytes!("../media/default_place_node_metal.1.ogg")),
    ("default_place_node_metal.2.ogg", include_bytes!("../media/default_place_node_metal.2.ogg")),
    ("chest_open.obj", include_bytes!("../media/chest_open.obj")),
    ("default_chest_top.png", include_bytes!("../media/default_chest_top.png")),
    ("default_chest_side.png", include_bytes!("../media/default_chest_side.png")),
    ("default_chest_front.png", include_bytes!("../media/default_chest_front.png")),
    ("default_chest_inside.png", include_bytes!("../media/default_chest_inside.png")),
    ("default_chest_lock.png", include_bytes!("../media/default_chest_lock.png")),
    ("default_chest_open.ogg", include_bytes!("../media/default_chest_open.ogg")),
    ("default_chest_close.ogg", include_bytes!("../media/default_chest_close.ogg")),
    ("esp_apple_tree.png", include_bytes!("../media/esp_apple_tree.png")),
    ("esp_apple_tree_top.png", include_bytes!("../media/esp_apple_tree_top.png")),
    ("esp_apple_leaves.png", include_bytes!("../media/esp_apple_leaves.png")),
    ("esp_apple_wood.png", include_bytes!("../media/esp_apple_wood.png")),
    ("default_apple.png", include_bytes!("../media/default_apple.png")),
    ("default_sapling.png", include_bytes!("../media/default_sapling.png")),
    ("default_aspen_sapling.png", include_bytes!("../media/default_aspen_sapling.png")),
    ("default_pine_sapling.png", include_bytes!("../media/default_pine_sapling.png")),
    ("default_tool_breaks.1.ogg", include_bytes!("../media/default_tool_breaks.1.ogg")),
    ("default_tool_breaks.2.ogg", include_bytes!("../media/default_tool_breaks.2.ogg")),
    ("default_tool_breaks.3.ogg", include_bytes!("../media/default_tool_breaks.3.ogg")),
    // minetest_game's doors (see media/LICENSE-doors.txt).
    ("door_a.b3d", include_bytes!("../media/door_a.b3d")),
    ("door_b.b3d", include_bytes!("../media/door_b.b3d")),
    ("doors_door_wood.png", include_bytes!("../media/doors_door_wood.png")),
    ("doors_door_steel.png", include_bytes!("../media/doors_door_steel.png")),
    ("doors_door_glass.png", include_bytes!("../media/doors_door_glass.png")),
    ("doors_item_wood.png", include_bytes!("../media/doors_item_wood.png")),
    ("doors_item_steel.png", include_bytes!("../media/doors_item_steel.png")),
    ("doors_item_glass.png", include_bytes!("../media/doors_item_glass.png")),
    ("doors_door_open.ogg", include_bytes!("../media/doors_door_open.ogg")),
    ("doors_fencegate_open.ogg", include_bytes!("../media/doors_fencegate_open.ogg")),
    ("doors_fencegate_close.ogg", include_bytes!("../media/doors_fencegate_close.ogg")),
    ("doors_fencegate_closed.obj", include_bytes!("../media/doors_fencegate_closed.obj")),
    ("doors_fencegate_open.obj", include_bytes!("../media/doors_fencegate_open.obj")),
    ("default_fence_overlay.png", include_bytes!("../media/default_fence_overlay.png")),
    ("default_fence_rail_overlay.png", include_bytes!("../media/default_fence_rail_overlay.png")),
    ("doors_door_close.ogg", include_bytes!("../media/doors_door_close.ogg")),
    ("doors_steel_door_open.ogg", include_bytes!("../media/doors_steel_door_open.ogg")),
    ("doors_steel_door_close.ogg", include_bytes!("../media/doors_steel_door_close.ogg")),
    ("doors_glass_door_open.ogg", include_bytes!("../media/doors_glass_door_open.ogg")),
    ("doors_glass_door_close.ogg", include_bytes!("../media/doors_glass_door_close.ogg")),
    ("doors_trapdoor.png", include_bytes!("../media/doors_trapdoor.png")),
    ("doors_trapdoor_side.png", include_bytes!("../media/doors_trapdoor_side.png")),
    ("doors_trapdoor_steel.png", include_bytes!("../media/doors_trapdoor_steel.png")),
    ("doors_trapdoor_steel_side.png", include_bytes!("../media/doors_trapdoor_steel_side.png")),
    // minetest_game's water and buckets (see media/LICENSE-items.txt).
    ("default_water_source_animated.png", include_bytes!("../media/default_water_source_animated.png")),
    ("default_water_flowing_animated.png", include_bytes!("../media/default_water_flowing_animated.png")),
    ("default_water.png", include_bytes!("../media/default_water.png")),
    ("default_water_footstep.1.ogg", include_bytes!("../media/default_water_footstep.1.ogg")),
    ("default_water_footstep.2.ogg", include_bytes!("../media/default_water_footstep.2.ogg")),
    ("default_water_footstep.3.ogg", include_bytes!("../media/default_water_footstep.3.ogg")),
    ("bucket.png", include_bytes!("../media/bucket.png")),
    ("bucket_water.png", include_bytes!("../media/bucket_water.png")),
    // minetest_game's farming, tall grass and wool (see media/LICENSE-farming.txt).
    ("farming_soil.png", include_bytes!("../media/farming_soil.png")),
    ("farming_soil_wet.png", include_bytes!("../media/farming_soil_wet.png")),
    ("farming_soil_wet_side.png", include_bytes!("../media/farming_soil_wet_side.png")),
    ("farming_wheat_seed.png", include_bytes!("../media/farming_wheat_seed.png")),
    ("farming_cotton_seed.png", include_bytes!("../media/farming_cotton_seed.png")),
    ("farming_wheat.png", include_bytes!("../media/farming_wheat.png")),
    ("farming_flour.png", include_bytes!("../media/farming_flour.png")),
    ("farming_bread.png", include_bytes!("../media/farming_bread.png")),
    ("farming_cotton.png", include_bytes!("../media/farming_cotton.png")),
    ("farming_string.png", include_bytes!("../media/farming_string.png")),
    ("farming_straw.png", include_bytes!("../media/farming_straw.png")),
    ("farming_cotton_wild.png", include_bytes!("../media/farming_cotton_wild.png")),
    ("farming_tool_woodhoe.png", include_bytes!("../media/farming_tool_woodhoe.png")),
    ("farming_tool_stonehoe.png", include_bytes!("../media/farming_tool_stonehoe.png")),
    ("farming_tool_steelhoe.png", include_bytes!("../media/farming_tool_steelhoe.png")),
    ("farming_wheat_1.png", include_bytes!("../media/farming_wheat_1.png")),
    ("farming_wheat_2.png", include_bytes!("../media/farming_wheat_2.png")),
    ("farming_wheat_3.png", include_bytes!("../media/farming_wheat_3.png")),
    ("farming_wheat_4.png", include_bytes!("../media/farming_wheat_4.png")),
    ("farming_wheat_5.png", include_bytes!("../media/farming_wheat_5.png")),
    ("farming_wheat_6.png", include_bytes!("../media/farming_wheat_6.png")),
    ("farming_wheat_7.png", include_bytes!("../media/farming_wheat_7.png")),
    ("farming_wheat_8.png", include_bytes!("../media/farming_wheat_8.png")),
    ("farming_cotton_1.png", include_bytes!("../media/farming_cotton_1.png")),
    ("farming_cotton_2.png", include_bytes!("../media/farming_cotton_2.png")),
    ("farming_cotton_3.png", include_bytes!("../media/farming_cotton_3.png")),
    ("farming_cotton_4.png", include_bytes!("../media/farming_cotton_4.png")),
    ("farming_cotton_5.png", include_bytes!("../media/farming_cotton_5.png")),
    ("farming_cotton_6.png", include_bytes!("../media/farming_cotton_6.png")),
    ("farming_cotton_7.png", include_bytes!("../media/farming_cotton_7.png")),
    ("farming_cotton_8.png", include_bytes!("../media/farming_cotton_8.png")),
    ("default_grass_1.png", include_bytes!("../media/default_grass_1.png")),
    ("default_jungletree.png", include_bytes!("../media/default_jungletree.png")),
    ("default_jungletree_top.png", include_bytes!("../media/default_jungletree_top.png")),
    ("default_jungleleaves.png", include_bytes!("../media/default_jungleleaves.png")),
    ("default_junglewood.png", include_bytes!("../media/default_junglewood.png")),
    ("default_junglesapling.png", include_bytes!("../media/default_junglesapling.png")),
    ("default_emergent_jungle_sapling.png", include_bytes!("../media/default_emergent_jungle_sapling.png")),
    ("default_acacia_tree.png", include_bytes!("../media/default_acacia_tree.png")),
    ("default_acacia_tree_top.png", include_bytes!("../media/default_acacia_tree_top.png")),
    ("default_acacia_leaves.png", include_bytes!("../media/default_acacia_leaves.png")),
    ("default_acacia_wood.png", include_bytes!("../media/default_acacia_wood.png")),
    ("default_acacia_sapling.png", include_bytes!("../media/default_acacia_sapling.png")),
    ("default_dry_dirt.png", include_bytes!("../media/default_dry_dirt.png")),
    ("default_dry_grass.png", include_bytes!("../media/default_dry_grass.png")),
    ("default_dry_grass_side.png", include_bytes!("../media/default_dry_grass_side.png")),
    ("default_rainforest_litter.png", include_bytes!("../media/default_rainforest_litter.png")),
    ("default_rainforest_litter_side.png", include_bytes!("../media/default_rainforest_litter_side.png")),
    ("default_dry_grass_1.png", include_bytes!("../media/default_dry_grass_1.png")),
    ("default_dry_grass_2.png", include_bytes!("../media/default_dry_grass_2.png")),
    ("default_dry_grass_3.png", include_bytes!("../media/default_dry_grass_3.png")),
    ("default_dry_grass_4.png", include_bytes!("../media/default_dry_grass_4.png")),
    ("default_dry_grass_5.png", include_bytes!("../media/default_dry_grass_5.png")),
    ("default_junglegrass.png", include_bytes!("../media/default_junglegrass.png")),
    ("default_cactus_top.png", include_bytes!("../media/default_cactus_top.png")),
    ("default_cactus_side.png", include_bytes!("../media/default_cactus_side.png")),
    ("default_papyrus.png", include_bytes!("../media/default_papyrus.png")),
    ("default_bush_stem.png", include_bytes!("../media/default_bush_stem.png")),
    ("default_leaves_simple.png", include_bytes!("../media/default_leaves_simple.png")),
    ("default_bush_sapling.png", include_bytes!("../media/default_bush_sapling.png")),
    ("default_blueberry_bush_leaves.png", include_bytes!("../media/default_blueberry_bush_leaves.png")),
    ("default_blueberry_overlay.png", include_bytes!("../media/default_blueberry_overlay.png")),
    ("default_blueberry_bush_sapling.png", include_bytes!("../media/default_blueberry_bush_sapling.png")),
    ("default_blueberries.png", include_bytes!("../media/default_blueberries.png")),
    ("default_acacia_bush_stem.png", include_bytes!("../media/default_acacia_bush_stem.png")),
    ("default_acacia_leaves_simple.png", include_bytes!("../media/default_acacia_leaves_simple.png")),
    ("default_acacia_bush_sapling.png", include_bytes!("../media/default_acacia_bush_sapling.png")),
    ("default_pine_bush_stem.png", include_bytes!("../media/default_pine_bush_stem.png")),
    ("default_pine_bush_sapling.png", include_bytes!("../media/default_pine_bush_sapling.png")),
    ("default_fern_1.png", include_bytes!("../media/default_fern_1.png")),
    ("default_fern_2.png", include_bytes!("../media/default_fern_2.png")),
    ("default_fern_3.png", include_bytes!("../media/default_fern_3.png")),
    ("default_marram_grass_1.png", include_bytes!("../media/default_marram_grass_1.png")),
    ("default_marram_grass_2.png", include_bytes!("../media/default_marram_grass_2.png")),
    ("default_marram_grass_3.png", include_bytes!("../media/default_marram_grass_3.png")),
    ("default_kelp.png", include_bytes!("../media/default_kelp.png")),
    ("default_coral_green.png", include_bytes!("../media/default_coral_green.png")),
    ("default_coral_pink.png", include_bytes!("../media/default_coral_pink.png")),
    ("default_coral_cyan.png", include_bytes!("../media/default_coral_cyan.png")),
    ("default_coral_brown.png", include_bytes!("../media/default_coral_brown.png")),
    ("default_coral_orange.png", include_bytes!("../media/default_coral_orange.png")),
    ("default_coral_skeleton.png", include_bytes!("../media/default_coral_skeleton.png")),
    ("default_large_cactus_seedling.png", include_bytes!("../media/default_large_cactus_seedling.png")),
    ("default_mineral_copper.png", include_bytes!("../media/default_mineral_copper.png")),
    ("default_meselamp.png", include_bytes!("../media/default_meselamp.png")),
    ("default_cloud.png", include_bytes!("../media/default_cloud.png")),
    ("esp_mist.png", include_bytes!("../media/esp_mist.png")),
    ("default_dry_shrub.png", include_bytes!("../media/default_dry_shrub.png")),
    ("default_sign_wood.png", include_bytes!("../media/default_sign_wood.png")),
    ("default_paper.png", include_bytes!("../media/default_paper.png")),
    ("vessels_glass_bottle.png", include_bytes!("../media/vessels_glass_bottle.png")),
    ("vessels_drinking_glass.png", include_bytes!("../media/vessels_drinking_glass.png")),
    ("vessels_drinking_glass_inv.png", include_bytes!("../media/vessels_drinking_glass_inv.png")),
    ("vessels_steel_bottle.png", include_bytes!("../media/vessels_steel_bottle.png")),
    ("vessels_glass_fragments.png", include_bytes!("../media/vessels_glass_fragments.png")),
    ("vessels_shelf.png", include_bytes!("../media/vessels_shelf.png")),
    ("vessels_shelf_slot.png", include_bytes!("../media/vessels_shelf_slot.png")),
    ("keys_key.png", include_bytes!("../media/keys_key.png")),
    ("keys_key_skeleton.png", include_bytes!("../media/keys_key_skeleton.png")),
    ("screwdriver.png", include_bytes!("../media/screwdriver.png")),
    ("binoculars_binoculars.png", include_bytes!("../media/binoculars_binoculars.png")),
    ("map_mapping_kit.png", include_bytes!("../media/map_mapping_kit.png")),
    ("default_book.png", include_bytes!("../media/default_book.png")),
    ("default_book_written.png", include_bytes!("../media/default_book_written.png")),
    ("default_bookshelf.png", include_bytes!("../media/default_bookshelf.png")),
    ("default_bookshelf_slot.png", include_bytes!("../media/default_bookshelf_slot.png")),
    ("gui_hb_bg.png", include_bytes!("../media/gui_hb_bg.png")),
    ("default_sign_steel.png", include_bytes!("../media/default_sign_steel.png")),
    ("default_sign_wall_wood.png", include_bytes!("../media/default_sign_wall_wood.png")),
    ("default_sign_wall_steel.png", include_bytes!("../media/default_sign_wall_steel.png")),
    ("default_mese_post_light_side.png", include_bytes!("../media/default_mese_post_light_side.png")),
    ("default_mese_post_light_side_dark.png", include_bytes!("../media/default_mese_post_light_side_dark.png")),
    ("flowers_rose.png", include_bytes!("../media/flowers_rose.png")),
    ("flowers_tulip.png", include_bytes!("../media/flowers_tulip.png")),
    ("flowers_dandelion_yellow.png", include_bytes!("../media/flowers_dandelion_yellow.png")),
    ("flowers_chrysanthemum_green.png", include_bytes!("../media/flowers_chrysanthemum_green.png")),
    ("flowers_geranium.png", include_bytes!("../media/flowers_geranium.png")),
    ("flowers_viola.png", include_bytes!("../media/flowers_viola.png")),
    ("flowers_dandelion_white.png", include_bytes!("../media/flowers_dandelion_white.png")),
    ("flowers_tulip_black.png", include_bytes!("../media/flowers_tulip_black.png")),
    ("flowers_mushroom_red.png", include_bytes!("../media/flowers_mushroom_red.png")),
    ("flowers_mushroom_brown.png", include_bytes!("../media/flowers_mushroom_brown.png")),
    ("flowers_waterlily.png", include_bytes!("../media/flowers_waterlily.png")),
    ("flowers_waterlily_bottom.png", include_bytes!("../media/flowers_waterlily_bottom.png")),
    ("dye_white.png", include_bytes!("../media/dye_white.png")),
    ("dye_grey.png", include_bytes!("../media/dye_grey.png")),
    ("dye_dark_grey.png", include_bytes!("../media/dye_dark_grey.png")),
    ("dye_black.png", include_bytes!("../media/dye_black.png")),
    ("dye_violet.png", include_bytes!("../media/dye_violet.png")),
    ("dye_blue.png", include_bytes!("../media/dye_blue.png")),
    ("dye_cyan.png", include_bytes!("../media/dye_cyan.png")),
    ("dye_dark_green.png", include_bytes!("../media/dye_dark_green.png")),
    ("dye_green.png", include_bytes!("../media/dye_green.png")),
    ("dye_yellow.png", include_bytes!("../media/dye_yellow.png")),
    ("dye_brown.png", include_bytes!("../media/dye_brown.png")),
    ("dye_orange.png", include_bytes!("../media/dye_orange.png")),
    ("dye_red.png", include_bytes!("../media/dye_red.png")),
    ("dye_magenta.png", include_bytes!("../media/dye_magenta.png")),
    ("dye_pink.png", include_bytes!("../media/dye_pink.png")),
    ("wool_grey.png", include_bytes!("../media/wool_grey.png")),
    ("wool_dark_grey.png", include_bytes!("../media/wool_dark_grey.png")),
    ("wool_black.png", include_bytes!("../media/wool_black.png")),
    ("wool_violet.png", include_bytes!("../media/wool_violet.png")),
    ("wool_blue.png", include_bytes!("../media/wool_blue.png")),
    ("wool_cyan.png", include_bytes!("../media/wool_cyan.png")),
    ("wool_dark_green.png", include_bytes!("../media/wool_dark_green.png")),
    ("wool_green.png", include_bytes!("../media/wool_green.png")),
    ("wool_yellow.png", include_bytes!("../media/wool_yellow.png")),
    ("wool_brown.png", include_bytes!("../media/wool_brown.png")),
    ("wool_orange.png", include_bytes!("../media/wool_orange.png")),
    ("wool_red.png", include_bytes!("../media/wool_red.png")),
    ("wool_magenta.png", include_bytes!("../media/wool_magenta.png")),
    ("wool_pink.png", include_bytes!("../media/wool_pink.png")),
    ("bones_top.png", include_bytes!("../media/bones_top.png")),
    ("bones_bottom.png", include_bytes!("../media/bones_bottom.png")),
    ("bones_side.png", include_bytes!("../media/bones_side.png")),
    ("bones_rear.png", include_bytes!("../media/bones_rear.png")),
    ("bones_front.png", include_bytes!("../media/bones_front.png")),
    ("xpanes_bar.png", include_bytes!("../media/xpanes_bar.png")),
    ("xpanes_bar_top.png", include_bytes!("../media/xpanes_bar_top.png")),
    ("xpanes_edge.png", include_bytes!("../media/xpanes_edge.png")),
    ("xpanes_edge_obsidian.png", include_bytes!("../media/xpanes_edge_obsidian.png")),
    ("xpanes_door_steel_bar.png", include_bytes!("../media/xpanes_door_steel_bar.png")),
    ("xpanes_item_steel_bar.png", include_bytes!("../media/xpanes_item_steel_bar.png")),
    ("xpanes_trapdoor_steel_bar.png", include_bytes!("../media/xpanes_trapdoor_steel_bar.png")),
    ("xpanes_trapdoor_steel_bar_side.png", include_bytes!("../media/xpanes_trapdoor_steel_bar_side.png")),
    ("xpanes_steel_bar_door_open.ogg", include_bytes!("../media/xpanes_steel_bar_door_open.ogg")),
    ("xpanes_steel_bar_door_close.ogg", include_bytes!("../media/xpanes_steel_bar_door_close.ogg")),
    ("default_gravel_dig.1.ogg", include_bytes!("../media/default_gravel_dig.1.ogg")),
    ("default_gravel_dig.2.ogg", include_bytes!("../media/default_gravel_dig.2.ogg")),
    ("default_gravel_dug.1.ogg", include_bytes!("../media/default_gravel_dug.1.ogg")),
    ("default_gravel_dug.2.ogg", include_bytes!("../media/default_gravel_dug.2.ogg")),
    ("default_gravel_dug.3.ogg", include_bytes!("../media/default_gravel_dug.3.ogg")),
    ("default_gravel_footstep.1.ogg", include_bytes!("../media/default_gravel_footstep.1.ogg")),
    ("default_gravel_footstep.2.ogg", include_bytes!("../media/default_gravel_footstep.2.ogg")),
    ("default_gravel_footstep.3.ogg", include_bytes!("../media/default_gravel_footstep.3.ogg")),
    ("default_gravel_footstep.4.ogg", include_bytes!("../media/default_gravel_footstep.4.ogg")),
    ("default_ice_dig.1.ogg", include_bytes!("../media/default_ice_dig.1.ogg")),
    ("default_ice_dig.2.ogg", include_bytes!("../media/default_ice_dig.2.ogg")),
    ("default_ice_dig.3.ogg", include_bytes!("../media/default_ice_dig.3.ogg")),
    ("default_ice_dug.ogg", include_bytes!("../media/default_ice_dug.ogg")),
    ("default_ice_footstep.1.ogg", include_bytes!("../media/default_ice_footstep.1.ogg")),
    ("default_ice_footstep.2.ogg", include_bytes!("../media/default_ice_footstep.2.ogg")),
    ("default_ice_footstep.3.ogg", include_bytes!("../media/default_ice_footstep.3.ogg")),
    ("default_gravel.png", include_bytes!("../media/default_gravel.png")),
    ("default_clay.png", include_bytes!("../media/default_clay.png")),
    ("default_brick.png", include_bytes!("../media/default_brick.png")),
    ("default_desert_sand.png", include_bytes!("../media/default_desert_sand.png")),
    ("default_desert_stone.png", include_bytes!("../media/default_desert_stone.png")),
    ("default_desert_cobble.png", include_bytes!("../media/default_desert_cobble.png")),
    ("default_mossycobble.png", include_bytes!("../media/default_mossycobble.png")),
    ("default_desert_stone_brick.png", include_bytes!("../media/default_desert_stone_brick.png")),
    ("default_desert_stone_block.png", include_bytes!("../media/default_desert_stone_block.png")),
    ("default_desert_sandstone.png", include_bytes!("../media/default_desert_sandstone.png")),
    ("default_desert_sandstone_brick.png", include_bytes!("../media/default_desert_sandstone_brick.png")),
    ("default_desert_sandstone_block.png", include_bytes!("../media/default_desert_sandstone_block.png")),
    ("default_silver_sand.png", include_bytes!("../media/default_silver_sand.png")),
    ("default_silver_sandstone.png", include_bytes!("../media/default_silver_sandstone.png")),
    ("default_silver_sandstone_brick.png", include_bytes!("../media/default_silver_sandstone_brick.png")),
    ("default_silver_sandstone_block.png", include_bytes!("../media/default_silver_sandstone_block.png")),
    ("default_snowball.png", include_bytes!("../media/default_snowball.png")),
    ("default_ice.png", include_bytes!("../media/default_ice.png")),
    ("default_permafrost.png", include_bytes!("../media/default_permafrost.png")),
    ("default_moss.png", include_bytes!("../media/default_moss.png")),
    ("default_moss_side.png", include_bytes!("../media/default_moss_side.png")),
    ("default_stones.png", include_bytes!("../media/default_stones.png")),
    ("default_stones_side.png", include_bytes!("../media/default_stones_side.png")),
    ("default_coniferous_litter.png", include_bytes!("../media/default_coniferous_litter.png")),
    ("default_coniferous_litter_side.png", include_bytes!("../media/default_coniferous_litter_side.png")),
    ("default_obsidian_brick.png", include_bytes!("../media/default_obsidian_brick.png")),
    ("default_obsidian_block.png", include_bytes!("../media/default_obsidian_block.png")),
    ("default_obsidian_glass.png", include_bytes!("../media/default_obsidian_glass.png")),
    ("default_obsidian_glass_detail.png", include_bytes!("../media/default_obsidian_glass_detail.png")),
    ("default_obsidian_shard.png", include_bytes!("../media/default_obsidian_shard.png")),
    ("default_flint.png", include_bytes!("../media/default_flint.png")),
    ("default_clay_lump.png", include_bytes!("../media/default_clay_lump.png")),
    ("default_clay_brick.png", include_bytes!("../media/default_clay_brick.png")),
    ("default_mineral_tin.png", include_bytes!("../media/default_mineral_tin.png")),
    ("default_mineral_gold.png", include_bytes!("../media/default_mineral_gold.png")),
    ("default_mineral_mese.png", include_bytes!("../media/default_mineral_mese.png")),
    ("default_mineral_diamond.png", include_bytes!("../media/default_mineral_diamond.png")),
    ("default_copper_lump.png", include_bytes!("../media/default_copper_lump.png")),
    ("default_tin_lump.png", include_bytes!("../media/default_tin_lump.png")),
    ("default_gold_lump.png", include_bytes!("../media/default_gold_lump.png")),
    ("default_mese_crystal.png", include_bytes!("../media/default_mese_crystal.png")),
    ("default_mese_crystal_fragment.png", include_bytes!("../media/default_mese_crystal_fragment.png")),
    ("default_diamond.png", include_bytes!("../media/default_diamond.png")),
    ("default_copper_ingot.png", include_bytes!("../media/default_copper_ingot.png")),
    ("default_tin_ingot.png", include_bytes!("../media/default_tin_ingot.png")),
    ("default_bronze_ingot.png", include_bytes!("../media/default_bronze_ingot.png")),
    ("default_gold_ingot.png", include_bytes!("../media/default_gold_ingot.png")),
    ("default_copper_block.png", include_bytes!("../media/default_copper_block.png")),
    ("default_tin_block.png", include_bytes!("../media/default_tin_block.png")),
    ("default_bronze_block.png", include_bytes!("../media/default_bronze_block.png")),
    ("default_gold_block.png", include_bytes!("../media/default_gold_block.png")),
    ("default_mese_block.png", include_bytes!("../media/default_mese_block.png")),
    ("default_diamond_block.png", include_bytes!("../media/default_diamond_block.png")),
    ("default_tool_bronzepick.png", include_bytes!("../media/default_tool_bronzepick.png")),
    ("default_tool_bronzeshovel.png", include_bytes!("../media/default_tool_bronzeshovel.png")),
    ("default_tool_bronzeaxe.png", include_bytes!("../media/default_tool_bronzeaxe.png")),
    ("default_tool_bronzesword.png", include_bytes!("../media/default_tool_bronzesword.png")),
    ("default_tool_mesepick.png", include_bytes!("../media/default_tool_mesepick.png")),
    ("default_tool_meseshovel.png", include_bytes!("../media/default_tool_meseshovel.png")),
    ("default_tool_meseaxe.png", include_bytes!("../media/default_tool_meseaxe.png")),
    ("default_tool_mesesword.png", include_bytes!("../media/default_tool_mesesword.png")),
    ("default_tool_diamondpick.png", include_bytes!("../media/default_tool_diamondpick.png")),
    ("default_tool_diamondshovel.png", include_bytes!("../media/default_tool_diamondshovel.png")),
    ("default_tool_diamondaxe.png", include_bytes!("../media/default_tool_diamondaxe.png")),
    ("default_tool_diamondsword.png", include_bytes!("../media/default_tool_diamondsword.png")),
    ("default_grass_2.png", include_bytes!("../media/default_grass_2.png")),
    ("default_grass_3.png", include_bytes!("../media/default_grass_3.png")),
    ("default_grass_4.png", include_bytes!("../media/default_grass_4.png")),
    ("default_grass_5.png", include_bytes!("../media/default_grass_5.png")),
    ("wool_white.png", include_bytes!("../media/wool_white.png")),
    ("default_lava.png", include_bytes!("../media/default_lava.png")),
    ("default_lava_source_animated.png", include_bytes!("../media/default_lava_source_animated.png")),
    ("default_lava_flowing_animated.png", include_bytes!("../media/default_lava_flowing_animated.png")),
    ("default_obsidian.png", include_bytes!("../media/default_obsidian.png")),
    ("bucket_lava.png", include_bytes!("../media/bucket_lava.png")),
    ("beds_bed.png", include_bytes!("../media/beds_bed.png")),
    ("beds_bed_fancy.png", include_bytes!("../media/beds_bed_fancy.png")),
    ("beds_bed_foot.png", include_bytes!("../media/beds_bed_foot.png")),
    ("beds_bed_head.png", include_bytes!("../media/beds_bed_head.png")),
    ("beds_bed_side1.png", include_bytes!("../media/beds_bed_side1.png")),
    ("beds_bed_side2.png", include_bytes!("../media/beds_bed_side2.png")),
    ("beds_bed_side_bottom.png", include_bytes!("../media/beds_bed_side_bottom.png")),
    ("beds_bed_side_bottom_r.png", include_bytes!("../media/beds_bed_side_bottom_r.png")),
    ("beds_bed_side_top.png", include_bytes!("../media/beds_bed_side_top.png")),
    ("beds_bed_side_top_r.png", include_bytes!("../media/beds_bed_side_top_r.png")),
    ("beds_bed_top1.png", include_bytes!("../media/beds_bed_top1.png")),
    ("beds_bed_top2.png", include_bytes!("../media/beds_bed_top2.png")),
    ("beds_bed_top_bottom.png", include_bytes!("../media/beds_bed_top_bottom.png")),
    ("beds_bed_top_top.png", include_bytes!("../media/beds_bed_top_top.png")),
    ("beds_bed_under.png", include_bytes!("../media/beds_bed_under.png")),
    // The crafting guide's icons (mtg_craftguide and sfinv, CC BY-SA 3.0,
    // see media/LICENSE-craftguide.txt).
    ("craftguide_clear_icon.png", include_bytes!("../media/craftguide_clear_icon.png")),
    ("craftguide_furnace.png", include_bytes!("../media/craftguide_furnace.png")),
    ("craftguide_next_icon.png", include_bytes!("../media/craftguide_next_icon.png")),
    ("craftguide_prev_icon.png", include_bytes!("../media/craftguide_prev_icon.png")),
    ("craftguide_search_icon.png", include_bytes!("../media/craftguide_search_icon.png")),
    ("craftguide_shapeless.png", include_bytes!("../media/craftguide_shapeless.png")),
    ("sfinv_crafting_arrow.png", include_bytes!("../media/sfinv_crafting_arrow.png")),
    // env_sounds' running water and lava (CC0, see
    // media/LICENSE-sounds.txt).
    // farming's desert sand soil (CC BY 3.0, see media/LICENSE-farming.txt).
    ("farming_desert_sand_soil.png", include_bytes!("../media/farming_desert_sand_soil.png")),
    ("farming_desert_sand_soil_wet.png", include_bytes!("../media/farming_desert_sand_soil_wet.png")),
    ("farming_desert_sand_soil_wet_side.png", include_bytes!("../media/farming_desert_sand_soil_wet_side.png")),
    // The fire mod's flames, flint and steel and sounds, and default's
    // smoke (CC BY-SA 3.0, CC BY 3.0 and CC0, see media/LICENSE-fire.txt
    // and media/LICENSE-sounds.txt).
    ("fire_basic_flame.png", include_bytes!("../media/fire_basic_flame.png")),
    ("fire_basic_flame_animated.png", include_bytes!("../media/fire_basic_flame_animated.png")),
    ("fire_flint_steel.png", include_bytes!("../media/fire_flint_steel.png")),
    ("fire_fire.1.ogg", include_bytes!("../media/fire_fire.1.ogg")),
    ("fire_fire.2.ogg", include_bytes!("../media/fire_fire.2.ogg")),
    ("fire_fire.3.ogg", include_bytes!("../media/fire_fire.3.ogg")),
    ("fire_extinguish_flame.1.ogg", include_bytes!("../media/fire_extinguish_flame.1.ogg")),
    ("fire_extinguish_flame.2.ogg", include_bytes!("../media/fire_extinguish_flame.2.ogg")),
    ("fire_extinguish_flame.3.ogg", include_bytes!("../media/fire_extinguish_flame.3.ogg")),
    ("fire_flint_and_steel.ogg", include_bytes!("../media/fire_flint_and_steel.ogg")),
    ("default_item_smoke.ogg", include_bytes!("../media/default_item_smoke.ogg")),
    // The tnt mod's (CC BY-SA 3.0 and CC0, see media/LICENSE-tnt.txt).
    ("tnt_bottom.png", include_bytes!("../media/tnt_bottom.png")),
    ("tnt_side.png", include_bytes!("../media/tnt_side.png")),
    ("tnt_top.png", include_bytes!("../media/tnt_top.png")),
    ("tnt_top_burning_animated.png", include_bytes!("../media/tnt_top_burning_animated.png")),
    ("tnt_tnt_stick.png", include_bytes!("../media/tnt_tnt_stick.png")),
    ("tnt_gunpowder_inventory.png", include_bytes!("../media/tnt_gunpowder_inventory.png")),
    ("tnt_gunpowder_straight.png", include_bytes!("../media/tnt_gunpowder_straight.png")),
    ("tnt_gunpowder_curved.png", include_bytes!("../media/tnt_gunpowder_curved.png")),
    ("tnt_gunpowder_t_junction.png", include_bytes!("../media/tnt_gunpowder_t_junction.png")),
    ("tnt_gunpowder_crossing.png", include_bytes!("../media/tnt_gunpowder_crossing.png")),
    ("tnt_gunpowder_burning_straight_animated.png", include_bytes!("../media/tnt_gunpowder_burning_straight_animated.png")),
    ("tnt_gunpowder_burning_curved_animated.png", include_bytes!("../media/tnt_gunpowder_burning_curved_animated.png")),
    ("tnt_gunpowder_burning_t_junction_animated.png", include_bytes!("../media/tnt_gunpowder_burning_t_junction_animated.png")),
    ("tnt_gunpowder_burning_crossing_animated.png", include_bytes!("../media/tnt_gunpowder_burning_crossing_animated.png")),
    ("tnt_explode.ogg", include_bytes!("../media/tnt_explode.ogg")),
    ("tnt_ignite.ogg", include_bytes!("../media/tnt_ignite.ogg")),
    ("tnt_gunpowder_burning.ogg", include_bytes!("../media/tnt_gunpowder_burning.ogg")),
    ("env_sounds_water.1.ogg", include_bytes!("../media/env_sounds_water.1.ogg")),
    ("env_sounds_water.2.ogg", include_bytes!("../media/env_sounds_water.2.ogg")),
    ("env_sounds_water.3.ogg", include_bytes!("../media/env_sounds_water.3.ogg")),
    ("env_sounds_water.4.ogg", include_bytes!("../media/env_sounds_water.4.ogg")),
    ("env_sounds_lava.1.ogg", include_bytes!("../media/env_sounds_lava.1.ogg")),
    ("env_sounds_lava.2.ogg", include_bytes!("../media/env_sounds_lava.2.ogg")),
    // Mobs (mobs.rs).
    ("mobs_mossling.png", include_bytes!("../media/mobs_mossling.png")),
    ("mobs_mossling_chatter.1.ogg", include_bytes!("../media/mobs_mossling_chatter.1.ogg")),
    ("mobs_mossling_chatter.2.ogg", include_bytes!("../media/mobs_mossling_chatter.2.ogg")),
    ("mobs_mossling_chatter.3.ogg", include_bytes!("../media/mobs_mossling_chatter.3.ogg")),
    ("mobs_mossling_call.ogg", include_bytes!("../media/mobs_mossling_call.ogg")),
    ("mobs_mossling_swipe.ogg", include_bytes!("../media/mobs_mossling_swipe.ogg")),
    ("mobs_mossling_hurt.ogg", include_bytes!("../media/mobs_mossling_hurt.ogg")),
    ("mobs_mossling_death.ogg", include_bytes!("../media/mobs_mossling_death.ogg")),
    ("mobs_wisp.png", include_bytes!("../media/mobs_wisp.png")),
    ("mobs_wisp_hum.ogg", include_bytes!("../media/mobs_wisp_hum.ogg")),
    ("mobs_wisp_whine.ogg", include_bytes!("../media/mobs_wisp_whine.ogg")),
    ("mobs_wisp_fizzle.ogg", include_bytes!("../media/mobs_wisp_fizzle.ogg")),
];

// ---------------------------------------------------------------------------
// Item definitions (TOCLIENT_ITEMDEF)
// ---------------------------------------------------------------------------

const ITEM_NONE: u8 = 0;
const ITEM_NODE: u8 = 1;
const ITEM_CRAFT: u8 = 2;
const ITEM_TOOL: u8 = 3;

struct Item<'a> {
    item_type: u8,
    name: &'a str,
    description: &'a str,
    inventory_image: &'a str,
    wield_image: &'a str,
    wield_scale: [f32; 3],
    place_prediction: &'a str,
    /// Serialised ToolCapabilities, or empty. The client aborts if the hand
    /// ("" item) has none.
    tool_capabilities: Vec<u8>,
    /// Played when the item is placed (a block's "place" sound).
    place_sound: Sound,
    stack_max: u16,
    groups: &'a [(&'a str, i16)],
    /// Left-click uses it (an apple is eaten) instead of digging.
    usable: bool,
    liquids_pointable: bool,
}

/// Serialised ToolCapabilities (version 5, protocol >= 38).
fn tool_capabilities(caps: &ToolCaps) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(5)
        .f32(caps.full_punch_interval)
        .i16(caps.max_drop_level)
        .u32(caps.groupcaps.len() as u32);
    for cap in caps.groupcaps {
        w.string16(cap.group).i16(cap.uses as i16).i16(cap.maxlevel).u32(cap.times.len() as u32);
        for &(rating, time) in cap.times {
            w.i16(rating).f32(time);
        }
    }
    w.u32(1).string16("fleshy").i16(caps.fleshy); // damage groups
    w.u16(0); // punch_attack_uses
    w.into_inner()
}

/// ItemImageDef for protocol >= 51: image name, then a TileAnimation (none).
fn item_image(w: &mut Writer, name: &str) {
    w.string16(name).u8(0);
}

/// SimpleSoundSpec: name, gain, pitch, fade.
fn no_sound(w: &mut Writer) {
    sound(w, ("", 1.0));
}

fn sound(w: &mut Writer, (name, gain): (&str, f32)) {
    w.string16(name).f32(gain).f32(1.0).f32(0.0);
}

/// A sound: file name (without number and ".ogg": the client picks one of
/// "name.1.ogg", "name.2.ogg" ... at random) and volume.
type Sound = (&'static str, f32);

/// The sounds of a block, as minetest_game's default.node_sound_*_defaults
/// give them.
struct Sounds {
    footstep: Sound,
    /// While digging. "__group" makes the client play "default_dig_<group>"
    /// for the group the tool digs it by.
    dig: Sound,
    dug: Sound,
    place: Sound,
}

const GROUP_DIG: Sound = ("__group", 1.0);

fn sounds_of(id: u16) -> Sounds {
    // Stairs and slabs sound like their block.
    if let Some((_, m)) = stairs::shape_of(id) {
        return sounds_of(m.base);
    }
    let stone = Sounds {
        footstep: ("default_hard_footstep", 0.2),
        dig: GROUP_DIG,
        dug: ("default_hard_footstep", 1.0),
        place: ("default_place_node_hard", 1.0),
    };
    let dirt = |footstep: Sound| Sounds {
        footstep,
        dig: ("default_dig_crumbly", 0.4),
        dug: ("default_dirt_footstep", 1.0),
        place: ("default_place_node", 1.0),
    };
    let wood = Sounds {
        footstep: ("default_wood_footstep", 0.15),
        dig: ("default_dig_choppy", 0.4),
        dug: ("default_wood_footstep", 1.0),
        place: ("default_place_node_hard", 1.0),
    };
    match id {
        CONTENT_DIRT => dirt(("default_dirt_footstep", 0.25)),
        CONTENT_GRASS => dirt(("default_grass_footstep", 0.25)),
        CONTENT_SNOW => dirt(("default_snow_footstep", 0.2)),
        // minetest_game's node_sound_snow_defaults, _gravel_ and _ice_.
        CONTENT_SNOW_LAYER | CONTENT_SNOW_BLOCK => Sounds {
            footstep: ("default_snow_footstep", 0.2),
            dig: ("default_snow_footstep", 0.3),
            dug: ("default_snow_footstep", 0.3),
            place: ("default_place_node", 1.0),
        },
        CONTENT_GRAVEL => Sounds {
            footstep: ("default_gravel_footstep", 0.25),
            dig: ("default_gravel_dig", 0.35),
            dug: ("default_gravel_dug", 1.0),
            place: ("default_place_node", 1.0),
        },
        CONTENT_ICE | CONTENT_CAVE_ICE => Sounds {
            footstep: ("default_ice_footstep", 0.15),
            dig: ("default_ice_dig", 0.5),
            dug: ("default_ice_dug", 0.5),
            place: ("default_place_node_hard", 1.0),
        },
        CONTENT_CLAY | CONTENT_PERMAFROST | CONTENT_PERMAFROST_STONES => dirt(("default_dirt_footstep", 0.25)),
        CONTENT_PERMAFROST_MOSS | CONTENT_CONIFEROUS_LITTER => dirt(("default_grass_footstep", 0.25)),
        CONTENT_DESERT_SAND | CONTENT_SILVER_SAND => sounds_of(CONTENT_SAND),
        // minetest_game's savanna grass and rainforest litter: grass steps.
        CONTENT_DRY_GRASS | CONTENT_LITTER => dirt(("default_grass_footstep", 0.4)),
        CONTENT_DRY_DIRT => dirt(("default_dirt_footstep", 0.25)),
        CONTENT_SAND => Sounds {
            footstep: ("default_sand_footstep", 0.05),
            dig: GROUP_DIG,
            dug: ("default_sand_footstep", 0.15),
            place: ("default_place_node", 1.0),
        },
        farming::SOIL | farming::SOIL_WET | farming::DRY_SOIL | farming::DRY_SOIL_WET => dirt(("default_dirt_footstep", 0.25)),
        farming::DESERT_SAND_SOIL | farming::DESERT_SAND_SOIL_WET => sounds_of(CONTENT_SAND),
        // farming's seeds: no digging sound, a soft rustle when dug.
        farming::SEED_WHEAT | farming::SEED_COTTON => Sounds {
            footstep: ("default_dirt_footstep", 0.25),
            dig: ("", 0.0),
            dug: ("default_grass_footstep", 0.2),
            place: ("default_place_node", 0.25),
        },
        id if id == farming::WOOL_WHITE || group_of(id, "wool") > 0 => Sounds {
            footstep: ("", 1.0),
            dig: ("default_dig_dig_immediate", 0.5),
            dug: ("default_dug_node", 0.25),
            place: ("default_place_node_hard", 1.0),
        },
        id if farming::is_attached_plant(id) || id == farming::STRAW => sounds_of(CONTENT_BIRCH_LEAVES),
        CONTENT_BIRCH_LEAVES | CONTENT_FIR_NEEDLES | CONTENT_BIRCH_SAPLING | CONTENT_FIR_SAPLING | CONTENT_APPLE_LEAVES
        | CONTENT_APPLE | CONTENT_APPLE_SAPLING | CONTENT_JUNGLE_LEAVES | CONTENT_JUNGLE_SAPLING | EMERGENT_JUNGLE_SAPLING | CONTENT_ACACIA_LEAVES
        | CONTENT_ACACIA_SAPLING => Sounds {
            footstep: ("default_grass_footstep", 0.45),
            dig: GROUP_DIG,
            dug: ("default_grass_footstep", 0.7),
            place: ("default_place_node", 1.0),
        },
        CONTENT_BIRCH_TREE | CONTENT_FIR_TREE | CONTENT_BIRCH_WOOD | CONTENT_FIR_WOOD | CONTENT_APPLE_TREE | CONTENT_APPLE_WOOD
        | CONTENT_JUNGLE_TREE | CONTENT_JUNGLE_WOOD | CONTENT_ACACIA_TREE | CONTENT_ACACIA_WOOD => wood,
        id if is_chest(id) => wood,
        CONTENT_BOOKSHELF | VESSELS_SHELF => wood,
        // vessels: glass, and the steel bottle node_sound_defaults.
        VESSELS_GLASS_BOTTLE | VESSELS_DRINKING_GLASS => sounds_of(CONTENT_GLASS),
        VESSELS_STEEL_BOTTLE => Sounds {
            footstep: ("", 1.0),
            dig: GROUP_DIG,
            dug: ("default_dug_node", 0.25),
            place: ("default_place_node_hard", 1.0),
        },
        id if def(id).is_some_and(|n| n.kind == Kind::Post) => wood,
        SIGN_WOOD => wood,
        SIGN_STEEL => sounds_of(CONTENT_STEEL_BLOCK),
        CONTENT_LADDER_WOOD => wood,
        // minetest_game's node_sound_glass_defaults and _metal_defaults.
        CONTENT_GLASS | CONTENT_MESE_LAMP | CONTENT_OBSIDIAN_GLASS => Sounds {
            footstep: ("default_glass_footstep", 0.3),
            dig: ("default_glass_footstep", 0.5),
            dug: ("default_break_glass", 1.0),
            place: ("default_place_node_hard", 1.0),
        },
        id if doors::door_of(id).is_some_and(|(d, _)| d.item == ITEM_DOOR_WOOD) => wood,
        id if doors::trapdoor_of(id).is_some_and(|(t, _)| !t.locked) => wood,
        id if doors::trapdoor_of(id).is_some() => sounds_of(CONTENT_STEEL_BLOCK),
        id if doors::door_of(id).is_some_and(|(d, _)| d.item == ITEM_DOOR_GLASS) => sounds_of(CONTENT_GLASS),
        id if doors::door_of(id).is_some_and(|(d, _)| d.item == ITEM_DOOR_STEEL || d.item == ITEM_DOOR_STEEL_BAR) => sounds_of(CONTENT_STEEL_BLOCK),
        // Glass panes sound like glass; steel bars like metal.
        panes::BAR_FLAT | panes::BAR => sounds_of(CONTENT_STEEL_BLOCK),
        // minetest_game's node_sound_gravel_defaults.
        bones::BONES => sounds_of(CONTENT_GRAVEL),
        id if panes::is_pane(id) => sounds_of(CONTENT_GLASS),
        // minetest_game's node_sound_water_defaults.
        // Lava: no sounds of its own in minetest_game.
        CONTENT_LAVA | CONTENT_LAVA_FLOWING => Sounds {
            footstep: ("", 1.0),
            dig: ("", 1.0),
            dug: ("", 1.0),
            place: ("default_place_node", 1.0),
        },
        CONTENT_WATER | CONTENT_WATER_FLOWING => Sounds {
            footstep: ("default_water_footstep", 0.2),
            dig: ("default_dig_dig_immediate", 0.5),
            dug: ("default_dug_node", 0.25),
            place: ("default_place_node_hard", 1.0),
        },
        CONTENT_STEEL_BLOCK | CONTENT_LADDER_STEEL | CONTENT_COPPER_BLOCK | CONTENT_TIN_BLOCK | CONTENT_BRONZE_BLOCK | CONTENT_GOLD_BLOCK => Sounds {
            footstep: ("default_metal_footstep", 0.2),
            dig: ("default_dig_metal", 0.5),
            dug: ("default_dug_metal", 0.5),
            place: ("default_place_node_metal", 0.5),
        },
        id if is_torch(id) => wood,
        id if beds::is_bed(id) => wood,
        // Cactus and bush stems sound like wood; kelp like sand, corals like
        // stone, but dug like plants (plants.rs).
        plants::CACTUS | plants::LARGE_CACTUS_SEEDLING | plants::BUSH_STEM | plants::ACACIA_BUSH_STEM | plants::FIR_BUSH_STEM => wood,
        colours::WATERLILY | plants::PAPYRUS | plants::BUSH_LEAVES | plants::BUSH_SAPLING | plants::BLUEBERRY_LEAVES_WITH_BERRIES | plants::BLUEBERRY_LEAVES
        | plants::BLUEBERRY_SAPLING | plants::ACACIA_BUSH_LEAVES | plants::ACACIA_BUSH_SAPLING | plants::FIR_BUSH_NEEDLES
        | plants::FIR_BUSH_SAPLING => sounds_of(CONTENT_BIRCH_LEAVES),
        plants::KELP => Sounds {
            dig: ("default_dig_snappy", 0.2),
            dug: ("default_grass_footstep", 0.25),
            ..sounds_of(CONTENT_SAND)
        },
        plants::CORAL_GREEN | plants::CORAL_PINK | plants::CORAL_CYAN => Sounds {
            dig: ("default_dig_snappy", 0.2),
            dug: ("default_grass_footstep", 0.25),
            ..stone
        },
        id if fences::burn_time(id) > 0.0 || fences::is_gate(id) => wood,
        // Gunpowder sounds like leaves, TNT like wood.
        tnt::GUNPOWDER | tnt::GUNPOWDER_BURNING => sounds_of(CONTENT_BIRCH_LEAVES),
        tnt::TNT | tnt::TNT_BURNING => wood,
        // Flames: none, but the group dig sound.
        id if fire::is_flame(id) => Sounds { footstep: ("", 1.0), dig: GROUP_DIG, dug: ("", 1.0), place: ("", 1.0) },
        // stone, sandstone, ores
        _ => stone,
    }
}

/// A trapdoor item's picture (minetest_game's inventory_image).
fn trapdoor_image(id: u16) -> &'static str {
    match doors::trapdoor_of(id) {
        Some((t, _)) if t.closed == panes::BAR_TRAPDOOR => TRAPDOOR_BAR_CLOSED[0],
        Some((t, _)) if t.locked => TRAPDOOR_STEEL_CLOSED[0],
        _ => TRAPDOOR_WOOD_CLOSED[0],
    }
}

/// A layer of snow: a quarter of a node.
const SNOW_LAYER_BOX: [f32; 6] = [-0.5, -0.5, -0.5, 0.5, -0.25, 0.5];

/// The plant growing out of kelp or a coral.
fn rooted_plant(id: u16) -> &'static str {
    match id {
        plants::CORAL_GREEN => "default_coral_green.png",
        plants::CORAL_PINK => "default_coral_pink.png",
        plants::CORAL_CYAN => "default_coral_cyan.png",
        _ => "default_kelp.png",
    }
}

/// A fence or rail item's picture (default.register_fence's and
/// register_fence_rail's inventory_image: the planks cut out by the
/// overlay's pink).
fn fence_image(id: u16) -> &'static str {
    match id {
        fences::RAIL_APPLE => "default_fence_rail_overlay.png^esp_apple_wood.png^default_fence_rail_overlay.png^[makealpha:255,126,126",
        fences::RAIL_FIR => "default_fence_rail_overlay.png^esp_fir_wood.png^default_fence_rail_overlay.png^[makealpha:255,126,126",
        fences::RAIL_BIRCH => "default_fence_rail_overlay.png^esp_birch_wood.png^default_fence_rail_overlay.png^[makealpha:255,126,126",
        fences::FENCE_FIR => "default_fence_overlay.png^esp_fir_wood.png^default_fence_overlay.png^[makealpha:255,126,126",
        fences::FENCE_BIRCH => "default_fence_overlay.png^esp_birch_wood.png^default_fence_overlay.png^[makealpha:255,126,126",
        fences::FENCE_JUNGLE => "default_fence_overlay.png^default_junglewood.png^default_fence_overlay.png^[makealpha:255,126,126",
        fences::FENCE_ACACIA => "default_fence_overlay.png^default_acacia_wood.png^default_fence_overlay.png^[makealpha:255,126,126",
        fences::RAIL_JUNGLE => "default_fence_rail_overlay.png^default_junglewood.png^default_fence_rail_overlay.png^[makealpha:255,126,126",
        fences::RAIL_ACACIA => "default_fence_rail_overlay.png^default_acacia_wood.png^default_fence_rail_overlay.png^[makealpha:255,126,126",
        _ => "default_fence_overlay.png^esp_apple_wood.png^default_fence_overlay.png^[makealpha:255,126,126",
    }
}

/// A bed item's picture (beds.lua's inventory_image).
fn bed_image(id: u16) -> &'static str {
    if id == beds::FANCY_BOTTOM { "beds_bed_fancy.png" } else { "beds_bed.png" }
}

/// The sound of placing a node, and its gain.
pub fn place_sound(id: u16) -> (&'static str, f32) {
    sounds_of(id).place
}

fn serialize_item(item: &Item) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(6) // version
        .u8(item.item_type)
        .string16(item.name)
        .string16(item.description);
    item_image(&mut w, item.inventory_image); // empty: node items get a 3D cube
    item_image(&mut w, item.wield_image);
    for v in item.wield_scale {
        w.f32(v);
    }
    w.i16(item.stack_max as i16)
        .u8(item.usable as u8)
        .u8(item.liquids_pointable as u8)
        .bytes16(&item.tool_capabilities)
        .u16(item.groups.len() as u16);
    for (group, rating) in item.groups {
        w.string16(group).i16(*rating);
    }
    w.string16(item.place_prediction);
    sound(&mut w, item.place_sound); // sound_place
    no_sound(&mut w); // sound_place_failed
    w.f32(4.0) // range
        .string16("") // palette image
        .u32(0xFFFF_FFFF); // colour (ARGB)
    item_image(&mut w, ""); // inventory overlay
    item_image(&mut w, ""); // wield overlay
    // >= 5.4: short description
    w.string16("");
    // >= 5.7: use sounds
    no_sound(&mut w);
    no_sound(&mut w);
    // >= 5.8: optional place_param2
    w.u8(0);
    // Later optional sections are omitted; the client uses defaults.
    w.into_inner()
}

/// The serialised ItemDefManager, before compression.
pub fn itemdef_bytes() -> Vec<u8> {
    let mut items = vec![Item {
        item_type: ITEM_NONE,
        name: "", // the hand
        description: "",
        inventory_image: "",
        wield_image: "esp_hand.png",
        wield_scale: [1.0, 1.0, 2.5],
        place_prediction: "",
        tool_capabilities: tool_capabilities(&HAND),
        place_sound: ("", 1.0),
        stack_max: 99,
        groups: &[],
        usable: false,
        liquids_pointable: false,
    }];
    for n in NODES {
        items.push(Item {
            item_type: ITEM_NODE,
            name: n.name,
            description: n.description,
            // A torch or sapling is a flat picture, not a cube.
            inventory_image: match n.kind {
                Kind::Torch => TORCH_IMAGE,
                // Tall grass shows a taller tuft than the one it places.
                _ if n.id == farming::GRASS[0] => "default_grass_3.png",
                _ if n.id == farming::DRY_GRASS[0] => "default_dry_grass_3.png",
                Kind::Sapling | Kind::Apple | Kind::Ladder | Kind::Plant | Kind::Seed => n.tiles[0],
                _ if n.id == VESSELS_DRINKING_GLASS => "vessels_drinking_glass_inv.png",
                Kind::Vessel => n.tiles[0],
                Kind::Trapdoor => trapdoor_image(n.id),
                Kind::Bed => bed_image(n.id),
                Kind::Fence => fence_image(n.id),
                Kind::Stalk | Kind::Stem => n.tiles[0],
                Kind::Rooted => rooted_plant(n.id),
                Kind::SnowLayer => "default_snowball.png",
                Kind::Lily => "flowers_waterlily.png",
                // A pane's item is its picture (xpanes' inventory_image).
                Kind::Pane => n.tiles[4],
                Kind::Sign => sign_image(n.id),
                Kind::Fire => fire::FLAME_IMAGE,
                Kind::Powder => tnt::POWDER_IMAGE,
                _ => "",
            },
            wield_image: match n.kind {
                Kind::Torch => TORCH_IMAGE,
                _ if n.id == farming::GRASS[0] => "default_grass_3.png",
                _ if n.id == farming::DRY_GRASS[0] => "default_dry_grass_3.png",
                Kind::Sapling | Kind::Apple | Kind::Ladder | Kind::Plant | Kind::Seed => n.tiles[0],
                _ if n.id == VESSELS_DRINKING_GLASS => "vessels_drinking_glass_inv.png",
                Kind::Vessel => n.tiles[0],
                Kind::Trapdoor => trapdoor_image(n.id),
                Kind::Bed => bed_image(n.id),
                Kind::Fence => fence_image(n.id),
                Kind::Stalk | Kind::Stem => n.tiles[0],
                Kind::Rooted => rooted_plant(n.id),
                Kind::SnowLayer => "default_snowball.png",
                Kind::Lily => "flowers_waterlily.png",
                Kind::Pane => n.tiles[4],
                Kind::Sign => sign_image(n.id),
                Kind::Fire => fire::FLAME_IMAGE,
                Kind::Powder => tnt::POWDER_IMAGE,
                _ => "",
            },
            wield_scale: [1.0, 1.0, 1.0],
            // A bed is placed as two nodes, or not at all (its on_place):
            // the client doesn't guess.
            // Nor are kelp and corals, which go into the node pointed at.
            place_prediction: if matches!(n.kind, Kind::Bed | Kind::Rooted | Kind::Lily) { "" } else { n.name },
            tool_capabilities: Vec::new(),
            place_sound: sounds_of(n.id).place,
            stack_max: stack_max(n.id),
            groups: n.groups,
            // Mushrooms are eaten.
            usable: n.kind == Kind::Apple || colours::is_mushroom(n.id),
            // A waterlily is put on water, pointed at.
            liquids_pointable: n.kind == Kind::Lily,
        });
    }
    for c in CRAFTITEMS {
        items.push(Item {
            item_type: ITEM_CRAFT,
            name: c.name,
            description: c.description,
            inventory_image: c.image,
            wield_image: c.image,
            wield_scale: [1.0, 1.0, 1.0],
            place_prediction: "",
            tool_capabilities: Vec::new(),
            place_sound: ("", 1.0),
            stack_max: stack_max(c.id),
            groups: c.groups,
            // The empty bucket is used (left-click) to scoop water; bread
            // is eaten.
            // (and books, which open to be written in or read)
            // (and the skeleton key, used on a locked chest or door; and
            // binoculars and the mapping kit, which update the player)
            usable: matches!(c.id, ITEM_BUCKET_EMPTY | ITEM_BREAD | ITEM_BLUEBERRIES | ITEM_BOOK | ITEM_BOOK_WRITTEN | ITEM_SKELETON_KEY | ITEM_BINOCULARS | ITEM_MAPPING_KIT),
            liquids_pointable: liquids_pointable(c.id),
        });
    }
    for t in TOOLS {
        items.push(Item {
            item_type: ITEM_TOOL,
            name: t.name,
            description: t.description,
            inventory_image: t.image,
            wield_image: t.wield_image,
            wield_scale: [1.0, 1.0, 1.0],
            place_prediction: "",
            tool_capabilities: tool_capabilities(&t.caps),
            place_sound: ("", 1.0),
            stack_max: stack_max(t.id),
            groups: t.groups,
            // Hoes are used (left-click) on dirt.
            usable: farming::hoe_uses(t.id).is_some() || t.id == TOOL_SCREWDRIVER || t.id == TOOL_FLINT_AND_STEEL,
            liquids_pointable: false,
        });
    }

    let mut w = Writer::plain();
    w.u8(0).u16(items.len() as u16);
    for item in &items {
        w.bytes16(&serialize_item(item));
    }
    w.u16(0); // aliases
    w.into_inner()
}

// ---------------------------------------------------------------------------
// Node definitions (TOCLIENT_NODEDEF)
// ---------------------------------------------------------------------------

const CONTENTFEATURES_VERSION: u8 = 13;
const TILE_FLAG_BACKFACE_CULLING: u16 = 1 << 0;
const NDT_NORMAL: u8 = 0;
/// Leaves: see-through, or solid if the player turns off "fancy leaves".
const NDT_ALLFACES_OPTIONAL: u8 = 6;
const POINTABLE: u8 = 1;
const ALPHAMODE_OPAQUE: u8 = 2;
const ALPHAMODE_CLIP: u8 = 1;
const NDT_MESH: u8 = 16;
const NDT_FIRELIKE: u8 = 14;
const NDT_RAILLIKE: u8 = 11;
const NDT_PLANTLIKE_ROOTED: u8 = 17;
const NDT_GLASSLIKE: u8 = 4;
const CPT2_LEVELED: u8 = 5;
const NDT_PLANTLIKE: u8 = 9;
const NDT_AIRLIKE: u8 = 1;
const NDT_SIGNLIKE: u8 = 8;
const NDT_GLASSLIKE_FRAMED_OPTIONAL: u8 = 15;
const NODEBOX_FIXED: u8 = 1;
const NDT_NODEBOX: u8 = 12;
const NDT_LIQUID: u8 = 2;
const NDT_FLOWINGLIQUID: u8 = 3;
const CPT2_FLOWINGLIQUID: u8 = 2;
const CPT2_MESHOPTIONS: u8 = 7;
const ALPHAMODE_BLEND: u8 = 0;
const LIQUID_FLOWING: u8 = 1;
const LIQUID_SOURCE: u8 = 2;
/// minetest_game's water colour under the surface: ARGB.
const WATER_POST_EFFECT: u32 = 0x671E_3C5A;
/// And inside lava: {a = 191, r = 255, g = 64, b = 0}.
const LAVA_POST_EFFECT: u32 = 0xBFFF_4000;
const TILE_FLAG_HAS_ALIGN_STYLE: u16 = 1 << 5;
const ALIGN_STYLE_WORLD: u8 = 1;
const TILE_FLAG_TILEABLE_HORIZONTAL: u16 = 1 << 1;
const TILE_FLAG_TILEABLE_VERTICAL: u16 = 1 << 2;
const TAT_VERTICAL_FRAMES: u8 = 1;
const CPT_LIGHT: u8 = 1;
const CPT2_FACEDIR: u8 = 3;
const CPT2_WALLMOUNTED: u8 = 4;
const NODEBOX_WALLMOUNTED: u8 = 2;
const NODEBOX_REGULAR: u8 = 0;
const LEVELED_MAX: u8 = 0x7F;

/// TileDef version 6: name, animation (none), flags.
fn tile(w: &mut Writer, name: &str, flags: u16) {
    w.u8(6).string16(name).u8(0).u16(flags);
}

/// How long a cycle of an animated texture takes (square frames stacked
/// vertically), or None for a still one.
fn animation_length(tile: &str) -> Option<f32> {
    match tile {
        TORCH_TILE => Some(TORCH_FRAMES_LENGTH),
        FURNACE_ACTIVE_TILE => Some(1.5),
        WATER_SOURCE_TILE => Some(2.0),
        WATER_FLOWING_TILE => Some(0.5),
        LAVA_SOURCE_TILE => Some(3.0),
        LAVA_FLOWING_TILE => Some(3.3),
        fire::FLAME_TILE => Some(1.0),
        t if tnt::is_animated(t) => Some(1.0),
        _ => None,
    }
}

/// A TileDef with an animation of square frames stacked vertically, the
/// whole cycle taking `length` seconds.
fn animated_tile(w: &mut Writer, name: &str, flags: u16, length: f32) {
    w.u8(6).string16(name);
    w.u8(TAT_VERTICAL_FRAMES).u16(16).u16(16).f32(length);
    w.u16(flags);
}

/// A selection box for wall-mounted nodes: (top, bottom, side) boxes in
/// BS units (10 per node), the side one against the -X wall. These are
/// minetest_game's torch boxes (each of its three torch nodes sets the one
/// it uses).
fn torch_selection_box(w: &mut Writer) {
    w.u8(6).u8(NODEBOX_WALLMOUNTED);
    let boxes: [[f32; 6]; 3] = [
        [-1.0 / 8.0, -1.0 / 16.0, -5.0 / 16.0, 1.0 / 8.0, 0.5, 1.0 / 8.0], // ceiling
        [-1.0 / 8.0, -0.5, -1.0 / 8.0, 1.0 / 8.0, 2.0 / 16.0, 1.0 / 8.0], // floor
        [-0.5, -0.5, -1.0 / 8.0, -1.0 / 8.0, 1.0 / 8.0, 1.0 / 8.0],     // wall
    ];
    for b in boxes {
        for v in b {
            w.f32(v * 10.0);
        }
    }
}

/// minetest_game's aspen sapling box: a thin upright box, in BS units.
fn sapling_selection_box(w: &mut Writer) {
    w.u8(6).u8(NODEBOX_FIXED).u16(1);
    for v in [-3.0 / 16.0, -0.5, -3.0 / 16.0, 3.0 / 16.0, 0.5, 3.0 / 16.0] {
        w.f32(v * 10.0);
    }
}

/// minetest_game's open chest box (the body, without the lid), in BS units.
fn open_chest_selection_box(w: &mut Writer) {
    w.u8(6).u8(NODEBOX_FIXED).u16(1);
    for v in [-0.5, -0.5, -0.5, 0.5, 3.0 / 16.0, 0.5] {
        w.f32(v * 10.0);
    }
}

/// A ladder's box: Luanti's default wallmounted boxes, a sixteenth of a
/// node thick against the wall (top, bottom, side), in BS units.
fn ladder_selection_box(w: &mut Writer) {
    w.u8(6).u8(NODEBOX_WALLMOUNTED);
    let t = 1.0 / 16.0;
    let boxes: [[f32; 6]; 3] = [
        [-0.5, 0.5 - t, -0.5, 0.5, 0.5, 0.5],
        [-0.5, -0.5, -0.5, 0.5, -0.5 + t, 0.5],
        [-0.5, -0.5, -0.5, -0.5 + t, 0.5, 0.5],
    ];
    for b in boxes {
        for v in b {
            w.f32(v * 10.0);
        }
    }
}

/// minetest_game's open chest model.
const CHEST_OPEN_MESH: &str = "chest_open.obj";

/// The selection boxes of farming's nodes (nodes.lua, api.lua): low
/// tufts of grass, taller wild cotton, and crops and seeds flat on the
/// ground.
fn farming_box(id: u16) -> [f32; 6] {
    if colours::is_flower(id) || colours::is_mushroom(id) {
        colours::selection_box(id)
    } else if plants::is_small_plant(id) {
        [-6.0 / 16.0, -0.5, -6.0 / 16.0, 6.0 / 16.0, -0.25, 6.0 / 16.0]
    } else if farming::is_grass(id) || farming::is_dry_grass(id) {
        [-6.0 / 16.0, -0.5, -6.0 / 16.0, 6.0 / 16.0, -5.0 / 16.0, 6.0 / 16.0]
    } else if id == farming::JUNGLEGRASS {
        [-6.0 / 16.0, -0.5, -6.0 / 16.0, 6.0 / 16.0, 0.5, 6.0 / 16.0]
    } else if id == DRY_SHRUB {
        [-6.0 / 16.0, -0.5, -6.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0]
    } else if id == farming::COTTON_WILD {
        [-6.0 / 16.0, -0.5, -6.0 / 16.0, 6.0 / 16.0, 5.0 / 16.0, 6.0 / 16.0]
    } else {
        [-0.5, -0.5, -0.5, 0.5, -5.0 / 16.0, 0.5]
    }
}

/// A door's selection and collision box: two nodes tall, a thin slab at the
/// front, turned by facedir.
const DOOR_BOX: [f32; 6] = [-0.5, -0.5, -0.5, 0.5, 1.5, -6.0 / 16.0];

/// minetest_game's apple box, in BS units.
fn apple_selection_box(w: &mut Writer) {
    w.u8(6).u8(NODEBOX_FIXED).u16(1);
    for v in [-3.0 / 16.0, -7.0 / 16.0, -3.0 / 16.0, 3.0 / 16.0, 4.0 / 16.0, 3.0 / 16.0] {
        w.f32(v * 10.0);
    }
}

/// A sign's picture in the inventory and the hand.
fn sign_image(id: u16) -> &'static str {
    if id == SIGN_STEEL { "default_sign_steel.png" } else { "default_sign_wood.png" }
}

/// A wallmounted nodebox: its box on the ceiling, on the floor, and on
/// the -X wall.
fn wallmounted_nodebox(w: &mut Writer, boxes: &[[f32; 6]; 3]) {
    w.u8(6).u8(NODEBOX_WALLMOUNTED);
    for b in boxes {
        for v in b {
            w.f32(v * 10.0); // in BS units, as all node boxes
        }
    }
}

fn regular_nodebox(w: &mut Writer) {
    w.u8(6).u8(NODEBOX_REGULAR);
}

/// A fixed node box made of `boxes` (in nodes), in BS units.
fn fixed_nodebox(w: &mut Writer, boxes: &[[f32; 6]]) {
    w.u8(6).u8(NODEBOX_FIXED).u16(boxes.len() as u16);
    for b in boxes {
        for v in b {
            w.f32(v * 10.0);
        }
    }
}

const NODEBOX_CONNECTED: u8 = 4;

/// A "connected" node box (in nodes), in BS units: the fixed boxes, then
/// those for joining top, bottom, front, left, back and right, then the
/// six "disconnected" side lists, "disconnected" and "disconnected_sides"
/// (NodeBox::serialize), which fences leave empty.
fn connected_nodebox(w: &mut Writer, c: &fences::Connected) {
    w.u8(6).u8(NODEBOX_CONNECTED);
    let none: &[[f32; 6]] = &[];
    for list in [c.fixed, none, none, c.front, c.left, c.back, c.right, none, none, none, none, none, none, none, none] {
        w.u16(list.len() as u16);
        for b in list {
            for v in b {
                w.f32(v * 10.0);
            }
        }
    }
}

/// The nodes a fence or rail joins (its connects_to, as content ids,
/// sorted): those in `groups`.
fn fence_joins(groups: &[&str]) -> Vec<u16> {
    NODES.iter().filter(|n| groups.iter().any(|g| n.groups.iter().any(|(ng, _)| ng == g))).map(|n| n.id).collect()
}

fn serialize_node(n: &Node) -> Vec<u8> {
    let mut w = Writer::plain();
    w.u8(CONTENTFEATURES_VERSION).string16(n.name);
    w.u16(n.groups.len() as u16);
    for (group, level) in n.groups {
        w.string16(group).i16(*level);
    }
    let torch = n.kind == Kind::Torch;
    let leaves = n.kind == Kind::Leaves;
    // Saplings and apples: plants, drawn as two crossed pictures.
    let sapling = matches!(n.kind, Kind::Sapling | Kind::Apple | Kind::Vessel);
    let mark = n.kind == Kind::Mark;
    let chest = matches!(n.kind, Kind::Chest | Kind::ChestOpen);
    let glass = n.kind == Kind::Glass;
    let ladder = n.kind == Kind::Ladder;
    let shaped = stairs::shape_of(n.id);
    let door = doors::door_of(n.id).map(|(d, _)| d);
    let hidden = n.kind == Kind::DoorHidden;
    let trapdoor = doors::trapdoor_of(n.id);
    let liquid = n.kind == Kind::Liquid;
    let flowing = matches!(n.id, CONTENT_WATER_FLOWING | CONTENT_LAVA_FLOWING);
    let lava = is_lava(n.id);
    let plant = n.kind == Kind::Plant;
    let seed = n.kind == Kind::Seed;
    // Wheat grows as a "#" of pictures (meshoptions; param2 says which shape).
    let wheat = farming::WHEAT_STAGES.contains(&n.id) || n.id == DRY_SHRUB;
    let bed = beds::is_bed(n.id);
    let wall = n.kind == Kind::Wall;
    // Walls are drawn and joined as fences are, with their own boxes.
    let fence = n.kind == Kind::Fence || wall;
    let gate = fences::gate_of(n.id);
    let fence_box = if wall {
        &fences::WALL_BOX
    } else if fences::is_rail(n.id) {
        &fences::RAIL_BOX
    } else {
        &fences::FENCE_BOX
    };
    // Papyrus and bush stems: plants, the stems solid; kelp and corals:
    // blocks with a plant on top.
    let stalk = matches!(n.kind, Kind::Stalk | Kind::Stem);
    let rooted = n.kind == Kind::Rooted;
    let pane = n.kind == Kind::Pane;
    let flat_pane = panes::is_flat(n.id);
    if torch || n.kind == Kind::Sign {
        w.u8(CPT_LIGHT).u8(CPT2_WALLMOUNTED);
    } else if leaves {
        w.u8(CPT_LIGHT).u8(0);
    } else if matches!(n.kind, Kind::Furnace | Kind::Bones | Kind::Shelf) {
        w.u8(0).u8(CPT2_FACEDIR);
    } else if ladder {
        w.u8(CPT_LIGHT).u8(CPT2_WALLMOUNTED);
    } else if glass {
        w.u8(CPT_LIGHT).u8(0);
    } else if matches!(n.kind, Kind::Chest | Kind::ChestOpen) {
        // paramtype "light", as minetest_game's chests.
        w.u8(CPT_LIGHT).u8(CPT2_FACEDIR);
    } else if sapling || mark {
        w.u8(CPT_LIGHT).u8(0);
    } else if liquid {
        w.u8(CPT_LIGHT).u8(if flowing { CPT2_FLOWINGLIQUID } else { 0 });
    } else if plant {
        w.u8(CPT_LIGHT).u8(if wheat { CPT2_MESHOPTIONS } else { 0 });
    } else if seed {
        w.u8(CPT_LIGHT).u8(CPT2_WALLMOUNTED);
    } else if fence || stalk || matches!(n.kind, Kind::Lamp | Kind::SnowLayer | Kind::Post | Kind::Mist | Kind::Fire | Kind::Powder) {
        w.u8(CPT_LIGHT).u8(0);
    } else if n.kind == Kind::Lily || flat_pane {
        // Turned any way (facedir), as minetest_game places it.
        w.u8(CPT_LIGHT).u8(CPT2_FACEDIR);
    } else if pane {
        w.u8(CPT_LIGHT).u8(0);
    } else if rooted {
        // Kelp's param2 is its height, in sixteenths of a node (leveled).
        w.u8(CPT_LIGHT).u8(if n.id == plants::KELP { CPT2_LEVELED } else { 0 });
    } else if shaped.is_some() || door.is_some() || hidden || trapdoor.is_some() || bed || gate.is_some() {
        w.u8(CPT_LIGHT).u8(CPT2_FACEDIR);
    } else {
        w.u8(0) // param_type: none
            .u8(0); // param_type_2: none
    }

    // visual
    let (drawtype, mesh) = match n.kind {
        Kind::Torch => (NDT_MESH, torch_mesh(n.id)),
        Kind::Leaves => (NDT_ALLFACES_OPTIONAL, ""),
        Kind::Solid | Kind::Furnace | Kind::Chest | Kind::Bones | Kind::Shelf => (NDT_NORMAL, ""),
        Kind::ChestOpen => (NDT_MESH, CHEST_OPEN_MESH),
        Kind::Glass => (NDT_GLASSLIKE_FRAMED_OPTIONAL, ""),
        Kind::Ladder => (NDT_SIGNLIKE, ""),
        Kind::Sapling | Kind::Apple | Kind::Vessel => (NDT_PLANTLIKE, ""),
        Kind::Mark => (NDT_AIRLIKE, ""),
        Kind::Shaped => (NDT_NODEBOX, ""),
        Kind::Door => (NDT_MESH, doors::mesh_of(n.id)),
        Kind::DoorHidden => (NDT_AIRLIKE, ""),
        Kind::Trapdoor => (NDT_NODEBOX, ""),
        Kind::Plant => (NDT_PLANTLIKE, ""),
        Kind::Seed => (NDT_SIGNLIKE, ""),
        Kind::Bed | Kind::BedTop => (NDT_NODEBOX, ""),
        Kind::Fence | Kind::Wall => (NDT_NODEBOX, ""),
        Kind::Gate => (NDT_MESH, fences::gate_mesh(n.id)),
        Kind::Stalk | Kind::Stem => (NDT_PLANTLIKE, ""),
        Kind::Rooted => (NDT_PLANTLIKE_ROOTED, ""),
        Kind::Lamp | Kind::Mist => (NDT_GLASSLIKE, ""),
        Kind::Fire => (NDT_FIRELIKE, ""),
        Kind::Powder => (NDT_RAILLIKE, ""),
        Kind::SnowLayer => (NDT_NODEBOX, ""),
        Kind::Lily | Kind::Pane | Kind::Post | Kind::Sign => (NDT_NODEBOX, ""),
        Kind::Liquid if flowing => (NDT_FLOWINGLIQUID, ""),
        Kind::Liquid => (NDT_LIQUID, ""),
    };
    // Jungle grass is drawn larger than its node (minetest_game's 1.69).
    // So are bush stems (1.41).
    let scale = if n.id == farming::JUNGLEGRASS {
        1.69
    } else if n.kind == Kind::Stem {
        1.41
    } else {
        1.0
    };
    w.u8(drawtype).string16(mesh).f32(scale); // drawtype, mesh, visual_scale
    // Mesh tiles: no backface culling (the flame is single planes), as
    // the Lua API defaults them.
    // Plant pictures are seen from both sides (Luanti turns culling off
    // for plantlike by default, as for meshes).
    let flags = if torch {
        TILE_FLAG_TILEABLE_HORIZONTAL | TILE_FLAG_TILEABLE_VERTICAL
    } else if let Some(d) = door {
        // Wooden and steel doors cull their back faces, as minetest_game
        // asks; the glass door, left to the mesh default, doesn't.
        let culling = if d.item == ITEM_DOOR_GLASS { 0 } else { TILE_FLAG_BACKFACE_CULLING };
        TILE_FLAG_TILEABLE_HORIZONTAL | TILE_FLAG_TILEABLE_VERTICAL | culling
    } else if gate.is_some() {
        TILE_FLAG_TILEABLE_HORIZONTAL | TILE_FLAG_TILEABLE_VERTICAL | TILE_FLAG_BACKFACE_CULLING
    } else if sapling || ladder || plant || seed || stalk {
        0
    } else {
        TILE_FLAG_BACKFACE_CULLING
    };
    w.u8(6);
    for (i, t) in n.tiles.iter().copied().enumerate() {
        // minetest_game's water: the first tile drawn from both sides, the
        // rest from the front.
        let flags = if liquid { if i == 0 { 0 } else { TILE_FLAG_BACKFACE_CULLING } } else { flags };
        match animation_length(t) {
            Some(length) => animated_tile(&mut w, t, flags, length),
            // Stairs of stone and the like continue their block's texture
            // across the world (minetest_game's worldaligntex).
            None if shaped.is_some_and(|(_, m)| m.world_aligned) => {
                w.u8(6).string16(t).u8(0).u16(flags | TILE_FLAG_HAS_ALIGN_STYLE).u8(ALIGN_STYLE_WORLD);
            }
            None => tile(&mut w, t, flags),
        }
    }
    for _ in 0..6 {
        tile(&mut w, "", flags); // overlay tiles
    }
    w.u8(6);
    for i in 0..6 {
        // Flowing water's two surfaces (top, then sides), which the client
        // draws flowing liquids with.
        match i {
            0 if flowing && lava => animated_tile(&mut w, LAVA_FLOWING_TILE, 0, 3.3),
            1 if flowing && lava => animated_tile(&mut w, LAVA_FLOWING_TILE, TILE_FLAG_BACKFACE_CULLING, 3.3),
            0 if flowing => animated_tile(&mut w, WATER_FLOWING_TILE, 0, 0.5),
            // The plant growing out of kelp and corals, seen from both sides.
            0 if rooted => tile(&mut w, rooted_plant(n.id), TILE_FLAG_TILEABLE_VERTICAL),
            1 if flowing => animated_tile(&mut w, WATER_FLOWING_TILE, TILE_FLAG_BACKFACE_CULLING, 0.5),
            _ => tile(&mut w, "", flags), // special tiles
        }
    }
    w.u8(255) // legacy alpha: opaque
        .u8(255)
        .u8(255)
        .u8(255) // colour
        .string16("") // palette
        .u8(if liquid && !lava { 3 } else if plant || rooted { 1 } else { 0 }) // waving: water ripples, plants sway
        // A gate or a flat pane is joined only on its left and right.
        .u8(if gate.is_some() || flat_pane { fences::GATE_SIDES } else { 0 }); // connect_sides
    // connects_to: what a fence or a connected pane joins.
    let joins = if wall {
        fence_joins(&fences::WALL_JOINS)
    } else if fence {
        fence_joins(if fences::is_rail(n.id) { &fences::RAIL_JOINS } else { &fences::JOINS })
    } else if pane && !flat_pane {
        fence_joins(&panes::JOINS)
    } else {
        Vec::new()
    };
    w.u16(joins.len() as u16);
    for id in joins {
        w.u16(id);
    }
    w
        .u32(if lava {
            LAVA_POST_EFFECT
        } else if liquid {
            WATER_POST_EFFECT
        } else if n.kind == Kind::Mist {
            MIST_POST_EFFECT
        } else {
            0
        }) // post_effect_color (ARGB)
        .u8(0); // leveled

    // lighting
    let doorway = door.is_some() || hidden;
    w.u8((torch || leaves || sapling || mark || chest || glass || ladder || shaped.is_some() || doorway || trapdoor.is_some() || liquid || plant || seed || bed || fence || gate.is_some() || stalk || matches!(n.kind, Kind::Lamp | Kind::SnowLayer | Kind::Lily | Kind::Pane | Kind::Post | Kind::Sign | Kind::Mist | Kind::Fire | Kind::Powder)) as u8) // light_propagates
        .u8(passes_sunlight(n.id) as u8) // sunlight_propagates
        .u8(glow(n.id));

    let tree = n.groups.iter().any(|(g, _)| *g == "tree");
    w.u8((n.kind == Kind::Solid && !tree) as u8); // is_ground_content

    // interaction
    // The hidden door node is walkable, as minetest_game's (falling nodes
    // stop on it), though its collision box is tiny.
    w.u8((walkable(n.id) || hidden) as u8) // walkable
        .u8(if mark || hidden || liquid || n.kind == Kind::Mist { 0 } else { POINTABLE })
        .u8(!(mark || hidden || liquid || n.kind == Kind::Mist) as u8) // diggable
        .u8(ladder as u8) // climbable
        .u8((mark || liquid || plant || matches!(n.kind, Kind::Mist | Kind::Fire)) as u8) // buildable_to
        .u8(matches!(n.kind, Kind::Furnace | Kind::Chest | Kind::ChestOpen | Kind::Door | Kind::Trapdoor | Kind::Bed | Kind::Gate | Kind::Bones | Kind::Shelf) as u8) // rightclickable
        .u32(damage_per_second(n.id) as u32); // damage_per_second

    // liquid
    if liquid {
        let (flowing_name, source_name, viscosity) =
            if lava { ("esp:lava_flowing", "esp:lava_source", 7) } else { ("esp:water_flowing", "esp:water_source", 1) };
        w.u8(if flowing { LIQUID_FLOWING } else { LIQUID_SOURCE })
            .string16(flowing_name)
            .string16(source_name)
            .u8(viscosity) // viscosity
            .u8(!lava as u8) // renewable
            .u8(8) // range
            .u8(1) // drowning
            .u8(0); // floodable
    } else {
        w.u8(0) // liquid_type: none
            .string16("")
            .string16("")
            .u8(0) // viscosity
            .u8(1) // renewable
            .u8(8) // range
            .u8(0) // drowning
            .u8(0); // floodable
    }

    match (shaped, trapdoor) {
        (Some((shape, _)), _) => fixed_nodebox(&mut w, shape.boxes()), // node_box
        (_, Some((_, open))) => fixed_nodebox(&mut w, &[doors::trapdoor_box(open)]),
        _ if bed => fixed_nodebox(&mut w, beds::boxes(n.id)),
        _ if fence => connected_nodebox(&mut w, fence_box),
        _ if n.kind == Kind::SnowLayer => fixed_nodebox(&mut w, &[SNOW_LAYER_BOX]),
        _ if n.kind == Kind::Lily => fixed_nodebox(&mut w, &[colours::LILY_BOX]),
        _ if n.kind == Kind::Post => fixed_nodebox(&mut w, &[MESE_POST_BOX]),
        _ if n.kind == Kind::Sign => wallmounted_nodebox(&mut w, &SIGN_BOXES),
        _ if flat_pane => fixed_nodebox(&mut w, &[panes::FLAT_BOX]),
        _ if pane => connected_nodebox(&mut w, &panes::CONNECTED_BOX),
        _ => regular_nodebox(&mut w), // node_box (unused by meshes)
    }
    if let Some((_, open)) = trapdoor {
        // Its selection box is its node box; so, left empty, is its
        // collision box.
        fixed_nodebox(&mut w, &[doors::trapdoor_box(open)]);
    } else if let Some((shape, _)) = shaped {
        // builtin's register_node: a nodebox's selection box is its node
        // box. Its collision box is left empty, so the client uses the
        // node box for that too.
        fixed_nodebox(&mut w, shape.boxes());
    } else if torch {
        torch_selection_box(&mut w);
    } else if n.kind == Kind::Apple {
        apple_selection_box(&mut w);
    } else if n.kind == Kind::ChestOpen {
        open_chest_selection_box(&mut w);
    } else if ladder {
        ladder_selection_box(&mut w);
    } else if n.kind == Kind::Vessel {
        fixed_nodebox(&mut w, &[VESSEL_BOX]);
    } else if n.kind == Kind::Powder {
        fixed_nodebox(&mut w, &[tnt::POWDER_BOX]);
    } else if sapling {
        sapling_selection_box(&mut w);
    } else if door.is_some() {
        fixed_nodebox(&mut w, &[DOOR_BOX]);
    } else if plant || seed {
        fixed_nodebox(&mut w, &[farming_box(n.id)]);
    } else if bed {
        fixed_nodebox(&mut w, &[beds::selection_box(n.id)]);
    } else if n.kind == Kind::SnowLayer {
        fixed_nodebox(&mut w, &[SNOW_LAYER_BOX]);
    } else if n.kind == Kind::Lily {
        fixed_nodebox(&mut w, &[colours::LILY_SELECTION]);
    } else if n.kind == Kind::Post {
        fixed_nodebox(&mut w, &[MESE_POST_BOX]);
    } else if n.kind == Kind::Sign {
        // builtin's register_node: a nodebox's selection box is its node box.
        wallmounted_nodebox(&mut w, &SIGN_BOXES);
    } else if stalk {
        let r = if n.kind == Kind::Stem { 7.0 / 16.0 } else { 6.0 / 16.0 };
        fixed_nodebox(&mut w, &[[-r, -0.5, -r, r, 0.5, r]]);
    } else if rooted {
        // The block, and the plant above it.
        let plant = if n.id == plants::KELP {
            [-2.0 / 16.0, 0.5, -2.0 / 16.0, 2.0 / 16.0, 3.5, 2.0 / 16.0]
        } else {
            [-4.0 / 16.0, 0.5, -4.0 / 16.0, 4.0 / 16.0, 1.5, 4.0 / 16.0]
        };
        fixed_nodebox(&mut w, &[[-0.5, -0.5, -0.5, 0.5, 0.5, 0.5], plant]);
    } else if fence {
        // builtin's register_node: a nodebox's selection box is its node box.
        connected_nodebox(&mut w, fence_box);
    } else if gate.is_some() {
        fixed_nodebox(&mut w, &[fences::GATE_SELECTION]);
    } else if flat_pane {
        fixed_nodebox(&mut w, &[panes::FLAT_BOX]);
    } else if pane {
        connected_nodebox(&mut w, &panes::CONNECTED_BOX);
    } else {
        regular_nodebox(&mut w); // selection_box
    }
    if door.is_some() {
        fixed_nodebox(&mut w, &[DOOR_BOX]);
    } else if hidden {
        // 1px block inside the door's hinge, near the top.
        fixed_nodebox(&mut w, &[[-15.0 / 32.0, 13.0 / 32.0, -15.0 / 32.0, -13.0 / 32.0, 0.5, -13.0 / 32.0]]);
    } else if bed {
        // Left empty in beds.lua, so the node box.
        fixed_nodebox(&mut w, beds::boxes(n.id));
    } else if n.kind == Kind::SnowLayer {
        // A little thinner underfoot than it looks (minetest_game's snow).
        fixed_nodebox(&mut w, &[[-0.5, -0.5, -0.5, 0.5, -7.0 / 16.0, 0.5]]);
    } else if n.kind == Kind::Post {
        fixed_nodebox(&mut w, &[MESE_POST_BOX]);
    } else if wall {
        connected_nodebox(&mut w, &fences::WALL_COLLISION);
    } else if fence {
        connected_nodebox(&mut w, &fences::FENCE_COLLISION);
    } else if let Some((_, open)) = gate {
        fixed_nodebox(&mut w, fences::gate_collision(open));
    } else if flat_pane {
        // Left empty in xpanes, so the node box.
        fixed_nodebox(&mut w, &[panes::FLAT_BOX]);
    } else if pane {
        connected_nodebox(&mut w, &panes::CONNECTED_BOX);
    } else {
        regular_nodebox(&mut w); // collision_box (torches aren't walkable)
    }

    let sounds = sounds_of(n.id);
    sound(&mut w, sounds.footstep);
    sound(&mut w, sounds.dig);
    sound(&mut w, sounds.dug);

    w.u8(0) // legacy_facedir_simple
        .u8(0); // legacy_wallmounted

    // A steel door's owner alone can dig it, so the client doesn't guess
    // that it went (minetest_game's node_dig_prediction = "").
    w.string16(match plants::after_dig(n.id) {
        _ if doors::is_locked(n.id) => "",
        Some(left) => name_of(left),
        None => "air",
    }) // node_dig_prediction
        .u8(LEVELED_MAX)
        .u8(if lava {
            ALPHAMODE_OPAQUE
        } else if liquid || n.kind == Kind::Mist {
            ALPHAMODE_BLEND
        } else if torch || leaves || sapling || mark || glass || ladder || doorway || trapdoor.is_some() || plant || seed || bed || gate.is_some() || stalk || rooted || n.kind == Kind::Lily || pane {
            ALPHAMODE_CLIP
        } else {
            ALPHAMODE_OPAQUE
        })
        // Swimming: water slows you as its viscosity says.
        .u8(if lava { 7 } else { liquid as u8 }) // move_resistance: the viscosity
        .u8(liquid as u8) // liquid_move_physics
        .u8(0); // post_effect_color_shaded
    w.into_inner()
}

/// The serialised NodeDefManager, before compression.
pub fn nodedef_bytes() -> Vec<u8> {
    let mut list = Writer::plain();
    for n in NODES {
        list.u16(n.id).bytes16(&serialize_node(n));
    }
    let mut w = Writer::plain();
    w.u8(1).u16(NODES.len() as u16).bytes32(&list.into_inner());
    w.into_inner()
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

pub struct MediaFile {
    pub name: String,
    pub sha1: [u8; 20],
    /// Built-in files are read straight from the firmware in flash (memory
    /// mapped); only files from the SD card are copied into RAM.
    pub data: Cow<'static, [u8]>,
}

pub struct Media {
    files: Vec<MediaFile>,
}

impl Media {
    /// Built-in textures, plus (and overridden by) any files in `dir`.
    pub fn load(dir: &str) -> Self {
        let mut files: Vec<MediaFile> = EMBEDDED_MEDIA
            .iter()
            .map(|(name, data)| MediaFile {
                name: name.to_string(),
                sha1: sha1(data),
                data: Cow::Borrowed(*data),
            })
            .collect();

        match fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let data = match fs::read(entry.path()) {
                        Ok(d) => d,
                        Err(e) => {
                            warn!("Reading media {name}: {e}");
                            continue;
                        }
                    };
                    let file = MediaFile {
                        sha1: sha1(&data),
                        name: name.clone(),
                        data: Cow::Owned(data),
                    };
                    match files.iter_mut().find(|f| f.name.eq_ignore_ascii_case(&name)) {
                        Some(existing) => {
                            info!("Media {name} from SD card replaces built-in version");
                            *existing = file;
                        }
                        None => files.push(file),
                    }
                }
            }
            Err(_) => info!("No media folder at {dir}, using built-in textures only"),
        }

        let total: usize = files.iter().map(|f| f.data.len()).sum();
        info!("{} media files, {total} bytes", files.len());
        Self { files }
    }

    pub fn files(&self) -> &[MediaFile] {
        &self.files
    }

    pub fn get(&self, name: &str) -> Option<&MediaFile> {
        self.files.iter().find(|f| f.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mist_and_cloud() {
        // Mist: walked through, built into, lets the sun through, and
        // veils the view from inside.
        assert!(!walkable(CONTENT_MIST) && buildable_to(CONTENT_MIST) && passes_sunlight(CONTENT_MIST));
        let bytes = serialize_node(def(CONTENT_MIST).unwrap());
        assert!(bytes.windows(4).any(|w| w == MIST_POST_EFFECT.to_be_bytes()));
        // Cloud: solid, and nothing digs it.
        assert!(walkable(CONTENT_CLOUD) && !passes_light(CONTENT_CLOUD));
        assert!(dig_params(CONTENT_CLOUD, &HAND, 0).is_none());
    }

    /// turned_by says what every node's definition sends as its
    /// paramtype2 (the screwdriver goes by it).
    #[test]
    fn turned_by_matches_definitions() {
        for n in NODES {
            let b = serialize_node(n);
            let mut i = 1 + 2 + u16::from_be_bytes([b[1], b[2]]) as usize;
            let groups = u16::from_be_bytes([b[i], b[i + 1]]);
            i += 2;
            for _ in 0..groups {
                i += 2 + u16::from_be_bytes([b[i], b[i + 1]]) as usize + 2;
            }
            let expected = match b[i + 1] {
                CPT2_FACEDIR => Some(Turned::Facedir),
                CPT2_WALLMOUNTED => Some(Turned::Wallmounted),
                _ => None,
            };
            assert_eq!(turned_by(n.id), expected, "{}", n.name);
        }
    }

    /// vessels: plants to look at, standing on the floor, in the shelf's
    /// group; the drinking glass has its own inventory picture.
    #[test]
    fn vessels() {
        for v in [VESSELS_GLASS_BOTTLE, VESSELS_DRINKING_GLASS, VESSELS_STEEL_BOTTLE] {
            assert!(is_vessel(v) && stands_on_floor(v) && !walkable(v) && !buildable_to(v) && passes_sunlight(v));
            assert_eq!(group_of(v, "vessel"), 1);
            assert!(dig_params(v, &HAND, 0).is_some_and(|(t, _)| t < 0.5), "dig_immediate");
        }
        assert!(stands_on_floor(CONTENT_BIRCH_SAPLING) && !stands_on_floor(CONTENT_STONE));
        assert_eq!(shelf_of(VESSELS_SHELF), Some(crate::chest::Shelf::Vessels));
        assert_eq!(shelf_of(CONTENT_BOOKSHELF), Some(crate::chest::Shelf::Books));
        assert!(takes_meta(VESSELS_SHELF) && walkable(VESSELS_SHELF));
        let items = String::from_utf8_lossy(&itemdef_bytes()).into_owned();
        assert!(items.contains("vessels_drinking_glass_inv.png"));
        assert_eq!(stack_max(ITEM_BINOCULARS), 1);
        assert_eq!(stack_max(ITEM_MAPPING_KIT), 1);
        assert_eq!(stack_max(TOOL_SCREWDRIVER), 1);
        assert_eq!(stack_max(ITEM_KEY), 99);
    }

    /// Nodes are listed in id order without gaps, so `def` finds a node by
    /// indexing (ids 125 to 127 are Luanti's own, so the list must stop
    /// short of them, or skip them).
    #[test]
    fn node_ids_in_order() {
        for (i, n) in NODES.iter().enumerate() {
            assert_eq!(node_index(n.id), i, "{} is out of place", n.name);
            assert!(!(125..=127).contains(&n.id), "{} has a reserved id", n.name);
        }
        assert!(is_liquid(CONTENT_WATER) && floodable(CONTENT_TORCH) && passes_light(CONTENT_WATER) && !passes_sunlight(CONTENT_WATER));
        assert!(buildable_to(CONTENT_WATER) && buildable_to(crate::world::CONTENT_AIR) && !buildable_to(CONTENT_STONE));
        assert!(!walkable(CONTENT_WATER));
        assert_eq!(stack_max(ITEM_BUCKET_WATER), 1);
        assert_eq!(stack_max(ITEM_BUCKET_EMPTY), 99);
    }
}
