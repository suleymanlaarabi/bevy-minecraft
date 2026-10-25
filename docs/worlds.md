# Hollow worlds — format 1

`WorldPlugin` composes the catalog, asynchronous opening, sessions, and BSN/Feathers
screens. Persistence uses explicit data models rather than serializing the ECS
`World`, entity identifiers, or GPU assets.

## Session lifecycle

`GameState::WorldSelect` and `WorldCreate` use the menu UI camera. A worker opens
the database and prepares a Pumpkin generator before entering `GameState::Game`.
Each session owns its generator and immutable generation settings.

`WorldSessionPhase` provides `Loading`, `Playing`, `Saving`, and `SaveError`.
Streaming runs during loading and gameplay; input and simulation run only during
`Playing`. Physics and virtual time pause during loading, final saving, and
errors. A temporary viewer loads the spawn area using the existing mesh and
collision budgets before spawning the player. Developer autoplay stays ephemeral.

Session entities remain attached to `GameState::Game` until the final commit
succeeds. Closing the inventory returns its cursor stack and crafting inputs to storage,
or creates drops if storage is full. Checkpoints write 45 slots: the original 36
storage slots, four armor slots, offhand, and four crafting inputs. Loaders also
accept the older 36-slot layout and initialize the extra slots as empty. The
crafting result is derived from the inputs and is never saved as an item. Item
stacks retain registry names and vanilla stack limits; armor slots are validated
on load. The checkpoint is captured in `Last`, after
deferred commands, while simulation is frozen. Any existing autosave finishes
first; a new final checkpoint follows. Only its acknowledgement allows returning
to the world list or emitting `AppExit`. Native window closing is disabled so the
error UI remains accessible if saving fails.

Chunk tasks belong to session entities, preventing old results from reaching a
new session. Spawn searches can be cancelled. Lighting contexts and generators
are destroyed off the ECS thread. The world lock stays held until storage closes;
reopening briefly waits for this background shutdown to finish.

## Container and transactions

Each UUID directory contains `world.sqlite3`, any SQLite WAL files, and a
`world.lock` held for the session. SQLite uses WAL and `synchronous=FULL`. One
protected connection serializes batched access. File access, SQL, compression,
and synchronization all happen outside ECS systems.

The container has three tables:

| Table | Contents |
| --- | --- |
| `records` | Metadata and the latest checkpoint, encoded as JSON |
| `generated` | Compact Pumpkin PUMP chunks, indexed by X/Z and generation stage |
| `edited` | Complete snapshots of edited or relit chunks, indexed by X/Z |

Chunk blobs contain a CRC32 followed by size-prefixed LZ4 data. Decoding bounds
the decompressed size, verifies the checksum, and rejects truncated snapshots.
Hollow's `HCHK` version 1 codec stores uniform/dense native-state and light
sections followed by edited cell indices. Rendering and collision data are rebuilt.

The generator reads PUMP chunks through `worldgen`'s `ChunkStorage` interface.
These preserve the biomes, stages, heightmaps, and light required by Pumpkin's
scheduler. Keeping generated terrain avoids recreating potentially different
features. A lower-stage record never replaces a higher-stage record. Edited
Hollow snapshots take precedence in gameplay streaming and are ready without
waiting for Pumpkin; dynamic lighting keeps its existing separate context. The
positions of saved snapshots stay in memory, so streaming queries SQLite only for
edited chunks. Reads use a separate WAL connection and never wait on a checkpoint
transaction.

One checkpoint transaction stores changed chunk snapshots, the player, inventory,
cursor stack, drops, and pending edits targeting absent chunks. Drops preserve
velocity, remaining lifetime, and pickup delay. Time outside the game does not
expire them. Pumpkin's generated bases are synchronized before committing the
checkpoint.

Snapshots share `Arc` sections with gameplay. Encoding does not copy whole
columns on the main thread. Disk revisions are independent of mesh revisions:
an acknowledgement validates only its captured revision, leaving newer edits
unsaved. Unloaded dirty chunks stay in memory until their commit succeeds, then
are released. Only one checkpoint task is in flight. Streaming copies and the
Pumpkin write queue are bounded.

## Compatibility and errors

The format, Hollow generator revision, Pumpkin commit, and Minecraft version are
checked before gameplay starts. Incompatible registry or generator changes must
increment `GENERATOR_VERSION`; incompatible container changes must increment
`FORMAT_VERSION`. Version 1 performs no implicit migration.

A missing chunk permits generation. A read error forbids regeneration and stops
that generator instance. Failed writes retain unacknowledged snapshots, including
evicted Pumpkin chunks. The error screen allows retrying a save or returning to
gameplay. Retrying a failed autosave resumes play; retrying a final save leaves
once it succeeds. During loading, it offers reopening. Leaving without saving
keeps the previous checkpoint; closing the window over the error screen does the
same. Leaving before the player spawns never saves. Saved player poses are
clamped to loadable values, so a fall below the world cannot make it unopenable.

## Verification

Regression cases cover native block properties, uniform/dense sections, truncated
snapshots, checksums, invalid sizes, signed seeds, incompatible versions, and
invalid stacks, transaction rollback, and preservation of higher generation
stages. `cargo check --all-targets` compiles these tests without executing
them. Checks with `dev` enabled and checks of the neighboring `worldgen` crate
also verify compilation. ECS access and transition ordering require explicit code
review. Do not launch the game, run tests or benchmarks, or build in release mode.
