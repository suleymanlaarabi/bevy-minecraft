# Hollow world generation

Bevy owns generation demand, spatial dependencies, reservations, task admission,
and publication. `VanillaGenerator::new(seed, dimension)` supplies the vanilla
terrain pipeline for the Overworld, Nether, and End. Sessions select Fast or
Vanilla terrain quality. `GenerationSession::probe_terrain` remains available for
terrain-only spawn searches.

Structure templates, jigsaw pools, and processor lists come from embedded static
Pumpkin data. Runtime datapack registration, external template loading, flat and
plugin generators, and the generic Pumpkin codec framework have been removed.
NBT remains in use for block metadata, embedded structure metadata, and generated
chunk persistence. Inventory components, crafting recipes, and equipment data
remain available to the game.

## Finalization and persistence

The generation stages end with `Features`, `Lighting`, and `Full`. `Full` waits
for the center and all eight adjacent chunks to reach `Lighting`. Its own read
and write reservations cover only the center; the lighting stage still reserves
its working neighborhood. Worker stage chaining also checks dependency stages
inside its reserved cache before finalizing.

Finalization discards pending structure entities, preserving the previous empty
entity-hook behavior. Block entities remain part of generated chunk metadata.
The spatial graph, shared voxel task budget, cancellation, and checkpoint flow
are retained.

Current persistence identities:

- Generator: `hollow-bevy-2`.
- Generated chunk payload: `HGEN` version 2.
- Game world format: 4.

Previous identities are rejected explicitly. There are no compatibility readers
or migrations. Block and item registry identifiers are unchanged.

## Refactor measurements

The baseline was recorded after `cargo fmt --all`. Both measurements use the
same lexical line counter over production Rust in `src` and `crates/worldgen`,
including crate build scripts and generated Rust. Blank lines, comment-only
lines, test-only items, and test/example/benchmark directories are excluded.
Formatting and naming changes are not counted as reductions.

| Production Rust | Before | After | Removed |
| --- | ---: | ---: | ---: |
| Manual | 99,286 | 94,942 | 4,344 |
| Generated | 1,436,114 | 1,425,168 | 10,946 |
| Total | 1,535,400 | 1,520,110 | 15,290 |

| Change, measured in implementation order | Production lines removed |
| --- | ---: |
| Remove `pumpkin-codecs`, `nbt_ops`, and utility codec implementations | 2,850 |
| Remove dynamic structure registries and external template parsers | 912 |
| Use the vanilla generator directly and remove custom configuration paths | 453 |
| Remove `Spawn` and preserve the lighting barrier in `Full` | 68 |
| Remove unused Bedrock, screen, statistic, and test-instance modules | 10,966 |
| Remove residual forwarding parameters, imports, and loader error plumbing | 41 |

All five planned changes reduce production code; no reduction target was missed.
Generated data removal includes both feature declarations and the actual Rust
modules, rather than only disabling their compilation.

## Verification

`cargo check --workspace` passed after the final code changes, without warnings.
The game, tests, benchmarks, and release builds were not run.

The affected runtime systems were also reviewed for ECS access conflicts:
`update_demands` accesses distinct session, graph, and deadline resources;
`collect` has one mutable task query and distinct graph/message resources;
`schedule` has a retention-marker query and distinct graph, budget, and deadline
resources. No overlapping component queries or change-detection filters were
introduced.

Code inspection covered the dependency expansion and worker chaining that guard
finalization, neighbor-light write reservations, unchanged terrain calculations,
terrain probes, cancellation and checkpoint ownership, and the rejection of old
world metadata and generated payload headers. Runtime behavior was not exercised.
