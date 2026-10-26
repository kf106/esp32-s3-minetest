//! Terrain generation: heights, biomes, caves, ores and trees.
//!
//! Everything is a pure function of the node position (and the fixed seeds
//! below), so any node can be worked out on its own, and generating a whole
//! map block gives exactly the same nodes as asking one at a time. The block
//! path only caches intermediate values; both paths share the arithmetic.
//!
//! The noise is gradient (Perlin) noise built from integer hashing and
//! plain + and *, so it gives identical results on any machine.

use crate::colours;
use crate::farming;
use crate::plants;
use crate::content::{
    self, CONTENT_ACACIA_LEAVES, CONTENT_ACACIA_TREE, CONTENT_APPLE, CONTENT_APPLE_LEAVES, CONTENT_APPLE_TREE, CONTENT_BIRCH_LEAVES,
    CONTENT_BIRCH_TREE, CONTENT_COAL, CONTENT_DIRT, CONTENT_DRY_DIRT, CONTENT_DRY_GRASS, CONTENT_FIR_NEEDLES, CONTENT_FIR_TREE, CONTENT_GRASS,
    CONTENT_COPPER, CONTENT_DIAMOND_ORE, CONTENT_GOLD, CONTENT_IRON, CONTENT_JUNGLE_LEAVES, CONTENT_MESE_BLOCK, CONTENT_MESE_ORE, CONTENT_TIN, CONTENT_JUNGLE_TREE, CONTENT_LAVA, CONTENT_LITTER, CONTENT_SAND, CONTENT_SNOW,
    CONTENT_STONE,
};
use crate::content::{
    CONTENT_CLAY, CONTENT_CONIFEROUS_LITTER, CONTENT_DESERT_SAND, CONTENT_DESERT_STONE, CONTENT_GRAVEL, CONTENT_ICE, CONTENT_PERMAFROST,
    CONTENT_PERMAFROST_MOSS, CONTENT_PERMAFROST_STONES, CONTENT_SILVER_SAND, CONTENT_SNOW_LAYER, CONTENT_WATER,
};
use crate::content::{CONTENT_CAVE_ICE, CONTENT_SANDSTONE, CONTENT_SNOW_BLOCK};
use crate::world::CONTENT_AIR;

/// Bump when the generator changes, so worlds made by an older one are set
/// aside rather than mixed with new terrain (see World::open).
/// 3: savanna and rainforest, with acacia and jungle trees, in part of
/// what was desert.
///
/// The plants added after that (bushes, cacti, papyrus, ferns, marram
/// grass, kelp and corals) came without a bump: they only fill air, or
/// replace the top of a lake bed or swampy shore, so a world's changes
/// still fit around them.
/// 4: dungeons (dungeons.rs).
/// 5: rivers, the icesheet (with its cave ice), and emergent jungle trees.
/// 6: (no note was kept).
/// 7: each mapchunk's dungeons kept within a box inside it (dungeons.rs).
pub const GENERATOR_VERSION: u32 = 7;

const SEED_HILLS: u32 = 0x1A2B_3C4D;
const SEED_MOUNTAINS: u32 = 0x5E6F_7081;
const SEED_TEMPERATURE: u32 = 0x92A3_B4C5;
const SEED_CAVE_A: u32 = 0xD6E7_F809;
const SEED_CAVE_B: u32 = 0x1B2C_3D4E;
const SEED_ORE: u32 = 0x5F60_7182;
const SEED_ENTRANCE: u32 = 0x93A4_B5C6;
const SEED_TREES: u32 = 0xA7B8_C9DA;
const SEED_FOREST: u32 = 0xEB0C_1D2E;
const SEED_GRASS: u32 = 0x2F40_5162;
const SEED_COTTON: u32 = 0x7384_95A6;
const SEED_LAVA: u32 = 0xB7C8_D9EA;
const SEED_HUMIDITY: u32 = 0xC3D4_E5F6;
const SEED_SEABED: u32 = 0x4A5B_6C7D;
const SEED_SHORE: u32 = 0x8E9F_A0B1;
const SEED_FLOWERS: u32 = 0xC2D3_E4F5;
const SEED_SHRUBS: u32 = 0x0329_5157;
const SEED_RIVERS: u32 = 0x6B7C_8D9E;
const SEED_EMERGENT: u32 = 0x2685_0A0B;
const SEED_INSECTS: u32 = 0x5C1E_7F02;

/// Deep caves are flooded with lava up to this height, in about half the
/// land (where the "lava" noise is positive). Between flooded and dry
/// caves a band of solid stone (where the noise is near zero) seals the
/// lava in, so no wall of lava stands open beside a dry tunnel.
///
/// Added without bumping GENERATOR_VERSION: only caves this deep change,
/// so existing worlds keep their changes.
pub const LAVA_LEVEL: i32 = -64;
const LAVA_DAM: f32 = 0.04;

/// Caves are sampled every CAVE_GRID nodes and interpolated in between.
const CAVE_GRID: i32 = 4;
/// Tunnels are where both cave noises are near zero: a^2 + b^2 below this.
const CAVE_WIDTH: f32 = 0.006;
/// Snow covers everything at or above this height.
const SNOW_LINE: i32 = 36;
/// Where hot land turns from savanna to rainforest (wetter) or desert
/// (drier), and cold land from tundra to fir forest or cold desert.
const HUMID: f32 = 0.1;
/// Desert stone goes down to here under deserts.
const DESERT_STONE_BOTTOM: i32 = -256;
/// The temperate lands colder than this (of -0.18 to 0.18) are snowy
/// grassland where they aren't wet.
const SNOWY_GRASSLAND_COLD: f32 = -0.08;
/// The temperate lands drier than this are sandstone desert: rarer than
/// the hot and cold deserts' -HUMID, as in minetest_game, where it is one
/// of the smaller biomes.
const SANDSTONE_DRY: f32 = 0.25;
/// The cold, wet land turns from snowy fir forest to icesheet below this
/// temperature (minetest_game's icesheet: heat 0, beside the taiga's 25).
const ICESHEET_COLD: f32 = -0.4;
/// The icesheet's own nodes (snow blocks over cave ice) go down to here;
/// below it is its "ocean": sand over stone (minetest_game's icesheet
/// biome's y_min).
pub const ICESHEET_BOTTOM: i32 = -8;
/// How deep the icesheet's lakes are frozen (its depth_water_top).
const ICESHEET_ICE: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Biome {
    Grassland,
    Desert,
    Snow,
    /// Hot and in between: dry grass and acacias (minetest_game's savanna).
    Savanna,
    /// Hot and wet: jungle (minetest_game's rainforest).
    Rainforest,
    /// Cold and in between: permafrost, moss and stones, no trees
    /// (minetest_game's tundra).
    Tundra,
    /// Cold and dry: silver sand (minetest_game's cold desert).
    ColdDesert,
    /// The coldest and wet: snow blocks over cave ice, and frozen lakes
    /// (minetest_game's icesheet).
    Icesheet,
    /// Temperate and dry: sand over sandstone (minetest_game's sandstone
    /// desert).
    SandstoneDesert,
    /// Cool, neither wet nor dry: snowy dirt under a dusting of snow, with
    /// bushes (minetest_game's snowy grassland).
    SnowyGrassland,
}

/// What is known about a column of nodes before looking at caves and ores.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    /// The y of the generated surface node.
    pub height: i32,
    pub biome: Biome,
    /// Caves reach no higher than this: a few nodes under the surface,
    /// except at the occasional cave entrance.
    pub cave_top: i32,
    /// What deep caves (at or below LAVA_LEVEL) hold here.
    pub deep: Deep,
    /// Tundra here is mossy rather than stony.
    pub mossy: bool,
}

/// What fills caves at or below LAVA_LEVEL.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deep {
    Air,
    Lava,
    /// No cave: the stone between lava and air.
    Stone,
}

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

pub(crate) fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed
        ^ (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ (z as u32).wrapping_mul(0xCB1A_B31F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    h
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn floor_split(v: f32) -> (i32, f32) {
    let i = v as i32 - (v < 0.0 && v != (v as i32) as f32) as i32;
    (i, v - i as f32)
}

/// 2D gradient noise, roughly -1..1.
fn noise2(x: f32, z: f32, seed: u32) -> f32 {
    let (ix, fx) = floor_split(x);
    let (iz, fz) = floor_split(z);
    let grad = |cx: i32, cz: i32, dx: f32, dz: f32| {
        // Eight directions around the circle
        const G: [(f32, f32); 8] = [
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (0.7071, 0.7071),
            (-0.7071, 0.7071),
            (0.7071, -0.7071),
            (-0.7071, -0.7071),
        ];
        let g = G[(hash(cx, 0, cz, seed) & 7) as usize];
        g.0 * dx + g.1 * dz
    };
    let (u, v) = (fade(fx), fade(fz));
    let a = lerp(grad(ix, iz, fx, fz), grad(ix + 1, iz, fx - 1.0, fz), u);
    let b = lerp(grad(ix, iz + 1, fx, fz - 1.0), grad(ix + 1, iz + 1, fx - 1.0, fz - 1.0), u);
    lerp(a, b, v) * 1.4
}

/// 3D gradient noise, roughly -1..1.
fn noise3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (ix, fx) = floor_split(x);
    let (iy, fy) = floor_split(y);
    let (iz, fz) = floor_split(z);
    let grad = |cx: i32, cy: i32, cz: i32, dx: f32, dy: f32, dz: f32| {
        // The 12 edge directions of a cube (Perlin's improved noise)
        match hash(cx, cy, cz, seed) % 12 {
            0 => dx + dy,
            1 => -dx + dy,
            2 => dx - dy,
            3 => -dx - dy,
            4 => dx + dz,
            5 => -dx + dz,
            6 => dx - dz,
            7 => -dx - dz,
            8 => dy + dz,
            9 => -dy + dz,
            10 => dy - dz,
            _ => -dy - dz,
        }
    };
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let x00 = lerp(grad(ix, iy, iz, fx, fy, fz), grad(ix + 1, iy, iz, fx - 1.0, fy, fz), u);
    let x10 = lerp(
        grad(ix, iy + 1, iz, fx, fy - 1.0, fz),
        grad(ix + 1, iy + 1, iz, fx - 1.0, fy - 1.0, fz),
        u,
    );
    let x01 = lerp(
        grad(ix, iy, iz + 1, fx, fy, fz - 1.0),
        grad(ix + 1, iy, iz + 1, fx - 1.0, fy, fz - 1.0),
        u,
    );
    let x11 = lerp(
        grad(ix, iy + 1, iz + 1, fx, fy - 1.0, fz - 1.0),
        grad(ix + 1, iy + 1, iz + 1, fx - 1.0, fy - 1.0, fz - 1.0),
        u,
    );
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Several octaves of 2D noise, each half the size and strength of the
/// last, scaled back to roughly -1..1.
fn fbm2(x: f32, z: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut total) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * noise2(x * freq, z * freq, seed.wrapping_add(o * 7919));
        total += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / total
}

/// Luanti's NoiseFractal3D: octaves of 3D noise, each `lacunarity` times
/// finer and `persistence` times as strong as the last, summed (not
/// scaled back), times `scale`, plus `offset`. Luanti's noise is value
/// noise, which spreads wider than this gradient noise; VALUE_SPREAD
/// widens each octave to match, so thresholds like "above zero" pick out
/// about the same share of places.
pub fn fractal3(p: (i32, i32, i32), np: &NoiseParams, seed: u32) -> f32 {
    const VALUE_SPREAD: f32 = 1.8;
    let (mut sum, mut amp, mut freq) = (0.0, 1.0, 1.0);
    for o in 0..np.octaves {
        let f = freq / np.spread;
        sum += amp * noise3(p.0 as f32 * f, p.1 as f32 * f, p.2 as f32 * f, seed.wrapping_add(np.seed).wrapping_add(o * 7919));
        amp *= np.persistence;
        freq *= np.lacunarity;
    }
    np.offset + np.scale * sum * VALUE_SPREAD
}

/// Luanti's NoiseFractal2D, as fractal3 in two dimensions.
pub fn fractal2(x: i32, z: i32, np: &NoiseParams, seed: u32) -> f32 {
    const VALUE_SPREAD: f32 = 1.8;
    let (mut sum, mut amp, mut freq) = (0.0, 1.0, 1.0);
    for o in 0..np.octaves {
        let f = freq / np.spread;
        sum += amp * noise2(x as f32 * f, z as f32 * f, seed.wrapping_add(np.seed).wrapping_add(o * 7919));
        amp *= np.persistence;
        freq *= np.lacunarity;
    }
    np.offset + np.scale * sum * VALUE_SPREAD
}

/// Luanti's NoiseParams (one spread for all three axes).
pub struct NoiseParams {
    pub offset: f32,
    pub scale: f32,
    pub spread: f32,
    pub seed: u32,
    pub octaves: u32,
    pub persistence: f32,
    pub lacunarity: f32,
}

// ---------------------------------------------------------------------------
// Columns: height and biome
// ---------------------------------------------------------------------------

/// The height of the land before rivers cut through it.
pub fn land_height(x: i32, z: i32) -> i32 {
    let (fx, fz) = (x as f32, z as f32);
    // Rolling hills
    let hills = fbm2(fx / 160.0, fz / 160.0, 4, SEED_HILLS) * 22.0;
    // Mountain ranges rise where a broad noise is high
    let m = fbm2(fx / 600.0, fz / 600.0, 3, SEED_MOUNTAINS);
    let mountains = if m > 0.0 { m * m * 280.0 } else { 0.0 };
    (4.0 + hills + mountains).round() as i32
}

/// The humidity at a place, on the scale of minetest.get_humidity (50 on
/// average, mostly 0 to 100): the noise the biomes are chosen by.
pub fn humidity(x: i32, z: i32) -> f32 {
    50.0 + 100.0 * fbm2(x as f32 / 700.0, z as f32 / 700.0, 2, SEED_HUMIDITY)
}

pub fn column(x: i32, z: i32) -> Column {
    let (fx, fz) = (x as f32, z as f32);
    let height = land_height(x, z);
    // A river's channel cuts the land down to its bed.
    let height = river_bed(x, z).map_or(height, |bed| height.min(bed));

    let temperature = fbm2(fx / 800.0, fz / 800.0, 2, SEED_TEMPERATURE);
    // The hot lands are desert where it's dry, rainforest where it's wet,
    // and savanna in between.
    // The cold lands are snowy fir forest (taiga) where it's wet, cold
    // desert where it's dry, and tundra in between; mountain tops are
    // snowy anywhere.
    let biome = if height >= SNOW_LINE {
        Biome::Snow
    } else if temperature < -0.18 {
        let humidity = fbm2(fx / 700.0, fz / 700.0, 2, SEED_HUMIDITY);
        if humidity > HUMID && temperature < ICESHEET_COLD {
            Biome::Icesheet
        } else if humidity > HUMID {
            Biome::Snow
        } else if humidity < -HUMID {
            Biome::ColdDesert
        } else {
            Biome::Tundra
        }
    } else if temperature > 0.18 {
        let humidity = fbm2(fx / 700.0, fz / 700.0, 2, SEED_HUMIDITY);
        if humidity > HUMID {
            Biome::Rainforest
        } else if humidity < -HUMID {
            Biome::Desert
        } else {
            Biome::Savanna
        }
    } else {
        // The temperate lands: grassland, sandstone desert where it's dry,
        // and snowy grassland in their cooler half where it isn't wet.
        let humidity = fbm2(fx / 700.0, fz / 700.0, 2, SEED_HUMIDITY);
        if humidity < -SANDSTONE_DRY {
            Biome::SandstoneDesert
        } else if temperature < SNOWY_GRASSLAND_COLD && humidity <= HUMID {
            Biome::SnowyGrassland
        } else {
            Biome::Grassland
        }
    };
    // No cave mouths under water.
    let entrance = height > WATER_LEVEL && noise2(fx / 90.0, fz / 90.0, SEED_ENTRANCE) > 0.55;
    let cave_top = if entrance { height } else { height - 4 };
    let lava = noise2(fx / 200.0, fz / 200.0, SEED_LAVA);
    let deep = if lava > LAVA_DAM {
        Deep::Lava
    } else if lava > -LAVA_DAM {
        Deep::Stone
    } else {
        Deep::Air
    };
    let mossy = biome == Biome::Tundra && noise2(fx / 60.0, fz / 60.0, SEED_SHORE ^ 7) > 0.0;
    // The icesheet's land has a snow block on top of its surface
    // (minetest_game's node_dust), so stands one higher.
    let height = if biome == Biome::Icesheet && height >= WATER_LEVEL { height + 1 } else { height };
    Column { height, biome, cave_top, deep, mossy }
}

/// mgv7's river noise (np_ridge_uwater: spread 1000, seed 85039, 5
/// octaves, persistence 0.6).
const NP_RIVERS: NoiseParams =
    NoiseParams { offset: 0.0, scale: 1.0, spread: 1000.0, seed: 85039, octaves: 5, persistence: 0.6, lacunarity: 2.0 };
const RIVER_SHIFT: (i32, i32) = (7391, -2853);
/// mgv7's widest river channel (its "width").
const RIVER_WIDTH: f32 = 0.2;

/// The bed of the river channel in a column, if one runs through it: the
/// highest node left below the channel. mgv7's getRiverChannelFromMap
/// carves a node at height y where (width - |2 n|) * (y - water_level +
/// 17) / 2.5 >= 0.6, with n the river noise: the channel is deepest (9
/// below the water) where n is 0, and its sides climb ever more steeply
/// out to where |2 n| reaches the width, cutting a valley through hills.
/// Below the water level the channel fills with water, as mgv7's does.
/// (mgv7 also adds a 3D "ridge" noise above the water, which leaves
/// overhangs and arches in the valley's sides; here the sides are
/// smooth.)
pub fn river_bed(x: i32, z: i32) -> Option<i32> {
    river_bed_seeded(x, z, SEED_RIVERS)
}

/// river_bed with another seed (for choosing one).
pub fn river_bed_seeded(x: i32, z: i32, seed: u32) -> Option<i32> {
    // Shifted off the noise's lattice, where gradient noise is always
    // zero: else a river would run through the origin whatever the seed.
    let a = fractal2(x + RIVER_SHIFT.0, z + RIVER_SHIFT.1, &NP_RIVERS, seed).abs() * 2.0;
    channel_bed(a)
}

/// The bed of mgv7's river channel where |2 n| is `a`.
fn channel_bed(a: f32) -> Option<i32> {
    if a >= RIVER_WIDTH {
        return None;
    }
    let reach = 1.5 / (RIVER_WIDTH - a);
    if reach > 500.0 {
        return None;
    }
    // The lowest carved node, less one.
    Some(WATER_LEVEL - 17 + reach.ceil() as i32 - 1)
}

/// Lakes and ponds fill low ground with still water up to this height
/// (Luanti's default water_level). About an eighth of the land is under
/// it, mostly a few nodes deep.
pub const WATER_LEVEL: i32 = 1;

/// The layered node at height y, before caves and ores.
fn layer(y: i32, col: &Column) -> u16 {
    let depth = col.height - y; // 0 at the surface
    if depth < 0 {
        return CONTENT_AIR;
    }
    // minetest_game's icesheet: snow blocks (four deep, and the dust on
    // top on land), over cave ice down to ICESHEET_BOTTOM; its lake beds
    // below that are sand over stone (icesheet_ocean).
    if col.biome == Biome::Icesheet {
        let snow = if col.height > WATER_LEVEL { 4 } else { 3 };
        return match depth {
            _ if col.height < ICESHEET_BOTTOM => match depth {
                0..=3 => CONTENT_SAND,
                _ => CONTENT_STONE,
            },
            d if d <= snow => CONTENT_SNOW_BLOCK,
            _ if y >= ICESHEET_BOTTOM => CONTENT_CAVE_ICE,
            _ => CONTENT_STONE,
        };
    }
    // The rainforest's and savanna's shores are swampy dirt where papyrus
    // grows (minetest_game's rainforest_swamp and savanna_shore).
    if (0..=WATER_LEVEL).contains(&col.height) && matches!(col.biome, Biome::Rainforest | Biome::Savanna) {
        return match depth {
            0..=3 if col.biome == Biome::Savanna => CONTENT_DRY_DIRT,
            0..=3 => CONTENT_DIRT,
            _ => CONTENT_STONE,
        };
    }
    // The tundra's lakes have gravel beaches (minetest_game's
    // tundra_beach).
    if col.height <= WATER_LEVEL && col.biome == Biome::Tundra {
        return match depth {
            0..=1 => CONTENT_GRAVEL,
            _ => CONTENT_STONE,
        };
    }
    // Lake beds and shores are sand, as Minetest Game's beaches.
    if col.height <= WATER_LEVEL && !matches!(col.biome, Biome::Desert | Biome::SandstoneDesert) {
        return match depth {
            0..=1 => CONTENT_SAND,
            2..=3 => CONTENT_DIRT,
            _ => CONTENT_STONE,
        };
    }
    match col.biome {
        // Under the firs, coniferous litter (minetest_game's coniferous
        // forest).
        Biome::Grassland if col.height >= FIR_LINE => match depth {
            0 => CONTENT_CONIFEROUS_LITTER,
            1..=3 => CONTENT_DIRT,
            _ => CONTENT_STONE,
        },
        Biome::Grassland => match depth {
            0 => CONTENT_GRASS,
            1..=3 => CONTENT_DIRT,
            _ => CONTENT_STONE,
        },
        // minetest_game's savanna and rainforest: dry grass on dry dirt,
        // leaf litter on dirt.
        Biome::Savanna => match depth {
            0 => CONTENT_DRY_GRASS,
            1..=3 => CONTENT_DRY_DIRT,
            _ => CONTENT_STONE,
        },
        Biome::Rainforest => match depth {
            0 => CONTENT_LITTER,
            1..=3 => CONTENT_DIRT,
            _ => CONTENT_STONE,
        },
        Biome::Snow => match depth {
            0 => CONTENT_SNOW,
            1..=3 => CONTENT_DIRT,
            _ => CONTENT_STONE,
        },
        // minetest_game's desert: desert sand, and desert stone down to
        // -256 (its desert_ocean biome's bottom), without ores.
        Biome::Desert => match depth {
            0..=2 => CONTENT_DESERT_SAND,
            _ if y > DESERT_STONE_BOTTOM => CONTENT_DESERT_STONE,
            _ => CONTENT_STONE,
        },
        Biome::Tundra => match depth {
            0 if col.mossy => CONTENT_PERMAFROST_MOSS,
            0 => CONTENT_PERMAFROST_STONES,
            1..=2 => CONTENT_PERMAFROST,
            _ => CONTENT_STONE,
        },
        Biome::ColdDesert => match depth {
            0..=1 => CONTENT_SILVER_SAND,
            _ => CONTENT_STONE,
        },
        // minetest_game's sandstone desert: sand, and sandstone down to
        // -256 (its ocean biome's bottom), without ores; its lake beds are
        // the same.
        Biome::SandstoneDesert => match depth {
            0..=1 => CONTENT_SAND,
            _ if y > DESERT_STONE_BOTTOM => CONTENT_SANDSTONE,
            _ => CONTENT_STONE,
        },
        Biome::SnowyGrassland => match depth {
            0 => CONTENT_SNOW,
            1 => CONTENT_DIRT,
            _ => CONTENT_STONE,
        },
        // (Handled above.)
        Biome::Icesheet => CONTENT_SNOW_BLOCK,
    }
}

// ---------------------------------------------------------------------------
// Caves
// ---------------------------------------------------------------------------

/// The two cave noises at a grid point (grid coordinates = node / 4).
pub fn cave_sample(gx: i32, gy: i32, gz: i32) -> (f32, f32) {
    let (x, y, z) = ((gx * CAVE_GRID) as f32, (gy * CAVE_GRID) as f32, (gz * CAVE_GRID) as f32);
    (
        noise3(x / 40.0, y / 28.0, z / 40.0, SEED_CAVE_A),
        noise3(x / 40.0, y / 28.0, z / 40.0, SEED_CAVE_B),
    )
}

/// Whether a node is inside a tunnel, given the samples at the 8 grid
/// corners around it, in the order (x, y, z) = 000, 100, 010, 110, 001,
/// 101, 011, 111.
pub fn cave_from_corners(c: &[(f32, f32); 8], x: i32, y: i32, z: i32) -> bool {
    if !cell_may_have_cave(c) {
        return false;
    }
    let t = |v: i32| v.rem_euclid(CAVE_GRID) as f32 / CAVE_GRID as f32;
    let (tx, ty, tz) = (t(x), t(y), t(z));
    let tri = |i: usize| -> f32 {
        let get = |k: usize| if i == 0 { c[k].0 } else { c[k].1 };
        let x00 = lerp(get(0), get(1), tx);
        let x10 = lerp(get(2), get(3), tx);
        let x01 = lerp(get(4), get(5), tx);
        let x11 = lerp(get(6), get(7), tx);
        lerp(lerp(x00, x10, ty), lerp(x01, x11, ty), tz)
    };
    let (a, b) = (tri(0), tri(1));
    a * a + b * b < CAVE_WIDTH
}

/// Whether any node between these corners can be in a tunnel. Interpolated
/// values stay between the corner values, so if every corner of either
/// noise is clearly on one side of zero, the tunnel condition (both near
/// zero) can't hold anywhere in the cell. The margin covers rounding.
pub fn cell_may_have_cave(c: &[(f32, f32); 8]) -> bool {
    let limit = CAVE_WIDTH.sqrt() * 1.02;
    let clear = |get: &dyn Fn(usize) -> f32| {
        (0..8).all(|k| get(k) > limit) || (0..8).all(|k| get(k) < -limit)
    };
    !(clear(&|k| c[k].0) || clear(&|k| c[k].1))
}

fn grid_of(v: i32) -> i32 {
    v.div_euclid(CAVE_GRID)
}

pub fn corners_at(x: i32, y: i32, z: i32, sample: &mut impl FnMut(i32, i32, i32) -> (f32, f32)) -> [(f32, f32); 8] {
    let (gx, gy, gz) = (grid_of(x), grid_of(y), grid_of(z));
    let mut c = [(0.0, 0.0); 8];
    for (k, item) in c.iter_mut().enumerate() {
        let (dx, dy, dz) = ((k & 1) as i32, ((k >> 1) & 1) as i32, ((k >> 2) & 1) as i32);
        *item = sample(gx + dx, gy + dy, gz + dz);
    }
    c
}

// ---------------------------------------------------------------------------
// Ores
// ---------------------------------------------------------------------------

/// Ore cells are 8x8x8 nodes. A cell holds at most one small cluster, kept
/// inside the cell, so a node's ore depends on its own cell only.
const ORE_CELL: i32 = 8;

fn ore_at(x: i32, y: i32, z: i32) -> Option<u16> {
    let (cx, cy, cz) = (x.div_euclid(ORE_CELL), y.div_euclid(ORE_CELL), z.div_euclid(ORE_CELL));
    let h = hash(cx, cy, cz, SEED_ORE);
    // Coal anywhere, in half the cells (about 1 stone node in 60). Iron
    // only below y = 0, and rarer. The other ores, added later, take cells
    // that had none, each only from the depth where minetest_game starts
    // scattering it, at about its density there: copper and tin below
    // -64, gold below -256, mese below -512, diamond below -1024, and
    // mese blocks below -2048.
    let ore = match h % 100 {
        0..=49 => CONTENT_COAL,
        50..=64 if y < 0 => CONTENT_IRON,
        65..=76 if y < -64 => CONTENT_COPPER,
        77..=84 if y < -64 => CONTENT_TIN,
        85..=90 if y < -256 => CONTENT_GOLD,
        91..=95 if y < -512 => CONTENT_MESE_ORE,
        96..=98 if y < -1024 => CONTENT_DIAMOND_ORE,
        99 if y < -2048 => CONTENT_MESE_BLOCK,
        _ => return None,
    };
    // Cluster centre, 1..6 within the cell, so the 3x3x3 around it fits.
    let centre = |shift: u32| 1 + ((h >> shift) % 6) as i32;
    let (ox, oy, oz) = (
        cx * ORE_CELL + centre(8),
        cy * ORE_CELL + centre(12),
        cz * ORE_CELL + centre(16),
    );
    let (dx, dy, dz) = (x - ox, y - oy, z - oz);
    if dx.abs() > 1 || dy.abs() > 1 || dz.abs() > 1 {
        return None;
    }
    // An irregular blob: about two thirds of the 3x3x3, always the centre.
    let inside = (dx, dy, dz) == (0, 0, 0) || hash(x, y, z, SEED_ORE ^ 0x55) % 3 != 0;
    inside.then_some(ore)
}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

/// The finishing steps shared by single nodes and whole blocks.
pub fn finish(x: i32, y: i32, z: i32, col: &Column, is_cave: impl FnOnce() -> bool) -> u16 {
    let base = layer(y, col);
    if base == CONTENT_AIR {
        return CONTENT_AIR;
    }
    if y <= col.cave_top && is_cave() {
        match col.deep {
            _ if y > LAVA_LEVEL => return CONTENT_AIR,
            Deep::Air => return CONTENT_AIR,
            Deep::Lava => return CONTENT_LAVA,
            Deep::Stone => {}
        }
    }
    if base == CONTENT_STONE {
        if y < col.height - 4 && gravel_at(x, y, z) {
            return CONTENT_GRAVEL;
        }
        if let Some(ore) = ore_at(x, y, z) {
            return ore;
        }
    }
    if y == col.height {
        if let Some(id) = seabed(x, z, col) {
            return id;
        }
    }
    base
}

/// Gravel blobs underground: cells of GRAVEL_CELL nodes, one in four
/// with a rough ball of gravel inside, kept clear of its edges so a node
/// depends only on its own cell (minetest_game's gravel blob ore).
const GRAVEL_CELL: i32 = 16;

fn gravel_at(x: i32, y: i32, z: i32) -> bool {
    let (cx, cy, cz) = (x.div_euclid(GRAVEL_CELL), y.div_euclid(GRAVEL_CELL), z.div_euclid(GRAVEL_CELL));
    let h = hash(cx, cy, cz, SEED_SEABED ^ 5);
    if h % 4 != 0 {
        return false;
    }
    let centre = |shift: u32| 4 + ((h >> shift) % 8) as i32;
    let (dx, dy, dz) = (
        x - (cx * GRAVEL_CELL + centre(4)),
        y - (cy * GRAVEL_CELL + centre(8)),
        z - (cz * GRAVEL_CELL + centre(12)),
    );
    let r2 = 4 + ((h >> 16) % 6) as i32;
    dx * dx + dy * dy + dz * dz <= r2 + (hash(x, y, z, SEED_SEABED ^ 6) % 3) as i32
}

/// The water of a lake at height y (at most WATER_LEVEL): frozen at the
/// top in the cold lands.
pub fn lake_node(y: i32, col: &Column) -> u16 {
    // The icesheet's lakes are ice ten deep (its water_top).
    if col.biome == Biome::Icesheet && y > WATER_LEVEL - ICESHEET_ICE {
        return CONTENT_ICE;
    }
    if y == WATER_LEVEL && matches!(col.biome, Biome::Snow | Biome::Tundra | Biome::ColdDesert) {
        CONTENT_ICE
    } else {
        CONTENT_WATER
    }
}

/// What lies on the lake over a column: the icesheet's snow block on its
/// ice (minetest_game's node_dust), or a waterlily.
pub fn on_lake(x: i32, z: i32, col: &Column) -> Option<u16> {
    if col.biome == Biome::Icesheet {
        return Some(CONTENT_SNOW_BLOCK);
    }
    lily_on(x, z, col)
}

/// A waterlily on the water over a column, if one floats there: on the
/// shallow swampy lakes of the rainforest and savanna (minetest_game's
/// waterlily decoration), in patches.
pub fn lily_on(x: i32, z: i32, col: &Column) -> Option<u16> {
    if col.height != WATER_LEVEL - 1 || !matches!(col.biome, Biome::Rainforest | Biome::Savanna) {
        return None;
    }
    let h = hash(x, 0x111, z, SEED_FLOWERS);
    (h % 3 == 0 && noise2(x as f32 / 100.0, z as f32 / 100.0, SEED_FLOWERS ^ 0xF) > -0.1).then_some(colours::WATERLILY)
}

/// Kelp grows on lake beds this deep or deeper.
pub const KELP_TOP: i32 = -5;

/// What grows on a lake bed (minetest_game's coral reef and kelp
/// decorations): corals in warm water 2 to 8 nodes deep, kelp in cool
/// water 6 or more deep, in patches. Nothing under the icesheet's ice.
fn seabed(x: i32, z: i32, col: &Column) -> Option<u16> {
    if col.height > 0 || col.biome == Biome::Icesheet {
        return None;
    }
    let h = hash(x, 0x5EA, z, SEED_SEABED);
    // Clay in the shallows, gravel further out, in patches (minetest_game's
    // clay and gravel blobs on beaches and lake beds).
    let shallow = col.height >= -3;
    if col.biome != Biome::Desert && shallow && noise2(x as f32 / 40.0, z as f32 / 40.0, SEED_SEABED ^ 3) > 0.35 {
        return Some(CONTENT_CLAY);
    }
    if !shallow && noise2(x as f32 / 60.0, z as f32 / 60.0, SEED_SEABED ^ 4) > 0.4 {
        return Some(CONTENT_GRAVEL);
    }
    if col.height > -2 {
        return None;
    }
    let warm = matches!(col.biome, Biome::Desert | Biome::Savanna | Biome::Rainforest);
    if warm && col.height >= -8 {
        if noise2(x as f32 / 50.0, z as f32 / 50.0, SEED_SEABED) < 0.15 || h % 3 != 0 {
            return None;
        }
        const CORALS: [u16; 6] =
            [plants::CORAL_GREEN, plants::CORAL_PINK, plants::CORAL_CYAN, plants::CORAL_BROWN, plants::CORAL_ORANGE, plants::CORAL_SKELETON];
        return Some(CORALS[((h >> 8) % 6) as usize]);
    }
    if !warm && col.height <= KELP_TOP && noise2(x as f32 / 200.0, z as f32 / 200.0, SEED_SEABED ^ 1) > 0.0 && h % 5 == 0 {
        return Some(plants::KELP);
    }
    None
}

/// The param2 of a generated node, which only kelp has: its height (3 to
/// 6 nodes, in sixteenths), always within the water above it.
pub fn generated_param2(x: i32, y: i32, z: i32, id: u16) -> u8 {
    if id == content::DRY_SHRUB {
        return content::DRY_SHRUB_PARAM2;
    }
    if id != plants::KELP {
        return 0;
    }
    let height = 3 + (hash(x, y, z, SEED_SEABED ^ 2) % 4) as i32;
    (height.min(WATER_LEVEL - y) * 16) as u8
}

/// The generated node at one position. `sample` gives the cave noises at
/// a grid point: `cave_sample`, possibly through a cache.
pub fn node(
    x: i32,
    y: i32,
    z: i32,
    col: &Column,
    sample: &mut impl FnMut(i32, i32, i32) -> (f32, f32),
) -> u16 {
    finish(x, y, z, col, || {
        let c = corners_at(x, y, z, sample);
        cave_from_corners(&c, x, y, z)
    })
}

// ---------------------------------------------------------------------------
// Trees
// ---------------------------------------------------------------------------
//
// The ground is divided into TREE_CELL x TREE_CELL patches, each with one
// possible tree at a spot picked by hashing the patch. Whether it grows
// depends on the biome and on a broad "forest" noise, so there are woods
// and clearings with the odd lone tree between. Birches grow in the
// lowland grass, firs higher up and in the snow, jungle trees thickly in
// the rainforest, acacias here and there on the savanna, nothing in the
// desert.
//
// Trees only fill air: a node that the terrain makes solid stays solid.

/// Size of the patches that each hold at most one tree.
const TREE_CELL: i32 = 5;
/// How far a tree's leaves reach sideways from its trunk.
pub const TREE_REACH: i32 = 3;
/// Firs replace birches from this height up.
const FIR_LINE: i32 = 22;
/// No trees above this height (bare mountain tops).
const TREE_LINE: i32 = 70;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TreeKind {
    Birch,
    Fir,
    Apple,
    Jungle,
    Acacia,
    // minetest_game's bushes, cactus and papyrus (plants.rs): grown like
    // trees, from patches or saplings.
    Bush,
    BlueberryBush,
    AcaciaBush,
    FirBush,
    Cactus,
    LargeCactus,
    Papyrus,
    /// minetest_game's emergent jungle tree: a giant above the jungle.
    Emergent,
}

#[derive(Clone, Copy, Debug)]
pub struct Tree {
    /// The trunk's column.
    pub x: i32,
    pub z: i32,
    /// The y of the ground node it stands on.
    pub ground: i32,
    pub kind: TreeKind,
    /// Trunk length.
    pub height: i32,
}

impl Tree {
    /// The highest y any part of the tree reaches.
    pub fn top(&self) -> i32 {
        self.ground + self.crown_top()
    }

    /// The highest part of the tree, above the ground.
    fn crown_top(&self) -> i32 {
        match self.kind {
            // minetest_game's apple tree: leaves one node above the trunk's
            // top and its last node.
            TreeKind::Apple | TreeKind::Acacia => self.height + 2,
            TreeKind::Bush | TreeKind::AcaciaBush | TreeKind::FirBush => 2,
            TreeKind::BlueberryBush => 1,
            TreeKind::Cactus | TreeKind::Papyrus => self.height,
            TreeKind::LargeCactus => 6,
            TreeKind::Emergent => emergent_slices(self).count_ones() as i32 - 5,
            _ => self.height + 1,
        }
    }

    /// The tree's node at (x, y, z), if it has one there.
    pub fn node_at(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        if self.kind == TreeKind::Emergent {
            return emergent_node(self, x, y, z);
        }
        let (dx, dz) = (x - self.x, z - self.z);
        let dy = y - self.ground; // 1 = just above the ground
        let h = self.height;
        if dx.abs() > TREE_REACH || dz.abs() > TREE_REACH || dy < 1 || dy > self.crown_top() {
            return None;
        }
        let trunk = dx == 0 && dz == 0 && dy <= h;
        match self.kind {
            // (Handled above.)
            TreeKind::Emergent => None,
            TreeKind::Birch => {
                if trunk {
                    return Some(CONTENT_BIRCH_TREE);
                }
                // A rounded crown over the top of the trunk.
                let (ax, az) = (dx.abs(), dz.abs());
                let leafy = match h + 1 - dy {
                    0 => ax + az <= 1,                    // the very top
                    1 => ax <= 1 && az <= 1,
                    2 | 3 => {
                        // Wide layers, with ragged corners.
                        ax <= 2 && az <= 2 && !(ax == 2 && az == 2 && hash(x, y, z, SEED_TREES ^ 1) % 3 != 0)
                    }
                    _ => false,
                };
                leafy.then_some(CONTENT_BIRCH_LEAVES)
            }
            TreeKind::Fir => {
                if trunk {
                    return Some(CONTENT_FIR_TREE);
                }
                // A cone of needle layers, alternately wider and narrower,
                // from the tip down to two nodes above the ground.
                if dy < 3 {
                    return None;
                }
                const RADIUS: [i32; 12] = [0, 1, 1, 2, 1, 2, 2, 3, 2, 3, 3, 3];
                let r = RADIUS[((h + 1 - dy) as usize).min(RADIUS.len() - 1)];
                let leafy = match r {
                    0 => dx == 0 && dz == 0,
                    1 => dx.abs() + dz.abs() <= 1,
                    _ => dx * dx + dz * dz <= r * r,
                };
                leafy.then_some(CONTENT_FIR_NEEDLES)
            }
            TreeKind::Bush | TreeKind::AcaciaBush | TreeKind::FirBush => {
                // minetest_game's bush schematics: a stem, leaves round it
                // (the corners now and then) and a few on top; the acacia
                // bush is fuller on top and sparser round its stem.
                let (stem, leaves) = match self.kind {
                    TreeKind::Bush => (plants::BUSH_STEM, plants::BUSH_LEAVES),
                    TreeKind::AcaciaBush => (plants::ACACIA_BUSH_STEM, plants::ACACIA_BUSH_LEAVES),
                    _ => (plants::FIR_BUSH_STEM, plants::FIR_BUSH_NEEDLES),
                };
                let (ax, az) = (dx.abs(), dz.abs());
                if ax > 1 || az > 1 {
                    return None;
                }
                let maybe = || hash(x, y, z, SEED_TREES ^ 8) % 2 == 0;
                let acacia = self.kind == TreeKind::AcaciaBush;
                let leafy = match (dy, ax + az) {
                    (1, 0) => return Some(stem),
                    (1, 1) => !acacia || maybe(),
                    (1, _) => maybe(),
                    (_, 0) => true,
                    (_, 1) => acacia || maybe(),
                    _ => maybe(),
                };
                leafy.then_some(leaves)
            }
            TreeKind::BlueberryBush => {
                // A single layer of leaves, berries on them.
                let leafy = dx.abs() <= 1 && dz.abs() <= 1 && ((dx, dz) == (0, 0) || hash(x, y, z, SEED_TREES ^ 9) % 3 != 0);
                leafy.then_some(plants::BLUEBERRY_LEAVES_WITH_BERRIES)
            }
            TreeKind::Cactus => trunk.then_some(plants::CACTUS),
            TreeKind::Papyrus => trunk.then_some(plants::PAPYRUS),
            TreeKind::LargeCactus => {
                // minetest_game's large cactus: a trunk with two arms that
                // bend up, along x or z.
                let along_x = hash(self.x, self.ground, self.z, SEED_TREES ^ 10) % 2 == 0;
                let (a, b) = if along_x { (dx, dz) } else { (dz, dx) };
                if b != 0 {
                    return None;
                }
                let arm_tip = || hash(x, y, z, SEED_TREES ^ 11) % 2 == 0;
                let cactus = match dy {
                    1 | 2 | 6 => a == 0,
                    3 => a.abs() <= 2,
                    4 => a.abs() != 1,
                    5 => a == 0 || (a.abs() == 2 && arm_tip()),
                    _ => false,
                };
                cactus.then_some(plants::CACTUS)
            }
            TreeKind::Jungle => {
                if trunk {
                    return Some(CONTENT_JUNGLE_TREE);
                }
                // A tall trunk with a dense crown at the top, two nodes out
                // and four deep, rounded at top and bottom, and a few tufts
                // of leaves along the trunk further down.
                let (ax, az) = (dx.abs(), dz.abs());
                let rel = dy - h;
                let leafy = match rel {
                    1 => ax <= 1 && az <= 1,
                    0 | -1 => ax <= 2 && az <= 2 && !(ax == 2 && az == 2 && hash(x, y, z, SEED_TREES ^ 4) % 2 == 0),
                    -2 => ax + az <= 2,
                    -6 | -5 => ax + az == 1 && hash(x, y, z, SEED_TREES ^ 5) % 3 == 0,
                    _ => false,
                };
                leafy.then_some(CONTENT_JUNGLE_LEAVES)
            }
            TreeKind::Acacia => {
                // A short trunk that forks into two branches leaning out
                // diagonally, under a wide, flat crown (minetest_game's
                // acacia, simplified). Which diagonal is fixed per tree.
                let (sx, sz) = if hash(self.x, self.ground, self.z, SEED_TREES ^ 6) % 2 == 0 { (1, 1) } else { (1, -1) };
                let branch = |k: i32| (dx == k * sx && dz == k * sz) || (dx == -k * sx && dz == -k * sz);
                let wood = match h - dy {
                    0 => branch(2),
                    1 => branch(1),
                    d if d >= 2 => dx == 0 && dz == 0,
                    _ => false,
                };
                if wood {
                    return Some(CONTENT_ACACIA_TREE);
                }
                let (ax, az) = (dx.abs(), dz.abs());
                let leafy = match dy - h {
                    1 => ax + az <= 5 && !(ax + az == 5 && hash(x, y, z, SEED_TREES ^ 7) % 2 == 0),
                    2 => {
                        let near = |tx: i32, tz: i32| (dx - tx).abs() <= 1 && (dz - tz).abs() <= 1;
                        near(2 * sx, 2 * sz) || near(-2 * sx, -2 * sz)
                    }
                    _ => false,
                };
                leafy.then_some(CONTENT_ACACIA_LEAVES)
            }
            TreeKind::Apple => {
                if trunk {
                    return Some(CONTENT_APPLE_TREE);
                }
                // minetest_game's add_trunk_and_leaves (size 2): a 3x4x3
                // block of leaves around the top of the trunk, from two below
                // its top to one above, then 2x2x2 clusters scattered within
                // two nodes, here picked by hashing. One leaf in eight is an
                // apple.
                let (ax, az) = (dx.abs(), dz.abs());
                let rel = dy - h; // 0: level with the trunk's last node
                if !(-2..=2).contains(&rel) || ax > 2 || az > 2 {
                    return None;
                }
                let core = ax <= 1 && az <= 1 && rel <= 1;
                let cluster = || {
                    // Clusters are 2x2x2: share one hash per pair of nodes.
                    let (cx, cy, cz) = (x.div_euclid(2), y.div_euclid(2), z.div_euclid(2));
                    hash(cx, cy, cz, SEED_TREES ^ 2) % 3 == 0
                };
                if !core && !cluster() {
                    return None;
                }
                if hash(x, y, z, SEED_TREES ^ 3) % 8 == 0 {
                    Some(CONTENT_APPLE)
                } else {
                    Some(CONTENT_APPLE_LEAVES)
                }
            }
        }
    }
}

/// minetest_game's emergent jungle tree decoration's noise.
const NP_EMERGENT: NoiseParams =
    NoiseParams { offset: 0.0, scale: 0.0025, spread: 250.0, seed: 2685, octaves: 3, persistence: 0.7, lacunarity: 2.0 };
/// Emergent jungle trees grow on ground up to this height (its y_max).
const EMERGENT_Y_MAX: i32 = 32;

/// minetest_game's emergent_jungle_tree.mts, 7 x 37 x 7: one string per
/// slice, bottom up, of 7 rows (z) of 7 nodes (x). '#' trunk, 't' trunk
/// one time in two, '*' leaves, 'o' leaves 110 times in 127, 'l' leaves
/// one time in two, '.' nothing. It stands 4 nodes into the ground (its
/// place_offset_y), on buttress roots; slices 13 to 24, of the trunk, are
/// each left out one time in two, so trees differ in height.
const EMERGENT: [&str; 37] = [
    "................#.#...........#.#................",
    ".........#.#...##.##.........##.##...#.#.........",
    "........##.##..##.##.........##.##..##.##........",
    "#.....#.##.##..##.##.........##.##..##.##.#.....#",
    "#.....#.##.##..#####...###...#####..##.##.#.....#",
    "#.....#.##.##..#####...###...#####..##.##.#.....#",
    "t.....t.##.##..#####...###...#####..##.##.t.....t",
    "........t#.#t..#####...###...#####..t#.#t........",
    ".........t.t...t###t...###...t###t...t.t.........",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "................###....###....###................",
    "ooo....o#o....oo###....###....###................",
    "................###....###....###....o#o....ooo..",
    "....ooo....o#o...##oo..###.....#.................",
    "..............ooo#...o####..ooo#.................",
    ".................#.....###.....##oo....o#o....ooo",
    "..ooo....o#o....o#o....###.....#.................",
    ".................#.....###..oo##...o#o....ooo....",
    ".................#ooo..####o...#ooo..............",
    "ooo....o#o....oo##.....###.....#.................",
    ".................#.....###....o#o....o#o....ooo..",
    "....ooo....o#o...##oo..###.....#.................",
    "..............ooo#...o####..ooo#.................",
    ".................#.....###.....#.................",
    ".lllll.l.....ll.###.ll.#.#.ll.###.ll.....l.lllll.",
    ".ooooo.o*...*oo.#.#.oo.....oo.#.#.oo*...*o.ooooo.",
    ".ooooo.o#***#oo**.**oo*...*oo**.**oo#***#o.ooooo.",
    ".ooooo.o*****oo*****oo*****oo*****oo*****o.ooooo.",
    "........ooooo..o***o..o***o..o***o..ooooo........",
];
/// The trunk slices that may be left out, and how often they stay.
const EMERGENT_SLICES: std::ops::RangeInclusive<usize> = 13..=24;

/// The schematic slices an emergent tree keeps, a bit each.
fn emergent_slices(t: &Tree) -> u64 {
    (0..EMERGENT.len())
        .filter(|&s| !EMERGENT_SLICES.contains(&s) || chance(hash(t.x, t.ground * 64 + s as i32, t.z, SEED_EMERGENT ^ 0x51), 63))
        .fold(0, |m, s| m | 1 << s)
}

/// The `k`th slice kept (0: the bottom one).
fn kept_slice(mut mask: u64, k: u32) -> Option<usize> {
    for _ in 0..k {
        mask &= mask.checked_sub(1)?;
    }
    (mask != 0).then(|| mask.trailing_zeros() as usize)
}

/// Luanti's schematic probabilities: placed if `prob` > a random 1..127.
fn chance(h: u32, prob: u32) -> bool {
    prob >= 127 || h % 127 + 1 < prob
}

/// An emergent tree's node at (x, y, z): the schematic, centred on its
/// trunk and turned one of four ways, from 4 nodes below its ground up.
fn emergent_node(t: &Tree, x: i32, y: i32, z: i32) -> Option<u16> {
    let (dx, dz) = (x - t.x + 3, z - t.z + 3);
    if !(0..7).contains(&dx) || !(0..7).contains(&dz) {
        return None;
    }
    let k = y - (t.ground - 4);
    if k < 0 {
        return None;
    }
    let slice = kept_slice(emergent_slices(t), k as u32)?;
    // Rotation as Luanti's blitToVManip turns a schematic.
    let (sx, sz) = match hash(t.x, t.ground, t.z, SEED_EMERGENT ^ 0x52) % 4 {
        1 => (6 - dz, dx),
        2 => (6 - dx, 6 - dz),
        3 => (dz, 6 - dx),
        _ => (dx, dz),
    };
    let c = EMERGENT[slice].as_bytes()[(sz * 7 + sx) as usize];
    let h = hash(x, y, z, SEED_EMERGENT ^ 0x53);
    match c {
        b'#' => Some(CONTENT_JUNGLE_TREE),
        b't' => chance(h, 63).then_some(CONTENT_JUNGLE_TREE),
        b'*' => Some(CONTENT_JUNGLE_LEAVES),
        b'o' => chance(h, 111).then_some(CONTENT_JUNGLE_LEAVES),
        b'l' => chance(h, 63).then_some(CONTENT_JUNGLE_LEAVES),
        _ => None,
    }
}

/// The tree in a patch, if one grows there.
fn tree_in_cell(cx: i32, cz: i32) -> Option<Tree> {
    let h = hash(cx, 0x7EE, cz, SEED_TREES);
    let x = cx * TREE_CELL + (h % TREE_CELL as u32) as i32;
    let z = cz * TREE_CELL + ((h >> 8) % TREE_CELL as u32) as i32;
    let col = column(x, z);
    // At a cave entrance the ground may be hollowed out.
    if col.cave_top >= col.height || col.height >= TREE_LINE {
        return None;
    }
    let roll = (h >> 16) % 100;
    // Papyrus on the rainforest's and savanna's swampy shores.
    if col.height == WATER_LEVEL && matches!(col.biome, Biome::Rainforest | Biome::Savanna) {
        return (roll < 60).then(|| Tree { x, z, ground: col.height, kind: TreeKind::Papyrus, height: 2 + ((h >> 24) % 3) as i32 });
    }
    // Nor in lakes or on their sandy shores.
    if col.height <= WATER_LEVEL {
        return None;
    }
    // Cacti in the desert, now and then a large one.
    if col.biome == Biome::Desert {
        return match roll {
            0..=3 => Some(Tree { x, z, ground: col.height, kind: TreeKind::Cactus, height: 2 + ((h >> 24) % 4) as i32 }),
            4..=5 => Some(Tree { x, z, ground: col.height, kind: TreeKind::LargeCactus, height: 6 }),
            _ => None,
        };
    }
    // Now and then an emergent jungle tree (minetest_game's decoration:
    // noise offset 0, scale 0.0025, spread 250, 3 octaves, persistence
    // 0.7, on rainforest litter from y 1 to 32), each one a patch's
    // worth of its expected number per node.
    if col.biome == Biome::Rainforest && (1..=EMERGENT_Y_MAX).contains(&col.height) {
        let nval = fractal2(x, z, &NP_EMERGENT, SEED_EMERGENT) * (TREE_CELL * TREE_CELL) as f32;
        if (hash(cx, 0xE3E, cz, SEED_EMERGENT) % 10_000) as f32 <= nval * 10_000.0 {
            return Some(Tree { x, z, ground: col.height, kind: TreeKind::Emergent, height: 0 });
        }
    }
    // The snowy grassland has no trees, but now and then a fir bush or a
    // blueberry bush (minetest_game's pine bush and blueberry bush).
    if col.biome == Biome::SnowyGrassland {
        let kind = match roll {
            0..=2 => TreeKind::FirBush,
            3..=4 => TreeKind::BlueberryBush,
            _ => return None,
        };
        return Some(Tree { x, z, ground: col.height, kind, height: 1 });
    }
    let kind = match col.biome {
        Biome::Desert | Biome::SandstoneDesert | Biome::SnowyGrassland => return None,
        Biome::Snow => TreeKind::Fir,
        Biome::Grassland if col.height >= FIR_LINE => TreeKind::Fir,
        Biome::Grassland => TreeKind::Birch,
        Biome::Rainforest => TreeKind::Jungle,
        Biome::Savanna => TreeKind::Acacia,
        // No trees on the tundra, in the cold desert or on the icesheet.
        Biome::Tundra | Biome::ColdDesert | Biome::Icesheet => return None,
    };
    // A few percent of patches in the open, most of them in a wood. The
    // rainforest is jungle almost everywhere; the savanna has scattered
    // acacias, a little closer together here and there.
    let forest = fbm2(x as f32 / 220.0, z as f32 / 220.0, 2, SEED_FOREST);
    let chance = match kind {
        TreeKind::Jungle => 50 + ((forest + 0.2) * 100.0).clamp(0.0, 35.0) as u32,
        TreeKind::Acacia => 5 + ((forest - 0.05) * 60.0).clamp(0.0, 10.0) as u32,
        _ => 4 + ((forest - 0.05) * 300.0).clamp(0.0, 66.0) as u32,
    };
    // Apple trees came later: they grow only in lowland patches that had
    // no tree, so the trees of worlds made before stay as they were. About
    // one lowland tree in three is an apple tree.
    // Bushes grow in some of the patches left over: plain and blueberry
    // bushes in the lowland grass, fir bushes higher up and in the snow
    // (where blueberries grow too), acacia bushes on the savanna.
    let spare = roll.saturating_sub(chance + chance / 2);
    let bush = || match (kind, (h >> 4) % 100) {
        (TreeKind::Birch, 0..=2) => Some(TreeKind::Bush),
        (TreeKind::Birch, 3..=4) => Some(TreeKind::BlueberryBush),
        (TreeKind::Fir, 0..=2) => Some(TreeKind::FirBush),
        (TreeKind::Fir, 3) if col.biome == Biome::Snow => Some(TreeKind::BlueberryBush),
        (TreeKind::Acacia, 0..=3) => Some(TreeKind::AcaciaBush),
        _ => None,
    };
    let kind = if roll < chance {
        kind
    } else if kind == TreeKind::Birch && roll < chance + chance / 2 {
        TreeKind::Apple
    } else if let Some(b) = bush().filter(|_| spare > 0) {
        b
    } else {
        return None;
    };
    let height = match kind {
        TreeKind::Birch => 5 + ((h >> 24) % 3) as i32,
        TreeKind::Fir => 7 + ((h >> 24) % 4) as i32,
        // minetest_game: random(4, 5).
        TreeKind::Apple => 4 + ((h >> 24) % 2) as i32,
        TreeKind::Jungle => 8 + ((h >> 24) % 5) as i32,
        TreeKind::Acacia => 5 + ((h >> 24) % 2) as i32,
        _ => 1,
    };
    Some(Tree { x, z, ground: col.height, kind, height })
}

/// The plant growing on the surface of a column, if any: tall grass in
/// drifts that come and go across the land (minetest_game's grass
/// decorations, five heights), dry grass on the savanna, jungle grass in
/// the rainforest, ferns under the firs, marram grass on sandy shores, and
/// here and there a patch of wild cotton. Not on snow or a cave mouth. It
/// stands in the node above the surface.
pub fn plant_on(x: i32, z: i32, col: &Column) -> Option<u16> {
    // Marram grass on the grassland's sandy shores (minetest_game's dunes).
    if col.height == WATER_LEVEL && col.biome == Biome::Grassland && col.cave_top < col.height {
        let h = hash(x, 0x3A2, z, SEED_SHORE);
        let patch = noise2(x as f32 / 16.0, z as f32 / 16.0, SEED_SHORE) > 0.1;
        return (patch && h % 3 == 0).then(|| plants::MARRAM[((h >> 8) % 3) as usize]);
    }
    // Snow lies on the snowy lands (minetest_game's taiga dust), and in
    // patches on the tundra.
    if col.cave_top < col.height && col.height > WATER_LEVEL {
        match col.biome {
            Biome::Snow | Biome::SnowyGrassland => return Some(CONTENT_SNOW_LAYER),
            Biome::Tundra if noise2(x as f32 / 30.0, z as f32 / 30.0, SEED_SHORE ^ 9) > 0.3 => return Some(CONTENT_SNOW_LAYER),
            _ => {}
        }
    }
    // Dry shrubs, here and there in patches, on the desert sand, the
    // sandstone desert's sand and the cold desert's silver sand (minetest_game's dry shrub decoration:
    // noise offset 0, scale 0.02, spread 200, 3 octaves, persistence 0.6).
    if matches!(col.biome, Biome::Desert | Biome::SandstoneDesert | Biome::ColdDesert) && col.height >= 1 && col.cave_top < col.height {
        let mut sum = 0.0;
        for (i, (amp, freq)) in [(1.0, 1.0), (0.6, 2.0), (0.36, 4.0)].into_iter().enumerate() {
            sum += amp * noise2(x as f32 / 200.0 * freq, z as f32 / 200.0 * freq, SEED_SHRUBS.wrapping_add(i as u32));
        }
        // Luanti's value noise spreads wider than this gradient noise.
        let fill = sum * 0.02 * 1.8;
        let h = hash(x, 0xD5, z, SEED_SHRUBS);
        return (((h % 10000) as f32) < fill * 10000.0).then_some(content::DRY_SHRUB);
    }
    let grasses = match col.biome {
        // Ferns under the firs (minetest_game's coniferous forest).
        Biome::Grassland if col.height >= FIR_LINE => [plants::FERN[0], plants::FERN[1], plants::FERN[2], plants::FERN[2], plants::FERN[1]],
        Biome::Grassland => farming::GRASS,
        Biome::Savanna => farming::DRY_GRASS,
        Biome::Rainforest => [farming::JUNGLEGRASS; 5],
        _ => return None,
    };
    if col.height <= WATER_LEVEL || col.cave_top >= col.height {
        return None;
    }
    let h = hash(x, 0x6A55, z, SEED_GRASS);
    // Flowers on the grassland, each kind in its own patches (minetest_game's
    // flower decorations); mushrooms in the woods, birch and fir.
    if col.biome == Biome::Grassland && (h >> 12) % 24 == 0 {
        if col.height < FIR_LINE {
            let k = (h >> 18) % colours::FLOWERS.len() as u32;
            if noise2(x as f32 / 200.0, z as f32 / 200.0, SEED_FLOWERS.wrapping_add(k)) > 0.3 {
                return Some(colours::flower(k as usize));
            }
        }
        if (h >> 18) % 4 == 0 && fbm2(x as f32 / 220.0, z as f32 / 220.0, 2, SEED_FOREST) > 0.1 {
            return Some(if (h >> 20) % 2 == 0 { colours::MUSHROOM_BROWN } else { colours::MUSHROOM_RED });
        }
    }
    // Wild cotton on grassland and savanna (where minetest_game grows it).
    if col.biome != Biome::Rainforest && noise2(x as f32 / 60.0, z as f32 / 60.0, SEED_COTTON) > 0.55 && h % 16 == 0 {
        return Some(farming::COTTON_WILD);
    }
    let density = ((fbm2(x as f32 / 100.0, z as f32 / 100.0, 2, SEED_GRASS) + 0.3) * 0.25).clamp(0.0, 0.2);
    // The savanna is grassier, the jungle floor sparser.
    let density = match col.biome {
        Biome::Savanna => density + 0.1,
        Biome::Rainforest => density * 0.5,
        _ => density,
    };
    ((h >> 8) % 1000 < (density * 1000.0) as u32).then(|| grasses[((h >> 20) % 5) as usize])
}

/// The butterfly or firefly hanging over a column, if any, and how far
/// above its surface (minetest_game's butterflies and fireflies
/// decorations): butterflies three nodes up over the grassland's grass
/// (fill ratio 0.005) where a flower grows beside (spawn_by group:flower),
/// one of the three kinds at random; fireflies, hidden by day, three or
/// four up in the woods (deciduous, coniferous and rainforest; 0.0005
/// each). `column` gives the columns round about. The caller checks that
/// the node is air (not in a tree).
pub fn insect_over(x: i32, z: i32, col: &Column, column: &dyn Fn(i32, i32) -> Column) -> Option<(i32, u16)> {
    use crate::insects;
    if col.height <= WATER_LEVEL || col.cave_top >= col.height {
        return None;
    }
    let h = hash(x, 0x1B5, z, SEED_INSECTS);
    let roll = h % 10_000;
    let woods = match col.biome {
        Biome::Grassland if col.height >= FIR_LINE => true,
        Biome::Grassland => fbm2(x as f32 / 220.0, z as f32 / 220.0, 2, SEED_FOREST) > 0.1,
        Biome::Rainforest => true,
        _ => false,
    };
    if woods && roll < 10 {
        return Some((if roll < 5 { 3 } else { 4 }, insects::HIDDEN_FIREFLY));
    }
    if col.biome != Biome::Grassland || col.height >= FIR_LINE || !(10..60).contains(&roll) {
        return None;
    }
    // A flower in one of the eight columns round about, on ground as high.
    let flower = (-1..=1).any(|dz| {
        (-1..=1).any(|dx| {
            if dx == 0 && dz == 0 {
                return false;
            }
            let c = column(x + dx, z + dz);
            c.height == col.height && plant_on(x + dx, z + dz, &c).is_some_and(|p| crate::content::group_of(p, "flower") > 0)
        })
    });
    flower.then(|| (3, insects::BUTTERFLIES[((h >> 16) % 3) as usize]))
}

/// An emergent jungle tree's trunk at (x, y, z) at or below the ground:
/// the map generator force-places it (its roots), into whatever is there.
pub fn forced_trunk_in(trees: &[Tree], x: i32, y: i32, z: i32) -> Option<u16> {
    trees
        .iter()
        .filter(|t| t.kind == TreeKind::Emergent && y >= t.ground - 4)
        .find_map(|t| t.node_at(x, y, z).filter(|&id| content::is_trunk(id)))
}

/// Every tree that could have a node in the columns from (x0, z0) to
/// (x1, z1) inclusive.
pub fn trees_in(x0: i32, z0: i32, x1: i32, z1: i32) -> Vec<Tree> {
    let cell = |v: i32| v.div_euclid(TREE_CELL);
    let mut trees = Vec::new();
    for cz in cell(z0 - TREE_REACH)..=cell(z1 + TREE_REACH) {
        for cx in cell(x0 - TREE_REACH)..=cell(x1 + TREE_REACH) {
            if let Some(t) = tree_in_cell(cx, cz) {
                let reaches = t.x + TREE_REACH >= x0 && t.x - TREE_REACH <= x1 && t.z + TREE_REACH >= z0 && t.z - TREE_REACH <= z1;
                if reaches {
                    trees.push(t);
                }
            }
        }
    }
    trees
}

/// The tree node at (x, y, z) among `trees`, if any; a trunk wins over
/// another tree's leaves. The caller makes sure the terrain is air there.
/// Birches and firs come first: apple trees (added later) only fill what
/// they leave empty, so the trees of older worlds stay exactly as they were.
pub fn tree_node_in(trees: &[Tree], x: i32, y: i32, z: i32) -> Option<u16> {
    let pick = |apple: bool| {
        let mut found = None;
        for t in trees.iter().filter(|t| (t.kind == TreeKind::Apple) == apple) {
            match t.node_at(x, y, z) {
                Some(id) if content::is_trunk(id) => return Some(id),
                Some(id) => found = found.or(Some(id)),
                None => {}
            }
        }
        found
    };
    pick(false).or_else(|| pick(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Butterflies over the grassland, each beside a flower; fireflies in
    /// the woods, hidden; neither over water.
    #[test]
    fn insects() {
        use crate::insects;
        let (mut butterflies, mut fireflies) = (0, 0);
        for z in (-600..600).step_by(1) {
            for x in (-600..600).step_by(3) {
                let col = column(x, z);
                let Some((dy, id)) = insect_over(x, z, &col, &column) else {
                    continue;
                };
                assert!(col.height > WATER_LEVEL);
                if id == insects::HIDDEN_FIREFLY {
                    fireflies += 1;
                    assert!(dy == 3 || dy == 4);
                } else {
                    butterflies += 1;
                    assert!(insects::BUTTERFLIES.contains(&id) && dy == 3 && col.biome == Biome::Grassland);
                    let flower = (-1..=1).any(|dz| (-1..=1).any(|dx| plant_on(x + dx, z + dz, &column(x + dx, z + dz)).is_some_and(|p| crate::content::group_of(p, "flower") > 0)));
                    assert!(flower);
                }
            }
        }
        assert!(butterflies > 5 && fireflies > 20, "{butterflies} butterflies, {fireflies} fireflies");
    }

    /// mgv7's channel: (0.2 - a) * (y - water_level + 17) / 2.5 >= 0.6.
    #[test]
    fn river_channel() {
        assert_eq!(channel_bed(0.0), Some(-9)); // carved from -8 up: 10 deep
        assert_eq!(channel_bed(0.12), Some(2));
        assert_eq!(channel_bed(0.16), Some(21));
        assert_eq!(channel_bed(0.2), None);
        for (a, bed) in [(0.0f32, -9), (0.12, 2), (0.16, 21)] {
            let carved = |y: i32| (RIVER_WIDTH - a) * (y - WATER_LEVEL + 17) as f32 / 2.5 >= 0.6;
            assert!(carved(bed + 1) && !carved(bed), "a {a}");
        }
        // The channel gets shallower towards its sides.
        let beds: Vec<i32> = (0..20).map(|i| channel_bed(i as f32 / 100.0).unwrap()).collect();
        assert!(beds.windows(2).all(|w| w[0] <= w[1]));
    }

    /// The emergent tree's schematic: 37 slices of 7 x 7, a trunk up the
    /// middle, the crown at the top.
    #[test]
    fn emergent_schematic() {
        assert!(EMERGENT.iter().all(|s| s.len() == 49 && s.bytes().all(|c| b".#t*ol".contains(&c))));
        for s in 9..32 {
            assert_eq!(EMERGENT[s].as_bytes()[3 * 7 + 3], b'#', "trunk in slice {s}");
        }
        let t = Tree { x: 100, z: -50, ground: 10, kind: TreeKind::Emergent, height: 0 };
        // The trunk, at every turn; nothing outside the 7 x 7.
        assert_eq!(t.node_at(100, 10, -50), Some(CONTENT_JUNGLE_TREE));
        assert_eq!(t.node_at(104, 15, -50), None);
        assert_eq!(t.node_at(100, 5, -50), None);
        let leaves = (t.top() - 4..=t.top()).flat_map(|y| (-3..=3).flat_map(move |dz| (-3..=3).map(move |dx| (dx, y, dz))))
            .filter(|&(dx, y, dz)| t.node_at(100 + dx, y, -50 + dz) == Some(CONTENT_JUNGLE_LEAVES)).count();
        assert!(leaves > 40);
        assert!((20..=32).contains(&(t.top() - t.ground)));
        assert_eq!(t.node_at(100, t.top() + 1, -50), None);
        assert_eq!(kept_slice(0b1011, 0), Some(0));
        assert_eq!(kept_slice(0b1011, 2), Some(3));
        assert_eq!(kept_slice(0b1011, 3), None);
    }
}
