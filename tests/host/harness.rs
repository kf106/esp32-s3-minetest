#![allow(dead_code)]
mod auth; mod beds; mod binoculars; mod board; mod books; mod bones; mod chest; mod clock; mod colours; mod content; mod crafting; mod craftguide; mod doors; mod env_sounds; mod dungeons; mod falling; mod fire; mod farming; mod fences; mod fasthash; mod font; mod furnace; mod indicator; mod insects; mod boats; mod carts; mod creative; mod inventory; mod items; mod jsonstring; mod keys; mod liquids; mod messages; mod meta; mod mobs; mod nature; mod panes; mod plants; mod privs; mod player; mod protocol;
mod screen; mod screwdriver; mod server; mod settings; mod sha1; mod sha256; mod skins; mod srp; mod stairs; mod status; mod storage; mod terrain; mod tnt; mod weather; mod wifi_config; mod world; mod zstd;

use std::cell::RefCell;
use std::collections::HashSet;

const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/out");

fn starter_kit() {
    let d = format!("{OUT}/invtest");
    let _ = std::fs::remove_dir_all(&d);
    let store = inventory::InventoryStore::open(&d).unwrap();
    let a = store.load("Steve");
    let names = |i: &inventory::Inventory| i.main.iter().flatten().map(|s| s.item_string()).collect::<Vec<_>>();
    assert_eq!(names(&a), vec!["esp:torch 20"]);
    assert_eq!(store.load("Steve"), a, "kit given twice");
    std::fs::write(format!("{d}/old.inv"), "0 esp:dirt 43\n").unwrap();
    let c = store.load("old");
    assert_eq!(names(&c), vec!["esp:dirt 43", "esp:torch 20"]);
    assert_eq!(store.load("old"), c);
    let mut t = c.clone();
    use inventory::List::*;
    assert_eq!(t.take(Main, 1, 1).map(|s| s.count), Some(1));
    assert_eq!(t.take(Main, 1, 0).map(|s| s.count), Some(19));
    assert_eq!(t.take(Main, 1, 0), None);
    assert_eq!(t.take(Main, 0, 100).map(|s| s.count), Some(43));
    match inventory::parse_action("Drop 0 current_player main 3") {
        Some(inventory::Action::Drop { count: 0, from }) => assert_eq!(from, ("current_player".into(), "main".into(), 3)),
        other => panic!("{other:?}"),
    }
    println!("STARTER KIT OK");
}

/// Crafting through the inventory, as a client would: logs into the grid,
/// craft planks, sticks and torches, take the result, save and reload.
fn crafting_flow() {
    use content::*;
    use inventory::Action;
    let d = format!("{OUT}/crafttest");
    let _ = std::fs::remove_dir_all(&d);
    let store = inventory::InventoryStore::open(&d).unwrap();
    let mut inv = store.load("Crafter"); // 20 torches in slot 0
    assert_eq!(inv.add(inventory::Stack::new(CONTENT_BIRCH_TREE, 3)), 0); // slot 1
    assert_eq!(inv.add(inventory::Stack::new(ITEM_COAL_LUMP, 5)), 0); // slot 2
    let mut furnaces = furnace::Furnaces::open(format!("{d}/furnaces.txt"));
    let mut chests = chest::Chests::open(format!("{d}/chests.txt"));
    let mut act = |inv: &mut inventory::Inventory, text: &str| -> bool {
        use furnace::Place::Own;
        let l = |n: &str| inventory::List::from_name(n).unwrap();
        let mut lists = furnace::Lists { player: inv, furnaces: &mut furnaces, chests: &mut chests };
        match inventory::parse_action(text).unwrap() {
            Action::Move { count, from, to } => lists.move_items((Own(l(&from.1)), from.2), (Own(l(&to.1)), to.2), count),
            Action::MoveSomewhere { count, from, to } => lists.move_somewhere((Own(l(&from.1)), from.2), Own(l(&to.1)), count),
            Action::Craft { count, .. } => lists.player.craft(count).is_some(),
            other => panic!("{other:?}"),
        }
    };
    // One log into the middle of the grid: the preview shows 4 planks.
    assert!(act(&mut inv, "Move 1 current_player main 1 current_player craft 4"));
    assert_eq!(inv.craft_preview().map(|s| s.item_string()), Some("esp:birch_wood 4".into()));
    assert!(act(&mut inv, "Craft 1 current_player"));
    assert_eq!(inv.craft_result[0].map(|s| s.item_string()), Some("esp:birch_wood 4".into()));
    assert_eq!(inv.craft[4], None);
    assert_eq!(inv.craft_preview(), None);
    // Craft again with nothing in the grid: refused.
    assert!(!act(&mut inv, "Craft 1 current_player"));
    // The client then puts the planks down from the hidden result list.
    assert!(act(&mut inv, "Move 4 current_player craftresult 0 current_player main 5"));
    assert_eq!(inv.main[5].map(|s| s.item_string()), Some("esp:birch_wood 4".into()));
    // Nothing goes into the craft result.
    assert!(!act(&mut inv, "Move 1 current_player main 5 current_player craftresult 0"));
    // Two planks: craft "as many as possible" (shift) makes 8 sticks.
    assert!(act(&mut inv, "Move 2 current_player main 5 current_player craft 0"));
    assert!(act(&mut inv, "Craft 0 current_player"));
    assert_eq!(inv.craft_result[0].map(|s| s.item_string()), Some("esp:stick 8".into()));
    // Shift-move the sticks into the main list.
    assert!(act(&mut inv, "MoveSomewhere 8 current_player craftresult 0 current_player main"));
    assert_eq!(inv.craft_result[0], None);
    // Coal above stick, in the left column: torches.
    assert!(act(&mut inv, "Move 2 current_player main 2 current_player craft 3"));
    let stick_slot = inv.main.iter().position(|s| s.is_some_and(|s| s.id == ITEM_STICK)).unwrap();
    assert!(act(&mut inv, &format!("Move 3 current_player main {stick_slot} current_player craft 6")));
    assert_eq!(inv.craft_preview().map(|s| s.item_string()), Some("esp:torch 4".into()));
    assert!(act(&mut inv, "Craft 10 current_player"));
    assert_eq!(inv.craft_result[0].map(|s| s.item_string()), Some("esp:torch 8".into()));
    assert_eq!(inv.craft[3], None);
    assert_eq!(inv.craft[6].map(|s| s.count), Some(1));
    // Shift-click a grid slot back to main.
    assert!(act(&mut inv, "MoveSomewhere 1 current_player craft 6 current_player main"));
    assert_eq!(inv.craft[6], None);
    // Saved and loaded with the craft result still waiting.
    store.save("Crafter", &inv);
    let back = store.load("Crafter");
    assert_eq!(back, inv);
    println!("{}", std::fs::read_to_string(format!("{d}/crafter.inv")).unwrap());
    // Digging gives MTG drops.
    assert_eq!(drop_of(CONTENT_STONE), Some((CONTENT_COBBLE, 1)));
    assert_eq!(drop_of(CONTENT_COAL), Some((ITEM_COAL_LUMP, 1)));
    assert!(!is_node(ITEM_STICK) && is_item(ITEM_STICK));
    assert_eq!(id_of("esp:stick"), Some(ITEM_STICK));
    assert_eq!(node_id_of("esp:stick"), None);
    println!("CRAFTING OK");
}

/// Tools as minetest_game has them: the hand can't dig stone, a wooden
/// pickaxe can (not iron ore), wear builds up and the tool breaks after
/// exactly its number of uses.
fn tools() {
    use content::*;
    let hand = |n| dig_params(n, &HAND, 0);
    assert_eq!(hand(CONTENT_STONE), None);
    assert_eq!(hand(CONTENT_COBBLE), None);
    assert_eq!(hand(CONTENT_SANDSTONE), None);
    assert_eq!(hand(CONTENT_DIRT), Some((0.7, 0)));
    assert_eq!(hand(CONTENT_BIRCH_TREE), Some((3.5, 0)));
    assert_eq!(hand(CONTENT_TORCH), Some((0.0, 0)));
    let pick = caps_of(Some(TOOL_PICK_WOOD));
    assert_eq!(dig_params(CONTENT_STONE, pick, 0), Some((1.6, 2184)));
    assert_eq!(dig_params(CONTENT_IRON, pick, 0), None);
    assert!(dig_params(CONTENT_IRON, caps_of(Some(TOOL_PICK_STONE)), 0).is_some());
    // A pickaxe on dirt: not with the pick (no wear), but the hand can.
    assert_eq!(dig_params(CONTENT_DIRT, pick, 0), None);
    assert_eq!(dig_params(CONTENT_FIR_TREE, caps_of(Some(TOOL_AXE_STONE)), 0).map(|p| p.0), Some(1.3));
    // Tools are maxlevel 1 and our nodes level 0, so Luanti triples the
    // uses: 30 stone digs for a wooden pick, 60 for a stone one.
    for (tool, uses) in [(TOOL_PICK_WOOD, 30), (TOOL_PICK_STONE, 60)] {
        let mut s = inventory::Stack::new(tool, 1);
        let mut n = 0;
        loop {
            n += 1;
            let (_, w) = dig_params(CONTENT_STONE, caps_of(Some(tool)), s.wear).unwrap();
            if !s.add_wear(w) { break; }
        }
        assert_eq!(n, uses, "{}", name_of(tool));
    }
    let worn = inventory::Stack { wear: 13107, ..inventory::Stack::new(TOOL_AXE_WOOD, 1) };
    assert_eq!(worn.item_string(), "esp:axe_wood 1 13107");
    assert_eq!(inventory::Stack::parse("esp:axe_wood 1 13107"), Some(worn));
    assert_eq!(inventory::Stack::parse("esp:axe_wood 5"), Some(inventory::Stack::new(TOOL_AXE_WOOD, 1)));
    assert_eq!(stack_max(TOOL_AXE_WOOD), 1);
    // Tools don't stack: two picks take two slots.
    let mut inv = inventory::Inventory::new();
    assert_eq!(inv.add(inventory::Stack::new(TOOL_PICK_WOOD, 1)), 0);
    assert_eq!(inv.add(inventory::Stack::new(TOOL_PICK_WOOD, 1)), 0);
    assert!(inv.main[0].is_some() && inv.main[1].is_some());
    // Crafting a tool needs an empty craft result to land in.
    inv.craft_result[0] = Some(inventory::Stack::new(TOOL_PICK_WOOD, 1));
    for (i, id) in [(0, CONTENT_FIR_WOOD), (1, CONTENT_FIR_WOOD), (2, CONTENT_FIR_WOOD), (4, ITEM_STICK), (7, ITEM_STICK)] {
        inv.craft[i] = Some(inventory::Stack::new(id, 1));
    }
    assert_eq!(inv.craft_preview(), Some(inventory::Stack::new(TOOL_PICK_WOOD, 1)));
    assert_eq!(inv.craft(1), None);
    inv.craft_result[0] = None;
    assert_eq!(inv.craft(1), Some(inventory::Stack::new(TOOL_PICK_WOOD, 1)));
    assert!(inv.craft.iter().all(|c| c.is_none()));
    println!("TOOLS OK");
}

fn run(items: &mut items::Items, solid: &dyn Fn(i32, i32, i32) -> bool, secs: f32) -> Vec<items::Change> {
    let mut all = Vec::new();
    let mut t = 0.0;
    while t < secs { all.extend(items.step(0.05, solid)); t += 0.05; }
    all
}

fn physics() {
    use items::Change::*;
    // Floor: everything at y <= 0, a wall at x = 3, minus holes.
    let holes: RefCell<HashSet<(i32, i32, i32)>> = RefCell::new(HashSet::new());
    let solid = |x: i32, y: i32, z: i32| !holes.borrow().contains(&(x, y, z)) && (y <= 0 || x == 3);
    let mut it = items::Items::new();
    // Thrown east at the wall from 1.2 above a player standing at y = 0.5.
    let (a, ev) = it.add(inventory::Stack::new(8, 5), ([0.0, 1.7, 0.0], [2.9, 2.0, 0.0]), &solid);
    assert_eq!(ev, None);
    let ch = run(&mut it, &solid, 2.0);
    let d = it.get(a).unwrap().clone();
    let half = messages::item_size(5);
    println!("thrown item rests at {:?} (half size {half:.3}), changes {ch:?}", d.pos);
    assert!(!d.falling && d.vel == [0.0; 3]);
    assert!((d.pos[1] - (0.5 + half)).abs() < 0.01, "on the floor");
    assert!(d.pos[0] <= 2.5 - half + 0.001 && d.pos[0] > 1.5, "stopped by the wall");
    assert_eq!(ch, vec![Moved(a)]);
    // A second matching drop nearby merges into one stack of 8.
    let (b, _) = it.add(inventory::Stack::new(8, 3), ([d.pos[0] - 0.5, 1.7, 0.3], [0.0, 0.0, 0.0]), &solid);
    let ch = run(&mut it, &solid, 1.0);
    println!("second drop: {ch:?}");
    assert_eq!(it.iter().count(), 1);
    let m = it.get(b).unwrap();
    assert_eq!(m.count, 8);
    assert!(ch.contains(&Removed(a)) && ch.contains(&Resized(b)));
    // A different item doesn't merge.
    let (c, _) = it.add(inventory::Stack::new(1, 2), ([d.pos[0] - 0.5, 1.7, -0.3], [0.0; 3]), &solid);
    run(&mut it, &solid, 1.0);
    assert_eq!(it.iter().count(), 2);
    // Dig the floor under b: it falls to the next floor... here, y = -1's top.
    let bp = it.get(b).unwrap().pos;
    let (bx, bz) = (bp[0].round() as i32, bp[2].round() as i32);
    for x in bx - 1..=bx + 1 { for z in bz - 1..=bz + 1 { holes.borrow_mut().insert((x, 0, z)); } }
    let woken = it.wake_near([bx, 0, bz]);
    assert!(woken.contains(&b), "{woken:?}");
    run(&mut it, &solid, 1.0);
    let y = it.get(b).unwrap().pos[1];
    println!("after digging under it: y {y:.3}");
    assert!((y - (-0.5 + messages::item_size(8))).abs() < 0.01);
    // Spawned inside a solid node: lifted out.
    let (e, _) = it.add(inventory::Stack::new(1, 1), ([5.0, -0.2, 5.0], [0.0; 3]), &solid);
    assert!(it.get(e).unwrap().pos[1] > 0.5, "{:?}", it.get(e));
    // Expiry
    let ch = run(&mut it, &solid, items::TIME_TO_LIVE + 1.0);
    assert!(ch.contains(&Removed(c)) && it.iter().count() == 0, "{ch:?}");
    // The cap: the oldest goes.
    let first = it.add(inventory::Stack::new(1, 1), ([0.0, 5.0, 0.0], [0.0; 3]), &solid).0;
    for _ in 1..items::MAX_ITEMS { it.add(inventory::Stack::new(1, 1), ([0.0, 5.0, 0.0], [0.0; 3]), &solid); }
    let (_, ev) = it.add(inventory::Stack::new(1, 1), ([0.0, 5.0, 0.0], [0.0; 3]), &solid);
    assert_eq!(ev, Some(first));
    println!("ITEM PHYSICS OK");
}

fn real_world() {
    let d = format!("{OUT}/itemworld");
    let _ = std::fs::remove_dir_all(&d);
    let w = world::World::open(&d).unwrap();
    let solid = |x: i32, y: i32, z: i32| content::walkable(w.node((x, y, z)));
    let mut it = items::Items::new();
    let ground = w.highest_solid(8, 8);
    for (yaw, pitch) in [(0.0f32, 0.0f32), (90.0, -30.0), (180.0, 60.0), (270.0, 0.0)] {
        let p = (-pitch).to_radians(); let y = (yaw + 90.0).to_radians();
        let dir = [p.cos() * y.cos(), p.sin(), p.cos() * y.sin()];
        let pos = [8.0, ground as f32 + 0.5 + 1.2, 8.0];
        let vel = [dir[0] * 2.9, dir[1] * 2.9 + 2.0, dir[2] * 2.9];
        let id = it.add(inventory::Stack::new(content::CONTENT_TORCH, 1), (pos, vel), &solid).0;
        run(&mut it, &solid, 3.0);
        let r = it.get(id).unwrap();
        let below = w.node((r.pos[0].round() as i32, (r.pos[1] - messages::item_size(1) - 0.5).round() as i32, r.pos[2].round() as i32));
        println!("yaw {yaw} pitch {pitch}: thrown {vel:.2?}, rests at {:.2?} on {}", r.pos, content::name_of(below));
        assert!(!r.falling && content::walkable(below));
    }
    // object packets for the verifier
    let id = it.iter().next().unwrap().id;
    let dd = it.get(id).unwrap();
    let init = messages::ItemInit { id, itemstring: "esp:torch 1", item_name: "esp:torch", description: "Torch", count: 1, light_source: 12, pos_bs: dd.pos_bs(), vel_bs: [5.0, 20.0, 0.0] };
    std::fs::write(format!("{OUT}/item_add.bin"), messages::build_remove_add(&[3], &[(id, messages::item_init_data(&init))])).unwrap();
    std::fs::write(format!("{OUT}/item_move.bin"), messages::build_object_messages(&[(id, messages::item_position_message(dd.pos_bs(), [0.0; 3]))])).unwrap();
    let init99 = messages::ItemInit { count: 99, itemstring: "esp:dirt 99", item_name: "esp:dirt", description: "Dirt", light_source: 0, ..init };
    std::fs::write(format!("{OUT}/item_props.bin"), messages::build_object_messages(&[(id, messages::item_properties_message(&init99))])).unwrap();
    println!("REAL WORLD DROPS OK");
}

fn content_dump() {
    let media_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/media");
    let media = content::Media::load(media_dir);
    for f in std::fs::read_dir(OUT).unwrap().flatten() {
        let n = f.file_name().to_string_lossy().into_owned();
        if n.starts_with("cmd_") || n.starts_with("media_") { std::fs::remove_file(f.path()).unwrap(); }
    }
    let w = |n: &str, d: Vec<u8>| std::fs::write(format!("{OUT}/{n}"), d).unwrap();
    w("cmd_3d.bin", messages::build_itemdef(&content::itemdef_bytes()));
    w("cmd_3a.bin", messages::build_nodedef(&content::nodedef_bytes()));
    w("cmd_3c.bin", messages::build_announce_media(media.files().iter().map(|f| (f.name.as_str(), &f.sha1))));
    let files: Vec<(&str, &[u8])> = media.files().iter().map(|f| (f.name.as_str(), &f.data[..])).collect();
    for (i, b) in messages::build_media_bunches(&files).into_iter().enumerate() { w(&format!("media_{i}.bin"), b); }
    w("cmd_45.bin", messages::build_movement());
    w("cmd_29.bin", messages::build_time_of_day(10000, 72.0));
    w("cmd_2a.bin", messages::build_csm_restriction_flags());
}

fn torch_variants() {
    use content::*;
    assert_eq!(torch_variant(CONTENT_TORCH, 1), CONTENT_TORCH);
    assert_eq!(torch_variant(CONTENT_TORCH, 0), CONTENT_TORCH_CEILING);
    for p in 2..6 { assert_eq!(torch_variant(CONTENT_TORCH, p), CONTENT_TORCH_WALL); }
    assert_eq!(torch_variant(CONTENT_TORCH_WALL, 1), CONTENT_TORCH);
    assert_eq!(torch_variant(CONTENT_STONE, 0), CONTENT_STONE);
    for id in [CONTENT_TORCH, CONTENT_TORCH_WALL, CONTENT_TORCH_CEILING] {
        assert!(is_torch(id) && light_source(id) == TORCH_LIGHT && !walkable(id) && passes_light(id));
        assert_eq!(drop_of(id), Some((CONTENT_TORCH, 1)));
    }
    // An old log with "esp:torch" on a wall and a ceiling loads as the right nodes, lit.
    let d = format!("{OUT}/torchvariants");
    let _ = std::fs::remove_dir_all(&d);
    let g = { let w = world::World::open(&d).unwrap(); w.highest_solid(8, 8) };
    let rec = |x: i32, y: i32, z: i32, name: &str, p2: u8| {
        let mut r = Vec::new();
        for v in [x, y, z] { r.extend(v.to_be_bytes()); }
        let mut f = name.as_bytes().to_vec(); if p2 != 0 { f.extend([0, p2]); }
        r.push(f.len() as u8); r.extend(f); r
    };
    let mut log = Vec::new();
    log.extend(rec(8, g + 1, 8, "esp:torch", 1));
    log.extend(rec(9, g + 1, 8, "esp:torch", 3));
    log.extend(rec(10, g + 1, 8, "esp:torch", 0));
    let logpath = format!("{d}/nodes.log");
    std::fs::write(&logpath, &log).unwrap();
    let w = world::World::open(&d).unwrap();
    let got: Vec<&str> = (8..=10).map(|x| content::name_of(w.node((x, g + 1, 8)))).collect();
    println!("old torches load as {got:?}");
    assert_eq!(got, ["esp:torch", "esp:torch_wall", "esp:torch_ceiling"]);
    assert_eq!(w.param2((9, g + 1, 8)), 3);
    let raw = w.block_raw(world::block_of_node((8, g + 1, 8)));
    assert!(!raw.is_empty());
    println!("TORCH VARIANTS OK");
}

/// Every node of the block columns around (x, z) from y0 to y1: built as a
/// block and asked one at a time, the same.
fn blocks_agree(w: &world::World, x: i32, z: i32, y0: i32, y1: i32) -> usize {
    let mut n = 0;
    for bz in [z.div_euclid(16) - 1, z.div_euclid(16), z.div_euclid(16) + 1] { for bx in [x.div_euclid(16) - 1, x.div_euclid(16), x.div_euclid(16) + 1] {
        for by in world::block_of(y0)..=world::block_of(y1) {
            let raw = w.block_raw((bx as i16, by, bz as i16));
            for k in 0..4096usize {
                let (zz, yy, xx) = ((k / 256) as i32, ((k / 16) % 16) as i32, (k % 16) as i32);
                let got = u16::from_be_bytes([raw[5 + 2 * k], raw[6 + 2 * k]]);
                let p = (bx * 16 + xx, by as i32 * 16 + yy, bz * 16 + zz);
                assert_eq!(got, w.node(p), "block and node differ at {p:?}");
                n += 1;
            }
        }
    } }
    n
}

/// Rivers, the icesheet (cave ice) and emergent jungle trees.
fn new_terrain() {
    use content::*;
    use terrain::{Biome, WATER_LEVEL};
    let w = world::World::builder();
    let ring = |r: i32| (-r..=r).step_by(8).flat_map(move |a| [(a, -r), (a, r), (-r, a), (r, a)]);
    let find = |want: &dyn Fn(i32, i32) -> bool| (0..6000).step_by(8).find_map(|r| ring(r).find(|&(x, z)| want(x, z))).unwrap();
    // A river: cut down to its bed, with water up to the water level.
    let (x, z) = find(&|x, z| terrain::river_bed(x, z).is_some_and(|b| b <= WATER_LEVEL - 6 && b < terrain::land_height(x, z) - 4)
        && !matches!(terrain::column(x, z).biome, Biome::Icesheet | Biome::Snow | Biome::Tundra | Biome::ColdDesert));
    let bed = terrain::river_bed(x, z).unwrap();
    let c = terrain::column(x, z);
    assert_eq!(c.height, bed);
    assert!(content::walkable(w.node((x, bed, z))));
    for y in bed + 1..=WATER_LEVEL { assert_eq!(w.node((x, y, z)), CONTENT_WATER, "river water at {y}"); }
    assert!(!content::is_liquid(w.node((x, WATER_LEVEL + 1, z))));
    println!("river at ({x}, {z}): bed {bed}, cut from {}, water above it", terrain::land_height(x, z));
    let n = blocks_agree(&w, x, z, bed - 20, terrain::land_height(x, z) + 20);
    // Its valley's side, above the water: cut down, dry.
    let (x, z) = find(&|x, z| terrain::river_bed(x, z).is_some_and(|b| b > WATER_LEVEL + 2 && b < terrain::land_height(x, z) - 3));
    let bed = terrain::river_bed(x, z).unwrap();
    assert_eq!(terrain::column(x, z).height, bed);
    println!("valley side at ({x}, {z}): {} cut down to {bed}; {n} river nodes agree", terrain::land_height(x, z));
    // The icesheet: snow blocks over cave ice, to -8.
    let (x, z) = find(&|x, z| { let c = terrain::column(x, z); c.biome == Biome::Icesheet && c.height > WATER_LEVEL + 3 && c.cave_top < c.height });
    let h = terrain::column(x, z).height;
    for d in 0..=4 { assert_eq!(w.node((x, h - d, z)), CONTENT_SNOW_BLOCK, "snow block {d} down"); }
    let mut cave_ice = 0;
    for y in terrain::ICESHEET_BOTTOM..h - 4 {
        let id = w.node((x, y, z));
        assert!(matches!(id, CONTENT_CAVE_ICE | world::CONTENT_AIR | CONTENT_GRAVEL) || id == crate::dungeons::MOSSYCOBBLE || id == CONTENT_ICE || crate::stairs::shape_of(id).is_some(), "{} at {y}", content::name_of(id));
        cave_ice += (id == CONTENT_CAVE_ICE) as u32;
    }
    assert!(cave_ice > 0);
    assert!(!matches!(w.node((x, terrain::ICESHEET_BOTTOM - 5, z)), CONTENT_CAVE_ICE));
    let n = blocks_agree(&w, x, z, -24, h + 8);
    println!("icesheet at ({x}, {h}, {z}): {cave_ice} cave ice under the snow; {n} nodes agree");
    assert_eq!(content::drop_of(CONTENT_CAVE_ICE), Some((CONTENT_ICE, 1)));
    // Its dungeons are of ice: look under the icesheet near there.
    let ice_stair = crate::stairs::BUILDING.first_stair() + 14;
    let mut walls = 0;
    'd: for cz in (z - 160..z + 160).step_by(2) { for cx in (x - 160..x + 160).step_by(2) {
        let c = terrain::column(cx, cz);
        if c.biome != Biome::Icesheet || c.height <= WATER_LEVEL { continue; }
        for y in -30..c.height - 6 {
            let id = w.node((cx, y, cz));
            if id == CONTENT_ICE || id == ice_stair { walls += 1; if walls > 50 { break 'd; } }
        }
    } }
    // (A mapchunk's dungeons take their nodes from the biome at its middle,
    // so one centred off the icesheet is cobblestone even under it.)
    println!("icesheet dungeon walls of ice found: {walls}");
    assert!(walls > 0);
    // Its lakes: ice ten deep, a snow block on top.
    let (x, z) = find(&|x, z| { let c = terrain::column(x, z); c.biome == Biome::Icesheet && c.height < WATER_LEVEL - 3 });
    let bed = terrain::column(x, z).height;
    for y in bed + 1..=WATER_LEVEL { assert_eq!(w.node((x, y, z)), if y > WATER_LEVEL - 10 { CONTENT_ICE } else { CONTENT_WATER }, "lake at {y}"); }
    assert_eq!(w.node((x, bed, z)), CONTENT_SNOW_BLOCK);
    assert_eq!(w.node((x, WATER_LEVEL + 1, z)), CONTENT_SNOW_BLOCK);
    assert_eq!(w.highest_solid(x, z), WATER_LEVEL + 1);
    println!("icesheet lake at ({x}, {z}): frozen to its bed at {bed}, snow on top");
    // An emergent jungle tree: roots and trunk forced into the ground, a
    // crown far above the jungle.
    let mut emergent: Vec<_> = terrain::trees_in(-1500, -1500, 1500, 1500).into_iter().filter(|t| t.kind == terrain::TreeKind::Emergent).collect();
    emergent.sort_by_key(|t| t.x.abs() + t.z.abs());
    let t = emergent[0];
    assert_eq!(w.node((t.x, t.ground, t.z)), CONTENT_JUNGLE_TREE, "trunk through the ground");
    let roots = (t.ground - 4..t.ground).flat_map(|y| (-3..=3).flat_map(move |dz| (-3..=3).map(move |dx| (t.x + dx, y, t.z + dz)))).filter(|&p| w.node(p) == CONTENT_JUNGLE_TREE).count();
    assert!(roots > 20, "roots {roots}");
    assert!((20..=32).contains(&(t.top() - t.ground)), "top {}", t.top() - t.ground);
    let crown = (t.top() - 4..=t.top()).flat_map(|y| (-3..=3).flat_map(move |dz| (-3..=3).map(move |dx| (t.x + dx, y, t.z + dz)))).filter(|&p| w.node(p) == CONTENT_JUNGLE_LEAVES).count();
    assert!(crown > 40, "crown {crown}");
    let n = blocks_agree(&w, t.x, t.z, t.ground - 8, t.top() + 2);
    println!("emergent jungle tree at ({}, {}, {}): {} tall, {roots} root nodes, {crown} crown leaves; {n} nodes agree; {} within 1500", t.x, t.ground, t.z, t.top() - t.ground, emergent.len());
    for y in (t.ground - 5..=t.top() + 1).rev() {
        let row: String = (t.x - 4..=t.x + 4).map(|x| match w.node((x, y, t.z)) { CONTENT_JUNGLE_TREE => '#', CONTENT_JUNGLE_LEAVES => '*', world::CONTENT_AIR => '.', id if content::is_liquid(id) => '~', _ => '=' }).collect();
        if std::env::var("SHOW_EMERGENT").is_ok() { println!("  {y:4} {row}"); }
    }
    println!("NEW TERRAIN OK");
}

fn trees() {
    use content::*;
    let d = format!("{OUT}/treeworld");
    let _ = std::fs::remove_dir_all(&d);
    let w = world::World::open(&d).unwrap();
    // Count trees and kinds over a 512x512 area, and dump a top-down map.
    let (mut birch, mut fir) = (0, 0);
    let mut map = Vec::new();
    for z in -256..256 { for x in -256..256 {
        let h = w.highest_solid(x, z);
        let id = w.node((x, h, z));
        if id == CONTENT_BIRCH_TREE || id == CONTENT_FIR_TREE {
            // a trunk seen from above would mean no crown: count trunks at ground+1 instead
        }
        let g = terrain::column(x, z).height;
        match w.node((x, g + 1, z)) { CONTENT_BIRCH_TREE => birch += 1, CONTENT_FIR_TREE => fir += 1, _ => {} }
        map.extend((h as i16).to_be_bytes()); map.extend(id.to_be_bytes());
    } }
    std::fs::write(format!("{OUT}/treemap.bin"), &map).unwrap();
    println!("trees in 512x512: {birch} birch, {fir} fir");
    assert!(birch > 100 && fir > 50);
    // Block path and single-node path agree, for blocks with trees in them.
    let mut checked = 0; let mut tree_nodes = 0;
    for (bx, bz) in [(0i16, 0i16), (3, -2), (-5, 7), (10, 10), (-12, -3), (6, 14), (-9, -11), (15, -15)] {
        let (lo, hi) = w.height_range(bx, bz);
        for by in world::block_of(lo - 1)..=world::block_of(hi + 1) {
            let raw = w.block_raw((bx, by, bz));
            for z in 0..16 { for y in 0..16 { for x in 0..16 {
                let k = 5 + 2 * (z * 256 + y * 16 + x) as usize;
                let got = u16::from_be_bytes([raw[k], raw[k + 1]]);
                let p = (bx as i32 * 16 + x, by as i32 * 16 + y, bz as i32 * 16 + z);
                assert_eq!(got, w.node(p), "block {:?} node {:?}", (bx, by, bz), p);
                if matches!(got, CONTENT_BIRCH_TREE | CONTENT_BIRCH_LEAVES | CONTENT_FIR_TREE | CONTENT_FIR_NEEDLES) { tree_nodes += 1; }
                checked += 1;
            } } }
        }
    }
    println!("block and single-node generation agree on {checked} nodes ({tree_nodes} tree nodes)");
    assert!(tree_nodes > 0);
    // Find a birch and a fir; print side views, and check the shade under them.
    for kind in [CONTENT_BIRCH_TREE, CONTENT_FIR_TREE] {
        let (mut tx, mut tz) = (i32::MAX, 0);
        'f: for z in -200..200 { for x in -200..200 {
            let g = terrain::column(x, z).height;
            if w.node((x, g + 1, z)) == kind && (-4..=4).all(|d| w.node((x + d, g + 1, z)) == kind || d != 0) { tx = x; tz = z; break 'f; }
        } }
        assert!(tx != i32::MAX);
        let g = terrain::column(tx, tz).height;
        println!("{} at ({tx}, {g}, {tz}):", content::name_of(kind));
        for y in (g + 1..=g + 12).rev() {
            let row: String = (tx - 4..=tx + 4).map(|x| match w.node((x, y, tz)) {
                CONTENT_BIRCH_TREE | CONTENT_FIR_TREE => '#', CONTENT_BIRCH_LEAVES | CONTENT_FIR_NEEDLES => '*',
                world::CONTENT_AIR => '.', _ => '=' }).collect();
            println!("   {row}");
        }
        // day light on the ground beside the trunk vs. in the open
        let raw = w.block_raw(world::block_of_node((tx + 1, g + 1, tz)));
        let lx = |x: i32, y: i32, z: i32| { let (x, y, z) = (x.rem_euclid(16), y.rem_euclid(16), z.rem_euclid(16)); raw[5 + 2 * 4096 + (z * 256 + y * 16 + x) as usize] & 15 };
        println!("   day light beside the trunk {}", lx(tx + 1, g + 1, tz));
        // standing on it: highest_solid is the crown
        assert!(w.highest_solid(tx, tz) > g + 3);
    }
    // Digging a log and leaves works like anything else.
    println!("TREES OK");
}

fn wifi_file() {
    use wifi_config::*;
    let d = format!("{OUT}/wifitest");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let files = Files { networks: &format!("{d}/wifi.txt"), written: &format!("{d}/wifi-written") };
    let home = Network { ssid: "yellow house".into(), password: "s3cret pass".into() };
    // No file: the built-in network is written.
    assert_eq!(load(&files, Some(home.clone()), "9A0B").networks, vec![home.clone()]);
    let text = std::fs::read_to_string(files.networks).unwrap();
    println!("{text}");
    // Hand edit: a second network, and a changed password. Same firmware: kept.
    std::fs::write(files.networks, "# mine\nnetwork yellow house\npassword changed\n\nnetwork Cafe\n  \nnetwork Office\r\npassword p w\r\n").unwrap();
    let nets = load(&files, Some(home.clone()), "9A0B").networks;
    assert_eq!(nets, vec![
        Network { ssid: "yellow house".into(), password: "changed".into() },
        Network { ssid: "Cafe".into(), password: "".into() },
        Network { ssid: "Office".into(), password: "p w".into() },
    ]);
    assert!(std::fs::read_to_string(files.networks).unwrap().starts_with("# mine"), "hand edits kept");
    // New firmware with a different built-in network: added at the top, rest kept.
    let new = Network { ssid: "Office".into(), password: "new".into() };
    let nets = load(&files, Some(new.clone()), "9A0B").networks;
    assert_eq!(nets[0], new); assert_eq!(nets.len(), 3);
    // No built-in network: the file as it is.
    let config = load(&files, None, "9A0B");
    assert_eq!(config.networks.len(), 3);
    // The default hotspot was added to the file, and is the one used.
    assert_eq!((config.hotspot.ssid.as_str(), config.hotspot.password.as_str()), ("MTG-9A0B", "minetest"));
    assert!(std::fs::read_to_string(files.networks).unwrap().contains("hotspot MTG-9A0B\npassword minetest\n"));
    // A file with the old default hotspot gets the board's id, the rest kept.
    let old = "# mine\nnetwork Cafe\n\nhotspot Luanti\npassword LuantiLuanti\n";
    std::fs::write(files.networks, old).unwrap();
    assert_eq!(load(&files, None, "9A0B").hotspot.ssid, "MTG-9A0B");
    assert_eq!(std::fs::read_to_string(files.networks).unwrap(), old.replace("hotspot Luanti\npassword LuantiLuanti\n", "hotspot MTG-9A0B\npassword minetest\n"));
    assert!(join_qr(&config.hotspot).is_some());
    // Too long SSIDs are skipped.
    assert!(parse(&format!("network {}\n", "x".repeat(33))).networks.is_empty());
    println!("WIFI FILE OK");
}

fn led() {
    use indicator::*;
    use std::time::Duration;
    let ms = Duration::from_millis;
    let mut ind = Indicator::new();
    let run = State::Running { players: 0, colour: screen::chip_colour(40.0) };
    // Breathing: dim to bright and back over 4 s, never above 60% power.
    let peak = |ind: &mut Indicator, state: State| {
        let cs: Vec<(u8, u8, u8)> = (0..134).map(|_| { let c = ind.colour(ms(30), state); (to_byte(c.0), to_byte(c.1), to_byte(c.2)) }).collect();
        (*cs.iter().max_by_key(|c| c.0 as u32 + c.1 as u32 + c.2 as u32).unwrap(), cs.iter().map(|c| c.1).min().unwrap())
    };
    let (top, lo) = peak(&mut ind, run);
    println!("breathing, cool: brightest {top:?}, dimmest green {lo}");
    assert!(top.1 > top.0 && top.1 > top.2 && top.1 <= 153 && lo < 5);
    // Warm: amber (red over green, little blue); hot: red.
    let (warm, _) = peak(&mut Indicator::new(), State::Running { players: 0, colour: screen::chip_colour(65.0) });
    let (hot, _) = peak(&mut Indicator::new(), State::Running { players: 0, colour: screen::chip_colour(80.0) });
    println!("warm {warm:?}, hot {hot:?}");
    assert!(warm.0 > warm.1 && warm.1 > warm.2 * 3 && hot.0 > hot.1 * 3 && hot.0 > hot.2 * 3);
    // A join: dark gap, blue, gap, blue.
    ind.event(Event::Join);
    let seq: Vec<(u8, u8, u8)> = (0..25).map(|_| { let c = ind.colour(ms(30), run); (to_byte(c.0), to_byte(c.1), to_byte(c.2)) }).collect();
    assert_eq!(seq[0], (0, 0, 0));
    assert!(seq.iter().any(|c| *c == (0, 0, 153)));
    // No Wi-Fi: amber blinking.
    let mut ind = Indicator::new();
    let blink: Vec<bool> = (0..40).map(|_| ind.colour(ms(50), State::NoWifi).0 > 0.0).collect();
    assert!(blink.iter().any(|b| *b) && blink.iter().any(|b| !*b));
    println!("LED OK");
}

fn coal_stats() {
    let at = |x: i32, y: i32, z: i32| { let col = terrain::column(x, z); if y > col.height { world::CONTENT_AIR } else { terrain::node(x, y, z, &col, &mut |a, b, c| terrain::cave_sample(a, b, c)) } };
    let (mut stone, mut coal, mut wall, mut exposed) = (0u32, 0u32, 0u32, 0u32);
    for z in -56..72 { for x in -56..72 {
        let h = terrain::column(x, z).height;
        for y in (h - 60)..=h {
            let n = at(x, y, z);
            if n == content::CONTENT_STONE { stone += 1 }
            if n == content::CONTENT_COAL { coal += 1 }
            if (n == content::CONTENT_STONE || n == content::CONTENT_COAL)
                && [(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)].iter().any(|&(dx,dy,dz)| at(x+dx,y+dy,z+dz) == world::CONTENT_AIR) {
                wall += 1; if n == content::CONTENT_COAL { exposed += 1 }
            }
        }
    } }
    println!("stone {stone} coal {coal} ({:.2}%); exposed faces {wall}, coal {exposed} ({:.2}%)",
        100.0 * coal as f64 / (stone + coal) as f64, 100.0 * exposed as f64 / wall as f64);
}

/// Iron and coal by depth, as the map generator makes them, around spawn.
fn iron_stats() {
    let at = |x: i32, y: i32, z: i32| { let col = terrain::column(x, z); if y > col.height { world::CONTENT_AIR } else { terrain::node(x, y, z, &col, &mut |a, b, c| terrain::cave_sample(a, b, c)) } };
    for (lo, hi) in [(-1, 0), (-4, -2), (-8, -5), (-16, -9), (-32, -17)] {
        let (mut stone, mut iron, mut coal) = (0u32, 0u32, 0u32);
        for z in -64..64 { for x in -64..64 { for y in lo..=hi {
            match at(x, y, z) {
                n if n == content::CONTENT_STONE => stone += 1,
                n if n == content::CONTENT_IRON => iron += 1,
                n if n == content::CONTENT_COAL => coal += 1,
                _ => {}
            }
        } } }
        println!("y {lo}..{hi}: stone {stone}, iron {iron} (1 in {}), coal {coal}", (stone + iron + coal) / iron.max(1));
    }
    let spawn = terrain::column(8, 8).height;
    println!("ground height at spawn: {spawn}");
}

/// The server's log on stderr (TEST_LOGS=1, as tests/run.py sets it).
struct StderrLog;
impl log::Log for StderrLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        eprintln!("{} {}", r.level(), r.args());
    }
    fn flush(&self) {}
}
static STDERR_LOG: StderrLog = StderrLog;

fn serve(port: &str) {
    if std::env::var("TEST_LOGS").is_ok() {
        let _ = log::set_logger(&STDERR_LOG);
        log::set_max_level(log::LevelFilter::Info);
    }
    let d = format!("{OUT}/serveworld");
    if std::env::var("KEEP_WORLD").is_err() {
        let _ = std::fs::remove_dir_all(&d);
    }
    let socket = std::net::UdpSocket::bind(format!("127.0.0.1:{port}")).unwrap();
    // An SD card skin in the 64x64 format, and one of the wrong size.
    std::fs::create_dir_all(format!("{d}/media")).unwrap();
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/data/skin_grey_wizard.png"), format!("{d}/media/skin_grey_wizard.png")).unwrap();
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/media/esp_torch_on_floor.png"), format!("{d}/media/skin_tiny.png")).unwrap();
    let setup = server::Setup {
        socket,
        motd: None,
        auth: auth::AuthStore::open(format!("{d}/auth")).unwrap(),
        world: world::World::open(format!("{d}/world")).unwrap(),
        inventories: inventory::InventoryStore::open(format!("{d}/players")).unwrap(),
        players: player::PlayerStore::open(format!("{d}/players")).unwrap(),
        furnaces: furnace::Furnaces::open(format!("{d}/world/furnaces.txt")),
        chests: chest::Chests::open(format!("{d}/world/chests.txt")),
        items: items::Items::open(format!("{d}/world/items.txt")),
        clock: clock::Clock::open(format!("{d}/world/env_meta.txt")),
        door_owners: doors::Owners::open(format!("{d}/world/doors.txt")),
        bed_spawns: beds::Spawns::open(format!("{d}/world/beds_spawns")),
        settings: settings::Settings::open(format!("{d}/world/settings.txt")),
        media: content::Media::load(&format!("{d}/media")),
        status: status::Status::new(format!("127.0.0.1:{port}")),
        max_players: server::MAX_PLAYERS,
    };
    {
        // Report Wi-Fi power saving changes.
        let st = setup.status.clone();
        std::thread::spawn(move || {
            let mut was = false;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let now = status::lock(&st).wifi_dozing;
                if now != was { eprintln!("DOZING {now}"); was = now; }
            }
        });
    }
    server::Server::new(setup).run().unwrap();
}

/// RIVERS=1: rivers, the icesheet and emergent jungle trees: how much
/// there is, where the nearest are, and a map (out/rivers.bin: per sample
/// of a 1024 x 1024 grid every 4 nodes, biome, river flag, height).
fn rivers() {
    if std::env::var("RIVER_SEEDS").is_ok() {
        // Seeds whose rivers carve nothing within 160 nodes of the spawn.
        let mut best = Vec::new();
        for k in 0..400u32 {
            let seed = 0x6B7C_8D9Eu32.wrapping_add(k.wrapping_mul(0x9E37_79B9));
            let mut near = i32::MAX;
            for z in (-160..=176).step_by(4) { for x in (-160..=176).step_by(4) {
                if let Some(b) = terrain::river_bed_seeded(x, z, seed) {
                    if b < terrain::land_height(x, z) { near = near.min((x - 8).abs().max((z - 8).abs())); }
                }
            } }
            best.push((near, seed));
        }
        best.sort();
        for (near, seed) in best.iter().rev().take(8) { println!("seed {seed:#x}: nearest carved {near}"); }
        return;
    }
    use terrain::Biome;
    let (mut river, mut ice, mut total) = (0u32, 0u32, 0u32);
    let mut nearest_river: Option<(i32, i32, i32)> = None;
    let mut map = Vec::new();
    for zi in 0..1024 { for xi in 0..1024 {
        let (x, z) = ((xi - 512) * 4, (zi - 512) * 4);
        let c = terrain::column(x, z);
        let r = terrain::river_bed(x, z).is_some();
        total += 1;
        if r { river += 1; let d = x.abs() + z.abs(); if nearest_river.map_or(true, |(_, _, e)| d < e) { nearest_river = Some((x, z, d)); } }
        if c.biome == Biome::Icesheet { ice += 1; }
        map.push(c.biome as u8); map.push(r as u8); map.extend((c.height.clamp(-100, 200) as i16).to_be_bytes());
    } }
    std::fs::write(format!("{OUT}/rivers.bin"), &map).unwrap();
    for z in (-24..=40).step_by(4) {
        let row: String = (-40..=60).step_by(2).map(|x| match terrain::river_bed(x, z) { Some(b) if b < terrain::column(x, z).height + 1 && b <= terrain::column(x, z).height => if terrain::column(x, z).height <= terrain::WATER_LEVEL { '~' } else { '^' }, Some(_) => '-', None => '.' }).collect();
        println!("z {z:4} {row}");
    }
    println!("river valleys {:.1}% and icesheet {:.1}% of 4096 x 4096 nodes; nearest river valley to the spawn {nearest_river:?}", river as f32 * 100.0 / total as f32, ice as f32 * 100.0 / total as f32);
    // Emergent jungle trees in a 2048 x 2048 square.
    let trees = terrain::trees_in(-1024, -1024, 1023, 1023);
    let emergent: Vec<_> = trees.iter().filter(|t| t.kind == terrain::TreeKind::Emergent).collect();
    let jungle = trees.iter().filter(|t| t.kind == terrain::TreeKind::Jungle).count();
    println!("emergent jungle trees {} (jungle trees {jungle}); nearest {:?}", emergent.len(),
        emergent.iter().min_by_key(|t| t.x.abs() + t.z.abs()).map(|t| (t.x, t.ground, t.z, t.top())));
}

fn biomes() {
    use content::*;
    use terrain::Biome;
    // A biome map: 512x512 samples, one every 16 nodes (8192 nodes across).
    let code = |b: Biome| match b { Biome::Grassland => 0u8, Biome::Desert => 1, Biome::Snow => 2, Biome::Savanna => 3, Biome::Rainforest => 4, Biome::Tundra => 5, Biome::ColdDesert => 6, Biome::Icesheet => 7, Biome::SandstoneDesert => 8, Biome::SnowyGrassland => 9 };
    let mut map = Vec::new();
    let mut counts = [0u32; 10];
    let (mut rain, mut sav) = (None, None);
    for zi in 0..512 { for xi in 0..512 {
        let (x, z) = ((xi - 256) * 16, (zi - 256) * 16);
        let c = terrain::column(x, z);
        let b = code(c.biome);
        counts[b as usize] += 1;
        let d = x.abs() + z.abs();
        let water = c.height <= terrain::WATER_LEVEL;
        if !water && c.biome == Biome::Rainforest && rain.map_or(true, |(_, _, e)| d < e) { rain = Some((x, z, d)); }
        if !water && c.biome == Biome::Savanna && sav.map_or(true, |(_, _, e)| d < e) { sav = Some((x, z, d)); }
        map.push(b); map.extend((c.height.clamp(-100, 200) as i16).to_be_bytes());
    } }
    std::fs::write(format!("{OUT}/biomes.bin"), &map).unwrap();
    let total: u32 = counts.iter().sum();
    println!("biomes (grassland, desert, snow, savanna, rainforest, tundra, cold desert, icesheet, sandstone desert, snowy grassland): {:?}%", counts.map(|c| (c * 1000 / total) as f32 / 10.0));
    println!("nearest rainforest {rain:?}, savanna {sav:?}");
    let d = format!("{OUT}/biomeworld");
    let _ = std::fs::remove_dir_all(&d);
    let w = world::World::open(&d).unwrap();
    for (name, spot) in [("rainforest", rain), ("savanna", sav)] {
        let (cx, cz, _) = spot.unwrap();
        // A top-down view 192 nodes across, and tree counts.
        let mut img = Vec::new();
        let (mut jungle, mut acacia, mut other) = (0, 0, 0);
        for z in cz - 96..cz + 96 { for x in cx - 96..cx + 96 {
            let h = w.highest_solid(x, z);
            let id = w.node((x, h, z));
            let above = w.node((x, h + 1, z));
            img.extend((h as i16).to_be_bytes()); img.extend(id.to_be_bytes()); img.extend(above.to_be_bytes());
            let g = terrain::column(x, z).height;
            match w.node((x, g + 1, z)) { CONTENT_JUNGLE_TREE => jungle += 1, CONTENT_ACACIA_TREE => acacia += 1, CONTENT_BIRCH_TREE | CONTENT_FIR_TREE | CONTENT_APPLE_TREE => other += 1, _ => {} }
        } }
        std::fs::write(format!("{OUT}/{name}.bin"), &img).unwrap();
        println!("{name} around ({cx}, {cz}): {jungle} jungle, {acacia} acacia, {other} other trees in 192x192");
        // Every leaf of every generated tree there has a trunk of its own
        // tree within the decay radius, and blocks agree with single nodes.
        let trees = terrain::trees_in(cx - 96, cz - 96, cx + 96, cz + 96);
        let mut leaves = 0;
        for t in &trees {
            for y in t.ground + 1..=t.top() { for z in t.z - 3..=t.z + 3 { for x in t.x - 3..=t.x + 3 {
                let Some(id) = t.node_at(x, y, z) else { continue };
                if let Some(trunk) = content::decay_trunk(id) {
                    leaves += 1;
                    let r = content::LEAF_DECAY_RADIUS;
                    let ok = (-r..=r).any(|dy| (-r..=r).any(|dz| (-r..=r).any(|dx| t.node_at(x + dx, y + dy, z + dz) == Some(trunk))));
                    assert!(ok, "{:?} leaf at {:?} has no trunk near", t.kind, (x, y, z));
                }
            } } }
        }
        println!("  {} trees, {leaves} leaves, all within reach of their trunk", trees.len());
        let (bx, bz) = ((cx / 16) as i16, (cz / 16) as i16);
        let (lo, hi) = w.height_range(bx, bz);
        let mut n = 0;
        for by in world::block_of(lo - 1)..=world::block_of(hi + 14) {
            let raw = w.block_raw((bx, by, bz));
            for k in 0..4096 {
                let (z, y, x) = (k / 256, (k / 16) % 16, k % 16);
                let got = u16::from_be_bytes([raw[5 + 2 * k as usize], raw[6 + 2 * k as usize]]);
                let p = (bx as i32 * 16 + x, by as i32 * 16 + y, bz as i32 * 16 + z);
                assert_eq!(got, w.node(p), "block and node differ at {p:?}");
                n += 1;
            }
        }
        println!("  block and single-node generation agree on {n} nodes");
        // A side view of one tree.
        let kind = if name == "rainforest" { CONTENT_JUNGLE_TREE } else { CONTENT_ACACIA_TREE };
        if let Some(t) = trees.iter().find(|t| t.node_at(t.x, t.ground + 1, t.z) == Some(kind) && w.node((t.x, t.ground + 1, t.z)) == kind) {
            for z in [t.z - 2, t.z, t.z + 2] {
                println!("  slice z = {}:", z - t.z);
                for y in (t.ground..=t.top()).rev() {
                    let row: String = (t.x - 3..=t.x + 3).map(|x| match t.node_at(x, y, z) {
                        Some(id) if content::is_trunk(id) => '#', Some(_) => '*', None => '.' }).collect();
                    println!("     {row}");
                }
            }
        }
    }
    // Every biome: blocks from well underground to above the trees agree
    // with single nodes (gravel blobs, desert stone, permafrost, ice, snow).
    for want in [Biome::Desert, Biome::Snow, Biome::Tundra, Biome::ColdDesert, Biome::Grassland, Biome::SandstoneDesert, Biome::SnowyGrassland] {
        let mut spot = None;
        'f: for r in (0..3000).step_by(16) {
            for (x, z) in (-r..=r).step_by(16).flat_map(|a| [(a, -r), (a, r), (-r, a), (r, a)]) {
                let c = terrain::column(x, z);
                if c.biome == want && c.height > 3 { spot = Some((x, z, c.height)); break 'f; }
            }
        }
        let (x, z, h) = spot.unwrap();
        let (bx, bz) = ((x.div_euclid(16)) as i16, (z.div_euclid(16)) as i16);
        let mut n = 0;
        let mut kinds: std::collections::BTreeMap<&str, u32> = Default::default();
        for by in world::block_of(h - 40)..=world::block_of(h + 16) {
            let raw = w.block_raw((bx, by, bz));
            for k in 0..4096usize {
                let (zz, yy, xx) = ((k / 256) as i32, ((k / 16) % 16) as i32, (k % 16) as i32);
                let got = u16::from_be_bytes([raw[5 + 2 * k], raw[6 + 2 * k]]);
                let p = (bx as i32 * 16 + xx, by as i32 * 16 + yy, bz as i32 * 16 + zz);
                assert_eq!(got, w.node(p), "block and node differ at {p:?} in {want:?}");
                *kinds.entry(content::name_of(got)).or_default() += 1;
                n += 1;
            }
        }
        println!("{want:?} at ({x}, {h}, {z}): {n} nodes agree; {kinds:?}");
    }
    // Cold lakes freeze over.
    let mut ice = 0;
    for z in (-1500..1500).step_by(7) { for x in (-1500..1500).step_by(7) {
        let c = terrain::column(x, z);
        if c.height < terrain::WATER_LEVEL && matches!(c.biome, Biome::Snow | Biome::Tundra | Biome::ColdDesert) {
            let top = w.node((x, terrain::WATER_LEVEL, z));
            // (A dungeon may break through the ice, as Luanti's do.)
            if top == content::CONTENT_COBBLE || top == dungeons::MOSSYCOBBLE { println!("dungeon wall in the lake ice at {:?}", (x, z)); continue; }
            assert_eq!(top, content::CONTENT_ICE, "at {:?}", (x, z));
            if c.height < terrain::WATER_LEVEL - 1 { assert_eq!(w.node((x, terrain::WATER_LEVEL - 1, z)), content::CONTENT_WATER); }
            ice += 1;
        }
    } }
    println!("{ice} frozen lake columns checked");
    // Plants over a wide area: what's on each column's surface, and on lake
    // beds.
    let mut counts: std::collections::BTreeMap<&str, u32> = Default::default();
    let mut kelp_heights = Vec::new();
    for z in (-1500..1500).step_by(3) { for x in (-1500..1500).step_by(3) {
        let c = terrain::column(x, z);
        if c.height <= -2 {
            let id = w.node((x, c.height, z));
            if crate::plants::is_rooted(id) || matches!(id, crate::plants::CORAL_BROWN | crate::plants::CORAL_ORANGE | crate::plants::CORAL_SKELETON) {
                *counts.entry(content::name_of(id)).or_default() += 1;
                if id == crate::plants::KELP { kelp_heights.push(w.param2((x, c.height, z)) / 16); }
            }
            continue;
        }
        let id = w.node((x, c.height.max(terrain::WATER_LEVEL) + 1, z));
        if id != world::CONTENT_AIR && !content::is_liquid(id) {
            *counts.entry(content::name_of(id)).or_default() += 1;
        }
    } }
    println!("surface plants (every 3rd column, 3000x3000): {counts:?}");
    kelp_heights.sort();
    println!("kelp heights: {:?}", kelp_heights.iter().fold(std::collections::BTreeMap::<u8, u32>::new(), |mut m, h| { *m.entry(*h).or_default() += 1; m }));
    for name in ["esp:cactus", "esp:papyrus", "esp:bush_stem", "esp:blueberry_bush_leaves_with_berries", "esp:acacia_bush_stem", "esp:fir_bush_stem", "esp:fern_1", "esp:marram_grass_1", "esp:sand_with_kelp", "esp:coral_pink"] {
        let n: u32 = counts.iter().filter(|(k, _)| k.starts_with(name.trim_end_matches("_1"))).map(|(_, v)| *v).sum();
        assert!(n > 0, "no {name} found");
    }
    println!("BIOMES OK");
}

fn find_plants() {
    // The nearest generated kelp, rooted coral and berried bush to the
    // spawn, for plantstest.py.
    let d = format!("{OUT}/findworld");
    let _ = std::fs::remove_dir_all(&d);
    let w = world::World::open(&d).unwrap();
    let mut found: Vec<(&str, (i32, i32, i32))> = Vec::new();
    for (name, want) in [("kelp", crate::plants::KELP), ("coral", crate::plants::CORAL_GREEN)] {
        'r: for r in 0..1500 {
            for (x, z) in (-r..=r).flat_map(|a| [(a, -r), (a, r), (-r, a), (r, a)]) {
                let c = terrain::column(x, z);
                if c.height <= -2 {
                    let id = w.node((x, c.height, z));
                    if id == want || (want == crate::plants::CORAL_GREEN && crate::plants::is_rooted(id) && id != crate::plants::KELP) {
                        found.push((name, (x, c.height, z)));
                        break 'r;
                    }
                }
            }
        }
    }
    'b: for r in 0..1500 {
        for (x, z) in (-r..=r).flat_map(|a| [(a, -r), (a, r), (-r, a), (r, a)]) {
            let c = terrain::column(x, z);
            if w.node((x, c.height + 1, z)) == crate::plants::BLUEBERRY_LEAVES_WITH_BERRIES {
                found.push(("berries", (x, c.height + 1, z)));
                break 'b;
            }
        }
    }
    'l: for r in 0..1500 {
        for (x, z) in (-r..=r).flat_map(|a| [(a, -r), (a, r), (-r, a), (r, a)]) {
            let c = terrain::column(x, z);
            if c.height < terrain::WATER_LEVEL - 1 && w.node((x, terrain::WATER_LEVEL, z)) == content::CONTENT_WATER && w.node((x, terrain::WATER_LEVEL + 1, z)) == world::CONTENT_AIR {
                found.push(("water", (x, terrain::WATER_LEVEL, z)));
                break 'l;
            }
        }
    }
    // The nearest dry shrub, and how common they are in the deserts.
    let (mut desert, mut shrubs) = (0, 0);
    let mut nearest: Option<(i32, (i32, i32, i32))> = None;
    for z in (-1500..1500).step_by(3) { for x in (-1500..1500).step_by(3) {
        let c = terrain::column(x, z);
        if !matches!(c.biome, terrain::Biome::Desert | terrain::Biome::ColdDesert) || c.height < 1 { continue; }
        desert += 1;
        if terrain::plant_on(x, z, &c) == Some(content::DRY_SHRUB) {
            shrubs += 1;
            let d = x * x + z * z;
            if nearest.is_none_or(|(nd, _)| d < nd) { nearest = Some((d, (x, c.height + 1, z))); }
        }
    } }
    println!("dry shrubs on {:.2}% of desert columns", 100.0 * shrubs as f32 / desert.max(1) as f32);
    if let Some((_, p)) = nearest {
        assert_eq!(w.node(p), content::DRY_SHRUB);
        assert_eq!(w.param2(p), content::DRY_SHRUB_PARAM2);
        found.push(("shrub", p));
    }
    for (n, p) in &found { println!("FOUND {n} {} {} {}", p.0, p.1, p.2); }
}

fn ore_stats() {
    let d = format!("{OUT}/oreworld");
    let _ = std::fs::remove_dir_all(&d);
    let w = world::World::open(&d).unwrap();
    for y0 in [-40, -100, -300, -600, -1100, -2100] {
        let mut counts: std::collections::BTreeMap<&str, u32> = Default::default();
        let mut stone = 0;
        for y in y0..y0 + 48 { for z in 0..64 { for x in 0..64 {
            let id = w.node((x, y, z));
            if id == content::CONTENT_STONE { stone += 1; } else if id != world::CONTENT_AIR && !content::is_liquid(id) {
                *counts.entry(content::name_of(id)).or_default() += 1;
            }
        } } }
        println!("y {y0}..{}: stone {stone}, {counts:?}", y0 + 48);
    }
}

fn main() {
    std::fs::create_dir_all(OUT).unwrap();
    if let Ok(port) = std::env::var("SERVE") { serve(&port); return; }
    if let Ok(path) = std::env::var("PLAYER_INIT") {
        let d = messages::player_init_data(&messages::PlayerInit { name: "Steve", id: 2, pos_bs: [1.0, 2.0, 3.0], yaw: 45.0, anim: messages::Animation::new(messages::ANIM_STAND, 30.0), skin: "character.png", zoom_fov: 0.0, posture: messages::Posture::Stand, attached: None });
        std::fs::write(path, d).unwrap();
        return;
    }
    if std::env::var("COAL_STATS").is_ok() { coal_stats(); return; }
    if std::env::var("RIVERS").is_ok() { rivers(); return; }
    if std::env::var("BIOMES").is_ok() { biomes(); return; }
    if std::env::var("ORE_STATS").is_ok() { ore_stats(); return; }
    if std::env::var("FIND_PLANTS").is_ok() { find_plants(); return; }
    if std::env::var("DUNGEONS").is_ok() { dungeon_stats(); return; }
    if std::env::var("IRON_STATS").is_ok() { iron_stats(); return; }
    if std::env::var("NEAREST_APPLE").is_ok() {
        let mut t: Vec<_> = terrain::trees_in(-60, -60, 76, 76).into_iter().filter(|t| t.kind == terrain::TreeKind::Apple).collect();
        t.sort_by_key(|t| (t.x - 8).pow(2) + (t.z - 8).pow(2));
        let w = world::World::open(format!("{OUT}/appleworld")).unwrap();
        for tree in &t {
            for y in tree.ground..=tree.top() { for z in tree.z - 2..=tree.z + 2 { for x in tree.x - 2..=tree.x + 2 {
                if w.node((x, y, z)) == content::CONTENT_APPLE {
                    println!("{} {} {} tree {} {} {} height {}", x, y, z, tree.x, tree.ground, tree.z, tree.height);
                    return;
                }
            } } }
        }
        println!("none");
        return;
    }
    if std::env::var("PLANTS").is_ok() {
        let w = world::World::builder();
        let (mut grass, mut cotton, mut cols) = (0, 0, 0);
        let mut first_cotton = None;
        for z in -150..150 { for x in -150..150 {
            let c = terrain::column(x, z);
            if c.biome != terrain::Biome::Grassland || c.height <= terrain::WATER_LEVEL { continue; }
            cols += 1;
            let id = w.node((x, c.height + 1, z));
            if farming::is_grass(id) { grass += 1; }
            if id == farming::COTTON_WILD { cotton += 1; if first_cotton.is_none() { first_cotton = Some((x, c.height + 1, z)); } }
        } }
        println!("{cols} grassland columns: {:.1}% tall grass, {:.2}% wild cotton; nearest-ish cotton at {:?}", 100.0 * grass as f32 / cols as f32, 100.0 * cotton as f32 / cols as f32, first_cotton);
        return;
    }
    if std::env::var("FLOODSIM").is_ok() {
        // Water poured on real terrain around (60, 90): does it settle?
        let d = format!("{OUT}/floodworld");
        let _ = std::fs::remove_dir_all(&d);
        let mut w = world::World::open(&d).unwrap();
        let mut n = 0;
        for z in (70..110).step_by(7) { for x in (40..80).step_by(7) {
            let y = w.highest_solid(x, z) + 1;
            w.set_node((x, y, z), content::CONTENT_WATER, 0);
            // and dig a hole beside it, two deep
            w.set_node((x + 1, y - 1, z), world::CONTENT_AIR, 0);
            w.set_node((x + 1, y - 2, z), world::CONTENT_AIR, 0);
            n += 1;
        } }
        println!("{n} sources");
        let mut last = Vec::new();
        for step in 0..400 {
            let ch = w.flow_liquids();
            if step % 10 == 0 || ch.is_empty() { println!("step {step}: {} changed, {} queued", ch.len(), w.liquid_queue_len()); }
            if ch.is_empty() { break; }
            if step > 380 { last.extend(ch); }
        }
        for c in last.iter().take(30) { println!("{:?}", c); }
        return;
    }
    if std::env::var("CAVEFLOOD").is_ok() {
        // A lake over a cave: dig a shaft from the lake bed down into the
        // cave and let it flood. How many changes, for how long?
        let d = format!("{OUT}/caveflood");
        let _ = std::fs::remove_dir_all(&d);
        let mut w = world::World::open(&d).unwrap();
        let mut found = None;
        'f: for r in 0..300 { for z in -r..=r { for x in [-r, r] {
            let (x, z) = (8 + x, 8 + z);
            if w.node((x, 1, z)) != content::CONTENT_WATER { continue; }
            let mut y = 1;
            while w.node((x, y, z)) == content::CONTENT_WATER { y -= 1; }
            for dy in 1..12 { if w.node((x, y - dy, z)) == world::CONTENT_AIR { found = Some((x, y, z, dy)); break 'f; } }
        } } }
        let (x, bed, z, depth) = found.unwrap();
        println!("lake at ({x}, {z}) bed {bed}, cave {depth} below");
        for y in (bed - depth + 1)..=bed { w.set_node((x, y, z), world::CONTENT_AIR, 0); }
        let mut total = 0;
        let t = std::time::Instant::now();
        for step in 0..1200 {
            let ch = w.flow_liquids();
            total += ch.len();
            let mut blocks = std::collections::HashSet::new();
            for c in &ch { blocks.insert(world::block_of_node(c.pos)); }
            if step % 20 == 0 || ch.is_empty() { println!("step {step}: {} changed in {} blocks, {} queued, {total} so far", ch.len(), blocks.len(), w.liquid_queue_len()); }
            if ch.is_empty() { break; }
        }
        println!("{:?} on the PC", t.elapsed());
        return;
    }
    if std::env::var("FUZZFLOW").is_ok() {
        // Random digging and pouring around (60, 90): does water always settle?
        let d = format!("{OUT}/fuzzworld");
        let _ = std::fs::remove_dir_all(&d);
        let mut w = world::World::open(&d).unwrap();
        let mut seed: u64 = std::env::var("FUZZFLOW").unwrap().parse().unwrap_or(1);
        let mut rnd = move |n: i32| { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed % n as u64) as i32 };
        for round in 0..200 {
            for _ in 0..5 {
                let (x, z) = (45 + rnd(30), 75 + rnd(30));
                let top = w.highest_solid(x, z);
                let y = top - rnd(3);
                if rnd(4) == 0 { w.set_node((x, top + 1, z), content::CONTENT_WATER, 0); }
                else { w.set_node((x, y, z), world::CONTENT_AIR, 0); }
            }
            let mut steps = 0;
            loop {
                let ch = w.flow_liquids();
                if ch.is_empty() { break; }
                steps += 1;
                if steps > 300 {
                    println!("round {round}: still changing after 300 steps: {} changed, {} queued", ch.len(), w.liquid_queue_len());
                    for c in ch.iter().take(20) { println!("  {:?}", c); }
                    return;
                }
            }
        }
        println!("all rounds settled");
        return;
    }
    if std::env::var("LAVASTATS").is_ok() {
        // Deep caves: how much lava, and does any lava touch open cave
        // (which would flow when disturbed, or look like a wall)?
        let w = world::World::builder();
        let (mut lava, mut air, mut open_sides) = (0u32, 0u32, 0u32);
        let mut first = None;
        let r = 128;
        for z in -r..r { for x in -r..r { for y in -100..=-64 {
            let n = w.node((x, y, z));
            if n == content::CONTENT_LAVA {
                lava += 1;
                if first.is_none() { first = Some((x, y, z)); }
                for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0)] {
                    if w.node((x + dx, y + dy, z + dz)) == world::CONTENT_AIR { open_sides += 1; }
                }
            } else if n == world::CONTENT_AIR { air += 1; }
        } } }
        println!("y -100..-64 over {}x{}: lava {lava}, cave air {air}, lava faces open to air (sides or below) {open_sides}; first lava at {first:?}", 2 * r, 2 * r);
        let t = std::time::Instant::now();
        let mut n = 0;
        for bz in -4..4 { for bx in -4..4 { for by in -7..-3 { let _ = w.block_payload((bx, by, bz)); n += 1; } } }
        println!("{n} deep blocks built in {:?}", t.elapsed());
        return;
    }
    if std::env::var("DIGBENCH").is_ok() {
        // What one dig costs the world: with many earlier changes, a block
        // builder holding a snapshot, and torches nearby.
        let d = format!("{OUT}/digbench");
        let _ = std::fs::remove_dir_all(&d);
        let mut w = world::World::open(&d).unwrap();
        for i in 0..20000 { w.set_node((1000 + i % 100, -40, 1000 + i / 100), world::CONTENT_AIR, 0); }
        w.flush_log();
        for t in 0..3 { w.set_node((10 * t, -20, 0), content::CONTENT_TORCH_WALL, 2); }
        let run = |w: &mut world::World, label: &str, snap: bool, near: bool| {
            let t = std::time::Instant::now();
            for i in 0..100 {
                let _s = if snap { Some(w.snapshot()) } else { None };
                let x = if near { 5 + i % 10 } else { 500 + i % 10 };
                w.set_node((x, -21, i / 10), world::CONTENT_AIR, 0);
            }
            println!("{label}: {:?} per dig", t.elapsed() / 100);
        };
        run(&mut w, "plain", false, false);
        run(&mut w, "snapshot held", true, false);
        run(&mut w, "torches near", false, true);
        run(&mut w, "both", true, true);
        return;
    }
    if std::env::var("LAVASURFACE").is_ok() {
        let w = world::World::builder();
        for r in 0..200 { for z in -r..=r { for x in [-r, r] {
            if w.node((x, -64, z)) == content::CONTENT_LAVA && w.node((x, -63, z)) == world::CONTENT_AIR {
                println!("lava surface at {x} -64 {z}"); return;
            }
        } } }
        return;
    }
    if std::env::var("SHORE").is_ok() {
        // From the lake near (47, 47), east to the first dry ground at the
        // water's height: the shore node, and its height.
        let w = world::World::builder();
        let z = 47;
        let mut x = 47;
        while w.node((x, 1, z)) == content::CONTENT_WATER { x += 1; }
        println!("shore {} {} {} {}", x, 1, z, content::name_of(w.node((x, 1, z))));
        println!("above {}", content::name_of(w.node((x, 2, z))));
        println!("beyond {} {}", content::name_of(w.node((x + 1, 1, z))), content::name_of(w.node((x + 1, 0, z))));
        for zz in 46..56 { let row: Vec<String> = (60..70).map(|xx| format!("{:2}", w.highest_solid(xx, zz))).collect(); println!("z {zz}: {}", row.join(" ")); }
        return;
    }
    if std::env::var("LAKE").is_ok() {
        let w = world::World::builder();
        let mut best = None;
        'find: for r in 0..400 { for z in -r..=r { for x in [-r, r] {
            let c = terrain::column(8 + x, 8 + z);
            if c.height <= std::env::var("LAKE").unwrap().parse::<i32>().unwrap_or(-3) { best = Some((8 + x, 8 + z, c.height)); break 'find; }
        } } }
        let (x, z, h) = best.unwrap();
        println!("lake at ({x}, {z}), bed at {h}");
        for y in (h - 3..=3).rev() {
            println!("  y={y:3}: {}", content::name_of(w.node((x, y, z))));
        }
        println!("  highest solid: {}", w.highest_solid(x, z));
        println!("  height range of its block column: {:?}", w.height_range(world::block_of(x), world::block_of(z)));
        let raw = w.block_raw((world::block_of(x) as i16, 0, world::block_of(z) as i16));
        println!("  block raw {} bytes", raw.len());
        return;
    }
    if std::env::var("WATERSTATS").is_ok() {
        let (r, step) = (2000i32, 4i32);
        let mut hs = Vec::new();
        let mut biome_under = [[0u32; 3]; 4];
        let mut z = -r; while z <= r { let mut x = -r; while x <= r {
            let c = terrain::column(x, z); hs.push(c.height);
            for (i, lvl) in [-2, -1, 0, 1].iter().enumerate() { if c.height < *lvl { biome_under[i][c.biome as usize] += 1; } }
            x += step; } z += step; }
        let n = hs.len() as f32;
        for (i, lvl) in [-2, -1, 0, 1].iter().enumerate() {
            let under = hs.iter().filter(|&&h| h < *lvl).count() as f32;
            let deep = hs.iter().filter(|&&h| h < *lvl - 4).count() as f32;
            println!("water level {lvl:2}: {:4.1}% of land under water, {:4.1}% more than 4 deep; by biome grass/desert/snow {:?}", 100.0 * under / n, 100.0 * deep / n, biome_under[i]);
        }
        let near: Vec<i32> = (-64..=64).step_by(8).map(|d| terrain::column(8 + d, 8).height).collect();
        println!("heights along z=8, x=-56..72: {near:?}");
        return;
    }
    if std::env::var("BLOCKSIZES").is_ok() {
        let w = world::World::builder();
        let mut sizes = Vec::new();
        for bx in 0..30i16 { for by in -2..3i16 { sizes.push(w.block_payload((bx * 3, by, bx)).len()); } }
        sizes.sort();
        let total: usize = sizes.iter().sum();
        println!("{} blocks: mean {} bytes, median {}, largest {}", sizes.len(), total / sizes.len(), sizes[sizes.len() / 2], sizes[sizes.len() - 1]);
        println!("Column {} bytes, Tree {} bytes", std::mem::size_of::<terrain::Column>(), std::mem::size_of::<terrain::Tree>());
        return;
    }
    if std::env::var("HEIGHTS").is_ok() {
        let w = world::World::open(format!("{OUT}/heightworld")).unwrap();
        for z in 2..15 {
            let row: Vec<String> = (6..15).map(|x| {
                let y = (-10..30).rev().find(|&y| content::walkable(w.node((x, y, z)))).unwrap_or(-99);
                format!("{y:3}")
            }).collect();
            println!("z={z:2} x6..14: {}", row.join(""));
        }
        return;
    }
    if std::env::var("NEAREST_TREE").is_ok() {
        let mut t = terrain::trees_in(-40, -40, 56, 56);
        t.sort_by_key(|t| (t.x - 8).pow(2) + (t.z - 8).pow(2));
        let t = t[0];
        println!("{} {} {} {} {:?}", t.x, t.ground, t.z, t.height, t.kind);
        return;
    }
    if std::env::var("SCREENS").is_ok() {
        let st = status::Status::new("192.168.68.128:30000".into());
        let mut s = status::lock(&st).clone();
        s.players = vec!["Jim".into(), "Steve".into()];
        s.blocks_sent = 1203; s.edits = 57; s.send_failures = 3;
        s.event("Jim joined".into()); s.event("Steve placed torch".into()); s.event("Jim dug birch tree".into());
        let sys = screen::System {
            internal_free_kb: 168, internal_min_kb: 151, internal_largest_kb: 96,
            psram_free_kb: 7612, psram_min_kb: 7480, rssi: Some(-53), channel: Some(9), hotspot_clients: None,
            packets_in_per_s: 21.0, packets_out_per_s: 64.0, cpu: [Some(37.0), Some(6.0)],
            server_busy: 31.0, block_ms: Some(48.6), sd: Some((31_914_983_424, 31_901_327_360)), chip_temp: Some(52.4),
        };
        for (n, t, a) in [("Jim", "anyone seen the birch wood near spawn?", false), ("Steve", "yes, just east of the desert", false),
                          ("Jim", "waves", true), ("Steve", "I put torches along the path so you can find it at night, follow them north", false),
                          ("Jim", "thanks!", false), ("Äijä", "Hyvää yötä! Mökillä on jäätä, Åland ja smörgåsbord", false)] {
            s.chat_line(status::ChatLine { name: n.into(), text: t.into(), action: a });
        }
        let hot = wifi_config::Network { ssid: "Luanti".into(), password: "LuantiLuanti".into() };
        let mut hs = s.clone();
        hs.address = "192.168.4.1:30000".into();
        hs.hotspot = Some(std::sync::Arc::new(status::Hotspot { ssid: hot.ssid.clone(), password: hot.password.clone(), ip: "192.168.4.1".into(), qr: wifi_config::join_qr(&hot) }));
        let mut f = screen::Frame::new();
        screen::render(&mut f, &screen::View { page: screen::Page::Join, status: &hs, system: &sys });
        let mut out = vec![0u8; screen::WIDTH * screen::HEIGHT * 2];
        f.write_be(0..screen::HEIGHT, &mut out);
        std::fs::write(format!("{OUT}/page0.rgb565"), out).unwrap();
        for (page, name) in [(screen::Page::Overview, "page1"), (screen::Page::Device, "page2"), (screen::Page::Chat, "page3")] {
            let mut f = screen::Frame::new();
            screen::render(&mut f, &screen::View { page, status: &s, system: &sys });
            let mut out = vec![0u8; screen::WIDTH * screen::HEIGHT * 2];
            f.write_be(0..screen::HEIGHT, &mut out);
            std::fs::write(format!("{OUT}/{name}.rgb565"), out).unwrap();
        }
        // BOOT held at start-up: half way to a hotspot, and past it, half
        // way to the USB drive. The bar along the bottom.
        for (held, name) in [(0.5, "hold"), (1.5, "hold2")] {
            let mut hold = status::lock(&st).clone();
            hold.boot_hold = Some(held);
            let mut f = screen::Frame::new();
            screen::render(&mut f, &screen::View { page: screen::Page::Overview, status: &hold, system: &sys });
            let mut out = vec![0u8; screen::WIDTH * screen::HEIGHT * 2];
            f.write_be(0..screen::HEIGHT, &mut out);
            std::fs::write(format!("{OUT}/{name}.rgb565"), out).unwrap();
        }
        // The USB drive's page: active, ejected, and failed.
        let files: Vec<String> = ["auth/", "media/", "players/", "world/", "motd.txt", "wifi-written", "wifi.txt"].iter().map(|n| n.to_string()).collect();
        for (name, drive) in [
            ("drive", status::UsbDrive { what: "SD card", bytes: 31_914_983_424, files: files.clone(), error: None, ejected: false }),
            ("drive2", status::UsbDrive { what: "Flash", bytes: 11_468_800, files, error: None, ejected: true }),
            ("drive3", status::UsbDrive { what: "Flash", bytes: 0, files: Vec::new(), error: Some("ESP_ERR_INVALID_STATE (no \"storage\" partition)".into()), ejected: false }),
        ] {
            let mut d = status::lock(&st).clone();
            d.usb_drive = Some(drive);
            let mut f = screen::Frame::new();
            screen::render(&mut f, &screen::View { page: screen::Page::Overview, status: &d, system: &sys });
            let mut out = vec![0u8; screen::WIDTH * screen::HEIGHT * 2];
            f.write_be(0..screen::HEIGHT, &mut out);
            std::fs::write(format!("{OUT}/{name}.rgb565"), out).unwrap();
        }
        return;
    }
    if std::env::var("BENCH").is_ok() {
        for round in 0..2 {
            let d = format!("{OUT}/benchworld{round}");
            let _ = std::fs::remove_dir_all(&d);
            let w = world::World::open(&d).unwrap();
            let t = std::time::Instant::now(); let mut n = 0;
            for bz in -40i16..-30 { for bx in -40i16..-30 {
                let (lo, hi) = w.height_range(bx, bz);
                for by in world::block_of(lo - 1)..=world::block_of(hi + 1) { w.block_raw((bx, by, bz)); n += 1; }
            } }
            println!("{n} blocks in {:?} ({:.3} ms/block)", t.elapsed(), t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
        // Underground, where the dungeons are: the first time round they
        // are worked out as well; the second, they are already known.
        for round in 0..2 {
            let w = world::World::builder();
            let t = std::time::Instant::now(); let mut n = 0;
            for bz in -40i16..-30 { for bx in -40i16..-30 { for by in -6i16..0 { w.block_raw((bx, by, bz)); n += 1; } } }
            println!("underground{}: {n} blocks in {:?} ({:.3} ms/block)", if round == 0 { ", with dungeons new" } else { "" }, t.elapsed(), t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
        return;
    }
    content_dump();
    torch_variants();
    trees();
    new_terrain();
    wifi_file();
    led();
    // The edge of the world: blocks -1937..=1937 on every axis.
    assert!(world::block_in_world((1937, 0, -1937)) && !world::block_in_world((1938, 0, 0)) && !world::block_in_world((0, -1938, 0)));
    assert_eq!(world::block_of(31007), 1937); assert_eq!(world::block_of(31008), 1938);
    assert_eq!(world::block_of(-30992), -1937); assert_eq!(world::block_of(-30993), -1938);
    println!("MAP EDGE OK");
    starter_kit();
    crafting_flow();
    tools();
    physics();
    real_world();
}

/// How many mapchunks have dungeons, how big they are, how long they take
/// to work out, and the loot chest nearest the spawn (printed as "FOUND
/// chest x y z", for the dungeon test).
fn dungeon_stats() {
    let mut terrain = |p: world::NodePos| {
        let col = terrain::column(p.0, p.2);
        if p.1 > col.height {
            if p.1 <= terrain::WATER_LEVEL { terrain::lake_node(p.1, &col) } else { world::CONTENT_AIR }
        } else {
            terrain::node(p.0, p.1, p.2, &col, &mut terrain::cave_sample)
        }
    };
    let (mut chunks, mut with, mut nodes, mut rooms) = (0, 0, 0, 0);
    let mut chests = Vec::new();
    let mut slowest = std::time::Duration::ZERO;
    let t = std::time::Instant::now();
    for cz in -3..=3 { for cy in -3..=1 { for cx in -3..=3 {
        let t1 = std::time::Instant::now();
        let c = dungeons::generate((cx, cy, cz), &mut terrain);
        slowest = slowest.max(t1.elapsed());
        chunks += 1;
        if c.nodes() > 0 { with += 1; nodes += c.nodes(); rooms += c.rooms.len(); }
        chests.extend(c.chests.iter().copied());
    } } }
    println!("{chunks} mapchunks in {:?} (slowest {:?}): {with} with dungeons, {} rooms and {} nodes changed on average, {} chests",
        t.elapsed(), slowest, rooms / with.max(1), nodes / with.max(1), chests.len());
    chests.sort_by_key(|p| p.0 * p.0 + p.2 * p.2 + p.1 * p.1);
    let w = world::World::builder();
    for &p in chests.iter().take(1) {
        assert_eq!(w.node(p), content::CONTENT_CHEST, "the world has the chest at {p:?}");
        assert!(w.is_dungeon_chest(p));
        // Room to stand in front of it.
        let front = (0..4).map(|f| [(0, 0, 1), (1, 0, 0), (0, 0, -1), (-1, 0, 0)][f]).find(|d| {
            w.node((p.0 + d.0, p.1, p.2 + d.2)) == world::CONTENT_AIR && w.node((p.0 + d.0, p.1 + 1, p.2 + d.2)) == world::CONTENT_AIR
        }).unwrap();
        println!("FOUND chest {} {} {} front {} {} {}", p.0, p.1, p.2, p.0 + front.0, p.1, p.2 + front.2);
    }
}
