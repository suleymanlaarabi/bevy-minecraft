# Extending ECS gameplay

Read the [architecture](block-behaviors.md) before adding a capability. All project
content is English. Replace obsolete interfaces and formats directly; reject
incompatible worlds and imported assets without migrations.

## Add a capability

1. Classify the native family in `catalog::Profile` and resolve state properties in
   the immutable `StateBehavior` table. Keep allocating property decoding outside
   gameplay loops and preserve the table's compile-time size bound.
2. Define a focused component and install it in capability classification. Extend
   worker discovery through `materialized` when the family needs a sparse entity.
   Use required components for the invariant assembly. Keep ordinary cells in
   compact terrain sections.
3. Select the capability through typed queries or a targeted observer. Input
   should resolve an entity and submit intent. Validate current voxel state and
   physical constraints before constructing an expected-state `EditBatch`.
4. Include consumption and drops in the accepted batch. A multiblock edit must
   validate every cell and collision before changing any inventory or voxel.
5. Observe effective `BlockChanged` notifications. Same-kind state changes preserve
   persistent components. Destruction resolves contents once; unloading only marks
   entities dormant and removes their terrain-owned views.

## Own persistent data

Create an independent gameplay column with `column` and a sparse address owner
with `block`. Keep the `BlockOf` relationship and immutable address together so
index hooks remain valid. Use native inverse relationships for traversal and
linked destruction. Do not maintain another dense array or generation counter.

Store mutable data on components even while terrain is absent. Worker-prepared
values serve transfer and encoding only. A retained initialized column takes
precedence over a newly loaded record. Include new storage in the existing common
checkpoint transaction, with explicit deletion when a record becomes empty.
Detect writes before acknowledgement and dormant cleanup; failures must retain
all dirty data. Retain an owner for outstanding dependencies and copies.

For another container, reuse `ItemSlots<N>` and the shared `Slots` operations.
Extend the borrowed screen adapter's layout policy instead of copying pointer
handlers or mirroring inventories in UI. Views must address their owner and local
slot. Resolve topology once when its dependencies change; verify reach and physical
rules again before every action. Keep crafting output derived and consumption
atomic. Deliberately update the bounded drag capacity for larger screens.

## Schedule bounded work

Define a component holding progress and dependency state, and give it the
`queue::enqueue::<Self>` add hook. Initialize its `EntityWork<T>` resource before
spawning instances. Inserting the component deduplicates queued work. A FIFO may
contain only identifiers; do not place positions, snapshots or progress maps there.

Count stale identifiers against the budget. Suspended work remains on the owner
and wakes on its dependency's arrival. Preserve support addresses in checkpoints.
Complete a generation before deferred commits can invalidate it again. For a
partial scan, retain a rerun flag when dependencies change during processing.
Use parallel queries for independent calculations and ordered commits when work
can share cells or inventories.

## Present an appearance

Cache geometry and materials by immutable variant. Relate a view to its block and
parent it to loaded terrain for transforms and unloading. Put animation activity
on a sparse marker and remove it at the target. Derive targets from native users
and topology; handle removal of empty relationship targets explicitly.

Use the instance light tag for local chest lighting, preserving its shader path,
shadow passes, culling and texture-pack updates. Budget view preparation and
relation traversal, including stale requests and chunk departure invalidations.
Load metadata through an asset loader, reject unsupported versions, and wake
texture work from asset events. Keep shared fallback visuals for failed imports.

## Review and verify

Order actual dependencies with system sets and command barriers. Keep independent
presentation tasks unchained. Preserve the character controller's tick/contact
ordering. For every system and observer, review all nested query/resource accesses.
`Added<T>` and `Changed<T>` read `T`; use one mutable query with change inspection,
or sequential `ParamSet` access, when data can overlap. Use `With`/`Without` only
for genuinely disjoint entity sets.

Verify exclusively with `cargo check --all-targets` and
`cargo check --all-targets --features dev`. Do not run the game, tests, benchmarks
or release builds. Record code measurements using the same formatted production
scope before and after refactoring, and report missed reduction targets.
