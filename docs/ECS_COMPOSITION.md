# Actor composition and fixed-step data flow

This refactor shares spatial processing across controllers and separates accepted
damage from its consumers. It builds on the existing content, fluid, slot and
articulation services. It does not turn every voxel or scalar attribute into an
entity or a component.

## Bevy API baseline

The workspace uses Bevy 0.20. The implementation was checked against the official
[0.20 release notes](https://bevy.org/news/bevy-0-20/), the
[0.19 to 0.20 migration guide](https://bevy.org/learn/migration-guides/0-19-to-0-20/),
the [0.18 to 0.19 migration guide](https://bevy.org/learn/migration-guides/0-18-to-0-19/)
and the installed 0.20 crate documentation and source.

`MessageReader`/`MessageWriter` provide buffered communication. Existing targeted
block-use and exhaustion events remain observer-driven. Required components
establish construction invariants; relationships own entity membership. Bulk
cleanup uses the released 0.20 `despawn_all`/`despawn_all_where` APIs. BSN remains
useful for UI scene composition; restoring a runtime component bundle uses normal
entity spawning and `with_related`.

## Shared motion capability

`hollow_features::motion::FixedPosition` groups the current and previous f64
simulation positions with the last rendered translation. It requires `Transform`.
Both `GameCharacter` and dropped-item motion compose this capability.

`FixedPositionPlugin` provides one interpolation system. Controllers advance the
two samples; gameplay reads `current`; rendering interpolates into `Transform`.
`externally_moved` and `acknowledge` let a controller recognize direct transform
writes without mistaking interpolation for a teleport. Character collision
recovery still resolves the small f32 rounding penetration on restore.

`InterpolationAlpha` stores one presentation sample for all pose consumers. The
Minecraft composition gates `InterpolatePosition` with its simulation condition,
so position, eye and walking interpolation freeze together during a simulation
pause. The fraction no longer needs a per-character render-time write.

Spatial activation, interaction reach, sound positioning, pickups, streaming and
save capture read the shared position. Adding another controller no longer
requires adding an optional component branch to `update_mobile_owners`.
Controllers declare their own membership in the actor schedule; the simulation
plugin no longer enumerates the survival sets. `SimulationPaused` belongs to the
simulation domain rather than the character controller.

The samples are always used together and stay in one component. No transient
motion marker, new identity registry, heap allocation or per-frame structural
change is introduced. Splitting controller state from position adds a component
column and change-tracking metadata. Drops also retain the last rendered sample
to use the shared teleport/interpolation contract. Their velocity units, sleeping
support revisions and reusable parallel collision buffers remain controller-owned.

Presentation updates bypass change detection only for the rendered sample;
simulation position changes still dirty snapshots. Drop persistence explicitly
filters dropped items, so character movement cannot dirty a column's drop record.

## Damage as a capability pipeline

Damage requests remain buffered and ordered. The resolver accesses health,
damage windows, effects, armor and optional equipment, then publishes
`DamageApplied` with the resolved impulse. It no longer borrows character motion,
movement input, nutrition or item-use state.

The character controller applies knockback and stops movement on fatal damage.
Nutrition applies exhaustion. Item use cancels on fatal damage. Each consumer
performs targeted entity lookups and runs in `SurvivalSystems::Reactions`, after
resolution in the same fixed tick. Presentation independently reads the same
message for audio. None of these reactions needs a new component or archetype.

Equipment durability deliberately remains in the ordered damage transaction:
breaking armor must affect the next request in that batch, including knockback
resistance. Deferring durability to a later message consumer would change that
behavior. The remaining equipment dependency has this concrete purpose.

The tradeoff is three small message-reader passes and an impulse in each result,
in exchange for narrower ECS accesses and independently composable consumers.
Requests do not allocate a per-target queue. This is an architectural tradeoff,
not a measured throughput improvement.

## Construction and relationship ownership

`SurvivalSnapshot` is also a native Bevy bundle. Spawn and safe automatic respawn
reuse the same component defaults rather than repeating field extraction and
reset schemas. Respawn keeps its saved point and waits for loaded terrain and a
safe support position. Resetting `FixedPosition` immediately keeps ownership and
save capture at the new position before the controller's next tick.

Skin parts use their existing `ChildOf` relationship to find their rig instead of
storing another entity ID. Their existing six-part and two-layer mesh indices fit
in u8 fields. Mesh construction shares one path for body and armor layers and
relies on `Mesh3d`'s required transform.

Held-item roots own their mesh children; rebuilding calls `despawn_children`
instead of scanning every held part. Chest views use `despawn_related` on their
existing relationship. Loading/save overlays and expired drops use native bulk
despawning, retaining command order while avoiding one queued command per entity.
No additional relationship or parent index is maintained alongside `ChildOf`.

## ECS access review and verification

| Systems | Access review |
| --- | --- |
| Position interpolation | One mutable position/transform query; time is read-only and the presentation fraction is a separate resource. |
| Character/drop simulation | One mutable actor query per controller; terrain queries read different components. Drop workers retain their collision buffers. |
| Spatial ownership and snapshots | Positions are read-only; relationship changes are deferred. Column tracking mutates column metadata, not the position query's components. |
| Damage resolution | One mutable actor query; attacker reads finish before target mutation. No second overlapping mutable actor query is introduced. |
| Damage reactions | Independent message cursors; motion, input, nutrition and item-use queries mutate distinct component types. |
| Rig animation | Rig queries exclude skin parts and part queries exclude rigs. Both read the canonical hierarchy; their mutable transform sets remain disjoint. |
| Respawn | Loader transforms are read through `With<RespawnLoader>`; mutable player transforms use `Without<RespawnLoader>`. |
| Cleanup | Built-in commands perform hierarchy/bulk removal at command application; no competing component query is added. |

No `Changed<T>`/`Added<T>` query conflicts with another parameter mutating `T`
were introduced. This review is necessary because `cargo check` does not detect
runtime query conflicts or schedule cycles.

Verification: `cargo check -p hollow-features`, `cargo check --workspace` and
`cargo check --workspace --all-targets --features dev`. All-target checking only
compiles targets; no tests, benchmarks, game launches or release builds were run.
Rendering and runtime behavior remain unverified. No measured speedup is claimed.

## Production-code accounting and remaining boundaries

Sources were formatted before taking the filesystem reference snapshot. Counting
nonblank, non-comment Rust production lines by the same method gives **9,534
before and 9,525 after: net -9** across the 31 accounted feature/game source files,
including the new shared module. The streaming adapter's position substitution
has zero line delta. Concurrent content/fluid/render changes are excluded from
this refactor's accounting; Git HEAD advanced during the work. Earlier tranche
metrics in `PLUGIN_ARCHITECTURE.md` describe different changes.

The reduction is small; most of the benefit is removal of controller-specific
spatial dispatch, duplicated interpolation, broad damage access and manual
relationship cleanup. Moving schemas or formatting is not counted as reduction.

Column persistence still knows dropped-item records. Inventory-screen sessions,
Minecraft damage rules, effects, terrain work dispatch and sky/audio policies
remain domain-specific. This change does not implement a universal actor engine,
generic machine persistence or a scene-based gameplay interpreter. Those areas
need separate execution-frequency and data-layout reviews before extraction.
