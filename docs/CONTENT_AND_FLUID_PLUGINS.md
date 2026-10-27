# Content and fluid plugins

## Implemented

`hollow-core::content` owns the content builder and frozen, shared catalog. Plugins
register namespaced blocks, mixed-radix property domains, states, cuboid shapes
and resource references before startup. Native Minecraft IDs are imported into
that same domain; custom IDs append to it. Both block and state identities are
32 bits. Runtime property edits use `PropertyId` and numeric values with direct
state arithmetic. Names resolve at registration/import boundaries. The frozen
name index is sorted numeric IDs, without a retained second copy of every name.

`ContentPlugin` freezes registrations in `PreStartup`. `RegisteredContent` is the
Bevy resource. `MinecraftContentPlugin` imports native definitions and binds the
value-cell generation adapter to this process-wide immutable catalog. The core
catalog itself is owned and can be used independently of Minecraft. Cells retain
only their state ID; definitions and shapes are not allocated per cell.

Generic reads, edits, terrain saves, ray selection, meshing and snapshot lighting
accept the appended state domain. Native generation remains a native-data provider;
it does not define the upper bound of gameplay state IDs. Ordinary custom blocks
can use an explicitly registered `BlockVisual` with six independent face resources.
Other model, UI and audio resources remain plugin-owned; arbitrary PNG resources
are not implicitly interpreted as terrain textures.

`hollow-engine::fluids` supplies propagation, falling/standing amounts, configurable
source renewal, slope search, collision passage, atomic prepared edits and contact
reactions. `FluidWorld` supplies terrain sampling, writable regions, vertical bounds
and optional position-dependent collision offsets. The Minecraft scheduling adapter
only handles budgets, loaded-column ownership, suspended work and commits. No
water/lava family enum or native block switch remains in the shared algorithm.

`WaterPlugin` and `LavaPlugin` register profiles and native state associations using
this API. Their cross-fluid reactions belong to `MinecraftFluidReactionsPlugin`;
neither provider requires the other. Native waterlogging uses the same container
registration API as custom containers. A state can accept multiple fluid families;
container rules are contiguous, sorted and searched by numeric ID.

Fluid profiles also supply contact effects, movement coefficients and optional fog
and surface rendering. `FluidContactState` publishes the current medium, eye medium
and depth to ordinary ECS queries. Plugins can implement arbitrary actor effects
without adding fields or cases to the common contact system. Damage resource names
resolve once through the Minecraft damage provider; runtime contacts carry IDs.

## Extension paths

- `FluidRegistrations::register_simple` declares a free fluid's amount/falling states.
  `register` accepts arbitrary existing state families instead.
- `FluidProfile::before_step` accepts an owned behavior or capturing closure with
  typed plugin configuration. It can sample the environment, stage checked edits
  and continue or replace standard propagation. The hook allocates once per
  registered behavior and dispatches dynamically only when explicitly installed.
- `automatic = false` lets a plugin own scheduling through Bevy systems and still
  call the shared `step` API. Failed steps clear staged edits; callers commit only
  successful batches through the normal terrain transaction API.
- `render = None` opts out of the built-in surface renderer; `fog = None` opts out
  of its camera fog. A plugin can use its own Bevy material/renderer. Shared
  `fluids::surface` provides allocation-free quads and cuboid clipping separately
  from renderer materials and native assets.
- `BlockRegistrations` still composes arbitrary bundles and structural callbacks
  on registered blocks. Additional capabilities can live in plugin-owned tables
  indexed by the existing IDs or in sparse ECS components.

The shipped `crates/minecraft/examples/custom_content.rs` composes the actual game
with an independent resin plugin. It declares a fluid, two PNG resources, a solid
block with a `polished` property, and a capturing behavior that cures resin above
that block. F8 places resin; F9 places its solid block. It is not installed in the
default executable. The example was compiled, not launched. It does not implement
the separately planned machine/inventory/UI showcase.

## Performance and access review

Registration does the string work, property compilation, shape deduplication and
resource resolution. Tick, collision and surface loops use IDs, slices and shared
profiles. Column sampling retains the prefetched 3-by-3 neighborhood and falls back
to sparse world indexing for larger custom slope searches. Reusable scratch buffers,
a fixed 512-entry passage cache and a directly indexed visited bitset bound common
flow work without per-tick temporary hash sets. Duplicate container validation is
performed after sorting rather than scanning all prior rules at every insertion.

The shared physical catalog adds immutable metadata alongside the generator's native
data. It replaces the separate native fluid/flag tables but is not a demonstrated
memory reduction. Mesher classification grows from four to eight bytes per state
to represent the full 16-bit fluid domain. Padded mesh samples therefore also grow.
Terrain cell state storage remains four bytes, as in the previous tranche.
Catalog `Arc`s are shared at resource/task boundaries; cells do not clone them.
Optional visuals allocate an index only when registered, and custom face slots
cover the custom block suffix rather than allocating another native-block table.

Custom PNGs are decoded and packed into a shared image array at initialization or
pack changes, with one transient source image at a time. Native-only composition
keeps asynchronous asset loading. The existing renderer has a 2,048-layer physical
texture limit and a 14-bit greedy-surface key; larger fluid IDs use explicit surfaces.
Custom renderers can consume the shared geometry API instead. These are concrete
renderer limits, not a claim that a fixed profile schema models every possible mod.

Scheduled flow has one mutable terrain query; its column simulation query mutates a
different component. Read/stage passes end before commits. Character physics has one
query mutating motion, character state, activity and `FluidContactState`; its terrain
query only reads chunks. `CharacterBody` requires the contact component, and writes
use `set_if_neq`. Hazard sampling only reads terrain and character state and mutates
hazards. Asset preparation mutates image/material asset resources with distinct types.
Registration/finalization systems access distinct registration resources and run
before dependent startup work. No overlapping mutable component queries or
`Changed<T>` versus `&mut T` conflicts were introduced.

Verification uses only `cargo check`; no application/example launch, test, benchmark
or release build was performed. Compilation does not establish visual correctness
or detect Bevy runtime access conflicts; the latter were reviewed from source.

## Remaining boundaries

The new authoring interface is Rust plugin registration, not a RON importer or hot
code loading. Full model/animation authoring, custom item catalogs, UI sessions,
machine persistence and the general scheduled-work API remain separate tasks.
The game still uses its default column layout; the standalone fluid world interface
can provide different vertical bounds. The standard fluid primitive represents one
medium per cell state; richer mixtures can own their simulation through the exposed
hooks/ECS APIs rather than forcing another built-in family switch.

## Code accounting

The same production Rust line counter and comment/blank-line filtering are used
before and after formatting. Unchanged generated/vendor sources cancel from the
delta. Formatting and moves are not counted as reductions. Final counts: **1,526,450 before; 1,528,416 after; net +1,966 lines**.
The mandatory net production-code reduction target is **not met**; the new
registration and shared-service APIs outweigh removal of the specialized paths.

This count is a whole-workspace snapshot. Independent fixed-step motion changes
appeared concurrently in the feature/game packages; their lines are included and
are not attributed solely to the content/fluid implementation. Those changes were
left in place. No isolated code-reduction claim is made.
