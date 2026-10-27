# Composable capabilities: item use, modal input and motion

This tranche changes the running Minecraft application and its shared feature
crate. It continues the [actor composition work](ECS_COMPOSITION.md). The wider
[plugin architecture](PLUGIN_ARCHITECTURE.md) remains incremental.

## Verified Bevy baseline

The workspace already uses released Bevy 0.20. Before editing, the APIs were
checked against the official [0.20 release notes](https://bevy.org/news/bevy-0-20/),
[0.19 to 0.20 migration guide](https://bevy.org/learn/migration-guides/0-19-to-0-20/),
[0.18 to 0.19 migration guide](https://bevy.org/learn/migration-guides/0-18-to-0-19/),
and 0.20 documentation for [components](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/component/trait.Component.html),
[messages](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/message/trait.Message.html)
and [relationships](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/relationship/trait.Relationship.html).
The installed 0.20 source was also inspected.

Messages buffer communication between scheduled systems. Events and observers
handle immediate reactions when triggered commands are applied. Required
components provide construction invariants. Relationships maintain entity links
through Bevy's hooks. Existing BSN scenes still author UI trees; gameplay does not
acquire a scene interpreter or an additional hook/index framework.

## Item use separates controllers, transactions and consequences

`SurvivalPlugin` installs `ItemUsePlugin`, which owns item-use scheduling,
completion messages and cancellation on world suspension or fatal damage.
Controllers supply `UseInput(Some(slot))`; clearing it cancels use. The local
player handles mouse release, hand selection and modal input blocking. Opening
local UI cancels only the player's use, rather than every consuming actor.

`UsingItem` requires `UseInput`, `Health` and `ActiveEffects`. An actor also
composes its inventory, fixed position and simulation membership. Nutrition is
optional: food needs this capability, while potion use does not require it.
Neither hotbar selection nor character movement, pose, hazards or a render
transform is required by the timed consumption query itself.

The fixed-step flow is:

1. Validate the intent and advance the existing inline use state.
2. Consume one item and insert/drop its remainder within one inventory borrow.
3. Publish `ItemConsumed { entity, stack }` with the consumed unit and contents.
4. Apply nutrition and effects after the transaction, in the same fixed tick.
   Only teleport actions look up character/pose/hazard data.
5. Continue through metabolism, damage and damage reactions.

Other systems can read `ItemConsumed` after `SurvivalSystems::Consume` without
joining the transaction's query. This message is a completion fact, not an
alternative request to consume inventory. Compiled item profiles remain shared;
no entity stores copied definitions or external names.

Effect action order and random draws remain together in the completion consumer.
Health change detection compares the small inline value before marking it dirty;
passive effects no longer cause a false health change through this path. Item-use
sounds trigger the existing audio event directly, removing the redundant
`SoundCue` message and translation pass.

The tradeoff is one additional scheduled message-reader pass and a small buffered
completion record. The frequent query drops from 13 fields to seven, and mutations
of effects, nutrition and teleport state happen only on completion. No per-tick
marker insertion, new effect archetype, string lookup or per-actor queue is added.
Minecraft effect semantics and the inventory layout remain domain-owned.

## Inventory actions use Bevy's buffer and one batch snapshot

Picking observers and keyboard input write `InventoryAction` messages. The sole
transaction consumer drains `Messages<InventoryAction>` in order. Draining is
intentional here: these are exclusive local-session commands, and an invalid
session must discard the whole batch. Independent completion facts use readers.
The custom `VecDeque` resource and its wrapper are removed; session teardown
still clears pending commands explicitly.

The ECS system validates the session and borrows its owners once. `ScreenSlots`
dispatches actions over the existing statically dispatched slot operations and
shares the crafting-output gate. Inventory and crafting-grid snapshots are now
compared once per batch, rather than once per action. Intermediate mutations are
still visible to subsequent actions; no other system could observe those
intermediate values inside the previous system invocation either.

Buffers retain capacity. Bevy's two buffers and message IDs add metadata compared
with the old queue; the UI batches are small, and this buys maintained buffer and
lifecycle handling rather than a claimed queue-speed improvement. Bevy's normal
message retention now bounds unprocessed input; disabling the consumer for multiple frames does not retain an indefinite
command backlog. Crafting, paired-chest ownership and persistence stay unchanged.
This does not introduce a universal machine-screen schema.

## Modal roots compose cursor behavior

`hollow_features::cursor::CursorCapturePlugin` is the only runtime writer of
window cursor capture. Live entities with `FreeCursor` release capture. With no
such entity, capture resumes. The initial window configuration remains in app
setup.

Menus, inventory overlays, developer panels and save/error overlays compose this
capability. Their domains no longer write cursor options, and respawn only closes
the dead player's inventory. For example, another modal can declare:

```rust
#[derive(Component, Default)]
#[require(hollow_features::cursor::FreeCursor)]
struct TradingDialog;
```

Closing a modal despawns its root. If a future screen keeps a dormant root alive,
it must remove `FreeCursor` explicitly; hiding a node or removing its requiring
component does not remove required components automatically.

The system runs in `PreUpdate`, after state transitions and before fixed-loop
input consumers. It checks query emptiness and the few windows, and writes only
when visibility or grab mode differs. This replaces duplicated policies in five
domains without a cached modal count or another state resource. The policy is
application-wide, matching the current single-window game. It does not implement
per-window arbitration. The zero-sized capability is added at screen creation;
there are no structural changes per frame.

## Following and travel share existing data

`hollow_features::spatial::FollowPlugin` replaces the Minecraft spatial module.
`Follow` requires `Transform` and `FollowOffset`, making incomplete follower
construction unnecessary. One follower iteration performs one target lookup;
there is no optional-offset branch or reverse-list traversal in the system.
Bevy still maintains `FollowedBy` for the relationship contract and authoring API.
Targets remain roots, as before; this pass does not resolve follower chains.
The current camera has one follower lookup. With many followers sharing a target,
this approach repeats that target lookup instead of reusing a root-loop sample;
no measured throughput improvement is claimed.

The mutable offset stays separate from the relationship. Updating an animated
offset every frame must not replace the relationship and rerun its hooks. The
existing camera already had both components, so this adds no camera archetype or
per-frame structural operation. This is a shared service with enforced defaults,
not a compatibility re-export of the deleted game module.

Character travel selects tick-local acceleration, drag and gravity, then uses one
acceleration/collision/drag path for flight, fluid and ground motion. Air gravity
still precedes drag; fluid gravity and shore recovery still follow it. Ground
coefficients use the pre-collision grounded state. Walking animation reuses the
already computed displacement length. Medium transitions do not insert/remove
components, allocate profiles or split character archetypes. The controller's
large pose/environment preparation remains domain-specific.

## Access and scheduling review

| Affected systems | Static review |
| --- | --- |
| Consumption timer | One mutable actor query. Terrain reads only chunk storage/index data. Health and optional nutrition are read-only. |
| Consumption consequences | Health/effects/nutrition and teleport queries access distinct component types. Terrain is read-only. The chain orders the message writer before its reader. |
| Player input | One player query and one camera query; their accessed component types do not overlap mutably. Existing modal run conditions and cancellation remain. |
| Inventory transaction | Player inventory/grid and container slot components are distinct types. Topology/workbench queries are read-only. The message buffer has one exclusive consumer. |
| Cursor capture | Modal query accesses only the marker; the other query mutates cursor options. No other runtime cursor writer remains. |
| Following | Target transforms use `Without<Follow>`; the mutable follower query requires `Follow`. These sets are disjoint. |
| Character physics | The existing single actor query is unchanged; force selection introduces no ECS access. |
| Save and respawn | Cursor queries were removed; save settlement still reads inventory state. Existing respawn loader/player transform disjointness is preserved. |

No overlapping `Changed<T>`/`Added<T>` filter and mutable query was introduced.
`cargo check` cannot establish runtime ECS access validity or schedule behavior;
the table records the explicit source review.

Verification passed with `cargo check --locked --offline --workspace` and
`cargo check --locked --offline --workspace --all-targets --features dev`.
No tests, benchmarks, release builds or game launches were run. Runtime visuals,
input feel and measured performance remain unverified.

## Code accounting and remaining boundaries

Sources were formatted before refactoring and after edits. Across every affected
production Rust file, including both new feature modules and the deleted spatial
module, nonblank lines containing code decreased from **6,112 to 6,070: net -42**.
Comment-only lines and example/test/benchmark directories are excluded. Other
production sources are unchanged, so this is also the total production-code delta.
Excluding import declarations on both sides gives **5,867 to 5,834: net -33**;
import regrouping alone does not explain the reduction. Moving the spatial code
is accounted for on both sides, not counted as deletion alone.

Removed work includes per-action inventory snapshots, repeated travel/collision
branches, the extra sound-message projection and competing cursor policies.
These are inspected reductions in work and coupling, not measured speedups.

The placement rule dispatcher, world checkpoint capture, container persistence,
and manual-opening door/gate branches remain large or specialized. The player
interaction pass also still rewrites its use intent during arbitration. Further
extraction needs its own locality and scheduling review; adding a generic event
or component for every scalar operation would not resolve those boundaries.
