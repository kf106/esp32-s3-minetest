# Features

What the server does, as of now. It covers most of Minetest Game (29 of its 34 mods); what is still missing is at the end, under [What doesn't (yet)](#what-doesnt-yet), and what never will be under [What won't be done](#what-wont-be-done).

![In game, connected to the ESP32 server](../assets/screen-002.png)

Contents: [The server](#the-server), [The world](#the-world), [Trees, plants and farming](#trees-plants-and-farming), [Players](#players), [Crafting and smelting](#crafting-and-smelting), [Building](#building), [Tools and items](#tools-and-items), [The board](#the-board).

## The server

- **Protocol:** the Luanti network protocol as spoken by Luanti 5.17 (protocol version 53), including the reliable-UDP layer with split packets, acknowledgements, resends and out-of-order delivery.
- **Accounts:** SRP-6a password login, the same scheme the real server uses. New players register on first connect, and passwords are never stored or sent.
- **Up to six players at once:** a seventh is turned away with Luanti's "too many users" message. The limit is `MAX_PLAYERS` in `src/server.rs`; more players would overload the ESP32, mainly in generating map blocks when several explore at once.
- **Welcome message:** joining players are told what the server is and who else is on.
- **Chat:** with Luanti's usual limits (500 characters, 8 messages per 10 seconds, commands included), and Luanti's chat commands: see [Administrator and privileges](admin.md).
- **Changing your password:** from the client's own Change Password dialogue.
- **World storage:** the world is generated on demand from a fixed seed. Only the changes players make are stored, as an append-only log on the SD card. Writing to the card is slow, so changes are appended once a second, and an inventory is saved within 5 s of changing (and when its player leaves): a power cut loses at most that much. Saving after every dig once held the server up so much that a player digging fast was disconnected.
- **Saving:** player positions, health, skins and inventories are saved and restored. So are dropped items (they fall back to the ground after a restart, keeping how long they have left) and the time of day, so a restart carries on where the game was. The board is usually stopped by pulling the plug, so the time is saved every minute and items 10 seconds after they change: at most that much is lost.

## The world

- **World generation:**
  - rolling hills and mountains;
  - rivers, as Luanti's v7 map generator makes them (with its river noise): channels up to 10 deep and a few dozen nodes wide, cutting valleys through the hills down to the water level, winding between the lakes and filled with water. Unlike mgv7's, the valleys' sides are smooth, without the overhangs its 3D "ridge" noise leaves;
  - Minetest Game's biomes: grassland, with coniferous forest in the hills, sandstone desert (sand over sandstone) where the temperate land is driest, and snowy grassland (snowy dirt under a dusting of snow, with fir and blueberry bushes) in its cooler parts; desert (desert sand over desert stone), savanna and rainforest in the hot lands, from dry to wet; cold desert (silver sand), tundra (permafrost, moss and stones, gravel beaches) and snowy fir forest in the cold lands, from dry to wet, and in the coldest wet land the icesheet: snow blocks over cave ice (which gives ice when dug) down to height -8, its lakes and rivers frozen ten deep, its dungeons built of ice; snow on the mountain tops. Snow lies on the snowy lands and in patches on the tundra, and cold lakes freeze over;
  - gravel in blobs underground, and clay and gravel on lake beds;
  - caves with entrances; below height -64, in about half the land, they are flooded with lava, sealed off from the dry ones by solid stone;
  - dungeons (see [Dungeons](#the-world) below);
  - dry shrubs scattered on the desert, sandstone desert and cold desert sand;
  - Minetest Game's ores, deeper for the better ones, at about its densities: coal everywhere, iron below 0, copper and tin below -64, gold below -256, mese below -512, diamond below -1024, and the odd mese block below -2048;
  - birch woods with apple trees in the lowlands and fir forests in the hills and snow, with clearings and the odd lone tree; dense jungle in the rainforest, with jungle grass on a floor of leaf litter, and now and then an emergent jungle tree, Minetest Game's giant, 20 to 30 nodes tall on buttress roots, its crown far above the rest; scattered flat-topped acacias in the savanna's dry grass. Leaves cast shade.
- **World edge:** the world ends about 31,000 nodes from the centre in every direction, at an invisible wall, as in Luanti.
- **Dungeons:** Luanti's dungeon generator, ported from its source: underground rooms joined by corridors, some with stairs, walled in cobblestone that is mossy in places (desert stone in the desert, sandstone brick with sandstone block stairs in the sandstone desert, ice on the icesheet). About four in ten mapchunks (80 nodes a side) have one, wherever there is stone, so they turn up in mines and caves at any depth, and now and then a room breaks out at the surface. As in Minetest Game, some rooms have a loot chest against a wall, up to two per mapchunk. What a chest holds depends on how deep it is: sticks, flint, apples, coal, gold and steel ingots, now and then mese crystals, a worn wooden sword, stone pickaxe or (rarely) diamond axe, sand and dirt near the surface (desert cobblestone and cactus in the desert, desert sand and cactus in the sandstone desert, snow on the icesheet), and obsidian and mese blocks below -512; and, as the other Minetest Game mods add, empty buckets, water buckets (lava buckets underground), string, wheat, cotton seeds and glass fragments. Mossy cobblestone cooks back to stone, and makes stairs, slabs and walls. As in Minetest Game, moss also grows by itself, slowly, on cobblestone (and its stairs, slabs and walls) with water beside it, so a cobblestone pier or well turns mossy over time.
- **Water:** lakes and ponds fill low ground up to height 1 (Luanti's water level), with sandy beds and shores; about an eighth of the land. Swim in it, and build into it. Water flows as in Luanti: dig beside or under a lake and it runs in, up to 8 nodes across flat ground, getting shallower as it goes, and falls down any hole; take the source away and the flow drains. Two sources with a gap between make a third, so a lake never runs dry. Water washes torches out. It spreads a node a second; the server looks at up to 1,000 places a second, so a flood into a big cave takes a while. Where more than 8 nodes in a map block change at once, the block is sent again whole rather than node by node, to spare the radio; while water is moving the log says how much every 10 s ("Liquids: ...").
- **Lava:** Minetest Game's lava, in deep caves (see above) or poured from a bucket. It flows like water, but creeps (its depth changes a level at a time), never makes new sources, and burns 8 HP a second while you stand in it. Lava touching water cools: a lava source into obsidian (a steel pickaxe digs it), flowing lava into stone, with a hiss. It glows in the dark but lights nothing around it: the client draws lava bright by itself, and the server spends no time spreading its light. So that what players see stays the same, a change to lava is always sent as its whole map block, never node by node.
- **Fire:** Minetest Game's `fire`. Flint beside a steel ingot makes flint and steel (66 uses). Struck on anything flammable, it lights a flame on top; on a coal block, a permanent flame that burns until the block is dug. A flame glows (it lights nothing round it, as lava doesn't), burns 4 HP a second if you stand in it, is put out by flowing water with a hiss, and goes out 30 to 60 s after nothing flammable is left beside it. You hear it crackle within 8 nodes, louder the more there is. Dropped items that can burn go up in smoke in lava, and now and then in a flame.
  - **Spreading:** as in Minetest Game, flames and lava set flammable nodes beside them alight (every 7 s, one time in 12) and flames burn flammable nodes away (every 5 s, one time in 18), near players. Burnt nodes are gone, with nothing dropped: a bookshelf's books burn with it, and leaves decay when their trunk burns.
  - **Kept small:** unlike Minetest Game's, a fire burns out within 8 nodes of where it started (each flame keeps count of how far it is from the first), and at most 256 flames burn at once. On a PC, 32 nodes and 4096 flames.
  - **Turning it off:** `/fire off` (for a moderator with `ban`, or an admin with `server`) stops fire spreading and burning things; flames still light and burn players, but go out at their next check (30 to 60 s), even beside wood or leaves. Minetest Game's would burn there for ever. `/fire on` turns it back on, and `/fire` says which it is and how many flames are burning. The setting is kept in `world/settings.txt` (Minetest Game's `enable_fire`); fire is on in a new world.
- **TNT and gunpowder:** Minetest Game's `tnt`. Coal and gravel make 5 gunpowder, six gunpowder either side of paper 2 TNT sticks, and nine sticks TNT.
  - **Gunpowder** lies on the ground in a trail that joins up as it is laid (like rails). Lit, it burns for a second, lights the gunpowder (or TNT) next to it, a node up or down too, and is gone.
  - **TNT** is lit by a punch with a torch, flint and steel, a flame or lava beside it, burning gunpowder, or another blast. It hisses for 4 s, then explodes: everything within about 3 nodes is destroyed and thrown out as items (stone as cobblestone, and so on), flammable things catch fire, gunpowder in the blast is lit and other TNT goes off a moment later, and sand and gravel above come down. TNT within 2 nodes of each other goes off together, in a bigger blast (8 together make a radius of 6, 27 a radius of 9). On the board 27 is as big as it gets: more go off with it, but make the blast no bigger, so that one blast doesn't hold the board up for long. On a PC, up to 125 (a 5×5×5 block: radius 15), as in Minetest Game. Players within twice the radius lose up to 12 HP, less further off, and are pushed away; dropped items are thrown.
  - Chests, furnaces and shelves in a blast are thrown out with what they hold. Locked chests, steel doors and trapdoors, open chests and bones are left alone, as in Minetest Game.
  - **Turning it off:** `/tnt off` (for a moderator with `ban`, or an admin with `server`) stops anything lighting TNT or gunpowder, and fire leaves them alone; TNT already lit goes out instead of off. `/tnt on` turns it back on, and `/tnt` says which it is. It is kept in `world/settings.txt` (Minetest Game's `enable_tnt`); TNT is on in a new world. Unlike Minetest Game, which has no TNT at all when it is off, the recipes stay.
  - Unlike Minetest Game's, lit TNT doesn't fall, and the blast has no smoke or flying debris (the client is sent no particles).
- **Day and night:** a normal day and night cycle. Map blocks carry separate day and night light.
- **Weather:** Minetest Game's `weather`. The clouds come and go over about ten minutes: how much of the sky they cover, how thick they are, and which way the wind takes them, all from slow noise, so every player sees the same sky; where the land is drier the clouds are fewer and thinner. With dynamic shadows on in the client, shadows are fainter the cloudier it is. Each player's sky is set when they join and every 8 s.

## Trees, plants and farming

- **Trees, as in Minetest Game:** chop a tree's trunk away and its leaves fall within a few seconds (leaves you placed yourself never do). Leaves give a sapling one time in 20, dug or fallen. Plant a sapling on dirt, grass, savanna dirt or leaf litter in the open and it grows into a birch, fir, apple, jungle or acacia tree after 5 to 25 minutes, in daylight. Nine jungle saplings make an emergent jungle tree sapling, which grows into an emergent jungle tree.
- **Apples and health:** apple trees carry apples. Pick one (punch it) and eat it (left-click with it) for 2 HP; health only comes back by eating, as in Minetest Game. A picked apple grows again in the same place after 5 to 25 minutes while the tree's leaves are there; apples fall with the leaves when the tree is chopped.
- **Grass:** as in Minetest Game, grass spreads onto bare dirt beside it in daylight (dig a grass block and the dirt under it greens over), and so do forest litter and snowy dirt; tall grass or snow on bare dirt seeds it too. Grass under anything that keeps out the light, or under water, dies back to dirt (savanna grass to savanna dirt). As Minetest Game's, it is checked every 6 s (spreading) and 8 s (dying), one time in 50, within 4 map blocks of a player. Unlike Minetest Game, only ground that players have changed (and what is next to it) is looked at, so a generated tree's trunk doesn't kill the grass under it.
- **Flowers, dyes and wool, as in Minetest Game:**
  - Eight flowers grow in patches on the grassland (red rose, orange tulip, yellow and white dandelions, green chrysanthemum, blue geranium, viola, black tulip), mushrooms in the woods, and waterlilies on the swampy lakes of the rainforest and savanna. A waterlily goes back only on still water.
  - A brown mushroom heals 1 HP; a red one costs 5.
  - A flower makes 4 dye of its colour, a coal lump 4 black, blueberries 2 violet; two dyes mix into two of another (red and blue make violet, white and red pink, and so on: all 19 of Minetest Game's mixes), for all 15 colours.
  - Any wool with a dye makes wool of that colour. Beds take any colour of wool.
- **Other plants, as in Minetest Game:**
  - **Cactus** grows in the desert, with the odd large one with arms; **papyrus** on the swampy shores of the rainforest and savanna. Planted ones grow a node at a time, up to 4 tall, in daylight: cactus on sand, papyrus on dirt with water within 3 nodes. Digging papyrus takes the papyrus above it too. Five cactus in a plus make a large cactus seedling, which grows into a large cactus on sand.
  - **Bushes:** plain and blueberry bushes in the grassland, fir bushes among the firs and in the snow, acacia bushes on the savanna. Their leaves give a bush sapling one time in 5, their stems a plank. Pick blueberries for 2 HP each; the bush bears again 5 to 25 minutes later.
  - **Ferns** under the firs and **marram grass** on sandy lake shores.
  - **Kelp** on the beds of deep, cool lakes, up to 6 nodes tall, and **corals** (green, pink, cyan, brown, orange) in warm lakes. Digging kelp leaves the sand and gives the kelp, which goes back into sand with water above; a coral leaves its skeleton.
- **Farming:** Minetest Game's wheat and cotton.
  - Tall grass grows on grassland; digging it gives a wheat seed one time in five. Jungle grass in the rainforest gives a cotton seed one time in eight. Wild cotton grows in patches on grassland and savanna and gives cotton seeds.
  - A hoe (two planks, stones or steel ingots over two sticks, like Minetest Game's) turns dirt, grass or forest litter into soil with a left-click, savanna dirt (with or without its grass) into savanna soil, and desert sand into desert sand soil (which falls like sand). Soil within 3 nodes of water turns wet; without water it dries, and bare dry soil turns back into its dirt or sand.
  - Plant seeds on soil. They sprout, then grow through 8 stages, a stage every 3 to 5 minutes, but only on wet soil in daylight. Dig a crop when ripe for one or two of the harvest and of seeds; earlier gives less.
  - 4 wheat make flour, which bakes into bread in the furnace (15 s); bread heals 5. 9 wheat make 3 straw blocks, and back. 4 cotton make white wool, 2 make 2 string.

## Players

- **Players:** other players are shown with the animated Minetest Game character and their name tags.
- **Skins:**
  - chosen with a live 3D preview in the inventory window;
  - extra skins can be added on the SD card;
  - 64×64 Minecraft skins are accepted.
- **Players are solid:** players can't walk through each other (in Luanti they can), except someone lying in bed. Everyone sees a teleported or respawned player where they now are straight away.
- **Survival inventory:** 32 slots, with stacks you can rearrange in the inventory window. New players start with 20 torches.
- **Health:** fall damage, a health bar, death and respawn.
- **Drowning:** as in Luanti, with your head in water (or lava) you lose a breath every 2 s, and with all 10 gone 1 HP every 2 s. Out of it, a breath comes back every half second. Luanti's bubbles show above the hotbar while your breath isn't full.
- **Bones:** Minetest Game's. Die with anything on you and it all stays where you fell, in a bones block (in the nearest air if you died inside something, and on the ground if you died in mid-air; dropped on the ground if there is nowhere at all). Right-click to open them, or punch them to take everything that fits; once they're empty they go, and you get the bones block. At first only you can take from them; after 20 minutes of game time they become "old bones" and anyone may. Nothing can be put back in.
- **Dropped items:**
  - `Q` drops the whole stack in your hand; `Shift+Q` drops one;
  - punch an item to pick it up;
  - items fall, land, and merge with matching items nearby.
- **Beds:** Minetest Game's simple bed (three white wool over three planks) and fancy bed (the same with a stick at the top right). A bed is two nodes long and points the way you look as you place it. Right-click it at night to lie down; a window shows how many players are in bed, and when everyone online is, the night is skipped to the morning (05:31). Once more than half are in bed, anyone lying down can force the night skip. Leave Bed (or Escape) gets you up again. Using a bed, by day or night, makes it where you respawn after dying; respawn points are kept in `world/beds_spawns`. A bed can't be dug while someone lies in it.
- **Sounds:** Minetest Game's footstep, digging, breaking and placing sounds for each kind of block, and a sound when you are hurt. The client plays them; the server only says which.
- **Ambient sounds:** Minetest Game's `env_sounds`. Every 3.5 s, flowing water within 8 nodes of you sounds like running water, and lava (sources and flowing) bubbles, from the middle of what makes them and louder the more there is. Each player hears their own.

## Crafting and smelting

- **Crafting guide:** Minetest Game's `mtg_craftguide`, as the inventory window's Recipes tab: every item that has a recipe or goes into one, 48 to a page (its own window holds 32), with a search box (by name or description). Click an item for how to make it (each recipe in turn, with "G" on any slot that takes a group of items, such as any planks, a furnace icon and cooking time for smelting, and an icon for shapeless recipes), and again for what it is used in. Fuel says so in its tooltip.
- **Crafting:** Minetest Game's 3×3 crafting grid in the inventory window, with its recipes and rules. Stone gives cobblestone, and ores give coal, iron, copper, tin and gold lumps, mese crystals and diamonds.
  - a tree trunk makes 4 planks (apple, birch, fir, jungle or acacia); planks make 4 sticks;
  - a coal lump above a stick makes 4 torches;
  - 4 sand make sandstone, and sandstone makes 4 sand; the same for desert sand and silver sand;
  - bricks and blocks from desert stone and from each sandstone; 4 clay lumps make clay, 4 clay bricks (clay lumps smelted) a brick block, 9 snow a snow block, 9 obsidian shards obsidian, and back; obsidian bricks and blocks from obsidian, obsidian glass from smelted shards. Gravel sometimes gives flint;
  - wooden and stone pickaxes, shovels, axes and swords, in Minetest Game's shapes (planks for wood, stone or cobblestone for stone);
  - steel, bronze, mese and diamond pickaxes, shovels, axes and swords, from steel or bronze ingots, mese crystals or diamonds, with Minetest Game's digging times and wear. A diamond block needs a mese or diamond pickaxe; mese and diamond ore a steel one or better.
  - a furnace from 8 stone or cobblestone in a ring;
  - a chest from 8 planks in a ring; a locked chest with a steel ingot in the middle, or from a chest and a steel ingot;
  - building blocks: 4 stone make 4 stone bricks and 9 make 9 stone blocks, and the same for sandstone; 9 coal lumps, steel, copper, tin, bronze or gold ingots, mese crystals or diamonds make a block of each, and back; 8 copper ingots round a tin one make 9 bronze ingots; a mese crystal breaks into 9 fragments, and 9 fragments make a crystal;
  - ladders: an H of sticks makes 5 wooden ladders, of steel ingots 15 steel ones.
  - lights: glass over a mese crystal makes a mese lamp; glass over three mese crystals in a row over planks makes 4 mese post lights of that wood, thin posts with glowing mese windows that light up as much as a lamp.
- **Furnace:** Minetest Game's, placed facing you. Right-click opens it, with slots for what to smelt, fuel, and a 2×2 output.
  - It smelts iron, copper, tin and gold lumps into ingots, cobblestone into stone (desert cobblestone into desert stone), sand into glass, clay lumps into bricks, obsidian shards into obsidian glass, glass fragments into glass and steel bottles into steel ingots, 3 seconds each, keys back into gold in 5, and bakes flour into bread in 15.
  - Fuel: lava buckets burn for 60 s, coal lumps 40 s, chests, bookshelves and vessels shelves 30 s, trunks 22 to 38 s, wooden doors 14 s, beds 12 to 13 s, wooden trapdoors, fences and gates 5 to 9 s, fence rails 3 to 7 s, straw 3 s, planks 5 to 9 s, leaves, grass, torches and wooden tools a few seconds, sticks, wheat, cotton and string 1 s. A piece of fuel that is alight burns to the end.
  - It burns whether or not anyone has it open. While lit, its front shows the fire, it glows, the window's flame and arrow fill up, and you hear it.
  - Only fuel goes in the fuel slot, and nothing into the output. Empty it before digging it (with a stone pickaxe). Its contents are kept on the SD card in `world/furnaces.txt`.
- **Glass:** smelt any sand in the furnace. Turn on "connected glass" in Luanti's settings for Minetest Game's joined-up panes.

## Building

- **Digging and building:** changes appear for every player at once. Sand (and, as in Minetest Game, gravel and the other sands) with nothing under it falls, under gravity, until it lands; what was stacked on it follows. Where it can't settle (on a torch, say) it drops as an item.
- **Stairs, slabs and corner stairs:** Minetest Game's, of each kind of planks, stone, cobblestone, each sandstone, desert stone and cobblestone, their bricks and blocks, brick, obsidian (and its bricks and blocks), ice, snow blocks, and the steel, copper, tin, bronze and gold blocks.
  - 6 blocks in a staircase (rising to the right) make 8 stairs, and 3 in a row make 6 slabs. 4 stairs in a square give 3 blocks back, and 2 slabs one above the other give 1.
  - Corner stairs: 6 blocks in a staircase shape with the middle left empty (one on top in the middle, one either side below, a full row underneath) make 7 inner corners; 4 as an upside-down T make 6 outer corners. They place like stairs, so which way a corner turns depends on where you stand.
  - Stairs face away from you. Placed against the underside of a block, or the upper half of its side, they go upside down. A slab placed on top of the same kind of slab joins it into a full block (or under it, if it is upside down), as in older Minetest Game; on a different slab it takes that slab's way up.
  - Light gets in, so a slab roof doesn't make a dark room. Wooden ones burn, for three quarters (stairs) or half (slabs) the time of the planks.
- **Doors:** Minetest Game's wooden, glass and steel doors: two columns of three planks, glass or steel ingots make one. A door faces the way you look as you place it, and one placed beside another hinges on the other side, so the pair opens outwards. Right-click opens and shuts it, with its sound; with sneak held, right-click builds against it instead. A steel door belongs to whoever placed it: only they (or someone holding its key: see [Keys](#tools-and-items)) can open or dig it (with a steel pickaxe). Owners are kept in `world/doors.txt`.
- **Trapdoors:** Minetest Game's wooden and steel ones. Two rows of three planks make 2 wooden trapdoors, a square of four steel ingots one steel trapdoor. Placed like a block (facing away from you), right-click flips it up against its back edge and down again. A steel trapdoor is locked to whoever placed it, like a steel door.
- **Fences, fence rails and fence gates:** Minetest Game's, in all five woods. Two rows of planks, stick, planks make 4 fences; two planks above two more, with an empty row between, 16 rails (a fence without its posts); two rows of stick, planks, stick a gate. Fences join up with each other, with gates and with planks and trunks next to them (the client draws the joins, from the node definitions). A gate is placed facing away from you, joins fences on its left and right, and opens and closes with a right-click and a creak. Rails join only fences, gates and other rails, and stop you like a fence does. Fences and gates burn for 5 to 7 s, as their planks, rails for 3 to 5 s.
- **Walls:** Minetest Game's cobblestone, desert cobblestone and mossy cobblestone walls. Two rows of three cobblestone make 6. A wall is a thick post that grows a lower wall out to each wall, fence or stone block beside it, and stops you as a full block does. Fences and rails join walls too.
- **Glass panes and steel bars:** Minetest Game's `xpanes`. Two rows of three glass, obsidian glass or steel ingots make 16. A pane on its own, or in a straight line, is a thin sheet running along the line; at a corner, a T or a cross it becomes a post with arms out to its neighbours. Panes join each other and stone, wood and trunks (not glass blocks, as in Minetest Game). Steel bars also make the **steel bar door** (two columns of three) and **steel bar trapdoor** (a square of four), locked to whoever places them like the steel ones.
- **Chests:** Minetest Game's, 32 slots, placed facing you. Right-click opens one: the lid lifts with a creak while anyone has it open (unless a block sits on it) and closes with a thud. A locked chest opens only for whoever placed it, or someone holding its key (see [Keys](#tools-and-items)). Empty a chest before digging it. Contents are kept in `world/chests.txt`.
- **Signs:** Minetest Game's wooden and steel signs: two rows of planks (or steel ingots) over a stick make 3. They hang on walls, lie on floors and hang under ceilings, and drop off when what they hang on is dug. Right-click one to write on it (up to 512 characters, over several lines); what it says shows, in quotes, when you point at it. Anyone who may interact can rewrite any sign, as in Minetest Game without protection. Wooden signs burn for 10 s. The text is node metadata, which the server keeps in `world/meta.txt` and sends with the map blocks, as Luanti does.
- **Cloud:** Minetest Game's white cloud block, which nobody can dig: for admins and map makers, given with `/give <name> cloud`.
- **Mist** (our own, not Minetest Game's): a half-see-through grey cube for filling hollows, valleys and dungeons with haze. You walk through it, it can't be pointed at or dug, and placing any block where it is replaces it (so to clear mist, build into it and dig the block out). From inside, it puts a grey veil over your view, as water does. For admins and map makers: `/give <name> mist`.

## Tools and items

- **Tools:** Minetest Game's digging rules. The bare hand digs earth, sand, leaves and (slowly) wood, but not stone, so your first job is a wooden pickaxe. Iron ore needs a stone pickaxe. Tools wear with use, show Luanti's wear bar, and break with Minetest Game's sound: a wooden pickaxe lasts 30 stone digs, a stone one 60, a steel one 180 (and digs twice as fast).
- **Torches:** they mount on floors, walls and ceilings using Minetest Game's 3D models. The server computes their light, so caves and nights are lit, and they fall off when the block they hang on is dug.
- **Buckets:** three steel ingots in a V make an empty bucket. Left-click water or lava with it to fill it; right-click to pour it out again, wherever you like. A lava bucket burns in a furnace for 60 s and leaves the empty bucket in the output.
- **Keys:** Minetest Game's `keys`. A gold ingot makes a skeleton key. Use it (left-click) on your own locked chest, steel door or steel trapdoor (or steel bar door or trapdoor) and it becomes a key to it, "Key to <you>'s Locked Chest" and so on. Whoever holds that key in their hand can then open the chest, take from it and dig it, or open, shut and dig the door, as its owner can; sneak and right-click a door with the key to open or shut it. Every key cut for the same lock fits it. A skeleton key on someone else's lock says "You do not own this chest." Keys and skeleton keys melt back into gold (5 s). The lock's secret is kept with the chest in `world/chests.txt`, or the door in `world/doors.txt`.
- **Screwdriver:** Minetest Game's. A steel ingot over a stick makes one. Left-click turns the node you point at about its face, right-click about its axis (sneak to turn chests, furnaces and the like, which a right-click would open): chests, furnaces, shelves, stairs and slabs, trapdoors, gates, signs, ladders and so on. Torches and doors don't turn; nor does a chest, furnace or shelf with something in it, or someone else's locked chest or trapdoor. A bed turns on its bottom half, if there is room for its top where it goes. Each turn wears the screwdriver; it lasts 200.
- **Binoculars and mapping kit:** Minetest Game's `binoculars` and `map`. Two obsidian glass either side of three bronze ingots, above and below, make binoculars; carried anywhere in your inventory they let you zoom (the Zoom key, to a 10° view). Glass, paper and a stick over a steel ingot, paper and a steel ingot over planks, paper and black dye make a mapping kit; carried, it lets you show the minimap (the Minimap key). As in Minetest Game, without a mapping kit there is no minimap, and the radar view is never on (there is no creative mode). The server looks again every 5 s or so, and when you use (left-click) the binoculars or the kit. A mapping kit burns for 5 s.
- **Bottles, glasses and the vessels shelf:** Minetest Game's `vessels`. Five glass in a bottle's shape (two columns of two over one in the middle) make 10 empty glass bottles, seven in a cup's shape (two columns of two over a row of three) 14 drinking glasses, and five steel ingots in the bottle's shape 5 heavy steel bottles. They stand on the floor (on something solid), you walk through them, and they fall off when what they stand on is dug. Two glass bottles, or two drinking glasses, break into glass fragments, which melt back into glass; a steel bottle melts back into a steel ingot. Three vessels of any kind between two rows of planks make a vessels shelf: like a bookshelf, but it holds 16 vessels (and nothing else), shows how many when pointed at, and burns for 30 s.
- **Paper, books and bookshelves:** three papyrus in a row make paper, three paper in a column a book, and three books between two rows of planks a bookshelf. A bookshelf holds 16 books (and nothing else), shows how many it holds when pointed at, and can only be dug when empty. It burns for 30 s.
- **Writing in books:** use a book (left-click with it in hand) to write a title and up to 10,000 characters in it; saved, it becomes a book with text, which shows its title and author in the inventory. Its author can go on editing it or read it; anyone else reads it, 14 lines a page. Saving a book with no title and no text leaves it an empty book (or makes it one again), where Minetest Game would keep a book with text titled "". A book with text and an empty book crafted together make a copy, and the original stays in the grid. What is written is kept in the item itself (Luanti's item metadata), so it goes wherever the book goes: chests, bookshelves, bones, dropped on the ground, other players.

## The board

- **Status light:** the board's RGB LED shows what is happening, whether the screen is on or off. It never goes above 60% of full power.
  - slow breathing while the server runs, a little quicker with players on: green, turning amber when the chip is warm (60 °C) and red when it is hot (75 °C), the same colours as the temperature on the Device page;
  - two blue flashes when a player joins, two purple when one leaves, and a white flash for chat;
  - three red flashes when something goes wrong, such as failing to save to the SD card;
  - amber blinking while looking for Wi-Fi.
- **Status screens:** the LCD has three pages, switched with the board's BOOT button, plus a fourth when the board runs its own hotspot. The screen switches off after a minute without a press, to keep the board cooler; the next press wakes it on the same page.
  - **Join** (hotspot only, first): a QR code that a phone camera offers to join, with the hotspot's name, password and address written beside it.
  - **Overview:** players online, packet and block counts, memory, and recent events.
  - **Device:** load on each CPU core, how busy the server is, time per map block, memory (free, lowest, largest piece), Wi-Fi signal and channel (on the hotspot: its channel and how many devices are on it), failed sends, SD card space (or flash space, without a card), and the chip's temperature.
  - **Chat:** the latest public chat, newest at the bottom. The screen font covers Latin-1 and Latin Extended-A, so Finnish, Swedish and most European letters show properly.

## What doesn't (yet)

Five of Minetest Game's 34 mods are still to come:

- **Boats:** wooden boats to row across water.
- **Carts:** minecarts, with rails (plain, powered and brake rails) to run them on.
- **Butterflies and fireflies:** the small creatures that flutter over grass and glow at night, and the bug net that catches them (fireflies in a bottle make a light).
- **Creative mode:** the creative inventory, with every item to take, and digging without limits.

(Boats and carts count as one mod each; butterflies and fireflies as two.)

Smaller gaps in what is here:

- **Particles:** none are sent, so there is no smoke or debris from TNT, and no puff when a dropped item burns.
- **Lit TNT doesn't fall** if what is under it goes.
- **Dirt with savanna grass** (Minetest Game's dirt_with_dry_grass) isn't there, so savanna grass on ordinary dirt doesn't spread.
- **Untouched ground:** grass under a generated tree's trunk doesn't die back, and generated deep lava lights nothing; the server only looks at ground players have changed, to save the board's time.
- **Clients:** only tested with Luanti 5.17. Older clients, down to protocol 37, are accepted but untested.

## What won't be done

- **Lua mods:** the server doesn't run Lua, so it can't load mods or other games. Minetest Game is built in, in Rust; anything new is added to the code.
- **More than one world at a time:** one world per SD card (or in the board's flash). Swap cards for another world.
