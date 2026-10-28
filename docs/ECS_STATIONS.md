# Station sessions and reactive presentation

This continues the [capability refactor](ECS_CAPABILITIES.md). It replaces two
inventory-session protocols, separates stack binding from rendering, and removes
the character controller's dependency on survival. The broader architecture work
in [PLUGIN_ARCHITECTURE.md](PLUGIN_ARCHITECTURE.md) remains incremental.

## Released API baseline

Before editing, the official [Bevy 0.20 release notes](https://bevy.org/news/bevy-0-20/),
[0.19 to 0.20 guide](https://bevy.org/learn/migration-guides/0-19-to-0-20/) and
[0.18 to 0.19 guide](https://bevy.org/learn/migration-guides/0-18-to-0-19/) were checked.
The installed 0.20 documentation/source supplied the precise component,
relationship, message and observer contracts. Lifecycle observers use the 0.20
`On<Remove<T>>` syntax. BSN remains restricted to presentation scene construction.

## One inventory-session protocol

Stations declare `InventoryStation::Crafting { side }` or `Container`. A viewer
has one `InventorySession` relationship to the canonical station; Bevy maintains
`StationViewers`. The old separate chest/workbench relationships, validation
systems, opening observers and closure registrations are removed.

`InventorySessionPlugin`, installed by `InventoryPlugin`, owns opening,
validation, closure and pending-action invalidation. `StationAccess` resolves a
small `InventoryLayout` on the stack. The action transaction validates the layout
again before borrowing slots. The validation schedule still runs after topology
and before inventory mutations; a changed crafting-grid size also invalidates the
session. Closing returns ingredients/cursor contents through the existing path.

The default station is a three-by-three crafting station. `WorkbenchPlugin` now
only registers this component through the existing block registry. The
compile-only `custom_content` example adds the same capability to its resin block:

```rust
world
    .resource_mut::<hollow_engine::blocks::BlockRegistrations>()
    .register_bundle::<hollow_minecraft::inventory::InventoryStation>(solid, true);
```

This uses the already allocated `BlockId`; it introduces no interaction system or
parallel identity registry. Non-default crafting sizes use an explicit component
value at registration. Required components add the container station capability
to chests. Factory detachment explicitly removes that capability: removing its
requiring component alone would leave it installed.

Chest pairing, canonical ownership, blocked lids and contents remain chest
policies. Their existing topology service validates both halves. The common
loaded-state/reach predicate is shared with crafting stations and manual opening.
`Container` does not promise arbitrary machine inventories or slot counts.

The single relationship merges player session archetypes. Each station gains a
small enum column; no component is added per slot or voxel. Reverse relationship
vectors replace the existing vectors. No layout, inventory copy or topology
cache is retained per viewer. Opening and validation add one station lookup;
validation remains a small per-frame pass over live sessions. Spatial indexes
remain sparse maps because block entities occupy a tiny subset of world cells.

## Stack values compose presentation

Slots, crafting results and the pointer publish `StackView(Option<ItemStack>)`.
It replaces the previous nested optional display cache. Both slot and cursor
markers require this component. Owners continue to store the actual items; the
view stores only the small value needed for display change detection.

`refresh_slots` binds ownership and selection. `follow_cursor_stack` positions the
pointer and publishes its value. `sync_stack_views` alone renders icons, counts,
empty-slot labels and optional durability children. Slot and cursor BSN scenes
share the icon/count templates. The cursor no longer owns a separate rendering
algorithm or count marker. Icon-resource changes also refresh stationary views.

`set_if_neq` avoids dirtying unchanged view values. Rendering inspects small live
UI grids each frame, then skips child queries and text construction unless the
value or icon resource changed. This adds one bounded lightweight view pass and
one icon child entity for the cursor, replacing duplicated rendering paths.
It adds no heap-backed view state, item-name lookup or per-frame structural change.

## Damage requests and accepted reactions have different roles

`DamageRequest` stays a buffered message. Requests are resolved sequentially so
armor broken by one request affects the next request in the batch.
`DamageApplied` is now a targeted entity event. Each accepted impact queues one
trigger command; observers run when deferred commands are applied after the
resolution system, not between requests in its loop.

Independent observers apply character knockback, nutrition exhaustion, fatal
item-use cancellation and player audio. They do not rely on observer ordering.
The completion-message buffer, four message readers and reaction scheduling set
are removed. Trigger-command storage and observer dispatch replace buffered
polling; large bursts could favor batching. No measured speedup is claimed and
no new per-actor marker, queue or archetype is introduced.

`Locomotion.enabled` groups suspension with the existing travel modifiers. The
survival preparation system sets it from health before character physics.
`CharacterPlugin` reads no survival component and registers no survival reaction;
the survival integration owns the knockback observer. Actors combine `Living`
and `GameCharacter` to receive this health-driven policy. Other controllers may
write the locomotion capability directly.

`PlayerSurvivalPlugin`, installed by `PlayerPlugin`, owns HUD, local hurt audio and
automatic safe respawn. `SurvivalPlugin` no longer installs player presentation.
Respawn shares the death check with inventory closure, eliminating a redundant
player query. It preserves the single streaming viewer and terrain/safe-position
gate. Station screens skip the unused player portrait camera and rig.

## Static access review

| Affected systems | Access and ordering |
| --- | --- |
| Station opening/validation | Block, terrain, topology and relationship queries are read-only; opening mutates only the actor grid and queues the relationship. Validation reads the grid. |
| Manual opening, pairing and lid updates | Shared access checks only read terrain. Reverse membership is read; relationship and joint changes remain deferred. |
| Inventory transactions | Player inventory/grid and container inventories are different component types. Nested station/topology queries do not read mutable slot storage. Actions retain one exclusive buffered consumer. |
| Slot binding and refresh | `Added<SlotOwner>` and the view query both read `SlotOwner`. No parameter also mutates it. Binding commands apply before refresh. |
| Shared stack renderer | View values/children are read-only. Icon and durability queries both mutate visibility, but durability excludes `SlotIcon`. Text mutations use a separate component type. |
| Cursor publishing | One mutable node/visibility/view query. It runs before the renderer in the presentation chain. |
| Character physics and locomotion | Physics has one actor query and read-only terrain. Survival writes locomotion in `Prepare`, before `CharacterSystems`. |
| Damage resolution/reactions | Attacker reads end before target mutation in the single actor query. Trigger commands apply after resolution. Observer queries mutate distinct capability types. |
| Player respawn | Loader transform reads require `RespawnLoader`; mutable player transforms exclude it. Inventory state resources access separate types. |

`cargo check` cannot detect runtime query conflicts or schedule behavior; these
parameters and nested SystemParams were explicitly inspected.

## Verification, accounting and remaining costs

Verification passed with `cargo check --locked --offline` and
`cargo check --locked --offline --workspace --all-targets --features dev`.
Targets were checked only: no tests, benchmarks, release builds or game launches
were run. Rendering, interaction timing and observer dispatch cost remain
unverified at runtime.

The maintained source roots (`src` and the `core`, `engine`, `features`,
`minecraft`, `worldgen` crate source directories) were formatted before the
reference snapshot and after edits. The existing production counter, applied
identically to both snapshots, reports **83,338 before and 83,311 after: net -27**.
It excludes blank/comment-only lines and test code; vendored generator crates
and examples are outside this production scope. The new session module is
included. Formatting, renaming and moves are not claimed as reductions.
Excluding complete import declarations on both sides gives **79,478 to 79,455:
net -23**; import regrouping alone does not account for the reduction.

The reduction is modest. Remaining adjacent costs include separate held/dropped
block-mesh caches, scans of loaded chest views, the large pose/environment travel
preparation, and specialized column persistence. They need separate ownership,
invalidation and scheduling designs; this change adds no speculative caches or
universal machine/effect framework to conceal those boundaries.
