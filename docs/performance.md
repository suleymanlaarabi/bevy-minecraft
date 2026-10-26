# Performance contracts

The optimization target is 144 FPS (6.94 ms per frame) at native 3840x2160 on the
Core Ultra 9 185H's integrated Intel Arc GPU, with FXAA, Low shadows and a render
distance of 24 chunks during exploration. Saved graphics preferences are preserved.
This is a target, not a measured result. Verification is restricted to `cargo check`
and code inspection; neither establishes achieved frame rates or visual correctness.

## Streaming and worker budgets

Mesh results are prepared on workers and integrated one section at a time, nearest
first, with player edits taking priority. A section's material passes are atomic.
The shared per-frame integration limits are 2 MiB, 32 actual meshes, or 1 ms of
integration work. One section may exceed the byte limit to guarantee progress.
Deferred ECS commands and subsequent render preparation are outside that timer.
Only applied section bits are cleared, and stale revisions are discarded before
charging the budget. An edit during partial integration keeps its new dirty bits.

Bevy's `RenderAssetBytesPerFrame` is also set to 4 MiB. Its asset preparation limit
is soft and permits one oversized asset. The explicit section budget remains
necessary because Bevy's mesh allocator uploads extracted meshes separately.
Transparent index patches have their own two-sort concurrency/application limit.

Both ordinary and edited chunks share the mesh-task ceiling: twice the async pool's
thread count. Collider requests retain a demand queue across frames, prioritize the
viewer, and limit concurrent builds to half that pool, with a minimum of one.
Required collisions still gate player movement; existing colliders remain until
their replacements are ready.
Finished and stale mesh/collider tasks are polled before new tasks are scheduled,
so their slots can be reused in the same frame. A result that cannot fit the
remaining integration budget does not mark its section metadata as changed.
Stale collider tasks are canceled before polling or charging integration limits.

Bevy's async pool uses a quarter of the available CPUs, bounded to one through four
workers. Pumpkin generation uses `clamp(available_cpus / 4, 1, 6)` through its public
options. Chunk snapshots continue to use section-level `Arc` copy-on-write.
Native generation data is compressed directly into section layers. Lighting workers
expand snapshots and compress results, keeping full-column conversions out of ECS.
Uniform native sections map their block kind once. Collider classification also
handles uniform sections once, reads dense arrays directly, and finds interior
shell neighbors through fixed linear offsets. It constructs coordinates only for
retained shell cells. Border edit notifications use a fixed array without a heap
allocation.

Lighting content/arrival/removal observers invalidate a synchronization version.
Once that version is synchronized and no worker/results remain, the lighting
system returns before collecting edited positions or scanning neighborhoods.
`BlockChanged` queues emission and opacity changes for local block-light propagation
before mesh scheduling in the same frame. Decreases finish before increases, which
read the current level rather than replaying stale queued light. Overlapping sources
are reseeded, and only affected sections and their border readers are invalidated.
Worker completions still check submitted content/entity versions before application.
In `pumpkin-worldgen`, emission-only edits preserve skylight and rebuild only block
light. Opacity or occlusion changes and streamed edits still reconcile skylight.
Replacing authoritative light compares it with displayed light separately only
when temporary rendering estimates actually exist.

Player edits have reserved mesh task capacity when streaming fills its ordinary
slots. A visible section retains its current mesh until the render world's
`RenderAssets<RenderMesh>` confirms that its replacement is prepared. Translucent
sorting data switches with its matching mesh, so uploading a replacement cannot
temporarily leave a wall without geometry or sort indices for the wrong mesh.

Chunk arrival observers feed neighbor refresh and readiness checks. Stable views
only reconsider waiting chunks touched by arrivals; view changes reconsider the
whole waiting set. Collision demand is refreshed on body/viewer chunk changes and
arrival/edit notifications, rather than probing every nearby column indefinitely.

## Rendering

Devices with `FLOAT32_FILTERABLE` use 128x64 sky and 32x32x32 aerial `RGBA32Float`
textures. Hardware filtering replaces manual interpolation of storage-buffer
samples. The aerial texture retains logarithmic values and the original coordinate
mapping. Other devices retain the storage-buffer path. Both paths keep HDR values,
exposure, immersion fog and the before-atmosphere compensation. Bind groups are
cached by buffer IDs/ranges, texture views and sampler identity. Water shares
its fog/reflection ray coordinates, and emissive models bypass diffuse lighting.
The fog amount is computed once per ray. Fully fogged terrain skips its surface
texture and diffuse lighting; cutout/model fragments retain required alpha tests
before skipping their lighting. Portal effects are skipped only when completely
covered by fog. Zero-shadow diffuse contributions skip atmospheric transmittance.
Water reuses the perspective view direction and avoids sky-reflection sampling
when its reflection weight is zero, including back faces. Orthographic views keep
their original view-direction calculation.

The first-person arm uses the same full-view perspective through `SubCameraView`.
Its render target and UI composite cover only the projected arm bounds, with an
eight-pixel margin and 64-pixel allocation buckets. UI positioning accounts for
window and UI scaling. Near-plane intersections or missing bounds use the full
target; fully offscreen arms disable their camera. Skin meshes retain explicit
bounds after GPU extraction, including the outer layer.
During swings, the cropped target retains its largest required allocation until
the window size changes; its crop can move without recreating color/depth targets.
Conservative full-target fallbacks do not inflate that retained allocation. This
trades a little unused transparent area for stable GPU resource allocations.

Transparent geometry uploads vertices once. Re-sorts send immutable index arrays
to the render world and patch the mesh allocator's current index allocation after
asset preparation. Mesh IDs protect replacements, index formats are checked, and
unprepared assets retain their latest pending order. Sorting keeps its existing
near/far position quantization, closest-first slot and oldest-first fairness.
New meshes and Bevy's visibility change ticks invalidate the idle shortcut. Rotation
alone preserves radial ordering; newly exposed sections after rotation/projection
changes still trigger a scan. Section-center extraction and sorting scratch buffers
are reused. Immutable sort metadata retains only each quad's cell, center and
winding, reconstructing its six indices from its four consecutive vertices.
Equality comparisons and disposal of unchanged index results run on workers.

Clouds greedily merge coplanar surfaces in both axes for all six orientations and
use 16-bit indices when possible. Low shadows retain the existing 1024 map size,
four cascades and 128-block distance. FXAA, bloom, cloud coverage, model detail,
texture animation and physical-sky settings retain their existing behavior.

## Section visibility

Workers compute six-face connectivity per section. Only opaque greedy full cubes
close passages: partial, cutout and model geometry remain conservative. Empty
sections connect every face; sealed solid sections connect none. Geometry-free
sections retain connectivity. Lighting-only rebuilds reuse current connectivity.
Meshing summaries skip absent model/cube passes and fully enclosed cube geometry.
Padded-row summaries also build the opaque bitset while that row is hot, avoiding
a second 4096-cell scan for connectivity. Lighting-only rebuilds skip rebuilding
that bitset. Connectivity floods start only on boundary cells, ignore sealed
interior components, and stop as soon as every face pair is connected.

A cached traversal starts from the camera section and the unknown outer/top
boundaries. The latter preserve paths that may pass through terrain outside the
loaded area. Traversal marks opaque boundary sections visible without traversing
through them. Batches with geometry crossing a section boundary remain visible
when an adjacent section is visible, preserving overhanging models and fluids.
Missing or outdated topology is treated as open. An unavailable,
non-finite, out-of-height or solid camera position reveals everything.

Traversal uses dense arrays and reusable queues. Incoming connections accumulate
into one pending item per section instead of one item per incoming face. Sections
without exits are marked visible without being queued. Each exit propagates once,
and layer writes are limited to changed visibility or changed section entities.
It runs after transform propagation and mesh integration, before visibility checks.
Ordinary frames reuse it until the camera section, camera validity, loaded area or
connectivity changes. Mesh-only replacements update their layers against the cached
traversal; they do not force another walk of the loaded grid.

Layer 4 hides section camera meshes from the world camera. Directional lights also
include layer 4, preserving cutout/model shadows. Dedicated depth meshes remain on
shadow layer 1 and are never hidden by camera connectivity. Inventory preview and
arm cameras retain their independent layers 2 and 3.

## Static review and allowed verification

Use only:

```bash
cargo check --all-targets
cargo check --all-targets --features dev
```

Review every changed system's complete accesses, including change filters and
nested parameters. The new visibility system reads chunk data/topology and only
mutates render layers; its change-filter query only reads section metadata. Neighbor
refresh consumes arrivals through one mutable voxel query. Collider invalidation
observers enqueue entities independently of mesh-marker removal; collision requests
only read voxel components. Arm cropping reads local transforms while
mutating separate Camera, Node, Visibility and Image components/assets. Render-world
fog preparation reads cached bind groups and replaces them through deferred commands.
Index uploads mutate their pending resource and write GPU buffers without mutating
main-world mesh assets. These accesses do not introduce overlapping mutable queries.
Transparent sorting's additional `Changed<ViewVisibility>` query is read-only;
its `With<SortState>` filter does not read the mutably queried state data. Lighting
invalidation observers only write their version resource. Moving completion polling
before scheduling retains the existing chained deferred-command synchronization.

Inspect negative coordinates, border arrivals, world/session changes, edits during
partial builds, stale collider/light results, camera-inside-solid recovery,
cave openings, transparent sections becoming visible, replaced GPU allocations,
window/UI scaling, arm swings, and graphics/texture-pack changes. Rust compilation
does not validate runtime ECS scheduling, shader compilation, GPU behavior, visual
equivalence or the 144 FPS target. No game, benchmark, test executable or release
build is run as part of verification.

## Block behaviors and memory

Shared registry-indexed profiles classify capabilities once. A compile-time bound
keeps the complete state-behavior table within 768 KiB, independent of view distance.
Climbing reuses the controller's local terrain visit and scratch allocation; it
never searches loaded blocks globally or decodes native property strings per tick.
Interactive and support-dependent cells have sparse capability entities; ordinary
terrain cells remain compact. Independent gameplay columns retain their ECS
inventories while dormant. Coordinate indices contain only column/local-cell
addresses and entity identifiers; native relationships supply traversal.

Player and chest slot arrays share immutable snapshots. Empty default inventories
allocate no slot array. Each double-chest half keeps its own 27 slots; pairing and
UI never copy items. Actions borrow owners once and use statically dispatched slot
operations. Change propagation to persistence runs once after writes. Drag scratch
retains the fixed 100-index upper bound.

Component add hooks deduplicate entity-only FIFO work. Support validation processes
256 candidates, view preparation 64 candidates, and relighting 256 units per frame.
Stale identifiers consume budgets. Dependency state and cursors remain on ECS
components. Relighting retains a full rerun when invalidated during a partial pass.
Support calculations, light samples and moving-lid animation use parallel queries;
commits sharing cells or inventories remain ordered. Six chest meshes and one
material remain shared. Stable lids do not animate or rebuild terrain.

Held-item manifests use an asset loader. Selection, texture waiting, construction
and equip transitions are separate; asset events wake texture work and refresh
cached visuals. Missing textures retain the shared fallback symbol.

I/O workers read compressed container records with terrain; compute workers perform
decompression, decoding and block-entity discovery in relevant sections. The total
in-flight copy limit remains sixteen across both stages. Persistence encoding and
compression remain on workers after sharing snapshots. Independent container
revisions prevent slot edits from invalidating terrain. Per-column storage versions
reject copies racing a commit or rollback without invalidating unrelated columns.
Short metadata locks never span SQL or fsync. These are implementation contracts established by
inspection, not measured performance or memory results.
