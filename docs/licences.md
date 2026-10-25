# Licences

The code is released under the [MIT licence](../LICENSE), except for the files that are translations of Luanti's or Minetest Game's own code, which keep their licence, the [GNU LGPL, version 2.1 or later](../COPYING.LESSER):

- `src/dungeons.rs` (Luanti's dungeon generator, with Minetest Game's dungeon loot)
- `src/liquids.rs` (Luanti's flowing liquids)
- `src/commands.rs` and `src/privs.rs` (Luanti's chat commands and privileges)
- `src/items.rs` (Luanti's dropped items)
- `src/crafting.rs` (Luanti's recipe matching, with Minetest Game's recipes)
- `src/meta.rs` and `src/jsonstring.rs` (Luanti's node metadata and item strings, with Minetest Game's signs)
- `src/books.rs`, `src/furnace.rs`, `src/stairs.rs`, `src/nature.rs` and `src/plants.rs` (Minetest Game's books, furnace, stairs, leaf decay and saplings, and plants)
- `src/keys.rs` and `src/screwdriver.rs` (Minetest Game's keys and screwdriver)
- `src/fire.rs` (Minetest Game's fire, and its burning of dropped items)
- `src/weather.rs` (Luanti's value noise, with Minetest Game's weather, which is MIT)

Each says so at its top, with the original copyright holders. The firmware built from all of it is a combined work; as the full source is here, anyone can change and rebuild the LGPL parts, as that licence asks. Files based on Minetest Game mods under the MIT licence (`src/bones.rs`, `src/panes.rs`, `src/doors.rs`, `src/beds.rs`, `src/farming.rs`, `src/colours.rs`, `src/binoculars.rs`, `src/craftguide.rs`, `src/env_sounds.rs`, `src/tnt.rs`, `src/insects.rs`, `src/boats.rs`, `src/carts.rs`) stay MIT, crediting those mods' authors at their top.

The media in `media/` have their own licences:
- **From [Minetest Game](https://github.com/luanti-org/minetest_game), CC BY-SA 3.0:** the player model `character.b3d`, the default skin `character.png`, the torch models and textures, and the item and tool pictures (`default_*.png`, `gui_*.png`). Also the doors, gate, farming and bed media (`doors_*`, `door_*.b3d`, `default_fence_overlay.png`, `default_fence_rail_overlay.png`, `farming_*`, `default_grass_*.png`, `wool_*.png`, `beds_*.png`), the flower and dye pictures (`flowers_*.png`, `dye_*.png`), the pane and steel bar media (`xpanes_*`), the bones textures (`bones_*.png`), the vessels textures (`vessels_*.png`), the key, screwdriver, binoculars and mapping kit pictures (`keys_*.png`, `screwdriver.png`, `binoculars_*.png`, `map_*.png`), the crafting guide's icons (`craftguide_*.png`, `sfinv_crafting_arrow.png`), the fire's textures and sounds (`fire_*`), TNT's (`tnt_*`), the butterflies' and fireflies' textures (`butterflies_*`, `fireflies_*`), and the boat's model and pictures (`boats_*`). The carts' model and textures (`carts_*`) are CC0 (some of the cart's textures from PixelBOX, under a permissive licence) and their rolling sound CC BY 3.0, as `media/LICENSE-carts.txt` says. Their credits are in `media/LICENSE-character.txt`, `media/LICENSE-torch.txt`, `media/LICENSE-items.txt`, `media/LICENSE-doors.txt`, `media/LICENSE-farming.txt`, `media/LICENSE-beds.txt`, `media/LICENSE-flowers.txt`, `media/LICENSE-xpanes.txt`, `media/LICENSE-bones.txt`, `media/LICENSE-vessels.txt`, `media/LICENSE-keys.txt`, `media/LICENSE-craftguide.txt`, `media/LICENSE-fire.txt`, `media/LICENSE-tnt.txt`, `media/LICENSE-butterflies.txt`, `media/LICENSE-fireflies.txt` and `media/LICENSE-boats.txt`. If you redistribute these files, the CC BY-SA terms apply to them, whatever you do with the code.
- **Sounds from Minetest Game** (`default_*.ogg`, `player_damage.ogg`, `env_sounds_*.ogg`; the fire's and TNT's are with their textures): CC BY-SA 3.0, CC BY 3.0 or CC0, with credits in `media/LICENSE-sounds.txt`.
- **The skins `esp_skin_wizard.png`, `esp_skin_elf.png`, `esp_skin_borgo.png` and `esp_skin_strider.png`, and the mob textures (`mobs_*.png`):** made by Keir Finlow-Bates, licensed under [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/).
- **The other `esp_*.png` textures, such as `esp_mist.png`, and the mobs' sounds (`mobs_*.ogg`, made by `tools/make_mob_sounds.py`):** made for this project and covered by the MIT licence. (`media/LICENSE-mobs.txt` lists the mob media.)

Luanti is a trademark of its respective owners. This project is not affiliated with the Luanti project.
