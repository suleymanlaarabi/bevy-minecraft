# ECS gameplay architecture

Compact voxel sections remain authoritative for terrain states, connections,
collision and lighting. Only blocks with interactive or support capabilities are
materialized. The catalog classifies native states once; `BlockState` is a derived
read model, never a second mutable opening state. See the
[extension cookbook](block-behavior-cookbook.md) and
[static review and measurements](ecs-gameplay-review.md).

## Ownership

`GameplayColumns` maps column coordinates to independent `GameplayColumn` entities.
Each column contains only its local-cell address index, initialization status and
persistence revisions. Native Bevy relationships maintain ownership:

| Source | Target collection | Lifetime |
| --- | --- | --- |
| `BlockOf` | `BlockEntities` | Despawning a gameplay column despawns its blocks |
| `LoadedColumn` on terrain | `LoadedTerrain` | Terrain unloading leaves gameplay entities alive |
| `PairedWith` on the right chest | `ChestPartners` on the left chest | Pairing never transfers items |
| `ViewingContainer` on an actor | `ContainerViewers` | The canonical chest owns the user list |
| `ViewOf` on a rendered root | `BlockViews` on its block | Block destruction removes its views |
| `ChildOf` on rendered roots/lids | `Children` | Terrain unloading destroys the rendered hierarchy |

The relationship components are the source of truth; inverse collections are
maintained by [Bevy's relationship hooks](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/relationship/trait.Relationship.html).
`BlockEntity` hooks maintain local-cell lookup entries. The two coordinate indices
contain entity identifiers, never inventories or mutable transfer records.
`Dormant` excludes unloaded blocks from active interaction and rendering queries.
Support work on a dormant owner suspends until its terrain returns.

`ManualOpen`, `Door`, `Gate`, `WallSupport`, `Vine` and `Chest` select capabilities.
Required components create chest storage and per-instance light data. Workers
prepare immutable column transfers; integration classifies capabilities and reuses
any existing dormant owner. It never replaces retained contents with a stale disk
copy. Ordinary terrain cells have no gameplay entity. A restored support task can
briefly have an address-only entity until terrain discovery determines its state.

## Actions and inventories

Player input resolves a usable entity and emits the targeted `BlockUse` event.
Sneaking with a block selected bypasses use before placement. Capability observers
validate expected states, reach and character collision. `EditBatch` validates all
cells before consuming placement material, editing voxels or producing drops.
Door edits and support removals retain atomic multiblock commits. Physics ticks
and climbing contacts remain in the character controller's existing order.

A chest half owns `ItemSlots<27>`; the player inventory uses the same contiguous,
copy-on-write slot storage. Empty default storage has no array allocation.
Immutable save snapshots share arrays. `HotbarSelection` and `CraftingResult` are
separate components. Crafting stages output placement, input consumption and
remainders in a copy before committing. Closing the screen returns the cursor and
crafting inputs, dropping only leftovers that cannot fit.

Pointer observers and screen keyboard inputs enqueue `InventoryAction` values.
One system drains them in order, validates the registered owners and physical
rules before each action, and borrows each inventory once. `ScreenSlots` is the
single borrowed adapter across owners; click, collect, swap, distribution, pickup
and transfer share the `Slots` primitives. Slot roots carry an ECS owner and local
index; decorative children use their parent slot. UI does not own item copies.

Native chest properties determine left/right pairing. Relevant edits and terrain
arrivals/departures mark topology dirty. Topology changes close affected users;
range and obstruction validation also close invalid screens. Closing or invalidating
a screen discards queued actions. Pairing changes never relocate stacks.

## Work and presentation

`EntityWork<T>` queues only entity identifiers. A work component's add hook queues
it once; that component contains all progress or dependency state. Suspended
support work remains on its block and is included in checkpoints. Arrival wakes
only work waiting for that coordinate. Every popped identifier counts against the
frame budget, including missing entities:

| Work | Budget | State |
| --- | --- | --- |
| View preparation | 64 candidates | `PrepareView` on the block |
| Relighting | 256 units | `Relight` cursor and rerun flag on the column |
| Support validation | 256 candidates | `SupportCheck` on the block |

Relighting walks `BlockEntities` with a cursor. Invalidations during a pass retain
a complete rerun, including after structural changes. Arrival and departure
invalidate neighboring light dependencies. Bounded samples are computed in
parallel and applied to instance tags sequentially. Support calculations also run
in parallel; their expected-state batches commit in FIFO order. Completion is
queued before commits can schedule a new generation of support work.

All normal chests share six body/lid meshes and one material. Body roots belong
to terrain for transforms and to blocks for ownership. Lid targets follow native
container users and removed relationships. Only `MovingLid` entities animate;
the animation query runs in parallel. There is no copied previous session.

Held-item metadata loads through a version-checking
[Bevy AssetLoader](https://docs.rs/bevy_asset/0.20.0/bevy_asset/trait.AssetLoader.html).
Selection, equip transitions, visual construction and texture waiting use separate
systems/components. Asset success, failure and reload events wake pending work or
rebuild cached visuals. Shared meshes/materials and the missing-item symbol remain
available, including after first-person views are recreated.

## Scheduling and persistence

Hotbar selection runs in `Update` before block actions in `PostUpdate`. The
post-update dependency sequence is player interaction, `VoxelPipeline`, topology,
validation, inventory actions, derived data, presentation and transform propagation.
Ordered structural boundaries flush commands. Lighting and lid targeting depend
on view preparation but have no artificial dependency on one another.

`Last` detects inventory writes before snapshot capture and acknowledgements, then
cleans dormant columns after acknowledgements. Structural container removals also
advance column revisions. Columns remain alive while dirty, while terrain is
retained, while a copy is pending, or while support work needs them. An old
acknowledgement can acknowledge only its captured revision; later changes retain
the column. Failed transactions acknowledge nothing.

The existing worker separation, per-column storage versions and common SQLite
transaction for terrain, containers, player, cursor and drops remain intact.
Empty container snapshots explicitly delete their row. Serialization records are
transfer values, never a parallel mutable runtime store. Current world and asset
formats reject incompatible data without migration or compatibility interfaces.
