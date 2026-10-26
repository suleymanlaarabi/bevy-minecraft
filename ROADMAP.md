# Hollow Roadmap

## Goal and scope

Build a complete single-player Minecraft-inspired sandbox: survive, gather resources, build, farm, fight, explore dimensions, automate production, complete an endgame, and keep playing afterward.

The target is coverage of the major gameplay and content families, with coherent rules and progression. Exact Vanilla timings, generation parity, combat formulas, redstone quirks, visual details, and every decorative variant are not completion requirements. Simplified mechanics must still serve their gameplay purpose. Multiplayer, protocol compatibility, and mod/plugin compatibility are outside this roadmap.

The bundled block, item, recipe, entity, and world-generation registries are the starting content inventory. Their presence does not mean their gameplay is implemented. Account for every family they contain during the content audit; a newer upstream Minecraft release does not automatically expand the target.

## Status and completion rules

This baseline comes from source inspection on 2026-10-26. The game was not run. **Implemented** means an implementation was found in the game code, not that runtime behavior was verified.

| Status | Meaning |
| --- | --- |
| Implemented | A game implementation exists for the stated capability. |
| Partial | Some integration exists, but important gameplay or content is missing. |
| Missing | No game implementation was identified during this audit. |
| Needs audit | Data or supporting code exists, but end-to-end coverage is unconfirmed. |

Checked boxes below identify existing capabilities. Remaining-work boxes stay unchecked until the complete behavior is implemented, including its inventory effects, presentation, chunk lifecycle, and persistence where applicable. Family examples describe coverage targets rather than an exhaustive list of individual registry entries.

## Current foundation

- [x] Seeded Overworld generation with Fast and Vanilla quality settings and asynchronous chunk streaming.
- [x] Voxel rendering, block models, fluid surfaces, texture-pack selection, shadows, and lighting updates after edits.
- [x] Character movement, collision, jumping, crouching, sprinting, swimming, climbing, and flight support exposed through developer controls.
- [x] First-person and third-person cameras, player skin/rig, held-item visuals, and arm animations.
- [x] Block targeting, timed breaking, placement, item consumption, and dropped block items.
- [x] Placement rules for multiple block families, including orientations, connections, slabs, and paired blocks.
- [x] Manual wooden door/trapdoor/gate interactions, basic support checks, and normal single/double chests.
- [x] Hotbar and inventory interactions, armor/offhand slots, and shaped/shapeless recipes fitting the personal 2x2 grid.
- [x] Dropped-item movement, pickup, expiration, and save/restore records.
- [x] World creation/selection/deletion, asynchronous saves, saved player inventory and pose, chest contents, and explicit incompatible-save rejection.
- [x] Day/night cycle, sun, moon phases, stars, clouds, and saved world time.
- [x] Main menu, graphics settings, and developer controls.

### Important gaps behind the existing foundation

| Area | Status | What remains |
| --- | --- | --- |
| Block/item registries and assets | Needs audit | Check actual family coverage, obtainability, models, item use, and behavior. |
| Mining | Partial | Tool effectiveness, harvest requirements, durability, loot rules, and experience; current breaking uses block durations and generally drops the block item. |
| Item stacks | Partial | Current game stacks track item identity and count; add per-stack state for damage, enchantments, names, potions, and stored contents. |
| Crafting | Partial | Personal 2x2 recipes exist; workbench 3x3, processing stations, and recipe discovery remain. |
| Equipment | Partial | Slots and visuals exist; armor protection, weapon effects, durability, and equipment abilities remain. |
| Interactive blocks | Partial | Normal chests and manual openings are integrated; most stations, machines, and special storage remain. |
| Fluids | Partial | Rendering and character interaction exist; dynamic flow, buckets, and fluid-driven world updates remain. |
| World generation | Needs audit | The generator contains many biome/feature/structure implementations; audit their game exposure, loot, entities, and progression. |
| Dimensions | Partial | Generation types include Nether and End; game sessions and world metadata currently select and accept only the Overworld. |
| Simulation | Partial | Fixed character physics and targeted block updates exist; a general gameplay scheduler and random block ticking remain. |
| Game modes | Partial | Developer flight exists; complete Survival and Creative rules and interfaces remain. |
| Survival, combat, and living mobs | Missing | Implement the player survival loop, damage, AI, spawning, and combat. |
| Audio and weather | Missing | Integrate sound playback and gameplay weather beyond the existing sky/cloud presentation. |

Source entry points: [game composition](src/game.rs), [player interactions](src/player/interaction.rs), [inventory and crafting](src/inventory/), [block behavior](src/block_behavior/), [voxel generation](src/voxel/generation.rs), [world lifecycle and saves](src/world/), and [world-generation crate](crates/worldgen/src/).

## Milestone order

| Milestone | Outcome | Prerequisites |
| --- | --- | --- |
| M1 | Persistent gameplay simulation foundation | Existing voxel/world foundation |
| M2 | Gather, craft, and process resources | M1 |
| M3 | Complete player survival loop | M1, M2 |
| M4 | Dynamic environment | M1; damage integrates with M3 |
| M5 | Renewable food and materials | M2, M4; animal tasks also need M6 |
| M6 | Living creatures and combat | M1, M2, M3 |
| M7 | Complete block and item families | Starts with M1; closes alongside M2-M12 |
| M8 | Functional stations and storage | M1, M2; specialized tasks need M7/M11 |
| M9 | Redstone and automation | M1, M4, M8 |
| M10 | Rewarding exploration and transport | M4, M6; dimension content needs M11 |
| M11 | Dimensions and advanced progression | M3, M4, M6, M8 |
| M12 | Endgame and continued play | M9, M10, M11 |
| M13 | Complete modes and everyday usability | Starts early; closes after gameplay milestones |
| M14 | Finished presentation and distribution | Starts early; closes after M1-M13 |

Milestones group dependencies rather than require strictly serial development. Start the content audit, audio, settings, and documentation early. The first useful survival slice is gathering wood, crafting tools and a furnace, preparing food, surviving a hostile night, sleeping, and saving progress; this slice is not the full completion target.

## M1 - Persistent gameplay simulation

**Status: Partial.** Reuse the existing fixed schedules, voxel edit pipeline, work budgets, and world lifecycle.

- [ ] Add a gameplay clock, scheduled block/fluid ticks, random ticks, and bounded neighbor-update processing; keep rendering and simulation clocks distinct.
- [ ] Separate simulation distance from render distance, with explicit activation and deactivation rules for chunks, entities, and block machines.
- [ ] Suspend simulation in unloaded chunks and restore pending work when they return; avoid unlimited catch-up work or forcing the whole explored world to stay active.
- [ ] Make simulation edits use validated voxel changes so blocks, inventory transactions, support rules, lighting, and meshes stay consistent.
- [ ] Extend game item stacks with per-instance data and correct stack equality for durability, enchantments, names, effects, and portable storage.
- [ ] Extend snapshots and saves for living entities, machines, timed work, player stats, progression, and later dimension identity.
- [ ] Preserve state across chunk unloads and saving; prevent duplication or loss when an action overlaps a save or unload.
- [ ] Keep generation and imported gameplay data explicit: handle supported records and reject incompatible assets or formats with actionable errors.

**Exit criteria:** A timed block behavior, a stateful item, and an entity all have defined activation, pause, unload, and save/restore behavior using the same world lifecycle.

## M2 - Gathering, crafting, and processing

**Status: Partial.** Personal 2x2 crafting and basic block breaking already exist.

- [ ] Add pickaxes, axes, shovels, hoes, and shears, with material tiers, suitable blocks, harvest requirements, and durability.
- [ ] Derive mining results from tool, block, and player state, including incorrect tools, underwater work, instant-break blocks, and unbreakable blocks.
- [ ] Replace generic self-drops with loot rules: stone/cobblestone, ores/raw materials/gems, leaves/saplings, gravel/flint, crops, multi-part blocks, and container contents.
- [ ] Add the crafting table and 3x3 shaped/shapeless recipes, ingredient tags, repeated crafting, and remaining containers such as empty buckets.
- [ ] Add furnaces, blast furnaces, smokers, fuel, burn/cook progress, valid input/output slots, experience, and lit block states.
- [ ] Add campfire cooking and charcoal production, with sensible alternative routes for early fuel and food.
- [ ] Add a searchable recipe interface showing ingredients, station requirements, available materials, and recipe output.
- [ ] Audit the resource progression from wood and stone through iron, diamond, and advanced materials; complete missing acquisition routes for ingredients and required stations.

**Exit criteria:** A new world supports the complete wood-to-iron loop through gathering, 3x3 crafting, smelting, tool use, and saved machine progress without developer item grants.

## M3 - Player survival

**Status: Missing.** Build on the existing character movement and equipment slots.

- [ ] Add health, hunger, saturation/exhaustion, eating/drinking, food effects, regeneration, and starvation.
- [ ] Add damage sources, invulnerability windows, armor mitigation, knockback, and damage feedback.
- [ ] Handle fall damage, drowning, suffocation, lava/fire, void damage, freezing, cactus/thorns, and hazardous blocks.
- [ ] Add oxygen, burning, and other timed player conditions; connect sprinting and movement costs to survival rules.
- [ ] Add death, a death screen, item/equipment/experience consequences, and safe respawning without duplicating inventory.
- [ ] Make beds usable for sleeping, advancing the night, and setting a persistent respawn point; handle obstruction, nearby threats, and destroyed beds.
- [ ] Add health, hunger, armor, air, experience, and status-effect HUD elements with clear low-resource feedback.
- [ ] Persist player stats and conditions; replace void recovery with the intended Survival death rules while retaining safe loading behavior.

**Exit criteria:** Food, shelter, equipment, sleep, danger, death, and respawning form a coherent survival loop, and save/restore preserves its state.

## M4 - Dynamic environment

**Status: Partial.** Static fluid surfaces, fluid movement interaction, and local placement updates already exist.

- [ ] Simulate water and lava sources, flow levels, falling flows, source regeneration rules, and updates after placement/removal.
- [ ] Add bucket filling/emptying, fluid replacement, waterlogging, sponges, cauldrons, and water/lava block reactions.
- [ ] Apply fluid currents to characters, items, and later vehicles; add bubble columns and their effects.
- [ ] Add falling sand/gravel and related blocks, landing resolution, concrete-powder conversion, and falling-block hazards.
- [ ] Add fire ignition, spread, extinguishing, fuel consumption, lava ignition, and TNT/explosion effects on terrain, entities, and drops.
- [ ] Add grass/mycelium spread, leaf distance and decay, plant support, growth, vines, and natural block transformations.
- [ ] Add rain, snow, thunderstorms, lightning, weather-dependent extinguishing, and dimension/biome weather rules.
- [ ] Add snow accumulation, ice formation/melting, powder snow, copper oxidation/waxing, and amethyst/dripstone growth.
- [ ] Complete special terrain interactions: farmland trampling, path creation, log stripping, moss/bonemeal transformations, and renewable stone generators.

**Exit criteria:** Changes propagate across loaded chunk boundaries, stop safely at inactive terrain, resume on activation, and preserve durable state without unbounded update cascades.

## M5 - Farming and renewable resources

**Status: Missing for the gameplay loop.** World-generated vegetation is only a starting point.

- [ ] Add tilling, farmland hydration, seed planting, growth stages, harvest yields, replanting, and bone meal.
- [ ] Cover grains, vegetables, stems and fruit, berries, cocoa, sugar cane, cactus, bamboo, mushrooms, kelp, and Nether wart.
- [ ] Add sapling/fungus growth into trees, required space and soil, renewable wood, and leaf-derived saplings/apples.
- [ ] Add passive-animal feeding, breeding, babies, maturation, following food, and population constraints.
- [ ] Add renewable wool, milk, eggs, leather, meat, feathers, and relevant animal products, including shearing and regrowth.
- [ ] Add bees, nests/hives, flowers, pollination, honey, honeycomb, collection, and defensive behavior.
- [ ] Add composters and renewable fertilizer; connect crop and animal products to cooking and recipes.
- [ ] Add fishing, bobber/cast/reel behavior, catches, fish buckets, and aquatic food sources.

**Exit criteria:** A player can maintain food, wood, crops, and animal products through renewable systems that persist across leaving and reopening the world.

## M6 - Creatures and combat

**Status: Missing.** Registry entity types and generated structures do not create living game entities by themselves.

- [ ] Build shared ECS capabilities for health, movement intent, sensing, targeting, navigation, animation, equipment, and loot.
- [ ] Add navigation around voxel obstacles, doors, steps, drops, and water, with bounded path work and reuse of the character collision foundation.
- [ ] Add natural spawning by biome, dimension, light, and space; population caps, despawning, persistence, and spawner blocks.
- [ ] Cover passive land animals, aquatic mobs, ambient creatures, neutral mobs, and hostile mobs with distinct useful behaviors.
- [ ] Implement representative hostile behaviors: melee pursuit, ranged attacks, exploding enemies, climbing, teleportation, sunlight vulnerability, and swimming/flying movement.
- [ ] Add taming, ownership, sitting/following, animal defense, names, leads, and persistent pets.
- [ ] Add swords, axes, bows, crossbows, arrows, thrown items, tridents, and other bundled weapon families, with hit resolution and appropriate attack timing.
- [ ] Add shields, equipment attributes, armor wear, projectile ownership, critical/special hits, and combat effects.
- [ ] Add experience orbs, entity loot, death behavior, and saved named/tamed/equipped creatures.
- [ ] Complete remaining registry mob families through the content matrix, including dimension-specific and event-specific creatures.

**Exit criteria:** Hostile nights, ranged and melee fights, passive animals, and pets all work with terrain, damage, loot, chunk activation, and persistence.

## M7 - Block and item family completion

**Status: Needs audit.** This is an ongoing content milestone, closed after the systems that support each family are ready.

- [ ] Audit all bundled registry families and map each to a gameplay system and milestone; record actual exceptions explicitly.
- [ ] Complete natural blocks, ores, raw resources, storage blocks, stone variants, soils, sand, gravel, clay, snow, and ice.
- [ ] Complete wood families: logs, stripped logs, planks, leaves, saplings, slabs, stairs, fences, gates, doors, trapdoors, signs, and boats.
- [ ] Complete construction/decorative families: masonry, glass/panes, metal blocks, copper states, wool/carpets, concrete, terracotta, dyes, and light sources.
- [ ] Complete vegetation, aquatic blocks, mushrooms, coral, hanging/climbing plants, crops, flowers, pots, and biome-specific growth blocks.
- [ ] Complete functional families: stations, storage, beds, redstone, transport, portals, spawners, and progression blocks.
- [ ] Complete non-voxel decorations and usable items: paintings, item/glow frames, armor stands, books, maps, banners, heads, and fireworks.
- [ ] Give every supported family correct obtainable items, recipes/loot, placement states, collision/selection shapes, interaction, support/removal rules, and relevant saved state.
- [ ] Replace unintended generic/fallback visuals for supported content; keep item icons, held models, and placed models consistent.
- [ ] Document any deliberately omitted individual variants; do not use that exception to omit an entire major gameplay family.

**Exit criteria:** Every major bundled content family has a complete gameplay route. No essential progression item exists only in data, and no advertised functional block is merely decorative.

### Family coverage matrix

The status column describes the current game integration, not upstream data completeness. Update rows as the owning milestone advances.

| Family | Current status | Remaining behavior or audit | Owner |
| --- | --- | --- | --- |
| Natural terrain, minerals, and ores | Partial | Harvest conditions, drops, processing, and resource availability | M2, M7 |
| Wood, leaves, saplings, and fungi | Partial | Renewable growth, decay, harvesting, and full variant coverage | M4, M5, M7 |
| Stone, masonry, slabs, and stairs | Partial | Audit shapes, connections, recipes, obtainability, and variants | M7 |
| Fences, walls, panes, bars, ladders, and vines | Partial | Audit connectivity, support/removal rules, and all family variants | M4, M7 |
| Doors, trapdoors, and gates | Partial | Powered behavior, metal variants, and remaining support cases | M7, M9 |
| Beds | Partial | Placement pairing exists; sleeping, spawn points, and dimension effects remain | M3, M11 |
| Glass, dyes, wool, carpet, concrete, and terracotta | Partial | Audit colors, recipes, transparency, and transformation rules | M4, M7 |
| Copper, waxed/oxidized variants, and metal decoration | Needs audit | Oxidation, scraping/waxing, special interactions, and recipes | M4, M7, M9 |
| Torches, lanterns, candles, and other light sources | Partial | Audit support, ignition, state changes, and lighting for all variants | M4, M7 |
| Water, lava, waterlogged blocks, and sponges | Partial | Dynamic flow, collection, absorption, reactions, and currents | M4 |
| Sand, gravel, concrete powder, and anvils | Partial | Falling behavior, conversion, landing, and impact damage | M4, M8 |
| Crops, farmland, fruit, and renewable plants | Partial | Generated/placed states exist; farming and harvest cycles remain | M4, M5 |
| Snow, ice, coral, moss, dripstone, and amethyst | Partial | Environmental rules, survival, growth, and resource renewal | M4, M5, M7 |
| Normal chests and double chests | Implemented | Integrate richer stack data and automated transfer | M1, M9 |
| Barrels, trapped chests, ender chests, shulker boxes, and bundles | Missing | Storage rules, portable contents, powered interactions, and persistence | M8, M9, M11 |
| Crafting tables, furnaces, smokers, blast furnaces, and campfires | Missing | Workstation interactions, processing, fuel, and progress | M2 |
| Anvils, grindstones, smithing tables, and stonecutters | Missing | Repair, upgrades, naming, recipes, and item modification | M8, M11 |
| Looms, banners, pots, signs, books, and bookshelves | Needs audit | Editing/custom data, display, decoration recipes, and interactions | M7, M8 |
| Maps, compasses, clocks, and cartography tables | Missing | Tracking, map data, exploration, and map modification | M8, M10 |
| Enchanting tables, brewing stands, and effect items | Missing | Experience costs, enchantments, brewing, and active effects | M11 |
| Redstone, repeaters, comparators, observers, and sensors | Missing | Signal propagation, logic, timing, and block reactions | M9 |
| Pistons, slime, honey, dispensers, droppers, hoppers, and crafters | Missing | Movement, transfer, item use, and automation | M9 |
| Rails, minecarts, boats, and rideable creatures | Missing | Vehicle entities, riding, tracks, steering, and storage | M10 |
| TNT, fire, fireworks, and explosive entities | Missing | Ignition, explosions, effects, projectiles, and terrain consequences | M4, M6, M7 |
| Sculk, vibration sensors, and stealth threats | Missing | Vibration events, detection, growth, summons, and counterplay | M9, M10 |
| Suspicious blocks, brushes, and archaeology rewards | Missing | Brushing progress, buried items, loot, and decorations | M10 |
| Spawners, trial encounters, vaults, and event blocks | Missing | Activation, enemies, keys/rewards, cooldowns, and saved encounter state | M6, M10 |
| Nether/End blocks, portals, gateways, and respawn anchors | Partial | Generator data exists; dimension gameplay, activation, and special rules remain | M7, M11 |
| Beacons, conduits, and other area-effect blocks | Missing | Structure requirements, resource costs, effects, and activation | M11, M12 |
| Tools, weapons, armor, shields, and advanced equipment | Partial | Item identities/slots/visuals exist; durability, effects, and gameplay remain | M2, M6, M11 |
| Food, potions, buckets, fishing rods, leads, and utility items | Partial | Identities exist; consumption, active use, state, and resulting effects remain | M3, M4, M5, M6, M11 |
| Passive, aquatic, ambient, neutral, hostile, and tameable mobs | Missing | Spawn, AI, animation, interactions, loot, and persistence | M5, M6 |
| Villagers, traders, golems, raiders, and bosses | Missing | Social/progression systems, encounters, and rewards | M11, M12 |

## M8 - Stations and storage

**Status: Partial.** Extend the existing inventory actions, sparse block entities, and chest persistence.

- [ ] Generalize station inventory views and interaction rules around the current inventory action pipeline, including slot restrictions, outputs, and transactional item movement.
- [ ] Add barrels, trapped chests, ender storage, shulker boxes, and bundles; preserve portable contents and prevent invalid nesting or duplication.
- [ ] Add anvils for repair, combining, and naming; grindstones for repair/disenchanting; smithing tables for upgrades and trims.
- [ ] Add stonecutters, looms/banner patterns, cartography tables, and their supported transformation recipes.
- [ ] Add editable signs/hanging signs, writable books, lecterns, bookshelves, decorated pots, and persistent custom text/display data.
- [ ] Add jukebox/music-disc and note-block interactions, connected to the audio work in M14.
- [ ] Handle breaking, explosions, blocked access, range checks, unload, and saving while a station UI is open.
- [ ] Persist station inventories, fuel/progress, upgrades, names, patterns, and portable storage without losing their per-stack data.

**Exit criteria:** Each station exposes only valid actions, processes items without duplication, and restores its inventories and state after unloading or reopening the world.

## M9 - Redstone and automation

**Status: Missing.** Target understandable circuits and useful automation; exact Vanilla update-order quirks are not required.

- [ ] Define stable signal strength, propagation, direct/indirect power, update ordering, and scheduled delays using bounded work queues.
- [ ] Add redstone wire/torches, switches, buttons, pressure plates, tripwire, daylight detectors, and target blocks.
- [ ] Add repeaters, comparators, observers, lamps, powered doors/trapdoors/gates, and container signal output.
- [ ] Add pistons and sticky pistons, movement limits, collision handling, slime/honey behavior, and block-entity movement restrictions.
- [ ] Add hoppers, droppers, dispensers, filterable inventory interactions through circuits, and automatic crafting.
- [ ] Add powered/detector/activator rails and powered interactions with vehicles and explosive blocks.
- [ ] Add vibration events, sculk sensors, calibrated detection, shriekers, and applicable sound-blocking behavior.
- [ ] Persist delayed circuit work and machine state, and define boundary behavior when only part of a circuit is active.

**Exit criteria:** A player can build a switched door, a delayed circuit, an observer-driven farm, item sorting, and a crafting/transport chain that survive chunk unload and save/restore. Feedback circuits cannot create unlimited work in a frame.

## M10 - Exploration and transport

**Status: Partial.** Reuse existing terrain/structure generation; audit generated content before adding parallel implementations.

- [ ] Audit biomes, caves, ores, vegetation, and structures exposed by both generation quality modes; list actual missing or unreachable content.
- [ ] Populate structures with contextual chest loot, mobs, spawners, interactive blocks, and renewable or unique rewards.
- [ ] Cover underground ruins/dungeons/mineshafts, temples, ocean structures, villages, strongholds, Nether structures, and End cities through the supported generator families.
- [ ] Add exploration maps, terrain mapping, map markers, compasses, lodestones, clocks, and position/orientation tools.
- [ ] Add boats and storage boats, minecarts and rail variants, mounting/dismounting, collision, passengers, and vehicle persistence.
- [ ] Add rideable land creatures, saddles, steering, jumping, and inventory/equipment where applicable.
- [ ] Add gliding equipment and flight boosts, durability costs, fall/impact consequences, and safe interaction with streaming.
- [ ] Add archaeology, brushing, suspicious blocks, treasure, and the associated decorative rewards.
- [ ] Add trial-style encounters, keys/vault rewards, cooldowns, and saved encounter completion.
- [ ] Connect sculk/stealth encounters and biome-specific dangers to their creatures, loot, and environmental counterplay.

**Exit criteria:** Exploration offers distinct challenges and useful rewards beyond raw terrain, with working transport and maps across long trips and saved sessions.

## M11 - Dimensions and advanced progression

**Status: Partial.** Nether/End generation support exists below an Overworld-only game integration.

- [ ] Make world sessions, chunk addresses, gameplay lookups, storage, and player location dimension-aware, including different height ranges.
- [ ] Add Nether portal construction, ignition, linking, coordinate conversion, return routes, and safe arrival/chunk loading.
- [ ] Add End portal activation, stronghold progression, entry platforms, exit portals, and End gateways.
- [ ] Integrate Nether/End atmospheres, biomes, resources, structures, fluids, hazards, creatures, and dimension-specific time/weather rules.
- [ ] Add dimension-dependent beds, respawn anchors, respawn fallback, and portable/ender storage behavior.
- [ ] Add experience levels and spending, enchanting tables, bookshelves, enchantment generation, enchanted books, and equipment enchantment effects.
- [ ] Add brewing ingredients, recipes, fuel, potion containers, splash/lingering forms, and a shared timed-effect system for players and mobs.
- [ ] Add advanced material upgrades, equipment special abilities, totems/death protection, and other progression items represented by the bundled content.
- [ ] Add villagers and traders: jobs, workstations, schedules, stock/restocking, prices, reputation, breeding, and golem protection.
- [ ] Add raids, illager encounters, event triggers, rewards, and persistent event state.
- [ ] Add conduits and other environment-dependent buffs, with activation requirements and costs.

**Exit criteria:** A player can progress naturally into the Nether and End, return safely, develop enchanted/brewed equipment, and use villages and trading. Each dimension and progression system restores its own state correctly.

## M12 - Endgame and continued play

**Status: Missing.** Completion requires a meaningful destination as well as an open sandbox afterward.

- [ ] Add the main End boss encounter, arena/crystal interactions, phases, damage windows, death, rewards, and persistent defeat state.
- [ ] Add the summonable Wither-style boss, summoning requirements, fight behavior, destruction, and unique drops.
- [ ] Add beacon construction, valid bases, fuel/payment, selected effects, and range; connect rewards to large builds and automation.
- [ ] Complete optional major threats and structure encounters, including underwater guardians, raids, and sculk-related threats, with distinct rewards and counterplay.
- [ ] Add progression goals/advancements that guide tools, food, exploration, dimensions, bosses, and automation without requiring prior Minecraft knowledge.
- [ ] Add a clear completion moment and continued-world option; keep postgame exploration, boss resummoning, building, and resource renewal available.
- [ ] Review the full dependency chain for essential materials, keys, loot, crafting stations, and return routes to prevent progression dead ends.

**Exit criteria:** The complete survival-to-endgame route is supported without developer controls, completion is recorded, and the same world remains useful for long-term building and automation.

## M13 - Modes and everyday usability

**Status: Partial.** Main menus, world management, graphics settings, and developer flight already exist.

- [ ] Add explicit Survival and Creative world/player rules, mode selection, saved mode/difficulty, and consistent transitions.
- [ ] Add a Creative item catalogue with categories/search, item picking, unlimited placement, instant breaking, flight controls, and appropriate damage rules.
- [ ] Add difficulty and relevant world rules such as inventory retention, mob spawning, fire spread, weather, and day progression.
- [ ] Add a real in-game pause menu with resume, settings, save/quit, and consistent simulation/timer pausing; Escape currently leaves the world when the inventory is closed.
- [ ] Add rebindable controls, sensitivity, audio settings, UI scale, readable tooltips, and contextual item/block names.
- [ ] Add accessible HUD cues, subtitle support for meaningful sounds, adjustable motion effects, and visible feedback for unavailable interactions.
- [ ] Add concise first-session guidance for movement, crafting, food, sleep, danger, and progression.
- [ ] Provide useful local commands for game mode, time, weather, teleportation, and item grants through explicit player controls.
- [ ] Complete world rename/backup/restore and metadata presentation; preserve explicit errors for unsupported or corrupt worlds.
- [ ] Keep developer diagnostics separate from the normal gameplay interface and document the supported controls and settings.

**Exit criteria:** A new player can start either mode, understand the controls, configure the game, pause, save, and return without needing developer tools.

## M14 - Presentation and distribution

**Status: Partial.** Terrain/sky presentation and player animations exist; audio and many gameplay effects remain.

- [ ] Integrate Bevy audio and sound assets for footsteps, blocks, items, UI, mobs, combat, machines, weather, music, and ambience, with volume categories and spatial playback.
- [ ] Add particles and feedback for block breaking, hits, death, eating, fire, smoke, fluids, portals, potions, and explosions.
- [ ] Add creature/vehicle animations, equipment poses, use animations, and readable visual states for functional blocks.
- [ ] Audit transparent/cutout content, waterlogged geometry, biome/dimension presentation, item icons, and supported texture-pack coverage.
- [ ] Review work admission, memory lifetime, simulation caps, chunk activation, inventory invalidation, and render uploads as new systems are added.
- [ ] Ensure resource/mob/circuit workloads stay bounded by active areas and explicit budgets; assess optimization changes through code inspection and reasoning.
- [ ] Complete loading/saving/progress indicators, asset/import errors, failure recovery, credits, and controls documentation.
- [ ] Audit asset provenance, licenses, and attribution; assemble a complete redistributable default asset set and keep user-imported assets optional.
- [ ] Prepare documented packaging and installation for supported platforms, including asset paths, user data locations, version information, and platform prerequisites.

**Exit criteria:** The intended distribution contains the assets and documentation needed to install and use the game, gameplay has coherent audiovisual feedback, and every major subsystem has bounded work and clear failure handling.

## Engineering rules for every milestone

- Use modern Bevy features, ECS components/relationships/observers, fixed schedules, and maintained framework functionality where they replace custom plumbing.
- Keep terrain in compact voxel storage. Materialize only entities requiring independent state or behavior; avoid an ECS entity for every ordinary block.
- Favor data-oriented layouts, shared immutable capability tables, change-driven updates, parallel work on independent data, and bounded asynchronous compute/I/O.
- Review all query and resource accesses of each affected system, including implicit accesses from `Changed<T>` and `Added<T>`. Prefer a single mutable query with change inspection, or sequential `ParamSet` access; use disjoint filters only for truly disjoint entities.
- Extend the current edit, inventory, streaming, and save pipelines rather than creating competing authorities for the same state.
- Replace obsolete beta interfaces/formats directly and reject incompatible saves/assets explicitly. Do not add compatibility shims, legacy paths, or migrations.
- For refactors, format first, record production lines excluding blank/comment-only lines, and measure the same way afterward. Reduce actual production code by removing duplication/dead plumbing; report missed reduction targets.
- Verify code changes only with `cargo check`. Never compile in release mode or run the game. Do not add or run tests or benchmarks to validate performance optimizations; use inspection and reasoning.
- Keep code, comments, UI text, and project documentation in English.

## Completion review

The scenarios below are acceptance criteria for implementation coverage, not instructions to run the game or introduce a test/benchmark suite. Under the repository rules, review their code/data paths and use `cargo check` for code changes. Track any remaining runtime uncertainty explicitly; source inspection cannot prove runtime correctness or detect every ECS access conflict.

- [ ] **Fresh survival world:** Obtain wood, tools, stone, fuel, food, and shelter; craft and smelt without developer item grants.
- [ ] **Danger and recovery:** Survive a hostile night, eat/heal, sleep, die, and respawn with consistent inventory/experience consequences.
- [ ] **Renewable base:** Grow crops/trees, breed animals, gather products, store them, and resume their durable state after leaving.
- [ ] **Machines and circuits:** Process resources, transport/sort items, and run a redstone-controlled farm/crafter across chunk lifecycle changes.
- [ ] **Exploration:** Travel using vehicles/maps, find populated structures, obtain their rewards, and return with persistent loot and companions.
- [ ] **Dimension progression:** Enter/leave Nether and End, preserve inventories and positions, and recover correctly from death in each dimension.
- [ ] **Advanced progression:** Trade, earn/spend experience, repair/upgrade/enchant gear, brew/use effects, and complete major encounters.
- [ ] **Endgame:** Defeat the main bosses, receive rewards, build a beacon, record completion, and continue using the world.
- [ ] **Creative:** Find every supported content family, build freely, fly, and switch/save modes through normal controls.
- [ ] **Content closure:** Every major registry family is accounted for, supported content has functional behavior, and documented variant omissions do not break progression.
- [ ] **State integrity:** Chunk unload, pause, autosave, final save, and restore preserve world/player/entity/machine state without duplicate rewards or lost inventories; incompatible data is rejected explicitly.
- [ ] **Delivery:** Controls/help, readable UI, sound/effects, redistributable assets, installation documentation, and error handling cover the completed feature set.

The game is complete when all milestones and completion-review items are closed. Finishing the initial survival slice, loading all block models, or registering every item alone does not close the roadmap.
