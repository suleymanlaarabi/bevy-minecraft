# Hollow

A Bevy voxel game. See the [architecture](docs/architecture.md) for code organization
and [world persistence](docs/worlds.md) for the save format and session lifecycle.
The [performance contracts](docs/performance.md) describe streaming budgets,
section visibility, GPU updates and the native-4K 144 FPS optimization target.

The neighboring `../worldgen` repository is required through local Cargo path
dependencies. Its `pumpkin-worldgen` facade embeds Pumpkin's Minecraft 26.3 terrain,
biomes, caves, ores, vegetation, structures, and lighting. `VoxelKind` uses native
16-bit block identifiers; `VoxelState` preserves properties such as orientation,
connections, halves, thickness, and waterlogging.

Chunks are 16×16×384. Gameplay uses positive Y coordinates:
`game Y = Minecraft Y + 64`, with sea level at Y=127. Seeds are signed 64-bit
integers. Generation uses its own pool and a shared Pumpkin context per session.
Negative coordinates and edited chunks follow the same streaming path. The player
waits for terrain meshes and collision readiness before entering the world.

## Worlds and saves

**Play** opens the world list. Creation accepts a name, an optional signed seed,
and **Fast** (default) or **Vanilla** terrain quality. These generation settings
remain fixed for the lifetime of the world.

Each world has a UUID directory containing `world.sqlite3` under the platform's
user data directory selected by `directories::ProjectDirs::from("org", "hollow",
"game")`, in `worlds/`. Set `HOLLOW_WORLDS_DIR` to override that root. To copy a
world, close its session and copy the entire directory; do not copy only the
SQLite file while the world is open.

Changed state is saved automatically every 30 seconds; an idle world is not
rewritten. Escape first closes the
inventory; otherwise it saves and returns to the world list. Closing the window
also waits for saving. A failed save keeps the window and in-memory data available
for retry; closing the window again leaves without saving.

Loading restores edited terrain, player position and orientation, inventory,
cursor stack, and dropped items. Physics stays paused until terrain and collisions
are ready. Hollow saves are not Minecraft-compatible. Renaming, deletion, and a
pause menu are outside this first version.

## Textures

The default **Minecraft** pack uses official textures extracted from the Minecraft
26.3 client downloaded from Mojang, with its SHA-1 verified. Textures and model
caches are imported locally; `blocks_array.png` and `models.json` are ignored by
Git. Without them, the game falls back to Faithful.

```bash
python3 -m pip install Pillow
python3 scripts/import_vanilla.py   # optional version argument, e.g. 26.3
```

The importer first checks `~/.minecraft/versions/26.3/26.3.jar`, verifies its
SHA-1, and downloads the official client if necessary. It resolves inherited
models, variants, multipart assemblies, and specialized model families into a
versioned `models.json`. Importing specialized geometry also requires the official
client libraries under `~/.minecraft/libraries` and a Java runtime with source-file
support. The importer prefers the launcher's `java-runtime-epsilon`; `JAVA_HOME`
overrides it. The headless helper invokes model factories only, without starting
Minecraft. Generated geometry and `special_array.png` remain local, ignored assets.

The import checks every block and property combination against the neighboring
Pumpkin registry. Missing visible geometry or textures, unmapped variants, and
packed-format overflows stop the import. Deliberately invisible models are
explicitly classified. Identical models share recipes across material aliases. After changing the importer, regenerate the
manifest, textures, and models together, then recompile to include the manifest.

It also writes `pack.ron` and extracts the ten official breaking textures into
`assets/voxel/breaking/vanilla/`. Breaking overlays multiply the already-lit
block color without adding brightness. These PNG files are ignored by Git;
without them, the included fallback textures are used.

Historical packs (Faithful, Replete, Meadow, Twilight) cover fewer blocks and use
`VoxelKind::fallback` for missing materials. See
`assets/voxel/packs/faithful/LICENSE.txt` and `scripts/import_faithful.py`.

## Water and spawning

New players search for dry ground and two free cells near the origin. Candidates
are probed with terrain-only generation before fully generating the chosen chunk.
Submerged blocks retain both their native state and water. Aquatic plants and
waterlogged blocks participate in water rendering and immersion checks; breaking
them leaves water behind. Vanilla aquifers remain enabled, so caves can contain
water. No global filling or draining pass is added.

Water meshes include every air interface, including sides and bottoms. No face
is emitted against a solid ceiling. Broad, flat water surfaces retain greedy
meshing. Uneven water, shore corners, falling states, and lava share a surface
builder that reads cached fluid families and amounts. Fluid ticks remain outside
this renderer.

## Lighting

Chunks retain Pumpkin's sky and block lighting. Terrain and water shaders combine
interpolated light with sunlight and Bevy shadows. Sheltered areas darken and
existing emitters light their surroundings. Lighting is always enabled.

Placement and breaking update native states in a separate context on a dedicated
worker. Changed light properties trigger local and neighboring updates without
regenerating terrain. Borders include diagonal neighbors, and merged faces retain
light gradients. Light-only changes do not rebuild collisions. Late worker
results cannot overwrite newer edits. Persisted chunks retain native edited cell
indices so lighting can reconcile them when reloaded.

## Shapes and interactions

Rendering covers the 1,286 native Minecraft 26.3 block types and 35,723 states
through imported recipes, including native specialized models for chests and
copper chests, shulkers, banners, skulls, decorated pots, conduits, copper golem
statues, and the enchanting book. Orientations, double-chest halves, statue poses,
colors, and generated state properties are retained. Technical invisible blocks
stay invisible instead of becoming fallback cubes. Import coverage describes
recipe availability; visual correctness still requires a separately authorized
in-game review.

Placement profiles derive wall aliases, attachment properties, and paired parts
from the registry. Doors, tall plants, and beds share pair placement/removal;
chests share connection handling across their material variants. Selection and
collision continue to use native shapes. Block drops use canonical item forms.

The [block behavior architecture](docs/block-behaviors.md) and
[extension cookbook](docs/block-behavior-cookbook.md) define the shared capability,
ECS ownership and persistence contracts. Right-click manually opens doors, trapdoors
and fence gates, or opens normal chests with 27/54 persistent slots and animated
lids. Sneak with a block selected to place against interactive blocks. Ladders and
vines support climbing, jumping upward and holding position while sneaking.
World format 2 rejects older saves without migration.

Compatible cubes keep greedy meshing and eight-byte vertices. Other models share
sixteen-byte vertices and section-level opaque, cutout, or translucent batches.
Static blocks do not create individual ECS entities. Deterministic multipart
assemblies are cached and shared, bypassing weighted selection during meshing.

Official animation timelines preserve frame order, duration, and interpolation on
the GPU. Static surfaces skip timeline reads entirely. Specialized textures retain
their source resolution in a shared atlas bank. State-dependent redstone/stem
colors and multi-index tints are resolved during import. End portals and gateways
use a layered procedural star effect. Water and glass share the translucent pass;
bounded tasks reorder only its indices when the camera changes cells. Icons and
dropped objects reuse the same model recipes; inventory icons remain still images.

Collision and selection use native shapes independently of opacity. Terrain's
voxel collider remains separate from convex partial-shape colliders; never nest
it inside a compound collider. Movement waits for collisions covering the player's
footprint and next step, and falling below the world restores the last safe point.
Small steps up to 0.6 blocks are traversed without a jump impulse after checking
clearance and support across the body. Movement also follows small descents and
keeps a gap matching the engine's contact tolerance. Auto-jump handles taller
obstacles.

## Current limitations

- No vanilla ticks: plant growth, falling dripstone, fluids, leaves, damage, and
  redstone are not simulated. Iron doors and trapdoors require future power logic.
- Storage and opening animation currently cover normal chests. Other containers,
  structure loot, signs, banner patterns, decorated pots, owner skins and other
  active block-entity effects remain unimplemented. Generated chests start empty.
- Exact model geometry requires the vanilla import. Without it, selection shapes
  provide a simplified fallback; incomplete packs reuse substitute textures.
- Biome tint is fixed to plains. Per-section and per-face translucent sorting
  does not provide exact composition for all intersecting geometry. Ambient
  occlusion still needs improvement.
- Lava has state-aware surface rendering and texture animation, without damage
  or fluid simulation.

## Verification

Use only `cargo check --all-targets` and
`cargo check --all-targets --features dev` to verify changes. Tests are compiled
without execution. Do not launch the game or run tests or benchmarks to verify
a change, and never compile in release mode.

Completed terrain sections appear progressively during loading. Assets are found
next to the executable, then in the build repository, or through `BEVY_ASSET_ROOT`.

## Inventory and skins

Press **E** to open the inventory. The original dark slots now include four armor
slots, an offhand slot, a 3D player portrait, and a 2×2 crafting grid. Standard
shaped and shapeless vanilla recipes that fit the grid are loaded from Pumpkin,
including ingredient tags, shifted patterns, and mirrored patterns. Click the
result to craft once; Shift-click crafts as many batches as fit in storage.
Number keys also move a crafted batch into the corresponding hotbar slot. Closing
the inventory returns crafting ingredients to storage and drops any overflow.
Armor slots accept compatible equipment only; Shift-click equips it. **F** swaps
the selected hotbar slot and offhand. **F5** cycles first-person, rear, and front
views. Third-person cameras shorten their distance when terrain blocks the view.

`assets/skin.png` is the default skin. Drop a PNG on the game window while the
inventory is open to import another skin. Imports are decoded and validated on a
worker before being stored in `assets/player/custom/`; the previous skin remains
available if import fails. **Classic arms / Slim arms** chooses the model, and
**Layers** toggles hats, jackets, sleeves, and trouser overlays. **Reset** restores
the supplied default. Skin choices use Bevy's persistent `skin` settings group.
Modern 64×64, legacy 64×32, and proportional HD skins up to 2048 pixels wide are
supported. Legacy left limbs are mirrored, base layers are opaque, and outer
layers preserve transparency. Shared meshes and textures drive the inventory
portrait, animated world model, and first-person arm. The portrait camera exists
only while the inventory is open. The hand swings on left-click, repeats while
mining, and swings when placing a block. Opening the inventory cancels the swing.
The hand follows Minecraft 26.3's first-person arm transforms and renders into a
transparent texture. The main world camera composites it behind the HUD, keeping
the hotbar above the arm without sharing window buffers between 2D and 3D cameras
or relying on matching their antialiasing settings. The hand target resizes with
the window and its camera is inactive outside the first-person inventory-closed view.

The selected main-hand stack replaces the bare arm with a cached block mesh or an
item sprite extruded along its opaque silhouette. Imported Minecraft first-person
display transforms set each item's position, rotation, and scale. Held objects use
their own swing transform, and switching item kinds lowers the old visual before
raising the new one. Empty slots show the skinned arm. The same cropped transparent
pass renders both, with lighting that follows the day cycle. Poses refresh when
returning from the inventory; failed item textures use a cached missing-item visual
and report the error once. Blocks reuse their
default-state geometry; specialized inventory-only block shapes, offhand rendering,
dynamic item models, and special use poses such as maps, drawn bows, or blocking
shields are outside this static main-hand renderer.

The vanilla importer also extracts item sprites, first-person display transforms,
and humanoid armor textures to
`assets/player/vanilla/`. To import these separately from a local official client:

```bash
python3 scripts/import_player_assets.py /path/to/minecraft-client.jar
```

These generated Mojang assets are ignored by Git. Without them, missing item
sprites use a placeholder and armor textures are unavailable; custom player skins
continue to work. Armor rendering covers the vanilla humanoid armor layers;
special equipment models such as elytra wings and wearable block heads are not
part of this renderer. Item durability, armor damage reduction, and special
crafting recipes (such as item repair or armor dyeing) are not simulated.
