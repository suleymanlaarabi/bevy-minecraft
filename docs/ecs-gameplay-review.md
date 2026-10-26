# ECS gameplay refactor review

## Measurement result

The architectural changes are implemented, but the requested reduction targets
are **not achieved**. Production Rust code increased. This is a missed acceptance
criterion, and this implementation must not be presented as meeting the complete
plan.

The baseline was recorded after `cargo fmt --all`, before implementation. The
same formatter and [counter](../scripts/measure_rust_production.py) were used after
implementation. The counter excludes blank lines, comment-only content, files
named `tests.rs`, and items gated by `#[cfg(test)]`. Feature-gated production code
is included. Raw measurements with the fixed file scope are available
[before](ecs-gameplay-before.json) and [after](ecs-gameplay-after.json).

The fixed gameplay scope is every Rust file under `src/block_behavior` and
`src/inventory`, plus `src/player/interaction.rs` and
`src/player/skin/held_item.rs`. New files in these directories count in the same
scope. The total covers all production Rust under `src`; moving code outside the
gameplay scope therefore cannot reduce that total. Other production languages
were unchanged by this refactor. Measurement tooling and documentation are
reported separately from application production code.

| Measure | Before | After | Change |
| --- | ---: | ---: | ---: |
| Gameplay production lines | 4,744 | 5,125 | +381 (8.03%) |
| All production Rust lines | 21,409 | 21,809 | +400 (1.87%) |
| Net production Rust added relative to `HEAD` | 2,927 | 3,327 | +400 |

The 30% gameplay target requires at most 3,320 production lines.
Both that target and the required decreases in total production code and net diff
size were missed. The diff row is a net production-line measure relative to the
same repository `HEAD`, not a claim about gross patch insertions. Formatting,
renaming, comments and spacing are not claimed as architectural reductions.

Reproduce the current measurement with:

```text
python scripts/measure_rust_production.py src
```

## Removed implementations

The mutable unloaded-container resource, dense container index and its structural
generation were removed. ECS columns and native block relationships now retain
ownership through unloads. Copied container sessions, the nested container
read/write adapters and their transaction callbacks were removed. The single
screen adapter borrows real owners directly. The central block-use dispatcher and
outcome enum were removed in favor of targeted capability observers.

Visual progression maps, queues carrying coordinates, and the copied previous UI
session were removed. Generic component add hooks enqueue entity identifiers;
progress and dependencies live on components. Player insertion/quick-transfer
code now uses the common slot primitives. Inventory storage no longer owns the
hotbar selection or derived crafting result. Synchronous held-manifest file I/O
was replaced by an asset loader and event-driven texture waiting.

These removals are real, but the new capability, lifecycle and action systems
outweighed their line savings. No measured performance or memory improvement is
claimed.

## Static ECS access review

All affected systems, observers, hooks and nested system parameters were reviewed.
The following contracts describe the actual access boundaries, including change
filters and relationships. Native relationship writes occur through deferred
structural operations, not through simultaneous mutable query parameters.

| Systems or observers | Access review |
| --- | --- |
| Capability synchronization, column integration | Deferred exclusive structural commands; copied scalar addresses are released before mutable column/entity access. Existing same-kind capabilities are retained. |
| Address hooks | Read immutable `BlockEntity`/`BlockOf`, then mutate only the owning `GameplayColumn` index. Missing owners during teardown are tolerated. |
| Terrain unload | Read terrain/column/block relationships; queue dormant/topology changes. Gameplay entities are not terrain children. |
| Topology update | Nested voxel, spatial lookup and chest queries are all read-only. The separate dirty/viewer queries are also read-only; relationship changes are deferred. |
| Pair removal and viewing removal | Read existing relationships before removal; defer user closure. Only actions and the optional next UI state are mutated as resources. |
| Active-screen validation | Read motion/viewing data and nested topology; defer invalid relationship removal. |
| `BlockTargets` and `BlockCollisions` | Read spatial metadata, capability presence and character state. Placement adds read-only player/voxel access and mutates only the arm swing. |
| Chest and manual-use observers | Read capability/state/actor data. Nested topology and collision queries only read components, including shared motion/state reads. Commits are deferred events. |
| Voxel batch commit | One mutable voxel query performs validation then writes. Mutable player inventory and immutable hotbar selection use different component types. |
| Support arrival | Mutable support-work query, read-only chunk query, one mutable entity FIFO. |
| Support evaluation/commit | One mutable support query is accessed in parallel and then sequentially. It reads addresses/capability presence/dormancy; the nested voxel reader is read-only. No mutable voxel query coexists. |
| Inventory input and pointer observers | Selection/offhand mutations use distinct component types; pointer actions mutate only the FIFO, hover handlers only the cursor. |
| Inventory actions | One mutable player query and one mutable `ItemSlots<27>` query. `PlayerInventory` is a different component; nested topology never reads either mutable inventory component. Owners are borrowed once per action. |
| Crafting derivation | One query reads `Ref<PlayerInventory>` and mutates `CraftingResult`, including change inspection inside the query. |
| Slot binding | Read player, partner and added slot components; defer owner insertion. |
| Slot refresh | Read inventory, selection, result, owner and parent data. Added filters read view markers; icon visibility/image, text and border mutations are different component types. |
| View preparation | Read blocks, terrain relationships, views and voxels. Queues/assets are distinct mutable resources; render hierarchy changes are deferred. |
| Relighting | Mutable `Relight`, `InstanceLight` and `MeshTag` belong to three distinct query component types. Bounded samples run in parallel before sequential tag updates; all voxel/view/relationship reads are immutable. |
| Lid targeting | Read views and user/pair relationships, including removed-component readers; mutate one `Lid` query and defer activity markers. |
| Lid animation | One parallel query mutates `Lid` and `Transform`; removals use `ParallelCommands`. |
| Held-item selection/equip/build | Selection mutates desired state; equip mutates held state; construction mutates both in one query. Part parent reads do not alias either component. Systems have explicit structural ordering. |
| Held asset events/loader | The loader has no ECS access. Event handling mutates desired state and caches; asset messages do not borrow image components. |
| Inventory persistence detection | `Changed<ItemSlots<27>>` reads that component; the only mutable query is `GameplayColumn`. No conflicting inventory writer exists in this system. |
| Checkpoint capture | Player, terrain, support and nested container snapshot queries only read components. Shared block/address reads are compatible. |
| Acknowledgement/cleanup | Mutable voxel and gameplay-column queries access different types. Cleanup runs afterward with read-only column/work queries and mutates only the spatial index plus deferred despawns. |

No affected system combines a change filter reading a component with another
query mutating that component. No artificial `Without` filter is used to conceal
an overlapping access. Character physics/contact systems were not split.

## Scenario inspection

| Scenario | Inspected path |
| --- | --- |
| Double chest across a chunk boundary | Native properties select the canonical left half; `PairedWith` keeps each 27-slot owner independent. Both owners must be active and within reach. |
| Pair changes or one half is destroyed | Relationship removal closes canonical users, including target-collection deletion after the last partner disappears. Pending actions are discarded. No pairing-time item moves occur. |
| Obstruction or actor leaves reach | Topology availability and per-action/current-screen physical checks reject use and close viewing relationships. |
| Destruction with contents | Effective block replacement removes storage once and queues its stacks as drops. Column dirtiness preserves an explicit empty-row deletion. |
| Unload/reload before save | Gameplay owners survive independently; terrain-owned views disappear. Integration reuses initialized columns, including clean contents retained for dirty terrain. |
| Old acknowledgement | Write detection precedes acknowledgement. Captured revisions cannot acknowledge later writes; dirty columns cannot be cleaned. Pending copies and retained terrain also prevent cleanup. |
| SQL failure | No acknowledgement is applied. ECS inventories and retained terrain remain available for retry. The common transaction and storage-version checks are preserved. |
| Missing support/dependency | Work state remains on its block, including dormancy and missing coordinate; arrivals wake it, and checkpoints capture sorted pending addresses. |
| Cascades and shared cells | Bounded parallel calculations produce expected-state proposals. FIFO sequential commits reject stale overlapping proposals and effective edits schedule new work afterward. |
| Crafting and cursor restitution | Output placement/input consumption/remainders stage atomically; shared insertion handles returns, with leftover drops. Derived output is not a writable slot. |
| Missing held texture | Load failure wakes construction and selects the missing-item image. Image/manifest changes invalidate relevant caches and schedule reconstruction. |
| Held visual reappearance | New first-person roots have required selection/equip state and rebuild from shared caches; selection changes and reloads restart their transition. |

## Verification limits

Only `cargo check --all-targets` and `cargo check --all-targets --features dev`
were used for Rust verification. Both passed. No game, test executable, benchmark
or release build was run. Static inspection cannot validate runtime ECS execution,
rendered appearance, shaders, live SQLite failure behavior or actual performance.
The existing test fixtures were adapted to the new component interfaces and
compiled as targets; they were not executed.
