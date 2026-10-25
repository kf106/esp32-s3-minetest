// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Keir Finlow-Bates
// Based on minetest_game's creative mod (mods/creative: init.lua,
// inventory.lua):
//   Copyright (C) 2012-2016 Perttu Ahola (celeron55) <celeron55@gmail.com>
//   Copyright (C) 2015-2016 Jean-Patrick G. (kilbith) <jeanpatrick.guerrero@gmail.com>
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

//! Creative mode, as minetest_game's creative mod has it, and the game
//! modes that `/mode` switches between.
//!
//! - In creative mode (the server's `creative_mode`, or a player's
//!   `creative` privilege) the inventory window opens on the creative
//!   inventory: every item (All), or the nodes, tools or other items, 32 a
//!   page, searched by name, with a trash slot. Taking an item from it
//!   leaves it there.
//! - The hand digs anything in a moment and reaches 10 nodes, nodes placed
//!   aren't used up, and digging doesn't give what the player already has.
//! - Here, as in Minecraft's creative mode, players also take no damage
//!   and may fly and noclip.
//!
//! The pure parts are here; the server keeps the modes and applies them.

use crate::content::{self, GroupCap, ToolCaps, CRAFTITEMS, NODES, TOOLS};
use crate::messages::formspec_escape as esc;

/// The game modes `/mode` sets: Minetest Game as it is (classic), with our
/// mobs (survival), or creative (no mobs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Classic,
    Survival,
    Creative,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "classic" => Some(Mode::Classic),
            "survival" => Some(Mode::Survival),
            "creative" => Some(Mode::Creative),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Classic => "classic",
            Mode::Survival => "survival",
            Mode::Creative => "creative",
        }
    }

    /// The mode the settings amount to.
    pub fn of(creative: bool, mobs: bool) -> Mode {
        match (creative, mobs) {
            (true, _) => Mode::Creative,
            (false, true) => Mode::Survival,
            (false, false) => Mode::Classic,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Mode::Classic => "classic (Minetest Game as it is: no mobs)",
            Mode::Survival => "survival (with mobs at night)",
            Mode::Creative => "creative (every item, instant digging, flying, no damage, no mobs)",
        }
    }
}

/// The creative hand: the item in a creative player's "hand" list, which
/// the client digs and punches with when nothing is wielded.
pub const HAND_ITEM: u16 = content::TOOL_CREATIVE_HAND;
/// How far it reaches (its range).
pub const HAND_RANGE: f32 = 10.0;

/// minetest_game's creative hand: every group in a moment (42 s over a
/// maxlevel of 256: a sixth of a second), instant for dig_immediate 3,
/// never wearing out, and 10 damage.
const DIGTIME: f32 = 42.0;
const ALL: &[(i16, f32)] = &[(1, DIGTIME), (2, DIGTIME), (3, DIGTIME)];
pub const HAND_CAPS: ToolCaps = ToolCaps {
    full_punch_interval: 0.5,
    max_drop_level: 3,
    groupcaps: &[
        GroupCap { group: "crumbly", times: ALL, uses: 0, maxlevel: 256 },
        GroupCap { group: "cracky", times: ALL, uses: 0, maxlevel: 256 },
        GroupCap { group: "snappy", times: ALL, uses: 0, maxlevel: 256 },
        GroupCap { group: "choppy", times: ALL, uses: 0, maxlevel: 256 },
        GroupCap { group: "oddly_breakable_by_hand", times: ALL, uses: 0, maxlevel: 256 },
        GroupCap { group: "dig_immediate", times: &[(2, DIGTIME), (3, 0.0)], uses: 0, maxlevel: 256 },
    ],
    fleshy: 10,
};

/// The creative inventory's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    All,
    Nodes,
    Tools,
    Items,
}

impl Tab {
    pub const EVERY: [Tab; 4] = [Tab::All, Tab::Nodes, Tab::Tools, Tab::Items];
}

/// The detached inventory showing a player's creative list, and the trash.
pub fn detached_name(player: &str) -> String {
    format!("creative_{player}")
}
pub const TRASH: &str = "trash";

/// 8 columns of 4 a page.
const PER_PAGE: usize = 8 * 4;

/// One player's view of the creative inventory.
#[derive(Clone, Debug, Default)]
pub struct View {
    /// The tab showing, if the window is on one.
    pub tab: Option<Tab>,
    pub filter: String,
    /// The first item shown (start_i).
    pub start: usize,
    /// What the list holds now.
    pub items: Vec<u16>,
}

/// Whether an item is in the creative inventory: anything with a
/// description not marked not_in_creative_inventory.
fn listed(id: u16) -> bool {
    content::group_of(id, "not_in_creative_inventory") != 1 && !content::description_of(id).is_empty()
}

/// The first line of an item's description.
fn description(id: u16) -> &'static str {
    content::description_of(id).split('\n').next().unwrap_or("")
}

/// How well a text matches the search: 0 for no search, then how much of
/// it is left over, else no match.
fn matches(s: &str, filter: &str) -> Option<usize> {
    if filter.is_empty() {
        return Some(0);
    }
    s.to_lowercase().contains(filter).then(|| s.len().saturating_sub(filter.len()))
}

/// A tab's items matching `filter` (update_creative_inventory): the closer
/// matches first, then by name.
pub fn items(tab: Tab, filter: &str) -> Vec<u16> {
    let ids: Vec<u16> = match tab {
        Tab::All => NODES.iter().map(|n| n.id).chain(CRAFTITEMS.iter().map(|c| c.id)).chain(TOOLS.iter().map(|t| t.id)).collect(),
        Tab::Nodes => NODES.iter().map(|n| n.id).collect(),
        Tab::Tools => TOOLS.iter().map(|t| t.id).collect(),
        Tab::Items => CRAFTITEMS.iter().map(|c| c.id).collect(),
    };
    let mut found: Vec<(usize, &str, u16)> = ids
        .into_iter()
        .filter(|&id| listed(id))
        .filter_map(|id| {
            let name = content::name_of(id);
            let m = [matches(description(id), filter), matches(name, filter)].into_iter().flatten().min()?;
            Some((m, name, id))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, id)| id).collect()
}

impl View {
    /// Open on a tab (the window's first page is All when creative).
    pub fn open(&mut self, tab: Tab) {
        self.tab = Some(tab);
        self.start = 0;
        self.refresh();
    }

    pub fn refresh(&mut self) {
        if let Some(tab) = self.tab {
            self.items = items(tab, &self.filter);
        }
    }

    fn pages(&self) -> usize {
        self.items.len().div_ceil(PER_PAGE).max(1)
    }

    /// The search, reset, and page buttons. Returns whether the view
    /// changed.
    pub fn on_fields(&mut self, fields: &[(String, String)]) -> bool {
        let get = |name: &str| fields.iter().find(|(f, _)| f == name).map(|(_, v)| v.as_str());
        if get("creative_clear").is_some() {
            self.start = 0;
            self.filter.clear();
        } else if (get("creative_search").is_some() || get("key_enter_field") == Some("creative_filter")) && get("creative_filter").is_some() {
            self.start = 0;
            self.filter = get("creative_filter")
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_control() || *c == '\t' || *c == '\n')
                .take(128)
                .collect::<String>()
                .to_lowercase();
        } else if get("creative_prev").is_some() {
            let size = self.items.len();
            self.start = match self.start.checked_sub(PER_PAGE) {
                Some(s) => s,
                None => {
                    let last = size - size % PER_PAGE;
                    if last == size { size.saturating_sub(PER_PAGE) } else { last }
                }
            };
        } else if get("creative_next").is_some() {
            self.start += PER_PAGE;
            if self.start >= self.items.len() {
                self.start = 0;
            }
        } else {
            return false;
        }
        self.refresh();
        true
    }

    /// The page's part of the inventory window (creative.register_tab's
    /// get): the list, the search box and buttons, the page number, the
    /// trash, and the player's inventory underneath.
    pub fn formspec(&self, player: &str) -> String {
        let page = self.start / PER_PAGE + 1;
        let empty = if self.items.is_empty() { "label[3.5,2.5;No items to show.]" } else { "" };
        format!(
            "{empty}\
             list[detached:{inv};main;0.5,0.5;8,4;{start}]\
             list[current_player;main;0.5,5.75;8,4;]\
             listring[current_player;main]\
             listring[detached:{inv};main]\
             field[10.85,0.75;3.2,0.8;creative_filter;;{filter}]\
             field_close_on_enter[creative_filter;false]\
             image_button[10.85,1.75;0.8,0.8;creative_search_icon.png;creative_search;]\
             image_button[11.85,1.75;0.8,0.8;creative_clear_icon.png;creative_clear;]\
             tooltip[creative_search;Search]\
             tooltip[creative_clear;Reset]\
             image_button[10.85,3.0;0.8,0.8;creative_prev_icon.png;creative_prev;]\
             label[11.95,3.4;{colour}{page}{plain} / {pages}]\
             image_button[13.25,3.0;0.8,0.8;creative_next_icon.png;creative_next;]\
             tooltip[creative_prev;Previous page]\
             tooltip[creative_next;Next page]\
             image[12.1,4.25;1,1;creative_trash_icon.png]\
             listcolors[#00000069;#5A5A5A;#141318;#30434C;#FFF]\
             list[detached:{TRASH};main;12.1,4.25;1,1;]",
            inv = detached_name(player),
            start = self.start,
            filter = esc(&self.filter),
            colour = "\u{1b}(c@#FFFF00)",
            plain = "\u{1b}(c@#ffffff)",
            pages = self.pages(),
        )
    }

    /// The item shown at index `i` of the list, if any.
    pub fn item_at(&self, i: usize) -> Option<u16> {
        self.items.get(i).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_search() {
        let all = items(Tab::All, "");
        let nodes = items(Tab::Nodes, "");
        let tools = items(Tab::Tools, "");
        let other = items(Tab::Items, "");
        assert_eq!(all.len(), nodes.len() + tools.len() + other.len());
        assert!(all.contains(&content::CONTENT_STONE) && tools.contains(&content::TOOL_PICK_STEEL) && other.contains(&content::ITEM_STICK));
        // Nothing hidden: not the creative hand, the burning TNT or
        // anything without a description.
        assert!(!all.contains(&HAND_ITEM) && !all.contains(&crate::tnt::TNT_BURNING));
        // Search: by name or description, the closest first.
        let found = items(Tab::All, "stone");
        assert!(found.contains(&content::CONTENT_STONE) && found.iter().all(|&id| content::name_of(id).contains("stone") || description(id).to_lowercase().contains("stone")));
        assert_eq!(found[0], content::CONTENT_STONE);
        assert!(items(Tab::All, "no such thing").is_empty());
    }

    #[test]
    fn paging() {
        let mut v = View::default();
        v.open(Tab::All);
        let pages = v.pages();
        assert!(pages > 3);
        let fields = |f: &str| vec![(f.to_string(), String::new())];
        assert!(v.on_fields(&fields("creative_next")));
        assert_eq!(v.start, PER_PAGE);
        assert!(v.on_fields(&fields("creative_prev")) && v.start == 0);
        // Back from the first: the last page.
        v.on_fields(&fields("creative_prev"));
        assert_eq!(v.start / PER_PAGE + 1, pages);
        // A search starts again from the first.
        let search = vec![("creative_search".to_string(), String::new()), ("creative_filter".to_string(), "Steel".to_string())];
        assert!(v.on_fields(&search));
        assert!(v.start == 0 && v.filter == "steel" && !v.items.is_empty());
        assert!(v.formspec("Bob").contains("list[detached:creative_Bob;main;0.5,0.5;8,4;0]"));
        assert!(v.on_fields(&fields("creative_clear")) && v.filter.is_empty());
        assert!(!v.on_fields(&fields("quit")));
    }

    #[test]
    fn modes() {
        assert_eq!(Mode::of(false, false), Mode::Classic);
        assert_eq!(Mode::of(false, true), Mode::Survival);
        assert_eq!(Mode::of(true, true), Mode::Creative);
        assert_eq!(Mode::parse("survival"), Some(Mode::Survival));
        assert_eq!(Mode::parse("hard"), None);
        // The creative hand digs stone in a moment, and dirt.
        let (t, wear) = content::dig_params(content::CONTENT_STONE, &HAND_CAPS, 0).unwrap();
        assert!(t < 0.2 && wear == 0);
        assert!(content::dig_params(content::CONTENT_DIRT, &HAND_CAPS, 0).is_some_and(|(t, _)| t < 0.2));
    }
}
