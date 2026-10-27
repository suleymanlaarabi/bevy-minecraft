# Plugin architecture implementation status

This refactor is **in progress**. The shared services described below are used by
the Minecraft application; they are not a second, disconnected engine. The full
architecture plan is not implemented yet. The content catalog and fluid services now accept plugin-defined blocks and states.
See [Content and fluid plugins](CONTENT_AND_FLUID_PLUGINS.md) for the implemented API and its boundaries.
The subsequent [actor composition refactor](ECS_COMPOSITION.md) adds shared fixed-step
positions and interpolation, separates damage reactions, and uses native hierarchy
and bulk cleanup APIs. Its scope, tradeoffs and code accounting are reported there.
[Composable capabilities](ECS_CAPABILITIES.md) continues this work with item-use
intents and completion messages, transactional inventory batches, shared following,
and declarative modal cursor behavior.

## Implemented boundaries

| Package | Responsibilities now extracted |
| --- | --- |
| `hollow-core` | Typed numeric keys, `VoxelCell`, coordinates, copy-on-write sections, cuboid faces, bounds and the generic 32-bit terrain codec. |
| `hollow-engine` | Generic chunk components, read access, checked terrain writes, sparse terrain indexes, pending edits, block factories, ray traversal and typed entity work queues. |
| `hollow-features` | Statically dispatched stack operations, contiguous copy-on-write inventories, shared elementary joint clips, fixed-step position interpolation, following and modal cursor capture. |
| `hollow-minecraft` | The existing game, its content policies, world generator adapter, renderer, player, survival, menus and settings. |
| `hollow` | Executable composition entry point. |

The core, engine and feature packages have no dependency on Pumpkin, the
Minecraft package or the world-generation package. The Minecraft package uses
the same extracted implementations for its running data flow.

`VoxelCell` supplies content semantics through static dispatch. It does not add a
trait object, allocation or name lookup per cell. This enables other applications
to use their own cells while the Minecraft adapter is being replaced.

The engine's headless `minimal_voxel` example declares states 70,000 and 70,001,
creates a column and submits a checked terrain edit without Minecraft data or its
generator. It has been compile-checked only. It is not a rendering showcase or the
planned resin-machine example.

## Block plugins and interactions

`BlockBehaviorPlugins` composes the block registry, lifecycle integration,
`ChestPlugin`, `WorkbenchPlugin` and `ManualOpeningPlugin`. A content plugin can
register a default bundle with `BlockRegistrations::register_bundle`, or provide
structural callbacks with `register`. Registration uses the existing block ID
values rather than allocating a second set of gameplay identities.

The registry is finalized in `PreStartup`. A dense block index addresses one
contiguous callback array. Ordinary terrain cells carry no callback, entity,
string or reference-counted pointer. Factories run when a block is materialized
or replaced; state-only refresh callbacks are explicitly opt-in. Default bundles
are not reset on every state change.

One registered interaction owner handles each interactive block. Other factories
can compose non-interactive components on that block. Targeting consults the
compiled interaction flag instead of enumerating chest, workbench and door types.
An owner can implement its own input arbitration using normal Bevy observers.
The generalized interaction-result protocol from the plan remains to be built.

Support evaluation already operates on the persistent neighbor-work queues.
Ladders, vines and support-only blocks therefore no longer receive unused ECS
marker entities. Their support rules and pending work remain intact.

## Terrain transactions and targeting

`VoxelWorldPlugin<T>` initializes the terrain resources and direct-write observer.
`VoxelWorld<T>` provides reads and distinguishes available cells, unavailable
terrain and positions outside bounds through `WorldSample`.

`VoxelEditWorld<T>` validates all expected loaded values before committing a
batch. Validation reports unavailable terrain, duplicate cells or a failed
precondition. Inventory consumption, durability, loot and exhaustion remain in
the Minecraft action layer. Terrain commits queue `BlockChanged<T>` notifications after applying accepted
writes; Minecraft subscribes to enqueue its neighbor work. The engine
has no player, inventory or Minecraft simulation dependency.

The existing action layer still performs inventory precondition checks before
calling `commit_validated` in the same system invocation. That method's contract
requires prior validation and no intervening terrain mutation or deferred flush.
It does not provide arbitrary rollback for user-written ECS side effects.

The common ray traversal accepts sampling and shape-selection functions through
static dispatch. Minecraft retains its own water/lava selection policy. The
unused server-style raycasting interface was removed from the generation crate;
the game uses the shared traversal and the existing shape intersection math.

## Items, inventories and articulated parts

Minecraft now has one item identity per native item ID. Block placement and model
selection use a precomputed optional block association in the item catalog.
Inventory persistence stores this single ID. Item caches no longer reserve
separate identity domains for block items and sprite items.

The generic slot service operates on a small `Stack` trait and has no Minecraft
item dependency. Merge, insertion, swapping, collection and distribution use the
same implementation as the player inventory, crafting and chest adapters.
Inventories retain their lazy, contiguous copy-on-write slot allocations.

`BlockBox::faces` emits cuboid quads without allocating. The Minecraft model
fallbacks and chest geometry use it. `JointClip` is a shared Bevy asset describing
one rotation, translation or scale channel. An instance stores its handle,
progress and target. `AnimatingJoint` restricts the parallel update to moving
parts. The chest uses this service instead of its own lid-animation system.
The chest target-update system explicitly runs before the articulation set and
transform propagation.

This is an elementary joint API, not yet the complete cuboid-model asset format,
model baker or multi-track animation API from the plan.

## Performance review

- Section storage, snapshots and their copy-on-write behavior are retained. State
  payloads are now four bytes: a dense 4,096-cell state section uses 16 KiB instead
  of 8 KiB. Uniform sections remain uniform. Save-admission accounting uses the
  actual state type size. Some per-state metadata also grows with this change.
- Block registries allocate for the registered block index range, not the 32-bit
  ID universe. Strings are absent from factory dispatch. The immutable catalog
  is shared with loading tasks through an `Arc`, not copied or retained per cell.
- Sparse coordinate indexes remain hash maps because world coordinates are
  unbounded. Sparse block-entity indexes remain appropriate for their tiny subset
  of cells. Inventory operations and state tables use direct indexing.
- The fluid passage cache now compares both complete 32-bit state IDs and a
  separate direction. Its fields retain a compact record layout instead of
  packing overlapping state bits into the old 16-bit key positions.
- Joint profiles are shared assets; inactive joints do not join the animation
  query. No per-instance graph is constructed for elementary joint motion.
- Chest-session removal IDs reuse a Bevy entity set rather than allocating a
  fresh set every frame. Cuboid face generation and item identity conversion
  plumbing have shared implementations.
- Duplicate-coordinate checks remain pairwise for the small placement and fluid
  batches currently submitted. This avoids a temporary hash set for those calls;
  a bulk-edit API would need a separate cost review.

Remaining costs include native 32-to-16-bit state adaptation, several native
metadata tables, scans of loaded chest views, the specialized inventory-screen routing. Fluid scheduling dispatch now uses numeric
profile IDs; the column work scheduler itself remains in the Minecraft adapter. No speedup or memory reduction
has been measured or claimed.

## ECS access review

- Generic terrain writes have one mutable chunk query. Read/validation passes and
  writes use it sequentially. Stored and pending columns are separate resources.
- Generic terrain reads have one immutable chunk query; `ChunkIndex` is read-only.
- Factory finalization reads the registration resource and inserts the immutable
  catalog before dependent startup work. Factories receive one mutable entity
  accessor; no competing queries are introduced.
- Joint animation has one query mutating `JointAnimation` and `Transform` and
  reads clip assets and time. The target observer is a separate system. Chest
  target selection only reads joint components and queues the observer command.
- The affected Minecraft queries retain their component access boundaries. No
  `Changed<T>` filter was combined with another parameter mutating the same `T`.

This is a source review. `cargo check` cannot establish runtime scheduler or
rendering correctness, and the application was not launched.

## Verification and code accounting

Verification is limited to `cargo check --workspace`,
`cargo check --workspace --features dev`, and
`cargo check --workspace --examples`. The engine example and feature package
are also checked separately to detect dependencies hidden by workspace feature
unification. No test, benchmark, release build or game launch was performed.

The tree was formatted before the reference count. The Rust source counter uses
the same comment/blank-line filtering before and after the refactor and includes
new libraries and the shipped example. Moves and formatting are not reductions.
Unchanged vendored/generated sources remain in the comparison; unchanged test
source contributes equally to both sides and does not affect the delta.

Rust source lines after formatting and excluding blank/comment-only lines:
**First tranche: 1,526,286 before; 1,526,450 after; net +164 lines**, including the example.
The subsequent content/fluid tranche is accounted for in CONTENT_AND_FLUID_PLUGINS.md.
The net production-code reduction target is **not met**. Extraction and removal of
obsolete plumbing have not yet offset the new service APIs and example. This
refactor must not be reported as complete or as a demonstrated code reduction.

## Remaining implementation work

1. Add a data-file importer on top of the Rust content registration API. The frozen
   catalog already accepts custom definitions; RON declarations are not implemented.
   Game column storage still uses the shared default 16-by-384-by-16 layout.
2. Expand model asset/animation authoring beyond the shared cuboid and fluid surfaces.
   Water, lava, fluid reactions and custom fluids already use the shared fluid API.
3. Generalize scheduled/random/neighbor work categories and dispatch. The existing
   simulation budgets and persistent queues still belong to the Minecraft layer.
4. Generalize slot ownership, screen sessions, recipe processing and persistence
   registration. Generic slot operations alone do not make custom machine screens
   and saved components automatically supported.
5. Expand actor, health, effect, audio and sky services beyond the shared position
   capability and separated damage reactions. Their content rules still belong to
   the Minecraft package.
6. Finish model assets and animation composition, then add the resin mixer and
   fluid example using only public APIs. Keep it disabled in the default game.
7. Meet the production-code reduction target and repeat the same static access
   review and permitted compilation checks for the remaining changes.
