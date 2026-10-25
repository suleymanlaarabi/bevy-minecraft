# Hollow architecture

Hollow targets Minecraft Java-style solo gameplay. Domains own their resources,
plugins, systems, and public facades. The engine stores blocks by chunk sections,
not as individual ECS entities. Performance favors shared snapshots, bounded work,
parallel workers, and event-driven view updates.

## Domains and entry points

| Domain | Responsibility |
| --- | --- |
| `app` | `HollowPlugin`, engine configuration, asset paths, optional developer tools |
| `game` | `GamePlugin`, top-level states, environment and Escape navigation |
| `menu` | Main menu BSN scenes and actions |
| `world` | World catalog, creation, sessions, checkpoints, transactional storage and UI |
| `player` | Player/camera components, asynchronous spawning, controls, interactions, recovery |
| `character` | Physics movement, sensors, stepping, water state and ground following |
| `dropped_item` | Drop spawning, physics, collection, lifetime and persistent snapshots |
| `spatial` | Collision layers and entity following relationships |
| `settings` | Graphics settings, BSN settings screen, engine application and Bevy Settings |
| `inventory` | Items, stacks, player inventory, icons, hotbar and inventory UI |
| `voxel` | Native states, chunks, streaming, edits, rules, persistence, lighting and collision |
| `voxel::render` | Texture packs, model geometry, materials, meshing and environment effects |

A domain root exposes its facade and composes its plugin. Small cohesive domains
remain in one file. Internal helpers are private or `pub(super)`; `pub(crate)` is
reserved for cross-domain integration. Public types retain established facade
paths such as `hollow::player::Player`, `hollow::settings::GraphicsSettings`, and
`hollow::voxel::VoxelWorld`.

Inventory UI separates scenes, interactions, and visual synchronization. The
voxel domain separates its API, coordinates, sections, chunks, streaming,
persistence, editing rules, lighting, collisions, and rendering. Tests that
inspect private implementation details remain next to that implementation.

## Plugin composition and states

`HollowPlugin` adds configured `DefaultPlugins`, `FeathersPlugins`,
`GameSettingsPlugin`, `PhysicsPlugins`, `VoxelPlugin`, and `GamePlugin`. Developer
tools follow when `dev` is enabled. Theme setup and Escape registration remain
in the application bootstrap. Automatic native window destruction is disabled;
world lifecycle code owns graceful exit requests.

`GamePlugin` initializes `GameState` and composes the world, inventory, player,
menu, character, spatial, and dropped-item plugins. Top-level states are `Menu`,
`Settings`, `WorldSelect`, `WorldCreate`, and `Game`.

`WorldPlugin` owns `WorldSessionPhase`, a substate of `Game`: `Loading`, `Playing`,
`Saving`, or `SaveError`. It opens storage and the generator before entering the
game, then loads the spawn area before enabling player simulation. Returning to
a menu keeps the session alive until its final checkpoint succeeds.
[World persistence](worlds.md) describes that lifecycle and its failure paths.

`InventoryState` is another substate of `Game`. Movement and interactions require
`Playing`, with the inventory closed where appropriate. Normal Escape handling
closes an open inventory before requesting a saved world exit.

`VoxelRenderPlugin` stays internal to the voxel domain. Shared chunk scheduling
remains in `VoxelPlugin`, preserving its ordering and deferred-command barriers.
Local `register` helpers compose systems; they are not alternative domain APIs.

## Resource ownership and data flow

| Owner | Resources and services |
| --- | --- |
| Settings | `GraphicsSettings`, registered and persisted through Bevy Settings |
| Worlds | Catalog, open/save tasks, `WorldSession`, immutable generation settings, database |
| Voxel | `VoxelSettings`, per-session `WorldGenerator`, chunk index, pending work, retained dirty chunks and streaming state |
| Lighting | `LightingState`, isolated Pumpkin context and worker |
| Rendering | `VoxelAssets`, texture packs, materials, image assets and mipmap jobs |
| Inventory | Inventory cursor and icon atlas; inventory data is a player component |
| Drops | Item mesh cache; stack and expiration deadlines belong to drop entities |

Consumers use their domain's facade. Texture icons and drops reuse internal
voxel rendering types rather than duplicating material and model logic. Player
spawning uses the generator adapter and `VoxelLoading` internally.

Uniform/dense sections and `Arc` copy-on-write snapshots keep cloning cheap.
Mesh, collider, lighting, and save tasks read shared snapshots. Sparse-set markers
represent transient chunk work. Budgets limit streaming copies, mesh uploads,
collision jobs, and despawns. Persistence revisions are independent of render
invalidations, so save acknowledgements do not trigger remeshing.

Pumpkin owns procedural terrain and its dependency stages. Hollow owns gameplay
edits and dynamic lighting. Persistent generator records retain native palettes,
biomes, heightmaps, stage and light; Hollow snapshots override generated block
state for gameplay. GPU assets, meshes, colliders and entity IDs are rebuilt.

## Scheduling contracts

The voxel pipeline remains a chained `PostUpdate` sequence before
`TransformSystems::Propagate`, active during `Loading` and `Playing`:

1. Collect completed chunk copies.
2. Refresh neighbors of newly spawned chunks.
3. Release chunks whose meshing neighbors are ready.
4. Update dynamic lighting.
5. Mark chunks requiring reconstruction.
6. Start mesh tasks.
7. Apply completed meshes.
8. Request required colliders.
9. Apply completed colliders.

Keep `.chain()` and its deferred-command synchronization. Streaming runs in
`Update`; asset preparation runs in `PreStartup`. Cleanup still runs on
`OnExit(GameState::Game)`, which occurs only after final saving succeeds or an
initial loading session is discarded before gameplay.

`PrepareSession` orders session setup before the player's spawn search.
Player orientation and input stay chained in `RunFixedMainLoop`, in
`RunFixedMainLoopSystems::BeforeFixedMainLoop`. Interactions run after camera
following and before transform propagation. Character movement remains between
physics preparation and simulation; ground following runs before physics
writeback. Drop collection follows transform propagation.

Checkpoints are captured in `Last`, after gameplay and deferred commands settle.
During final saving, physics and virtual time are paused. Failed writes retain
unacknowledged data. The writer barrier must finish before clearing session
resources or emitting `AppExit`.

## Maintenance and verification

All code and project content must be in English; see `AGENTS.md`.
Review every system's complete parameter access, including nested `SystemParam`,
`Added<T>`, and `Changed<T>`. Prefer a single mutable query with `is_changed()` or
`is_added()`. Use sequential `ParamSet` access when necessary, and disjoint filters
only when the underlying entity sets are actually disjoint.

Only the following verification commands are allowed:

```bash
cargo check --all-targets
cargo check --all-targets --features dev
cargo check --manifest-path ../worldgen/Cargo.toml --all-targets
```

These compile existing and new tests without executing them. Do not launch the
game, run tests or benchmarks, or compile in release mode. Compilation does not
validate ECS access conflicts, visual appearance, GPU behavior, or runtime
transitions; explicit static review remains required.

## Remaining boundaries

Native states drive shapes, collisions, targeting, and indexed rendering recipes.
Official specialized models are extracted headlessly at import time and assembled
through shared transforms; the game has no Java dependency or per-block render
systems. Equal geometry and deterministic multipart assemblies share immutable
quad lists. Materials share animation timelines and texture banks; static
surfaces bypass animation reads and fluid family/amount fits in the existing
four-byte state classification record.

Vanilla ticking, block-entity contents and decoration data, gameplay-driven model
animations, a pause menu, and networking remain separate work. Fast terrain stays the default for new
worlds; users may explicitly choose Vanilla at creation, and saves retain that
choice permanently.
