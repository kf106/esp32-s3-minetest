// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2026 Keir Finlow-Bates
// Translated from Luanti's shaped and shapeless recipe matching (src/craftdef.cpp) and
// minetest_game's recipes (mods/default/crafting.lua and the other mods):
//   Copyright (C) 2013 celeron55, Perttu Ahola <celeron55@gmail.com>
//   Copyright (C) 2011-2018 Various Minetest Game developers and contributors
// Licensed under the GNU Lesser General Public License, version 2.1 or (at
// your option) any later version: see COPYING.LESSER.

//! Crafting recipes and the 3x3 grid, as Luanti does it for minetest_game's
//! shaped recipes (CraftDefinitionShaped in src/craftdef.cpp): the items in
//! the grid, cut down to the box around them, must match a recipe's
//! pattern exactly (it can sit anywhere in the grid, but is not mirrored).
//! Each crafting uses up one item from every filled cell.

use crate::content::{self, *};
use crate::colours;
use crate::farming;
use crate::fences;
use crate::plants;
use crate::tnt;
use crate::inventory::Stack;

pub const GRID_WIDTH: usize = 3;
pub const GRID_SIZE: usize = GRID_WIDTH * GRID_WIDTH;

/// One cell of a recipe pattern.
#[derive(Clone, Copy, Debug)]
pub enum Ingredient {
    Empty,
    Item(u16),
    /// Any item in this group ("group:wood" in a minetest_game recipe).
    Group(&'static str),
}

use Ingredient::{Empty as E, Group as G, Item as I};

pub struct Recipe {
    pub width: usize,
    /// Rows of `width` cells, top row first.
    pub pattern: &'static [Ingredient],
    pub output: u16,
    pub count: u16,
}

/// minetest_game's recipes for the items we have (crafting.lua,
/// craftitems.lua, torch.lua, tools.lua).
pub const RECIPES: &[Recipe] = &[
    // Trunk to four planks.
    Recipe { width: 1, pattern: &[I(CONTENT_BIRCH_TREE)], output: CONTENT_BIRCH_WOOD, count: 4 },
    Recipe { width: 1, pattern: &[I(CONTENT_FIR_TREE)], output: CONTENT_FIR_WOOD, count: 4 },
    Recipe { width: 1, pattern: &[I(CONTENT_APPLE_TREE)], output: CONTENT_APPLE_WOOD, count: 4 },
    Recipe { width: 1, pattern: &[I(CONTENT_JUNGLE_TREE)], output: CONTENT_JUNGLE_WOOD, count: 4 },
    Recipe { width: 1, pattern: &[I(CONTENT_ACACIA_TREE)], output: CONTENT_ACACIA_WOOD, count: 4 },
    // A bush stem to one plank of its wood.
    Recipe { width: 1, pattern: &[I(plants::BUSH_STEM)], output: CONTENT_APPLE_WOOD, count: 1 },
    Recipe { width: 1, pattern: &[I(plants::ACACIA_BUSH_STEM)], output: CONTENT_ACACIA_WOOD, count: 1 },
    Recipe { width: 1, pattern: &[I(plants::FIR_BUSH_STEM)], output: CONTENT_FIR_WOOD, count: 1 },
    // Five cactus in a plus: a large cactus seedling.
    Recipe {
        width: 3,
        pattern: &[E, I(plants::CACTUS), E, I(plants::CACTUS), I(plants::CACTUS), I(plants::CACTUS), E, I(plants::CACTUS), E],
        output: plants::LARGE_CACTUS_SEEDLING,
        count: 1,
    },
    // Any planks to four sticks.
    Recipe { width: 1, pattern: &[G("wood")], output: ITEM_STICK, count: 4 },
    // Coal lump on a stick: four torches.
    Recipe { width: 1, pattern: &[I(ITEM_COAL_LUMP), G("stick")], output: CONTENT_TORCH, count: 4 },
    // Four sand to sandstone, and back.
    Recipe {
        width: 2,
        pattern: &[I(CONTENT_SAND), I(CONTENT_SAND), I(CONTENT_SAND), I(CONTENT_SAND)],
        output: CONTENT_SANDSTONE,
        count: 1,
    },
    Recipe { width: 1, pattern: &[I(CONTENT_SANDSTONE)], output: CONTENT_SAND, count: 4 },
    // Bricks from four, blocks from nine (crafting.lua).
    Recipe { width: 2, pattern: &[I(CONTENT_STONE); 4], output: CONTENT_STONE_BRICK, count: 4 },
    Recipe { width: 3, pattern: &[I(CONTENT_STONE); 9], output: CONTENT_STONE_BLOCK, count: 9 },
    Recipe { width: 2, pattern: &[I(CONTENT_SANDSTONE); 4], output: CONTENT_SANDSTONE_BRICK, count: 4 },
    Recipe { width: 3, pattern: &[I(CONTENT_SANDSTONE); 9], output: CONTENT_SANDSTONE_BLOCK, count: 9 },
    // The same for desert stone and the desert and silver sandstones, which
    // come from four sand as sandstone does (crafting.lua).
    Recipe { width: 2, pattern: &[I(CONTENT_DESERT_STONE); 4], output: CONTENT_DESERT_STONE_BRICK, count: 4 },
    Recipe { width: 3, pattern: &[I(CONTENT_DESERT_STONE); 9], output: CONTENT_DESERT_STONE_BLOCK, count: 9 },
    Recipe { width: 2, pattern: &[I(CONTENT_DESERT_SAND); 4], output: CONTENT_DESERT_SANDSTONE, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_DESERT_SANDSTONE)], output: CONTENT_DESERT_SAND, count: 4 },
    Recipe { width: 2, pattern: &[I(CONTENT_DESERT_SANDSTONE); 4], output: CONTENT_DESERT_SANDSTONE_BRICK, count: 4 },
    Recipe { width: 3, pattern: &[I(CONTENT_DESERT_SANDSTONE); 9], output: CONTENT_DESERT_SANDSTONE_BLOCK, count: 9 },
    Recipe { width: 2, pattern: &[I(CONTENT_SILVER_SAND); 4], output: CONTENT_SILVER_SANDSTONE, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_SILVER_SANDSTONE)], output: CONTENT_SILVER_SAND, count: 4 },
    Recipe { width: 2, pattern: &[I(CONTENT_SILVER_SANDSTONE); 4], output: CONTENT_SILVER_SANDSTONE_BRICK, count: 4 },
    Recipe { width: 3, pattern: &[I(CONTENT_SILVER_SANDSTONE); 9], output: CONTENT_SILVER_SANDSTONE_BLOCK, count: 9 },
    // Clay: four lumps to a block, and back; four bricks to a brick block,
    // and back.
    Recipe { width: 2, pattern: &[I(ITEM_CLAY_LUMP); 4], output: CONTENT_CLAY, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_CLAY)], output: ITEM_CLAY_LUMP, count: 4 },
    Recipe { width: 2, pattern: &[I(ITEM_CLAY_BRICK); 4], output: CONTENT_BRICK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_BRICK)], output: ITEM_CLAY_BRICK, count: 4 },
    // Snow: nine layers make a block, a block nine layers.
    Recipe { width: 3, pattern: &[I(CONTENT_SNOW_LAYER); 9], output: CONTENT_SNOW_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_SNOW_BLOCK)], output: CONTENT_SNOW_LAYER, count: 9 },
    // Obsidian: nine shards, and back; bricks and blocks.
    Recipe { width: 3, pattern: &[I(ITEM_OBSIDIAN_SHARD); 9], output: CONTENT_OBSIDIAN, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_OBSIDIAN)], output: ITEM_OBSIDIAN_SHARD, count: 9 },
    Recipe { width: 2, pattern: &[I(CONTENT_OBSIDIAN); 4], output: CONTENT_OBSIDIAN_BRICK, count: 4 },
    Recipe { width: 3, pattern: &[I(CONTENT_OBSIDIAN); 9], output: CONTENT_OBSIDIAN_BLOCK, count: 9 },
    // Nine lumps or ingots to a block, and back (crafting.lua,
    // craftitems.lua).
    Recipe { width: 3, pattern: &[I(ITEM_COAL_LUMP); 9], output: CONTENT_COAL_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_COAL_BLOCK)], output: ITEM_COAL_LUMP, count: 9 },
    Recipe { width: 3, pattern: &[I(ITEM_STEEL_INGOT); 9], output: CONTENT_STEEL_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_STEEL_BLOCK)], output: ITEM_STEEL_INGOT, count: 9 },
    Recipe { width: 3, pattern: &[I(ITEM_COPPER_INGOT); 9], output: CONTENT_COPPER_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_COPPER_BLOCK)], output: ITEM_COPPER_INGOT, count: 9 },
    Recipe { width: 3, pattern: &[I(ITEM_TIN_INGOT); 9], output: CONTENT_TIN_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_TIN_BLOCK)], output: ITEM_TIN_INGOT, count: 9 },
    Recipe { width: 3, pattern: &[I(ITEM_BRONZE_INGOT); 9], output: CONTENT_BRONZE_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_BRONZE_BLOCK)], output: ITEM_BRONZE_INGOT, count: 9 },
    Recipe { width: 3, pattern: &[I(ITEM_GOLD_INGOT); 9], output: CONTENT_GOLD_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_GOLD_BLOCK)], output: ITEM_GOLD_INGOT, count: 9 },
    Recipe { width: 3, pattern: &[I(ITEM_DIAMOND); 9], output: CONTENT_DIAMOND_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_DIAMOND_BLOCK)], output: ITEM_DIAMOND, count: 9 },
    Recipe { width: 3, pattern: &[I(ITEM_MESE_CRYSTAL); 9], output: CONTENT_MESE_BLOCK, count: 1 },
    Recipe { width: 1, pattern: &[I(CONTENT_MESE_BLOCK)], output: ITEM_MESE_CRYSTAL, count: 9 },
    // Nine mese fragments make a crystal, and a crystal breaks into nine.
    Recipe { width: 3, pattern: &[I(ITEM_MESE_FRAGMENT); 9], output: ITEM_MESE_CRYSTAL, count: 1 },
    Recipe { width: 1, pattern: &[I(ITEM_MESE_CRYSTAL)], output: ITEM_MESE_FRAGMENT, count: 9 },
    // A mese lamp: glass over a mese crystal.
    Recipe { width: 1, pattern: &[I(CONTENT_GLASS), I(ITEM_MESE_CRYSTAL)], output: CONTENT_MESE_LAMP, count: 1 },
    // Paper from three papyrus in a row; a book from three paper in a
    // column; a bookshelf from three books between two rows of planks.
    Recipe { width: 3, pattern: &[I(crate::plants::PAPYRUS); 3], output: ITEM_PAPER, count: 1 },
    Recipe { width: 1, pattern: &[I(ITEM_PAPER); 3], output: ITEM_BOOK, count: 1 },
    Recipe { width: 3, pattern: &[G("wood"), G("wood"), G("wood"), I(ITEM_BOOK), I(ITEM_BOOK), I(ITEM_BOOK), G("wood"), G("wood"), G("wood")], output: CONTENT_BOOKSHELF, count: 1 },
    // vessels: glass bottles (10) and steel bottles (5) from six in a
    // bottle's shape, drinking glasses (14) from seven in a cup's; a
    // shelf from three vessels between two rows of planks.
    Recipe { width: 3, pattern: &bottle(I(CONTENT_GLASS)), output: VESSELS_GLASS_BOTTLE, count: 10 },
    Recipe {
        width: 3,
        pattern: &[I(CONTENT_GLASS), E, I(CONTENT_GLASS), I(CONTENT_GLASS), E, I(CONTENT_GLASS), I(CONTENT_GLASS), I(CONTENT_GLASS), I(CONTENT_GLASS)],
        output: VESSELS_DRINKING_GLASS,
        count: 14,
    },
    Recipe { width: 3, pattern: &bottle(I(ITEM_STEEL_INGOT)), output: VESSELS_STEEL_BOTTLE, count: 5 },
    Recipe { width: 3, pattern: &[G("wood"), G("wood"), G("wood"), G("vessel"), G("vessel"), G("vessel"), G("wood"), G("wood"), G("wood")], output: VESSELS_SHELF, count: 1 },
    // Nine jungle saplings make an emergent jungle tree sapling.
    Recipe { width: 3, pattern: &[I(CONTENT_JUNGLE_SAPLING); 9], output: EMERGENT_JUNGLE_SAPLING, count: 1 },
    // keys: a gold ingot makes a skeleton key.
    Recipe { width: 1, pattern: &[I(ITEM_GOLD_INGOT)], output: ITEM_SKELETON_KEY, count: 1 },
    // screwdriver: a steel ingot on a stick.
    Recipe { width: 1, pattern: &[I(ITEM_STEEL_INGOT), G("stick")], output: TOOL_SCREWDRIVER, count: 1 },
    // The tnt mod: six gunpowder either side of paper make 2 TNT sticks,
    // and nine sticks TNT.
    Recipe {
        width: 3,
        pattern: &[I(tnt::GUNPOWDER), E, I(tnt::GUNPOWDER), I(tnt::GUNPOWDER), I(ITEM_PAPER), I(tnt::GUNPOWDER), I(tnt::GUNPOWDER), E, I(tnt::GUNPOWDER)],
        output: ITEM_TNT_STICK,
        count: 2,
    },
    Recipe { width: 3, pattern: &[I(ITEM_TNT_STICK); 9], output: tnt::TNT, count: 1 },
    // Flint beside a steel ingot: flint and steel (fire mod).
    Recipe { width: 2, pattern: &[I(ITEM_FLINT), I(ITEM_STEEL_INGOT)], output: TOOL_FLINT_AND_STEEL, count: 1 },
    // Four string over a stick: a bug net; a firefly over a glass bottle:
    // a firefly in a bottle (fireflies mod).
    Recipe { width: 2, pattern: &[I(ITEM_STRING), I(ITEM_STRING), I(ITEM_STRING), I(ITEM_STRING), I(ITEM_STICK), E], output: TOOL_BUG_NET, count: 1 },
    Recipe { width: 1, pattern: &[I(crate::insects::FIREFLY), I(VESSELS_GLASS_BOTTLE)], output: crate::insects::FIREFLY_BOTTLE, count: 1 },
    // binoculars: obsidian glass lenses either side of three bronze ingots.
    Recipe {
        width: 3,
        pattern: &[I(CONTENT_OBSIDIAN_GLASS), E, I(CONTENT_OBSIDIAN_GLASS), I(ITEM_BRONZE_INGOT), I(ITEM_BRONZE_INGOT), I(ITEM_BRONZE_INGOT), I(CONTENT_OBSIDIAN_GLASS), E, I(CONTENT_OBSIDIAN_GLASS)],
        output: ITEM_BINOCULARS,
        count: 1,
    },
    // Rails, 18 at a time: steel ingots either side of planks, nothing
    // and planks; with a mese crystal in the middle, powered rails, with
    // a coal lump, brake rails. A cart: five steel ingots in a U (carts
    // mod).
    Recipe {
        width: 3,
        pattern: &[I(ITEM_STEEL_INGOT), G("wood"), I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), E, I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), G("wood"), I(ITEM_STEEL_INGOT)],
        output: crate::carts::RAIL,
        count: 18,
    },
    Recipe {
        width: 3,
        pattern: &[I(ITEM_STEEL_INGOT), G("wood"), I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), I(ITEM_MESE_CRYSTAL), I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), G("wood"), I(ITEM_STEEL_INGOT)],
        output: crate::carts::POWERRAIL,
        count: 18,
    },
    Recipe {
        width: 3,
        pattern: &[I(ITEM_STEEL_INGOT), G("wood"), I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), I(ITEM_COAL_LUMP), I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), G("wood"), I(ITEM_STEEL_INGOT)],
        output: crate::carts::BRAKERAIL,
        count: 18,
    },
    Recipe {
        width: 3,
        pattern: &[I(ITEM_STEEL_INGOT), E, I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT), I(ITEM_STEEL_INGOT)],
        output: ITEM_CART,
        count: 1,
    },
    // A boat: five planks in a U (boats mod).
    Recipe {
        width: 3,
        pattern: &[E, E, E, G("wood"), E, G("wood"), G("wood"), G("wood"), G("wood")],
        output: ITEM_BOAT,
        count: 1,
    },
    // map: a mapping kit.
    Recipe {
        width: 3,
        pattern: &[
            I(CONTENT_GLASS), I(ITEM_PAPER), G("stick"),
            I(ITEM_STEEL_INGOT), I(ITEM_PAPER), I(ITEM_STEEL_INGOT),
            G("wood"), I(ITEM_PAPER), I(colours::dye(colours::BLACK)),
        ],
        output: ITEM_MAPPING_KIT,
        count: 1,
    },
    // Signs: two rows of planks or steel ingots over a stick make 3.
    Recipe { width: 3, pattern: &sign(G("wood")), output: SIGN_WOOD, count: 3 },
    Recipe { width: 3, pattern: &sign(I(ITEM_STEEL_INGOT)), output: SIGN_STEEL, count: 3 },
    // Mese post lights: glass over three mese crystals over planks make 4
    // (default.register_mesepost).
    Recipe { width: 3, pattern: &mese_post(CONTENT_APPLE_WOOD), output: MESE_POST_APPLE, count: 4 },
    Recipe { width: 3, pattern: &mese_post(CONTENT_BIRCH_WOOD), output: MESE_POST_BIRCH, count: 4 },
    Recipe { width: 3, pattern: &mese_post(CONTENT_FIR_WOOD), output: MESE_POST_FIR, count: 4 },
    Recipe { width: 3, pattern: &mese_post(CONTENT_JUNGLE_WOOD), output: MESE_POST_JUNGLE, count: 4 },
    Recipe { width: 3, pattern: &mese_post(CONTENT_ACACIA_WOOD), output: MESE_POST_ACACIA, count: 4 },
    // Bronze: eight copper ingots round a tin one make nine.
    Recipe { width: 3, pattern: &ring(I(ITEM_COPPER_INGOT), I(ITEM_TIN_INGOT)), output: ITEM_BRONZE_INGOT, count: 9 },
    // Ladders: an H of sticks makes 5, of steel ingots 15.
    Recipe { width: 3, pattern: &ladder(G("stick")), output: CONTENT_LADDER_WOOD, count: 5 },
    Recipe { width: 3, pattern: &ladder(I(ITEM_STEEL_INGOT)), output: CONTENT_LADDER_STEEL, count: 15 },
    // Tools: planks for wooden ones, stone or cobblestone for stone ones.
    Recipe { width: 3, pattern: &pick(G("wood")), output: TOOL_PICK_WOOD, count: 1 },
    Recipe { width: 3, pattern: &pick(G("stone")), output: TOOL_PICK_STONE, count: 1 },
    Recipe { width: 1, pattern: &shovel(G("wood")), output: TOOL_SHOVEL_WOOD, count: 1 },
    Recipe { width: 1, pattern: &shovel(G("stone")), output: TOOL_SHOVEL_STONE, count: 1 },
    Recipe { width: 2, pattern: &axe(G("wood")), output: TOOL_AXE_WOOD, count: 1 },
    Recipe { width: 2, pattern: &axe(G("stone")), output: TOOL_AXE_STONE, count: 1 },
    Recipe { width: 1, pattern: &sword(G("wood")), output: TOOL_SWORD_WOOD, count: 1 },
    Recipe { width: 1, pattern: &sword(G("stone")), output: TOOL_SWORD_STONE, count: 1 },
    Recipe { width: 3, pattern: &pick(I(ITEM_STEEL_INGOT)), output: TOOL_PICK_STEEL, count: 1 },
    Recipe { width: 1, pattern: &shovel(I(ITEM_STEEL_INGOT)), output: TOOL_SHOVEL_STEEL, count: 1 },
    Recipe { width: 2, pattern: &axe(I(ITEM_STEEL_INGOT)), output: TOOL_AXE_STEEL, count: 1 },
    Recipe { width: 1, pattern: &sword(I(ITEM_STEEL_INGOT)), output: TOOL_SWORD_STEEL, count: 1 },
    // Bronze, mese and diamond tools, the same shapes.
    Recipe { width: 3, pattern: &pick(I(ITEM_BRONZE_INGOT)), output: TOOL_PICK_BRONZE, count: 1 },
    Recipe { width: 1, pattern: &shovel(I(ITEM_BRONZE_INGOT)), output: TOOL_SHOVEL_BRONZE, count: 1 },
    Recipe { width: 2, pattern: &axe(I(ITEM_BRONZE_INGOT)), output: TOOL_AXE_BRONZE, count: 1 },
    Recipe { width: 1, pattern: &sword(I(ITEM_BRONZE_INGOT)), output: TOOL_SWORD_BRONZE, count: 1 },
    Recipe { width: 3, pattern: &pick(I(ITEM_MESE_CRYSTAL)), output: TOOL_PICK_MESE, count: 1 },
    Recipe { width: 1, pattern: &shovel(I(ITEM_MESE_CRYSTAL)), output: TOOL_SHOVEL_MESE, count: 1 },
    Recipe { width: 2, pattern: &axe(I(ITEM_MESE_CRYSTAL)), output: TOOL_AXE_MESE, count: 1 },
    Recipe { width: 1, pattern: &sword(I(ITEM_MESE_CRYSTAL)), output: TOOL_SWORD_MESE, count: 1 },
    Recipe { width: 3, pattern: &pick(I(ITEM_DIAMOND)), output: TOOL_PICK_DIAMOND, count: 1 },
    Recipe { width: 1, pattern: &shovel(I(ITEM_DIAMOND)), output: TOOL_SHOVEL_DIAMOND, count: 1 },
    Recipe { width: 2, pattern: &axe(I(ITEM_DIAMOND)), output: TOOL_AXE_DIAMOND, count: 1 },
    Recipe { width: 1, pattern: &sword(I(ITEM_DIAMOND)), output: TOOL_SWORD_DIAMOND, count: 1 },
    // Chests: a ring of planks, with a steel ingot in the middle for a
    // locked one (chests.lua).
    Recipe { width: 3, pattern: &ring(G("wood"), E), output: CONTENT_CHEST, count: 1 },
    Recipe { width: 3, pattern: &ring(G("wood"), I(ITEM_STEEL_INGOT)), output: CONTENT_CHEST_LOCKED, count: 1 },
    // Doors: two columns of three planks, steel ingots or glass
    // (doors/init.lua).
    Recipe { width: 2, pattern: &[G("wood"); 6], output: ITEM_DOOR_WOOD, count: 1 },
    Recipe { width: 2, pattern: &[I(ITEM_STEEL_INGOT); 6], output: ITEM_DOOR_STEEL, count: 1 },
    Recipe { width: 2, pattern: &[I(CONTENT_GLASS); 6], output: ITEM_DOOR_GLASS, count: 1 },
    // Trapdoors: two rows of three planks make 2, a square of four steel
    // ingots 1.
    Recipe { width: 3, pattern: &[G("wood"); 6], output: crate::doors::FIRST_TRAPDOOR, count: 2 },
    Recipe { width: 2, pattern: &[I(ITEM_STEEL_INGOT); 4], output: crate::doors::FIRST_TRAPDOOR + 2, count: 1 },
    // Walls (walls mod): two rows of three cobblestone make 6.
    Recipe { width: 3, pattern: &[I(CONTENT_COBBLE); 6], output: crate::fences::WALL_COBBLE, count: 6 },
    Recipe { width: 3, pattern: &[I(CONTENT_DESERT_COBBLE); 6], output: crate::fences::WALL_DESERT_COBBLE, count: 6 },
    Recipe { width: 3, pattern: &[I(crate::dungeons::MOSSYCOBBLE); 6], output: crate::dungeons::WALL_MOSSYCOBBLE, count: 6 },
    // Panes (xpanes): two rows of three glass, obsidian glass or steel
    // ingots make 16; the steel bars make a door (two columns of three)
    // and a trapdoor (a square of four).
    Recipe { width: 3, pattern: &[I(CONTENT_GLASS); 6], output: crate::panes::PANE_FLAT, count: 16 },
    Recipe { width: 3, pattern: &[I(CONTENT_OBSIDIAN_GLASS); 6], output: crate::panes::OBSIDIAN_PANE_FLAT, count: 16 },
    Recipe { width: 3, pattern: &[I(ITEM_STEEL_INGOT); 6], output: crate::panes::BAR_FLAT, count: 16 },
    Recipe { width: 2, pattern: &[I(crate::panes::BAR_FLAT); 6], output: ITEM_DOOR_STEEL_BAR, count: 1 },
    Recipe { width: 2, pattern: &[I(crate::panes::BAR_FLAT); 4], output: crate::panes::BAR_TRAPDOOR, count: 1 },
    // Farming (farming/init.lua, hoes.lua): hoes, wool and string from
    // cotton, straw from wheat and back.
    Recipe { width: 2, pattern: &hoe(G("wood")), output: TOOL_HOE_WOOD, count: 1 },
    Recipe { width: 2, pattern: &hoe(G("stone")), output: TOOL_HOE_STONE, count: 1 },
    Recipe { width: 2, pattern: &hoe(I(ITEM_STEEL_INGOT)), output: TOOL_HOE_STEEL, count: 1 },
    Recipe { width: 2, pattern: &[I(ITEM_COTTON); 4], output: farming::WOOL_WHITE, count: 1 },
    Recipe { width: 1, pattern: &[I(ITEM_COTTON); 2], output: ITEM_STRING, count: 2 },
    Recipe { width: 3, pattern: &[I(ITEM_WHEAT); 9], output: farming::STRAW, count: 3 },
    Recipe { width: 1, pattern: &[I(farming::STRAW)], output: ITEM_WHEAT, count: 3 },
    // Beds (beds.lua): three wool (of any colour) on three planks; the
    // fancy bed has a stick on top at the right.
    Recipe {
        width: 3,
        pattern: &[G("wool"), G("wool"), G("wool"), G("wood"), G("wood"), G("wood")],
        output: crate::beds::BED_BOTTOM,
        count: 1,
    },
    Recipe {
        width: 3,
        pattern: &[
            E,
            E,
            I(ITEM_STICK),
            G("wool"),
            G("wool"),
            G("wool"),
            G("wood"),
            G("wood"),
            G("wood"),
        ],
        output: crate::beds::FANCY_BOTTOM,
        count: 1,
    },
    // Fences: planks either side of sticks, two rows, make 4
    // (default.register_fence); gates: sticks either side of planks
    // (doors.register_fencegate).
    Recipe { width: 3, pattern: &fence(CONTENT_APPLE_WOOD), output: fences::FENCE_APPLE, count: 4 },
    Recipe { width: 3, pattern: &fence(CONTENT_FIR_WOOD), output: fences::FENCE_FIR, count: 4 },
    Recipe { width: 3, pattern: &fence(CONTENT_BIRCH_WOOD), output: fences::FENCE_BIRCH, count: 4 },
    Recipe { width: 3, pattern: &fence(CONTENT_JUNGLE_WOOD), output: fences::FENCE_JUNGLE, count: 4 },
    Recipe { width: 3, pattern: &fence(CONTENT_ACACIA_WOOD), output: fences::FENCE_ACACIA, count: 4 },
    // Rails: two planks above two more, a row apart, make 16
    // (default.register_fence_rail).
    Recipe { width: 2, pattern: &rail(CONTENT_APPLE_WOOD), output: fences::RAIL_APPLE, count: 16 },
    Recipe { width: 2, pattern: &rail(CONTENT_FIR_WOOD), output: fences::RAIL_FIR, count: 16 },
    Recipe { width: 2, pattern: &rail(CONTENT_BIRCH_WOOD), output: fences::RAIL_BIRCH, count: 16 },
    Recipe { width: 2, pattern: &rail(CONTENT_JUNGLE_WOOD), output: fences::RAIL_JUNGLE, count: 16 },
    Recipe { width: 2, pattern: &rail(CONTENT_ACACIA_WOOD), output: fences::RAIL_ACACIA, count: 16 },
    Recipe { width: 3, pattern: &gate(CONTENT_APPLE_WOOD), output: fences::GATE_APPLE, count: 1 },
    Recipe { width: 3, pattern: &gate(CONTENT_FIR_WOOD), output: fences::GATE_FIR, count: 1 },
    Recipe { width: 3, pattern: &gate(CONTENT_BIRCH_WOOD), output: fences::GATE_BIRCH, count: 1 },
    Recipe { width: 3, pattern: &gate(CONTENT_JUNGLE_WOOD), output: fences::GATE_JUNGLE, count: 1 },
    Recipe { width: 3, pattern: &gate(CONTENT_ACACIA_WOOD), output: fences::GATE_ACACIA, count: 1 },
    // A flower gives four of its dye; coal four black, blueberries two
    // violet (dye/init.lua).
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 0)], output: colours::dye(colours::FLOWERS[0].2), count: 4 },
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 1)], output: colours::dye(colours::FLOWERS[1].2), count: 4 },
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 2)], output: colours::dye(colours::FLOWERS[2].2), count: 4 },
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 3)], output: colours::dye(colours::FLOWERS[3].2), count: 4 },
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 4)], output: colours::dye(colours::FLOWERS[4].2), count: 4 },
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 5)], output: colours::dye(colours::FLOWERS[5].2), count: 4 },
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 6)], output: colours::dye(colours::FLOWERS[6].2), count: 4 },
    Recipe { width: 1, pattern: &[I(colours::FIRST_FLOWER + 7)], output: colours::dye(colours::FLOWERS[7].2), count: 4 },
    Recipe { width: 1, pattern: &[I(ITEM_COAL_LUMP)], output: colours::dye(colours::BLACK), count: 4 },
    Recipe { width: 1, pattern: &[I(ITEM_BLUEBERRIES)], output: colours::dye(colours::VIOLET), count: 2 },
    // A bucket: three steel ingots in a V (bucket mod).
    Recipe {
        width: 3,
        pattern: &[I(ITEM_STEEL_INGOT), E, I(ITEM_STEEL_INGOT), E, I(ITEM_STEEL_INGOT), E],
        output: ITEM_BUCKET_EMPTY,
        count: 1,
    },
    // A ring of stone (furnace.lua).
    Recipe {
        width: 3,
        pattern: &[G("stone"), G("stone"), G("stone"), G("stone"), E, G("stone"), G("stone"), G("stone"), G("stone")],
        output: CONTENT_FURNACE,
        count: 1,
    },
];

/// What a furnace makes of an item, and how long it takes in seconds
/// (minetest_game's cooking recipes, all with Luanti's default 3 s).
pub fn cook_result(id: u16) -> Option<(u16, f32)> {
    match id {
        ITEM_IRON_LUMP => Some((ITEM_STEEL_INGOT, 3.0)),
        ITEM_COPPER_LUMP => Some((ITEM_COPPER_INGOT, 3.0)),
        ITEM_TIN_LUMP => Some((ITEM_TIN_INGOT, 3.0)),
        ITEM_GOLD_LUMP => Some((ITEM_GOLD_INGOT, 3.0)),
        CONTENT_COBBLE => Some((CONTENT_STONE, 3.0)),
        CONTENT_DESERT_COBBLE => Some((CONTENT_DESERT_STONE, 3.0)),
        crate::dungeons::MOSSYCOBBLE => Some((CONTENT_STONE, 3.0)),
        // group:sand: glass.
        CONTENT_SAND | CONTENT_DESERT_SAND | CONTENT_SILVER_SAND => Some((CONTENT_GLASS, 3.0)),
        ITEM_CLAY_LUMP => Some((ITEM_CLAY_BRICK, 3.0)),
        ITEM_OBSIDIAN_SHARD => Some((CONTENT_OBSIDIAN_GLASS, 3.0)),
        // farming: flour bakes into bread, slowly.
        ITEM_FLOUR => Some((ITEM_BREAD, 15.0)),
        // vessels: fragments melt back into glass, a steel bottle into
        // steel.
        ITEM_GLASS_FRAGMENTS => Some((CONTENT_GLASS, 3.0)),
        VESSELS_STEEL_BOTTLE => Some((ITEM_STEEL_INGOT, 3.0)),
        // keys: back into gold, in 5 s.
        ITEM_KEY | ITEM_SKELETON_KEY => Some((ITEM_GOLD_INGOT, 5.0)),
        _ => None,
    }
}

/// How long an item burns in a furnace, in seconds (0: it isn't fuel).
/// minetest_game's fuel recipes, with birch as its aspen and fir as its
/// pine.
pub fn burn_time(id: u16) -> f32 {
    match id {
        ITEM_COAL_LUMP => 40.0,
        CONTENT_FIR_TREE => 26.0,
        CONTENT_APPLE_TREE => 30.0,
        CONTENT_APPLE_WOOD => 7.0,
        CONTENT_APPLE_SAPLING => 5.0,
        CONTENT_APPLE_LEAVES => 4.0,
        CONTENT_BIRCH_TREE => 22.0,
        CONTENT_FIR_WOOD => 6.0,
        CONTENT_BIRCH_WOOD => 5.0,
        CONTENT_BIRCH_LEAVES | CONTENT_FIR_NEEDLES | CONTENT_JUNGLE_LEAVES | CONTENT_ACACIA_LEAVES => 4.0,
        CONTENT_JUNGLE_TREE => 38.0,
        CONTENT_ACACIA_TREE => 34.0,
        CONTENT_JUNGLE_WOOD => 9.0,
        CONTENT_ACACIA_WOOD => 8.0,
        CONTENT_JUNGLE_SAPLING | CONTENT_ACACIA_SAPLING => 6.0,
        EMERGENT_JUNGLE_SAPLING => 7.0,
        farming::JUNGLEGRASS => 3.0,
        plants::BUSH_LEAVES | plants::BLUEBERRY_LEAVES | plants::BLUEBERRY_LEAVES_WITH_BERRIES | plants::ACACIA_BUSH_LEAVES
        | plants::FIR_BUSH_NEEDLES => 4.0,
        plants::BUSH_STEM => 7.0,
        plants::ACACIA_BUSH_STEM => 8.0,
        plants::FIR_BUSH_STEM => 6.0,
        plants::BUSH_SAPLING => 3.0,
        plants::ACACIA_BUSH_SAPLING => 4.0,
        plants::FIR_BUSH_SAPLING => 2.0,
        plants::CACTUS => 15.0,
        plants::LARGE_CACTUS_SEEDLING => 5.0,
        plants::PAPYRUS => 3.0,
        id if id == plants::FERN[0] || id == plants::MARRAM[0] => 2.0,
        CONTENT_TORCH => 4.0,
        CONTENT_BIRCH_SAPLING => 4.0,
        CONTENT_FIR_SAPLING => 5.0,
        ITEM_STICK => 1.0,
        SIGN_WOOD => 10.0,
        DRY_SHRUB => 2.0,
        CONTENT_BOOKSHELF | VESSELS_SHELF => 30.0,
        ITEM_MAPPING_KIT => 5.0,
        ITEM_BOAT => 20.0,
        TOOL_PICK_WOOD | TOOL_AXE_WOOD => 6.0,
        TOOL_SWORD_WOOD => 5.0,
        TOOL_SHOVEL_WOOD => 4.0,
        CONTENT_CHEST | CONTENT_CHEST_LOCKED => 30.0,
        CONTENT_COAL_BLOCK => 370.0,
        CONTENT_LADDER_WOOD => 7.0,
        ITEM_DOOR_WOOD => 14.0,
        ITEM_WHEAT | ITEM_COTTON | ITEM_STRING => 1.0,
        TOOL_HOE_WOOD => 5.0,
        farming::STRAW => 3.0,
        id if id == farming::GRASS[0] || id == farming::DRY_GRASS[0] => 2.0,
        crate::doors::FIRST_TRAPDOOR => 7.0,
        // A lava bucket burns for a minute (the bucket mod), leaving the
        // empty bucket (see furnace.rs).
        ITEM_BUCKET_LAVA => 60.0,
        crate::beds::BED_BOTTOM => 12.0,
        crate::beds::FANCY_BOTTOM => 13.0,
        id if fences::burn_time(id) > 0.0 => fences::burn_time(id),
        id => crate::stairs::burn_time(id, burn_time),
    }
}

/// A fence: two rows of planks, stick, planks.
const fn fence(planks: u16) -> [Ingredient; 6] {
    [I(planks), G("stick"), I(planks), I(planks), G("stick"), I(planks)]
}

/// A rail: two planks, an empty row, two planks.
const fn rail(planks: u16) -> [Ingredient; 6] {
    [I(planks), I(planks), E, E, I(planks), I(planks)]
}

/// A gate: two rows of stick, planks, stick.
const fn gate(planks: u16) -> [Ingredient; 6] {
    [G("stick"), I(planks), G("stick"), G("stick"), I(planks), G("stick")]
}

/// minetest_game's ladder shape: two sides joined in the middle.
/// A sign of `m`.
const fn sign(m: Ingredient) -> [Ingredient; 9] {
    [m, m, m, m, m, m, E, G("stick"), E]
}

/// A bottle of `m`: two columns of two, over one in the middle.
const fn bottle(m: Ingredient) -> [Ingredient; 9] {
    [m, E, m, m, E, m, E, m, E]
}

/// A mese post light of `wood`.
const fn mese_post(wood: u16) -> [Ingredient; 9] {
    [E, I(CONTENT_GLASS), E, I(ITEM_MESE_CRYSTAL), I(ITEM_MESE_CRYSTAL), I(ITEM_MESE_CRYSTAL), E, I(wood), E]
}

const fn ladder(m: Ingredient) -> [Ingredient; 9] {
    [m, E, m, m, m, m, m, E, m]
}

/// Eight of `m` round the edge of the grid, `middle` in the middle.
const fn ring(m: Ingredient, middle: Ingredient) -> [Ingredient; 9] {
    [m, m, m, m, middle, m, m, m, m]
}

/// A shapeless recipe: these items, one each, anywhere in the grid and
/// nothing else (CraftDefinitionShapeless).
pub struct Shapeless {
    pub items: &'static [Ingredient],
    pub output: u16,
    pub count: u16,
}

/// minetest_game's shapeless recipes for what we have.
pub const SHAPELESS: &[Shapeless] = &[
    // Coal and gravel, anyhow, make 5 gunpowder (tnt mod).
    Shapeless { items: &[I(ITEM_COAL_LUMP), I(CONTENT_GRAVEL)], output: tnt::GUNPOWDER, count: 5 },
    // A chest and a steel ingot make a locked chest (chests.lua).
    Shapeless { items: &[I(CONTENT_CHEST), I(ITEM_STEEL_INGOT)], output: CONTENT_CHEST_LOCKED, count: 1 },
    // A book and a written book make a copy of it; the original stays
    // (register_craft_metadata_copy, in inventory.rs).
    Shapeless { items: &[I(ITEM_BOOK), I(ITEM_BOOK_WRITTEN)], output: ITEM_BOOK_WRITTEN, count: 1 },
    // Four wheat, anyhow, make flour (farming/init.lua).
    Shapeless { items: &[I(ITEM_WHEAT); 4], output: ITEM_FLOUR, count: 1 },
    // vessels: two bottles or glasses break into glass fragments.
    Shapeless { items: &[I(VESSELS_GLASS_BOTTLE); 2], output: ITEM_GLASS_FRAGMENTS, count: 1 },
    Shapeless { items: &[I(VESSELS_DRINKING_GLASS); 2], output: ITEM_GLASS_FRAGMENTS, count: 1 },
    // Two dyes, anyhow, make two of a third (dye/init.lua's dye_recipes).
    Shapeless { items: &[I(colours::dye(colours::RED)), I(colours::dye(colours::BLUE))], output: colours::dye(colours::VIOLET), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::YELLOW)), I(colours::dye(colours::RED))], output: colours::dye(colours::ORANGE), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::YELLOW)), I(colours::dye(colours::BLUE))], output: colours::dye(colours::GREEN), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::YELLOW)), I(colours::dye(colours::VIOLET))], output: colours::dye(colours::DARK_GREY), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::BLUE)), I(colours::dye(colours::ORANGE))], output: colours::dye(colours::DARK_GREY), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::CYAN)), I(colours::dye(colours::YELLOW))], output: colours::dye(colours::GREEN), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::CYAN)), I(colours::dye(colours::MAGENTA))], output: colours::dye(colours::BLUE), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::YELLOW)), I(colours::dye(colours::MAGENTA))], output: colours::dye(colours::RED), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::RED)), I(colours::dye(colours::GREEN))], output: colours::dye(colours::BROWN), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::MAGENTA)), I(colours::dye(colours::BLUE))], output: colours::dye(colours::VIOLET), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::GREEN)), I(colours::dye(colours::BLUE))], output: colours::dye(colours::CYAN), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::PINK)), I(colours::dye(colours::VIOLET))], output: colours::dye(colours::MAGENTA), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::WHITE)), I(colours::dye(colours::BLACK))], output: colours::dye(colours::GREY), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::GREY)), I(colours::dye(colours::BLACK))], output: colours::dye(colours::DARK_GREY), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::GREEN)), I(colours::dye(colours::BLACK))], output: colours::dye(colours::DARK_GREEN), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::ORANGE)), I(colours::dye(colours::BLACK))], output: colours::dye(colours::BROWN), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::WHITE)), I(colours::dye(colours::RED))], output: colours::dye(colours::PINK), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::WHITE)), I(colours::dye(colours::DARK_GREY))], output: colours::dye(colours::GREY), count: 2 },
    Shapeless { items: &[I(colours::dye(colours::WHITE)), I(colours::dye(colours::DARK_GREEN))], output: colours::dye(colours::GREEN), count: 2 },
    // Any wool and a dye: wool of the dye's colour (wool/init.lua).
    Shapeless { items: &[I(colours::dye(colours::WHITE)), G("wool")], output: colours::wool(colours::WHITE), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::GREY)), G("wool")], output: colours::wool(colours::GREY), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::DARK_GREY)), G("wool")], output: colours::wool(colours::DARK_GREY), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::BLACK)), G("wool")], output: colours::wool(colours::BLACK), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::VIOLET)), G("wool")], output: colours::wool(colours::VIOLET), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::BLUE)), G("wool")], output: colours::wool(colours::BLUE), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::CYAN)), G("wool")], output: colours::wool(colours::CYAN), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::DARK_GREEN)), G("wool")], output: colours::wool(colours::DARK_GREEN), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::GREEN)), G("wool")], output: colours::wool(colours::GREEN), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::YELLOW)), G("wool")], output: colours::wool(colours::YELLOW), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::BROWN)), G("wool")], output: colours::wool(colours::BROWN), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::ORANGE)), G("wool")], output: colours::wool(colours::ORANGE), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::RED)), G("wool")], output: colours::wool(colours::RED), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::MAGENTA)), G("wool")], output: colours::wool(colours::MAGENTA), count: 1 },
    Shapeless { items: &[I(colours::dye(colours::PINK)), G("wool")], output: colours::wool(colours::PINK), count: 1 },
];

impl Shapeless {
    fn fits(&self, grid: &[Option<Stack>; GRID_SIZE]) -> bool {
        let filled: Vec<Stack> = grid.iter().flatten().copied().collect();
        if filled.len() != self.items.len() {
            return false;
        }
        // Match each ingredient to a different filled cell (few enough to
        // try greedily, most specific ingredients first).
        let mut used = vec![false; filled.len()];
        let mut wanted: Vec<Ingredient> = self.items.to_vec();
        wanted.sort_by_key(|i| matches!(i, G(_)));
        wanted.iter().all(|&want| {
            let found = filled.iter().enumerate().find(|(n, s)| !used[*n] && matches(Some(**s), want));
            found.map(|(n, _)| used[n] = true).is_some()
        })
    }
}

// The tool shapes of tools.lua, for a material.
const STICK: Ingredient = G("stick");
const fn pick(m: Ingredient) -> [Ingredient; 9] {
    [m, m, m, E, STICK, E, E, STICK, E]
}
const fn shovel(m: Ingredient) -> [Ingredient; 3] {
    [m, STICK, STICK]
}
const fn axe(m: Ingredient) -> [Ingredient; 6] {
    [m, m, m, STICK, E, STICK]
}
const fn sword(m: Ingredient) -> [Ingredient; 3] {
    [m, m, STICK]
}
const fn hoe(m: Ingredient) -> [Ingredient; 6] {
    [m, m, E, STICK, E, STICK]
}

/// The box (min x, min y, width, height) around the filled cells of a
/// grid of `width` columns, or None if it is empty (craftGetBounds).
fn bounds(filled: impl Iterator<Item = bool>, width: usize) -> Option<(usize, usize, usize, usize)> {
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for (i, f) in filled.enumerate() {
        if !f {
            continue;
        }
        let (x, y) = (i % width, i / width);
        b = Some(match b {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    b.map(|(x0, y0, x1, y1)| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

fn matches(cell: Option<Stack>, want: Ingredient) -> bool {
    match (cell, want) {
        (None, E) => true,
        (Some(s), I(id)) => s.id == id,
        (Some(s), G(group)) => content::group_of(s.id, group) > 0,
        _ => false,
    }
}

impl Recipe {
    fn fits(&self, grid: &[Option<Stack>; GRID_SIZE]) -> bool {
        fits(grid, self.width, self.pattern)
    }
}

/// Whether the grid holds `pattern` (rows of `width` cells), anywhere in it.
fn fits(grid: &[Option<Stack>; GRID_SIZE], width: usize, pattern: &[Ingredient]) -> bool {
    let Some((gx, gy, gw, gh)) = bounds(grid.iter().map(Option::is_some), GRID_WIDTH) else {
        return false;
    };
    let Some((rx, ry, rw, rh)) = bounds(pattern.iter().map(|c| !matches!(c, E)), width) else {
        return false;
    };
    if (gw, gh) != (rw, rh) {
        return false;
    }
    (0..gh).all(|y| {
        (0..gw).all(|x| {
            let cell = grid[(gy + y) * GRID_WIDTH + gx + x];
            matches(cell, pattern[(ry + y) * width + rx + x])
        })
    })
}

/// The stairs mod's recipes (see stairs::recipes).
fn stair_result(grid: &[Option<Stack>; GRID_SIZE]) -> Option<(u16, u16)> {
    crate::stairs::recipes()
        .find(|(width, cells, item, _, _)| {
            let pattern = cells.map(|c| if c == 0 { E } else { I(*item) });
            // Empty cells after the pattern change nothing: only the box
            // round the filled ones counts.
            fits(grid, *width, &pattern)
        })
        .map(|(_, _, _, output, count)| (output, count))
}

/// What the grid would make: (item, count).
pub fn result(grid: &[Option<Stack>; GRID_SIZE]) -> Option<(u16, u16)> {
    RECIPES
        .iter()
        .find(|r| r.fits(grid))
        .map(|r| (r.output, r.count))
        .or_else(|| SHAPELESS.iter().find(|r| r.fits(grid)).map(|r| (r.output, r.count)))
        .or_else(|| stair_result(grid))
}

/// Craft once: use one item from each filled cell and give the result.
pub fn craft(grid: &mut [Option<Stack>; GRID_SIZE]) -> Option<(u16, u16)> {
    let made = result(grid)?;
    for cell in grid.iter_mut() {
        if let Some(s) = cell {
            s.count -= 1;
            if s.count == 0 {
                *cell = None;
            }
        }
    }
    Some(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cells: &[(usize, u16, u16)]) -> [Option<Stack>; GRID_SIZE] {
        let mut g = [None; GRID_SIZE];
        for &(i, id, count) in cells {
            g[i] = Some(Stack::new(id, count));
        }
        g
    }

    #[test]
    fn anywhere_in_the_grid() {
        assert_eq!(result(&grid(&[(4, CONTENT_BIRCH_TREE, 1)])), Some((CONTENT_BIRCH_WOOD, 4)));
        assert_eq!(result(&grid(&[(8, CONTENT_FIR_WOOD, 1)])), Some((ITEM_STICK, 4)));
        // Coal above a stick, in the right-hand column.
        assert_eq!(result(&grid(&[(2, ITEM_COAL_LUMP, 1), (5, ITEM_STICK, 1)])), Some((CONTENT_TORCH, 4)));
        // Upside down is no torch.
        assert_eq!(result(&grid(&[(5, ITEM_COAL_LUMP, 1), (2, ITEM_STICK, 1)])), None);
        // Not in line.
        assert_eq!(result(&grid(&[(0, ITEM_COAL_LUMP, 1), (4, ITEM_STICK, 1)])), None);
    }

    #[test]
    fn tools() {
        let (w, c, s) = (CONTENT_BIRCH_WOOD, CONTENT_COBBLE, ITEM_STICK);
        let pick = grid(&[(0, w, 1), (1, CONTENT_FIR_WOOD, 1), (2, w, 1), (4, s, 1), (7, s, 1)]);
        assert_eq!(result(&pick), Some((TOOL_PICK_WOOD, 1)));
        let pick = grid(&[(0, c, 1), (1, CONTENT_STONE, 1), (2, c, 1), (4, s, 1), (7, s, 1)]);
        assert_eq!(result(&pick), Some((TOOL_PICK_STONE, 1)));
        // Mixed wood and stone makes nothing.
        let mixed = grid(&[(0, c, 1), (1, w, 1), (2, c, 1), (4, s, 1), (7, s, 1)]);
        assert_eq!(result(&mixed), None);
        // Axe in the right two columns; its mirror image is no axe.
        assert_eq!(result(&grid(&[(1, w, 1), (2, w, 1), (4, w, 1), (5, s, 1), (8, s, 1)])), Some((TOOL_AXE_WOOD, 1)));
        assert_eq!(result(&grid(&[(1, w, 1), (2, w, 1), (5, w, 1), (4, s, 1), (7, s, 1)])), None);
        assert_eq!(result(&grid(&[(2, c, 1), (5, s, 1), (8, s, 1)])), Some((TOOL_SHOVEL_STONE, 1)));
        assert_eq!(result(&grid(&[(0, w, 1), (3, w, 1), (6, s, 1)])), Some((TOOL_SWORD_WOOD, 1)));
        // Furnace: a ring of stone and cobblestone, mixed.
        let ring: Vec<(usize, u16, u16)> =
            [0, 1, 2, 3, 5, 6, 7, 8].iter().map(|&i| (i, if i % 2 == 0 { c } else { CONTENT_STONE }, 1)).collect();
        assert_eq!(result(&grid(&ring)), Some((CONTENT_FURNACE, 1)));
        // Chests from planks; locked with a steel ingot in the middle, or
        // from a chest and an ingot anywhere.
        let planks: Vec<(usize, u16, u16)> = [0, 1, 2, 3, 5, 6, 7, 8].iter().map(|&i| (i, w, 1)).collect();
        assert_eq!(result(&grid(&planks)), Some((CONTENT_CHEST, 1)));
        let mut locked = planks.clone();
        locked.push((4, ITEM_STEEL_INGOT, 1));
        assert_eq!(result(&grid(&locked)), Some((CONTENT_CHEST_LOCKED, 1)));
        assert_eq!(result(&grid(&[(7, CONTENT_CHEST, 1), (2, ITEM_STEEL_INGOT, 1)])), Some((CONTENT_CHEST_LOCKED, 1)));
        assert_eq!(result(&grid(&[(7, CONTENT_CHEST, 1), (2, ITEM_STEEL_INGOT, 1), (3, ITEM_STICK, 1)])), None);
        // Bricks, blocks, ladders.
        let two: Vec<(usize, u16, u16)> = [4, 5, 7, 8].iter().map(|&i| (i, CONTENT_STONE, 1)).collect();
        assert_eq!(result(&grid(&two)), Some((CONTENT_STONE_BRICK, 4)));
        let nine: Vec<(usize, u16, u16)> = (0..9).map(|i| (i, ITEM_STEEL_INGOT, 1)).collect();
        assert_eq!(result(&grid(&nine)), Some((CONTENT_STEEL_BLOCK, 1)));
        let h: Vec<(usize, u16, u16)> = [0, 2, 3, 4, 5, 6, 8].iter().map(|&i| (i, s, 1)).collect();
        assert_eq!(result(&grid(&h)), Some((CONTENT_LADDER_WOOD, 5)));
        assert_eq!(cook_result(CONTENT_SAND), Some((CONTENT_GLASS, 3.0)));
        let mut full = ring.clone();
        full.push((4, c, 1));
        assert_eq!(result(&grid(&full)), None);
    }

    #[test]
    fn tnt() {
        let g = crate::tnt::GUNPOWDER;
        assert_eq!(result(&grid(&[(3, ITEM_COAL_LUMP, 1), (7, CONTENT_GRAVEL, 1)])), Some((g, 5)));
        let sticks = grid(&[(0, g, 1), (2, g, 1), (3, g, 1), (4, ITEM_PAPER, 1), (5, g, 1), (6, g, 1), (8, g, 1)]);
        assert_eq!(result(&sticks), Some((ITEM_TNT_STICK, 2)));
        let nine: Vec<(usize, u16, u16)> = (0..9).map(|i| (i, ITEM_TNT_STICK, 1)).collect();
        assert_eq!(result(&grid(&nine)), Some((crate::tnt::TNT, 1)));
        assert_eq!(result(&grid(&[(0, ITEM_FLINT, 1), (1, ITEM_STEEL_INGOT, 1)])), Some((TOOL_FLINT_AND_STEEL, 1)));
        let net = [(0, ITEM_STRING, 1), (1, ITEM_STRING, 1), (3, ITEM_STRING, 1), (4, ITEM_STRING, 1), (6, ITEM_STICK, 1)];
        assert_eq!(result(&grid(&net)), Some((TOOL_BUG_NET, 1)));
        assert_eq!(result(&grid(&[(1, crate::insects::FIREFLY, 1), (4, VESSELS_GLASS_BOTTLE, 1)])), Some((crate::insects::FIREFLY_BOTTLE, 1)));
    }

    #[test]
    fn stairs_and_slabs() {
        use crate::stairs::{shaped, Shape};
        let c = CONTENT_COBBLE;
        let stair = shaped(Shape::Stair, c).unwrap();
        let slab = shaped(Shape::Slab, c).unwrap();
        let steps: Vec<(usize, u16, u16)> = [2, 4, 5, 6, 7, 8].iter().map(|&i| (i, c, 1)).collect();
        assert_eq!(result(&grid(&steps)), Some((stair, 8)));
        // Mirrored: no stairs.
        let mirrored: Vec<(usize, u16, u16)> = [0, 3, 4, 6, 7, 8].iter().map(|&i| (i, c, 1)).collect();
        assert_eq!(result(&grid(&mirrored)), None);
        assert_eq!(result(&grid(&[(3, c, 1), (4, c, 1), (5, c, 1)])), Some((slab, 6)));
        let four: Vec<(usize, u16, u16)> = [1, 2, 4, 5].iter().map(|&i| (i, stair, 1)).collect();
        assert_eq!(result(&grid(&four)), Some((c, 3)));
        assert_eq!(result(&grid(&[(2, slab, 1), (5, slab, 1)])), Some((c, 1)));
        // Side by side, two slabs are nothing.
        assert_eq!(result(&grid(&[(4, slab, 1), (5, slab, 1)])), None);
        // Wooden ones are fuel, for their share of the planks' time.
        assert_eq!(burn_time(shaped(Shape::Stair, CONTENT_APPLE_WOOD).unwrap()), 5.0);
        assert_eq!(burn_time(stair), 0.0);
        // Corner stairs: 7 inner, 6 outer.
        let inner: Vec<(usize, u16, u16)> = [1, 3, 5, 6, 7, 8].iter().map(|&i| (i, c, 1)).collect();
        assert_eq!(result(&grid(&inner)), Some((shaped(Shape::Inner, c).unwrap(), 7)));
        let outer: Vec<(usize, u16, u16)> = [4, 6, 7, 8].iter().map(|&i| (i, c, 1)).collect();
        assert_eq!(result(&grid(&outer)), Some((shaped(Shape::Outer, c).unwrap(), 6)));
        // Two stone in a row are still nothing, three a slab.
        assert_eq!(result(&grid(&[(0, CONTENT_STONE, 1), (1, CONTENT_STONE, 1)])), None);
    }

    #[test]
    fn farming() {
        let (w, s, c) = (CONTENT_BIRCH_WOOD, ITEM_STICK, ITEM_COTTON);
        assert_eq!(result(&grid(&[(1, w, 1), (2, w, 1), (5, s, 1), (8, s, 1)])), Some((TOOL_HOE_WOOD, 1)));
        // The axe's shape is still an axe.
        assert_eq!(result(&grid(&[(1, w, 1), (2, w, 1), (4, w, 1), (5, s, 1), (8, s, 1)])), Some((TOOL_AXE_WOOD, 1)));
        assert_eq!(result(&grid(&[(0, c, 1), (1, c, 1), (3, c, 1), (4, c, 1)])), Some((farming::WOOL_WHITE, 1)));
        assert_eq!(result(&grid(&[(2, c, 1), (5, c, 1)])), Some((ITEM_STRING, 2)));
        assert_eq!(result(&grid(&[(0, ITEM_WHEAT, 1), (4, ITEM_WHEAT, 1), (6, ITEM_WHEAT, 1), (8, ITEM_WHEAT, 1)])), Some((ITEM_FLOUR, 1)));
        let nine: Vec<(usize, u16, u16)> = (0..9).map(|i| (i, ITEM_WHEAT, 1)).collect();
        assert_eq!(result(&grid(&nine)), Some((farming::STRAW, 3)));
        assert_eq!(cook_result(ITEM_FLOUR), Some((ITEM_BREAD, 15.0)));
        assert_eq!(burn_time(farming::GRASS[0]), 2.0);
    }

    #[test]
    fn beds() {
        let (w, a, x) = (farming::WOOL_WHITE, CONTENT_APPLE_WOOD, CONTENT_FIR_WOOD);
        let simple = [(3, w, 1), (4, w, 1), (5, w, 1), (6, a, 1), (7, x, 1), (8, a, 1)];
        assert_eq!(result(&grid(&simple)), Some((crate::beds::BED_BOTTOM, 1)));
        let mut fancy = simple.to_vec();
        fancy.push((2, ITEM_STICK, 1));
        assert_eq!(result(&grid(&fancy)), Some((crate::beds::FANCY_BOTTOM, 1)));
        // Six planks are still trapdoors.
        let planks: Vec<(usize, u16, u16)> = (3..9).map(|i| (i, a, 1)).collect();
        assert_eq!(result(&grid(&planks)), Some((crate::doors::FIRST_TRAPDOOR, 2)));
        assert_eq!(burn_time(crate::beds::FANCY_BOTTOM), 13.0);
    }

    #[test]
    fn fences() {
        let (a, f, s) = (CONTENT_APPLE_WOOD, CONTENT_FIR_WOOD, ITEM_STICK);
        let fence = [(0, f, 1), (1, s, 1), (2, f, 1), (3, f, 1), (4, s, 1), (5, f, 1)];
        assert_eq!(result(&grid(&fence)), Some((fences::FENCE_FIR, 4)));
        let gate = [(3, s, 1), (4, a, 1), (5, s, 1), (6, s, 1), (7, a, 1), (8, s, 1)];
        assert_eq!(result(&grid(&gate)), Some((fences::GATE_APPLE, 1)));
        // Woods don't mix.
        let mixed = [(0, f, 1), (1, s, 1), (2, a, 1), (3, f, 1), (4, s, 1), (5, f, 1)];
        assert_eq!(result(&grid(&mixed)), None);
        assert_eq!(burn_time(fences::FENCE_BIRCH), 5.0);
        assert_eq!(burn_time(fences::GATE_APPLE), 7.0);
        assert_eq!(burn_time(fences::GATE_APPLE + 1), 0.0);
        let rail = [(0, a, 1), (1, a, 1), (6, a, 1), (7, a, 1)];
        assert_eq!(result(&grid(&rail)), Some((fences::RAIL_APPLE, 16)));
        assert_eq!(burn_time(fences::RAIL_BIRCH), 3.0);
        let j = CONTENT_JUNGLE_WOOD;
        let rail = [(0, j, 1), (1, j, 1), (6, j, 1), (7, j, 1)];
        assert_eq!(result(&grid(&rail)), Some((fences::RAIL_JUNGLE, 16)));
        assert_eq!(result(&grid(&[(4, CONTENT_ACACIA_TREE, 1)])), Some((CONTENT_ACACIA_WOOD, 4)));
        assert_eq!(burn_time(fences::GATE_JUNGLE), 9.0);
        assert_eq!(burn_time(crate::stairs::LATER.first_slab() + 1), 4.0);
        let c = plants::CACTUS;
        let plus = [(1, c, 1), (3, c, 1), (4, c, 1), (5, c, 1), (7, c, 1)];
        assert_eq!(result(&grid(&plus)), Some((plants::LARGE_CACTUS_SEEDLING, 1)));
        assert_eq!(result(&grid(&[(0, plants::FIR_BUSH_STEM, 1)])), Some((CONTENT_FIR_WOOD, 1)));
        assert_eq!(burn_time(plants::CACTUS), 15.0);
    }

    #[test]
    fn ores() {
        let (cu, sn) = (ITEM_COPPER_INGOT, ITEM_TIN_INGOT);
        let mut bronze: Vec<(usize, u16, u16)> = (0..9).filter(|&i| i != 4).map(|i| (i, cu, 1)).collect();
        bronze.push((4, sn, 1));
        assert_eq!(result(&grid(&bronze)), Some((ITEM_BRONZE_INGOT, 9)));
        let d = ITEM_DIAMOND;
        assert_eq!(result(&grid(&[(0, d, 1), (1, d, 1), (2, d, 1), (4, ITEM_STICK, 1), (7, ITEM_STICK, 1)])), Some((TOOL_PICK_DIAMOND, 1)));
        assert_eq!(result(&grid(&[(4, ITEM_MESE_CRYSTAL, 1)])), Some((ITEM_MESE_FRAGMENT, 9)));
        let blocks: Vec<(usize, u16, u16)> = (0..9).map(|i| (i, ITEM_GOLD_INGOT, 1)).collect();
        assert_eq!(result(&grid(&blocks)), Some((CONTENT_GOLD_BLOCK, 1)));
        assert_eq!(cook_result(ITEM_TIN_LUMP), Some((ITEM_TIN_INGOT, 3.0)));
        // Digging: a diamond block wants a mese or diamond pickaxe; a steel
        // one does nothing. Diamond ore gives a diamond.
        let caps = |t: u16| &content::TOOLS.iter().find(|x| x.id == t).unwrap().caps;
        assert!(content::dig_params(CONTENT_DIAMOND_BLOCK, caps(TOOL_PICK_STEEL), 0).is_none());
        assert!(content::dig_params(CONTENT_DIAMOND_BLOCK, caps(TOOL_PICK_DIAMOND), 0).is_some());
        assert!(content::dig_params(CONTENT_DIAMOND_ORE, caps(TOOL_PICK_STEEL), 0).is_some());
        assert!(content::dig_params(CONTENT_DIAMOND_ORE, caps(TOOL_PICK_STONE), 0).is_none());
        assert_eq!(content::drop_of(CONTENT_DIAMOND_ORE), Some((ITEM_DIAMOND, 1)));
    }

    #[test]
    fn ground_blocks() {
        let four = |id: u16| -> Vec<(usize, u16, u16)> { [0, 1, 3, 4].iter().map(|&i| (i, id, 1)).collect() };
        assert_eq!(result(&grid(&four(ITEM_CLAY_LUMP))), Some((CONTENT_CLAY, 1)));
        assert_eq!(result(&grid(&four(ITEM_CLAY_BRICK))), Some((CONTENT_BRICK, 1)));
        assert_eq!(result(&grid(&four(CONTENT_DESERT_SAND))), Some((CONTENT_DESERT_SANDSTONE, 1)));
        assert_eq!(result(&grid(&four(CONTENT_DESERT_STONE))), Some((CONTENT_DESERT_STONE_BRICK, 4)));
        let nine: Vec<(usize, u16, u16)> = (0..9).map(|i| (i, CONTENT_SNOW_LAYER, 1)).collect();
        assert_eq!(result(&grid(&nine)), Some((CONTENT_SNOW_BLOCK, 1)));
        assert_eq!(cook_result(ITEM_CLAY_LUMP), Some((ITEM_CLAY_BRICK, 3.0)));
        assert_eq!(cook_result(CONTENT_SILVER_SAND), Some((CONTENT_GLASS, 3.0)));
        assert_eq!(cook_result(CONTENT_DESERT_COBBLE), Some((CONTENT_DESERT_STONE, 3.0)));
        assert_eq!(cook_result(ITEM_OBSIDIAN_SHARD), Some((CONTENT_OBSIDIAN_GLASS, 3.0)));
        // Desert cobblestone is stone for tools and furnaces.
        let c = CONTENT_DESERT_COBBLE;
        assert_eq!(result(&grid(&[(0, c, 1), (1, c, 1), (2, c, 1), (4, ITEM_STICK, 1), (7, ITEM_STICK, 1)])), Some((TOOL_PICK_STONE, 1)));
        assert_eq!(content::drop_of(CONTENT_CLAY), Some((ITEM_CLAY_LUMP, 4)));
        assert_eq!(burn_time(crate::stairs::BUILDING.first_stair()), 0.0);
    }

    #[test]
    fn dyes_and_wool() {
        assert_eq!(result(&grid(&[(4, colours::flower(0), 1)])), Some((colours::dye(colours::RED), 4)));
        assert_eq!(result(&grid(&[(0, ITEM_COAL_LUMP, 1)])), Some((colours::dye(colours::BLACK), 4)));
        // Red and blue, anywhere: violet.
        let mix = [(0, colours::dye(colours::BLUE), 1), (8, colours::dye(colours::RED), 1)];
        assert_eq!(result(&grid(&mix)), Some((colours::dye(colours::VIOLET), 2)));
        // White wool and pink dye: pink wool; that and blue dye: blue wool.
        let pink = [(3, farming::WOOL_WHITE, 1), (5, colours::dye(colours::PINK), 1)];
        assert_eq!(result(&grid(&pink)), Some((colours::wool(colours::PINK), 1)));
        let blue = [(3, colours::wool(colours::PINK), 1), (1, colours::dye(colours::BLUE), 1)];
        assert_eq!(result(&grid(&blue)), Some((colours::wool(colours::BLUE), 1)));
        // A bed from mixed wool.
        let (w, a) = (colours::wool(colours::RED), CONTENT_APPLE_WOOD);
        let bed = [(3, w, 1), (4, farming::WOOL_WHITE, 1), (5, w, 1), (6, a, 1), (7, a, 1), (8, a, 1)];
        assert_eq!(result(&grid(&bed)), Some((crate::beds::BED_BOTTOM, 1)));
    }

    #[test]
    fn panes() {
        use crate::panes::*;
        let rows = |id: u16, top: usize| grid(&(top..top + 6).map(|i| (i, id, 1)).collect::<Vec<_>>());
        assert_eq!(result(&rows(CONTENT_GLASS, 0)), Some((PANE_FLAT, 16)));
        assert_eq!(result(&rows(CONTENT_OBSIDIAN_GLASS, 3)), Some((OBSIDIAN_PANE_FLAT, 16)));
        assert_eq!(result(&rows(ITEM_STEEL_INGOT, 0)), Some((BAR_FLAT, 16)));
        assert_eq!(result(&rows(CONTENT_COBBLE, 3)), Some((crate::fences::WALL_COBBLE, 6)));
        assert_eq!(result(&rows(CONTENT_DESERT_COBBLE, 0)), Some((crate::fences::WALL_DESERT_COBBLE, 6)));
        assert_eq!(result(&rows(crate::dungeons::MOSSYCOBBLE, 0)), Some((crate::dungeons::WALL_MOSSYCOBBLE, 6)));
        assert_eq!(cook_result(crate::dungeons::MOSSYCOBBLE), Some((CONTENT_STONE, 3.0)));
        // A mese lamp: glass with a mese crystal under it, anywhere.
        for top in [0, 1, 2, 3, 4, 5] {
            assert_eq!(result(&grid(&[(top, CONTENT_GLASS, 1), (top + 3, ITEM_MESE_CRYSTAL, 1)])), Some((CONTENT_MESE_LAMP, 1)));
        }
        // Paper, books and bookshelves.
        assert_eq!(result(&grid(&[(3, crate::plants::PAPYRUS, 1), (4, crate::plants::PAPYRUS, 1), (5, crate::plants::PAPYRUS, 1)])), Some((ITEM_PAPER, 1)));
        assert_eq!(result(&grid(&[(2, ITEM_PAPER, 1), (5, ITEM_PAPER, 1), (8, ITEM_PAPER, 1)])), Some((ITEM_BOOK, 1)));
        let shelf: Vec<(usize, u16, u16)> = (0..9).map(|i| (i, if (3..6).contains(&i) { ITEM_BOOK } else { CONTENT_BIRCH_WOOD }, 1)).collect();
        assert_eq!(result(&grid(&shelf)), Some((CONTENT_BOOKSHELF, 1)));
        assert_eq!(burn_time(CONTENT_BOOKSHELF), 30.0);
        // Signs: two rows of planks or steel ingots over a stick make 3.
        let planks = grid(&[(0, CONTENT_FIR_WOOD, 1), (1, CONTENT_FIR_WOOD, 1), (2, CONTENT_FIR_WOOD, 1), (3, CONTENT_APPLE_WOOD, 1), (4, CONTENT_APPLE_WOOD, 1), (5, CONTENT_APPLE_WOOD, 1), (7, ITEM_STICK, 1)]);
        assert_eq!(result(&planks), Some((SIGN_WOOD, 3)));
        let steel = grid(&[(0, ITEM_STEEL_INGOT, 1), (1, ITEM_STEEL_INGOT, 1), (2, ITEM_STEEL_INGOT, 1), (3, ITEM_STEEL_INGOT, 1), (4, ITEM_STEEL_INGOT, 1), (5, ITEM_STEEL_INGOT, 1), (7, ITEM_STICK, 1)]);
        assert_eq!(result(&steel), Some((SIGN_STEEL, 3)));
        assert_eq!(burn_time(SIGN_WOOD), 10.0);
        // Mese post lights: glass, three mese crystals, planks, in a column
        // through the middle.
        for (post, wood) in MESE_POSTS {
            let g = grid(&[(1, CONTENT_GLASS, 1), (3, ITEM_MESE_CRYSTAL, 1), (4, ITEM_MESE_CRYSTAL, 1), (5, ITEM_MESE_CRYSTAL, 1), (7, wood, 1)]);
            assert_eq!(result(&g), Some((post, 4)));
            assert_eq!(content::light_source(post), MESE_LAMP_LIGHT);
            assert!(content::walkable(post) && content::passes_sunlight(post));
        }
        // Steel ingots in two columns are still a steel door.
        let cols = |id: u16| grid(&[0, 1, 3, 4, 6, 7].map(|i| (i, id, 1)));
        assert_eq!(result(&cols(ITEM_STEEL_INGOT)), Some((ITEM_DOOR_STEEL, 1)));
        assert_eq!(result(&cols(BAR_FLAT)), Some((ITEM_DOOR_STEEL_BAR, 1)));
        assert_eq!(result(&grid(&[0, 1, 3, 4].map(|i| (i, BAR_FLAT, 1)))), Some((BAR_TRAPDOOR, 1)));
    }

    /// The vessels, keys, screwdriver, binoculars and map mods' recipes,
    /// cooking and fuel.
    #[test]
    fn small_mods() {
        // vessels' bottle: {g, "", g}, {g, "", g}, {"", g, ""}.
        let cells = |id: u16| grid(&[0, 2, 3, 5, 7].map(|i| (i, id, 1)));
        let mut cornered = cells(CONTENT_GLASS);
        cornered[8] = Some(Stack::new(CONTENT_GLASS, 1));
        assert_eq!(result(&cornered), None, "a bottle with a corner is nothing");
        assert_eq!(result(&cells(CONTENT_GLASS)), Some((VESSELS_GLASS_BOTTLE, 10)));
        assert_eq!(result(&cells(ITEM_STEEL_INGOT)), Some((VESSELS_STEEL_BOTTLE, 5)));
        let cup = grid(&[0, 2, 3, 5, 6, 7, 8].map(|i| (i, CONTENT_GLASS, 1)));
        assert_eq!(result(&cup), Some((VESSELS_DRINKING_GLASS, 14)));
        let shelf: Vec<(usize, u16, u16)> =
            (0..9).map(|i| (i, [CONTENT_FIR_WOOD, VESSELS_GLASS_BOTTLE, CONTENT_APPLE_WOOD][i / 3], 1)).collect();
        assert_eq!(result(&grid(&shelf)), Some((VESSELS_SHELF, 1)));
        let mixed: Vec<(usize, u16, u16)> = (0..9)
            .map(|i| (i, if i == 4 { VESSELS_STEEL_BOTTLE } else if i == 5 { VESSELS_DRINKING_GLASS } else { [CONTENT_FIR_WOOD, VESSELS_GLASS_BOTTLE, CONTENT_APPLE_WOOD][i / 3] }, 1))
            .collect();
        assert_eq!(result(&grid(&mixed)), Some((VESSELS_SHELF, 1)), "any vessels");
        assert_eq!(result(&grid(&[(1, VESSELS_GLASS_BOTTLE, 1), (7, VESSELS_GLASS_BOTTLE, 1)])), Some((ITEM_GLASS_FRAGMENTS, 1)));
        assert_eq!(result(&grid(&[(0, VESSELS_DRINKING_GLASS, 1), (8, VESSELS_DRINKING_GLASS, 1)])), Some((ITEM_GLASS_FRAGMENTS, 1)));
        assert_eq!(result(&grid(&[(0, VESSELS_DRINKING_GLASS, 1), (8, VESSELS_GLASS_BOTTLE, 1)])), None);
        assert_eq!(cook_result(ITEM_GLASS_FRAGMENTS), Some((CONTENT_GLASS, 3.0)));
        assert_eq!(cook_result(VESSELS_STEEL_BOTTLE), Some((ITEM_STEEL_INGOT, 3.0)));
        assert_eq!(burn_time(VESSELS_SHELF), 30.0);
        // Nine jungle saplings: an emergent one.
        assert_eq!(result(&grid(&(0..9).map(|i| (i, CONTENT_JUNGLE_SAPLING, 1)).collect::<Vec<_>>())), Some((EMERGENT_JUNGLE_SAPLING, 1)));
        assert_eq!(burn_time(EMERGENT_JUNGLE_SAPLING), 7.0);
        // keys
        assert_eq!(result(&grid(&[(4, ITEM_GOLD_INGOT, 1)])), Some((ITEM_SKELETON_KEY, 1)));
        assert_eq!(cook_result(ITEM_KEY), Some((ITEM_GOLD_INGOT, 5.0)));
        assert_eq!(cook_result(ITEM_SKELETON_KEY), Some((ITEM_GOLD_INGOT, 5.0)));
        // screwdriver
        assert_eq!(result(&grid(&[(2, ITEM_STEEL_INGOT, 1), (5, ITEM_STICK, 1)])), Some((TOOL_SCREWDRIVER, 1)));
        // binoculars
        let o = CONTENT_OBSIDIAN_GLASS;
        let b = ITEM_BRONZE_INGOT;
        assert_eq!(result(&grid(&[(0, o, 1), (2, o, 1), (3, b, 1), (4, b, 1), (5, b, 1), (6, o, 1), (8, o, 1)])), Some((ITEM_BINOCULARS, 1)));
        // map
        let kit = grid(&[
            (0, CONTENT_GLASS, 1), (1, ITEM_PAPER, 1), (2, ITEM_STICK, 1),
            (3, ITEM_STEEL_INGOT, 1), (4, ITEM_PAPER, 1), (5, ITEM_STEEL_INGOT, 1),
            (6, CONTENT_JUNGLE_WOOD, 1), (7, ITEM_PAPER, 1), (8, colours::dye(colours::BLACK), 1),
        ]);
        assert_eq!(result(&kit), Some((ITEM_MAPPING_KIT, 1)));
        assert_eq!(burn_time(ITEM_MAPPING_KIT), 5.0);
        // boat: five planks (any wood) in a U, in the lower or upper rows.
        let (a, f) = (CONTENT_APPLE_WOOD, CONTENT_FIR_WOOD);
        let low = [(3, a, 1), (5, a, 1), (6, a, 1), (7, f, 1), (8, a, 1)];
        assert_eq!(result(&grid(&low)), Some((ITEM_BOAT, 1)));
        let high = [(0, a, 1), (2, a, 1), (3, a, 1), (4, a, 1), (5, a, 1)];
        assert_eq!(result(&grid(&high)), Some((ITEM_BOAT, 1)));
        assert_eq!(burn_time(ITEM_BOAT), 20.0);
        // rails and carts
        let (s, w, m, c) = (ITEM_STEEL_INGOT, CONTENT_BIRCH_WOOD, ITEM_MESE_CRYSTAL, ITEM_COAL_LUMP);
        let rails = |mid: Option<u16>| {
            let mut g = vec![(0, s, 1), (1, w, 1), (2, s, 1), (3, s, 1), (5, s, 1), (6, s, 1), (7, w, 1), (8, s, 1)];
            if let Some(mid) = mid {
                g.push((4, mid, 1));
            }
            result(&grid(&g))
        };
        assert_eq!(rails(None), Some((crate::carts::RAIL, 18)));
        assert_eq!(rails(Some(m)), Some((crate::carts::POWERRAIL, 18)));
        assert_eq!(rails(Some(c)), Some((crate::carts::BRAKERAIL, 18)));
        let cart = [(3, s, 1), (5, s, 1), (6, s, 1), (7, s, 1), (8, s, 1)];
        assert_eq!(result(&grid(&cart)), Some((ITEM_CART, 1)));
    }

    #[test]
    fn uses_one_of_each() {
        let mut g = grid(&[(0, ITEM_COAL_LUMP, 3), (3, ITEM_STICK, 1)]);
        assert_eq!(craft(&mut g), Some((CONTENT_TORCH, 4)));
        assert_eq!(g[0], Some(Stack::new(ITEM_COAL_LUMP, 2)));
        assert_eq!(g[3], None);
        // Coal lumps alone make black dye.
        assert_eq!(craft(&mut g), Some((colours::dye(colours::BLACK), 4)));
        assert_eq!(g[0], Some(Stack::new(ITEM_COAL_LUMP, 1)));
        let mut empty = grid(&[]);
        assert_eq!(craft(&mut empty), None);
    }
}
