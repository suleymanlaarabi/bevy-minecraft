# Hollow

A Bevy voxel game. See the [architecture](docs/architecture.md) for code organization
and [world persistence](docs/worlds.md) for the save format and session lifecycle.

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
models, variants, and multipart assemblies in `models.json`, extending the
texture array to model faces. After changing the importer, regenerate the
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
is emitted against a solid ceiling. Side normals remain intact; waves affect
top surfaces.

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

Rendering imports vanilla models for glass and panes, foliage, grasses, flowers,
tall and aquatic plants, vines, roots, bamboo, cactus, pointed dripstone, crystals,
torches, snow layers, carpets, slabs, stairs, fences, gates, walls, doors,
trapdoors, and bars. Generated states keep their properties. Local placement rules
orient blocks, merge slabs, update neighboring connections, and manage both halves
of doors and tall plants with one drop.

Compatible cubes keep greedy meshing and eight-byte vertices. Models are batched
by section into cutout or translucent passes. Water and glass share the
translucent pass; bounded tasks reorder only its indices when the camera changes
cells. Icons are rasterized from models in the background and drop meshes are
created on demand.

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
  redstone are not simulated. Doors and gates display their state but do not
  open interactively.
- Blocks rendered by block entities or specialized renderers use shape and
  material fallbacks. Their contents and animations are not implemented.
- Exact model geometry requires the vanilla import. Without it, selection shapes
  provide a simplified fallback; incomplete packs reuse substitute textures.
- Biome tint is fixed to plains. Per-section and per-face translucent sorting
  does not provide exact composition for all intersecting geometry. Ambient
  occlusion still needs improvement.
- Lava retains fallback rendering without damage or simulation.

## Verification

Use only `cargo check --all-targets` and
`cargo check --all-targets --features dev` to verify changes. Tests are compiled
without execution. Do not launch the game or run tests or benchmarks to verify
a change, and never compile in release mode.

Completed terrain sections appear progressively during loading. Assets are found
next to the executable, then in the build repository, or through `BEVY_ASSET_ROOT`.
