# M1 persistent simulation

Implemented with Bevy 0.20. Verification is limited to `cargo check`,
`cargo check --features dev`, and source inspection. No game launch, test,
benchmark, GPU validation, or measured performance claim accompanies this change.

## Time and ownership

`GameplaySimulationPlugin` owns the 20 Hz fixed timestep and the 100 ms virtual
clock clamp. All fixed phases share the playing/session-pause condition. Physics,
survival, drop movement, pickup delays and drop lifetimes use that schedule.
Mining progress consumes integer gameplay ticks from its existing interaction
pipeline. World time advances in fixed updates; sky presentation uses the fixed
overstep. Real time still drives autosaving and interface work.

The ordered phases are activation/clocks, neighbors, scheduled ticks, random
sampling, machine systems, actors, and spatial finalization. The machine phase
is available for future systems; no demonstration machine is registered.

`GameplayColumn` remains the persistent owner. `LoadedColumn` connects terrain to
that owner, `BlockOf` owns stateful blocks, and `SimulatedIn` maintains native
inverse relationships for mobile actors. Mobile relationships are deliberately
not linked-spawn: retiring a column must never despawn the globally saved player.
Drops are captured and explicitly despawned when their terrain retires.

Simulation membership uses physical character positions. The initial loading
viewer supplies demand before a player exists; the respawn viewer supplies it
while the dead body is waiting for safe terrain. Simulation distance defaults to
6 chunks and is independently persisted through Bevy settings, clamped to 2–12.
The settings page and developer panel both expose Simulation Distance.

The existing generation graph receives the exact union of visible and simulated
columns, expanded by all eight neighbor offsets. It admits roots nearest first.
Terrain activates only after its gameplay owner is initialized and its 3x3 data
neighborhood exists. Data-only columns do not build meshes. Leaving the visible
region cancels builds and releases presentation children; returning dirties every
section and requests chest views again. Gameplay owners survive both transitions.

## Work and transactions

A column's integer local clock advances only while it is active. Persisting local
deadlines avoids rewriting every task on deactivation and prevents offline or
out-of-range catch-up. An ordered set stores scheduled ticks, with a sparse
cell/channel index for direct cancellation and deduplication. Identity changes
invalidate pending work; execution checks identity again and removes the entry
before dispatch. New requests are due no earlier than the following local tick.
Ready-tick and neighbor markers restrict processing to columns with actual work;
only queue heads are examined while advancing active clocks. Column-coordinate
ordering and round-robin cursors distribute budgets across ready owners.

Neighbor reasons are merged by cell. Sparse section vectors retain isolated edits;
at 64 cells a section switches to three reason bitsets, using about 1.5 KiB to
bound dense-cascade merge costs. Empty sections release their storage. Connections
and supports use the same persistent column work. The old synchronous connection
cascade, global support queue, per-cell SupportCheck entities, and checkpoint
support list were removed. Border discovery runs outside observers, at most one
arrival per presentation frame, before adding persistent work.

`VoxelEditWorld::validate` and `commit` are shared by player batches and simulation.
They validate every expected loaded value before changing any cell, then update
terrain/save revisions, lighting, mesh invalidation, BlockChanged publication,
and subsequent neighbor work. Door pairs retain atomic edits and existing loot.
Fluid transactions commit sequentially, so later ticks read accepted changes.

Frontier waits are deduplicated by cell/channel on their owner and indexed by
chunk coordinates globally. Missing data or an inactive destination discards the
entire proposed fluid transaction and suspends its tick. Activation wakes only
indexed owners, including restored owners whose dependency is already active.
Suspended work never requests generation and never retains a terrain reference.
Eviction removes its transient global owner index; saved coordinates rebuild it.

## Fluids and random sampling

Flow rules were inspected in the installed Minecraft 26.3 client's FlowingFluid,
WaterFluid, LavaFluid and LiquidBlock classes using local decompilation. Runtime
lookup tables use native voxel state IDs and a closed water/lava family enum.
They include amounts, source/falling flags, waterlogged states and the native
washed-away tag. Propagation parameters come from the client rules rather than
the imported fluid tuning fields.

Implemented paths include downward/sideways flow, bounded slope searches, source
withdrawal, renewable water, non-renewable lava, waterlogging, partial collision
shape passages, washed-away water loot, and obsidian/cobblestone/stone/basalt
reactions. Water waits 5 ticks; Overworld lava waits 30, with the conditional
fourfold delay when a non-falling lava level rises. Only Overworld sessions are
currently instantiated by the game.

Fluid scratch buffers are reused. A fixed 512-entry passage cache bounds shape
pair storage; offset-dependent shapes bypass it. Face coverage uses projected
collision rectangles, not opacity. Each fluid evaluation resolves its 3x3 terrain
entities once, avoiding repeated sparse-coordinate hashing during slope samples.

Generation fluid ticks carry numeric registry IDs and native coordinates through
ProtoChunk, completion, the generation codec and shared snapshots. Import offsets
vertical coordinates by min_y. Only absence of a gameplay record seeds these
initial ticks; empty gameplay records are always written and never deleted.

The random registry allocates its direct state table only when a behavior is
registered. Eligible-section masks refresh after affected edits or activation.
Three full-section uniform cell attempts derive from seed, column, gameplay tick,
section and attempt index. Saturation drops unused attempts without debt. M1
registers no crop, fire or decay handlers, so it performs no random sampling.

## Persistence and pressure

The SQLite gameplay table directly replaces the containers table. A column record
contains inventories, dropped items, local time, scheduled work, neighbor work
and suspended frontiers. The global checkpoint contains player/global state,
world time and the gameplay clock; it no longer owns drops or support work.
Saved records contain no ECS entity IDs and introduce no format versions or
migration paths.

Save captures flatten queues into compact transfer vectors instead of cloning
both runtime tree indexes. Terrain and inventory arrays retain their existing
copy-on-write sharing. Terrain, gameplay records and checkpoint use the same WAL
transaction and existing asynchronous encoding pool. Reload during a pending
write reuses the in-memory owner. An old acknowledgement only acknowledges its
captured revision; eviction requires equality with the current revision and no
loaded/pending terrain.

Per-frame limits, shared by both possible fixed steps:

| Work | Limit |
| --- | ---: |
| Scheduled executions, including invalid/blocked attempts | 2,048 |
| Neighbor cells | 4,096 |
| Simulation mutations, reserved for whole transactions | 4,096 |
| Random attempts | 8,192 |
| Additional writeback reservation | 16 MiB |

The reservation is split into 8 MiB for retired owners, 6 MiB for coherent
captures plus codec buffers, and 2 MiB for indexes, candidates and headers.
Snapshots flatten only compact queues and share immutable terrain/inventory
arrays. Admission checks the entire coherent capture and the largest encoding
batch before allocating gameplay transfer vectors. Per-record reservations
include JSON growth and compression input/output, rather than only shared handles.
Encoding groups up to 1 MiB of reserved work per compute-budget job; a larger
single record runs alone only if the capture reservation still fits.

SQLite consumes these bounded encoding batches inside **one transaction**, then
writes the captured player/global checkpoint and commits once. No intermediate
batch can durably save a chest without its corresponding player inventory state.
The I/O worker keeps the transaction local while waiting for the existing compute
budget. There is no dedicated writer thread, per-tick task, channel backlog or
whole-world vector of compressed records. SQL failure rolls back every batch and
only successful final commit publishes acknowledgements.

Retirement reserves space before releasing dirty terrain. Early saving starts
at 4 MiB of retired state; admission stops if retirement cannot reserve space.
An oversized resident owner stays loaded while its save completes. If even the
coherent compact snapshot plus one codec batch exceeds its reservation, the save
reports SaveError before capturing it and retains all live work. Resuming
simulation can drain that backlog before retrying. This is a deliberate capacity
tradeoff: no essential task is silently dropped to satisfy the memory budget.
Reservation values are conservative accounting limits, not measured process RSS.
The developer-only ephemeral session has no database and keeps changes in memory;
acknowledged eviction applies to persistent sessions.

With the existing HOLLOW_DIAGNOSTICS report enabled, the dev build reports active
columns, neighbor cells plus ready tick heads, suspended work, consumed quotas,
maximum observed task lateness and estimated bytes waiting for writing.

## Static acceptance review

| Scenario | Reviewed path |
| --- | --- |
| Simulation below/above view distance; live settings changes | Exact union/halo demand; independent region and render markers |
| Initial load, teleport, fast borders, respawn | Physical motion, readiness gate, dead-body loading fallback; non-linked mobile ownership |
| Pause, saving, write errors, world changes | Shared fixed condition, local clocks, Real autosaving, session cleanup |
| Pending fluid/support/drop unload and resume | Column snapshots, remaining drop durations, local deadlines, frontier reconstruction |
| Return during writing; obsolete acknowledgement | Resident owner reuse, revision acknowledgement and eviction predicates |
| Empty queue after generation/bootstrap | Mandatory gameplay record and complete-generation tick transport |
| Source change before execution; duplicates | Identity invalidation, cell/channel indexes and execution-time recheck |
| Downward/side flow, withdrawal, renewal, waterlogging, mixtures | Fluid planning, shape passage, reaction and ordered commit paths |
| Inactive destination or missing data | Full transaction suspension; no generation request or per-tick frontier retry |
| Saturated budgets | Persisted essential queues, exact-deadline deferral, bounded save admission and discarded random debt |
| Explored-column release | Terrain acknowledgement, owner cleanup and frontier-index removal |

These are source-review conclusions, not executed acceptance scenarios. The
changed systems were reviewed for resource/query access including nested
VoxelEditWorld, VoxelWorld, VoxelLoading and ColumnSnapshots parameters. Simulation
mutation systems use one mutable ChunkVoxels query; the loading/render readiness
queries access distinct components. Ref/Changed filters do not overlap a mutable
query for the same component inside an affected system. Deferred ownership and
observer commands are separated by the ordered schedule dependencies. Player
survival, character physics and dropped-item actors filter Simulating.

## Production-line accounting

The pre-change sources were formatted before implementation. Counts below exclude
blank/comment-only lines and include dereference assignments as production code.
The same counter was applied to the formatted baseline and final sources.

| Scope | Before | After |
| --- | ---: | ---: |
| Changed existing Rust files | 10,953 | 11,345 |
| New simulation modules | 0 | 1,881 |
| Total changed production scope | 10,953 | 13,226 |

The net production-code reduction target was not met: this feature adds 2,273
production lines. Existing edits/support modules shrink from 719 to 589 lines,
but their replacement work infrastructure and new fluid/persistence behavior
must be included in the total. Formatting, shorter names, removed comments and
statement merging are not counted as reductions. No measured CPU or memory
improvement is claimed.

## Targeted performance follow-up

This follow-up changes only paths introduced or modified by the M1 diff. It
preserves simulation phases, spatial ordering, round-robin quotas, fluid search
order, and the atomic save transaction. It introduces no public API or save-format
change.

- Ready-column buffers carry coordinates beside entity IDs, resolving the sort
  key once instead of issuing ECS lookups during comparisons. The reused buffers
  retain eight additional bytes per candidate in exchange for eliminating those
  repeated lookups; no persistent index or invalidation machinery is added.
- Slope searches use a stack `u128` visited bitset instead of an allocated vector
  of coordinates with linear membership searches. The maximum four horizontal
  steps bound the domain to 81 cells in a 9x9 grid. Lava uses a smaller subset.
  The existing reusable breadth-first frontier and direction order are retained;
  blocked candidates are still reconsidered from other directions as before.
- Player/support edit batches validate terrain once before inventory effects,
  then call a private validated-application method. Other simulation commits
  continue to validate themselves. Deferred commands cannot run between the
  observer's validation and application, preserving expected-state rejection
  and whole-batch terrain writes.
- Mobile activation changes no longer reinsert an unchanged `SimulatedIn`
  relationship. Both ownership passes remain necessary to account for activation
  before actors and movement/teleports after actors.
- Save-pressure accounting skips work-tree size estimation for loaded columns
  still inside data demand. Dirty retired columns and loaded retirement
  candidates retain the same accounting and force-save thresholds.
- Render transitions and mesh readiness share one candidate set. View changes
  scan all loaded terrain; arrivals inspect only each arrival and its eight
  neighbors through `ChunkIndex`. The existing full-scan fallback when arrival
  notifications are absent is retained. Retry methods no longer receive unused
  stream offsets, removing that resource access from load completion polling.

The affected systems were inspected for ECS access conflicts. Neighbor and fluid
processors each retain one mutable `ColumnSimulation` query and one mutable
`ChunkVoxels` query inside `VoxelEditWorld`; the active-owner query only filters
membership. Edit observers read and write terrain sequentially through that same
parameter, with separate inventory components. Render selection shares immutable
`VoxelChunk`/ownership reads and mutates only `ChunkVoxels`. Mobile ownership and
save-pressure systems retain their previous read accesses and deferred/resource
writes. No overlapping read/write component queries or new deferred-command
boundaries were introduced.

Remaining costs include sorting ready columns each fixed step, sparse tree
indexes for scheduled/frontier work, and per-frame global pressure/diagnostic
scans. `BoundaryWaits::remove_owner` also scans all frontier buckets on owner
eviction. Those structures are unchanged by this targeted pass; replacing them
would require separate lifecycle/index design. No extra cache, synchronization,
or parallel scheduling is added to the ordered terrain commits.

Verification: `cargo check`, `cargo check --features dev`, and source inspection
of maximum slope bounds, chunk-border arrivals, view changes, ownership/activity
transitions, rejected edits, retirement pressure and old save acknowledgements.
These scenarios were not executed. No tests, benchmarks, release build or game
launch were performed. CPU and memory effects are expectations from the data
flow, not measured results.

### Follow-up production-line accounting

The baseline is the existing M1 working diff, formatted before this follow-up.
Both counts exclude blank/comment-only lines using the same counter; formatting
and documentation are not counted as code reduction.

| Rust source | Before | After |
| --- | ---: | ---: |
| `src/simulation/fluids.rs` | 706 | 705 |
| `src/simulation/mod.rs` | 431 | 433 |
| `src/simulation/work.rs` | 744 | 745 |
| `src/voxel/edits.rs` | 457 | 460 |
| `src/voxel/streaming.rs` | 621 | 612 |
| `src/world/mod.rs` | 828 | 830 |
| Total | 3,787 | 3,785 |

The follow-up reduces production code by two lines overall, principally by
sharing render/readiness candidate selection and removing obsolete retry
plumbing. The original feature's net reduction target above remains unmet.
