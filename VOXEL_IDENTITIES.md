# Native voxel identities

Terrain stores one copy-on-write `Layer<VoxelState>`. Each state is a validated,
two-byte native state ID. Block identity comes from that state; chunks no longer
retain a second block-kind layer or an optional state representation. Saved IDs
are validated while decoding. Snapshot bytes retain their existing layout because
the registry IDs and the serialized data have not changed.

## Properties and capabilities

`pumpkin-data` owns `BlockId`, `BlockStateId`, `PropertyKey`, `Property`, and the
property value enums. Property reads, iteration, edits, waterlogging, rotation,
and mirroring do not allocate property vectors or boxed trait objects.
Mixed-radix descriptors share value domains across native property families.
Unsupported edits return `InvalidBlockProperty`; gameplay checks optional
capabilities before submitting edits. The native equivalence between wall and
redstone `none` values is preserved at text import boundaries.

Placement, connections, and block behavior share one immutable capability table.
Per-state tables use `BlockStateId::COUNT` (35,723), rather than 65,536 slots.
Water and full-collision flags share one byte per valid state. Native shape offset
limits replace the previous sampled reconstruction of those limits.

Worldgen configuration codecs and structure palettes retain resolved native
states instead of owned block names and property maps. Structure palettes occupy
two bytes per entry. NBT and JSON create names only at their serialization
boundaries. Held-item manifest definitions use numeric item registry IDs.

## Assets and generation

Model import resolves block/model names, variant conditions, and multipart
conditions once. Rotation cache keys use model IDs. The temporary source maps are
discarded after building the runtime tables. The Vanilla model cache remains
version 3: its source schema did not change. Vanilla's `item_frame` and
`glow_item_frame` blockstate files describe entities and are excluded from the
native block catalog, including when reading an existing cache.

Texture pack manifests use version 2 and canonical names such as `grass_block`,
`oak_log`, and `oak_leaves`. Bundled packs and their importer are updated together.
Unknown block names, duplicate resolved identities, incompatible pack versions,
and invalid property edits are rejected explicitly.

The following generators run manually after a native registry update:

```sh
python3 ../worldgen/tools/generate-block-properties.py
python3 scripts/generate_block_relations.py
```

The first generator checks native enum encoding and property-family cardinality.
The second writes a 7,730-byte table of canonical, wall, and material relationships.
The Rust consumer checks its version, length, registry count, and registry-name
fingerprint at compile time. Runtime relationship lookup reads three compact
16-bit columns. Python is not used by the game or invoked by Cargo builds.

## Verification and code accounting

Verification uses `cargo check --all-targets` for the game, worldgen facade,
`pumpkin-world`, and `pumpkin-data`; no game execution, test execution, benchmark,
or release build is part of this refactor. The existing `pumpkin-world` test
targets needed their missing `tempfile` dev dependency declared to compile.

ECS access review: lighting reads `VoxelChunk` and mutates `ChunkVoxels` through
one query; edit systems use one mutable chunk query; support checks mutate
`SupportCheck` while `VoxelWorld` reads terrain; manual interactions only read
block state, terrain, collisions, and actors. Held-item visual building has one
mutable component query and distinct asset resources. No additional overlapping
query access or `Changed<T>`/`Added<T>` filter was introduced.

Counts compare the formatted baseline with the final working trees. Blank lines,
comment-only lines, Rust `cfg(test)` items, dedicated test source files, and Python
docstrings are excluded. Production source under `src`, crate `src` directories,
and the import/generation scripts is counted; generated Rust is shown separately.

| Source | Before | After | Change |
| --- | ---: | ---: | ---: |
| Game Rust | 21,445 | 21,553 | +108 |
| Game Python tooling | 850 | 901 | +51 |
| Worldgen handwritten Rust | 89,371 | 89,395 | +24 |
| Worldgen generated Rust | 1,545,303 | 1,543,189 | -2,114 |
| Worldgen Python tooling | 311 | 380 | +69 |
| Total | 1,657,280 | 1,655,418 | -1,862 |

The total production-code reduction target is met. The game and handwritten Rust
do not shrink individually; their increases are reported above. Existing changes
and concurrent lighting work are preserved in these working-tree comparisons.
Formatting and naming changes are not credited as eliminated implementation.

Structural memory bounds: eliminating a dense block-kind layer removes 8 KiB per
affected section, up to 192 KiB per fully dense column. State-indexed tables have
approximately 45% fewer entries. These are representation-level bounds, not
measured frame-time or throughput claims.
