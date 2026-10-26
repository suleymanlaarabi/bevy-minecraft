# Gathering and crafting

This implements the first four M2 tasks. Cooking, experience, enchantments, tool
transformations on right-click, crop growth, recipe search, and the complete
resource-progression audit remain outside this slice.

## Rules

All gathering tools in the embedded registry are supported: pickaxes, axes,
shovels, hoes, and shears, including the copper tier. Registry tool rules determine
speed, harvest eligibility, durability, and wear per positive-hardness block.
Items with durability have stack size one. Moving, splitting, merging, collecting,
saving, and restoring a stack preserve its damage.

Mining uses the actual state's hardness in seconds divided by tool speed. A tool
that cannot harvest a required block multiplies duration by three and produces no
block loot. Submerged eyes and lack of ground support each multiply duration by
five, cumulatively. Zero-hardness blocks break immediately without wear; negative
hardness, air, and fluids cannot be mined. Changing the target, state, selected
slot, or held stack resets progress; environmental changes affect subsequent
progress. Cracks display the same normalized progress used by mining.

Loot is intentionally simpler than vanilla:

- Stone and deepslate yield their cobbled forms; ores yield one corresponding raw
  material, gem, or resource. Tool requirements still apply.
- Leaves yield their block with shears, otherwise a corresponding sapling or
  propagule with 5% probability. Common shear-only plants and cobwebs require
  shears to yield their block.
- Gravel yields flint with 10% probability, otherwise gravel.
- The supported age-based crops are wheat, beetroots, carrots, potatoes, nether
  wart, cocoa, torchflower crops, and pitcher crops. Immature crops yield one
  planting item. Mature crops yield one product plus one distinct planting item,
  or two products when the product is also the planting item.
- Paired blocks yield one logical loot result from either half. Double slabs yield
  two slab items. Common glass and ice blocks and budding amethyst yield nothing.
- Chest removal ejects only that chest half's contents through the existing block
  removal observer. Compatible contents are consolidated before spawning drops.

Personal crafting uses four slots; a crafting table exposes nine. Both use the
same shaped/shapeless recipes, ingredient tags, horizontal mirrors, and translated
patterns. Shift-click repeats the original recipe and stops when ingredients,
output space, or remainder space run out. It never switches to another recipe
when the partially consumed grid happens to match one.

Remainders include buckets from milk/water/lava buckets and glass bottles from
honey bottles/dragon breath. A remainder occupies its freed ingredient slot, then
the main inventory if the ingredient stack remains. Each recipe is atomic: failed
placement leaves ingredients, output, and remainders unchanged.

## Ownership and scheduling

The terrain remains compact voxel data. Only interactive/capability-bearing
blocks become ECS entities. A crafting table has a marker and viewer relationship;
its temporary grid belongs to the player, not to the block or UI.

A workbench session is checked before inventory actions and closed if its block
is removed, unloaded, or more than five blocks from the player. Closing returns
the cursor and all nine ingredient slots to the inventory and drops only overflow.
Actions and dragging are cleared. The grid and cursor are included in snapshots;
restoring returns them through the same ownership path without restoring a session
entity reference.

Checked voxel transactions revalidate every block and the selected stack before
consumption or wear. Loot randomness is advanced only for accepted loot rolls.
The existing PostUpdate sequence remains player interaction, voxel publication,
topology, session validation, inventory actions, derived crafting, and presentation.
Deferred edits and chest-content ejection finish before the Last checkpoint capture.
Container snapshots retain their immutable copy-on-write ownership.

Save format **6** requires stack damage and crafting-grid records. Earlier save
formats are rejected with an explicit message; no migrations or compatibility
paths are provided.

## Performance and static ECS review

- Tool profiles and block results are compiled once. A compact item-to-profile
  table avoids a matrix for every registered item. No registry/tag resolution or
  allocation occurs while mining progress advances.
- Loot rules and item properties are prepared before play. Recipe ingredients
  share interned bitsets; tag expansion occurs once per tag. Shaped candidates
  are indexed by dimensions and shapeless candidates by occupied-slot count.
- Shapeless matching uses bounded bipartite augmenting paths over nine slots,
  including overlapping tags. Matching and crafting use fixed stack buffers.
  Repeated crafting commits once and does not clone a heap-backed inventory for
  each output.
- Recipe previews read only grid changes. Slot presentation caches the last shown
  stack, including damage, and inspects only visible chest inventories. Existing
  parallel derived-crafting and support-work systems remain parallel.
- Mining queries access separate target, progress, swing, inventory, and character
  components. The edit observer performs chunk validation and mutation through one
  mutable chunk query, separate from its inventory query.
- Workbench opening mutates only the actor's grid; session validation and its
  nested SystemParams read block, terrain, and character components. Inventory
  actions mutate player inventories, player grids, and container inventories of
  distinct component types.
- Presentation's icon and durability queries both mutate `Visibility`; durability
  entities explicitly exclude `SlotIcon`, matching their separate UI nodes.
  `Added<SlotOwner>` reads the same component that the view query reads; neither
  query mutates it. Other presentation writes target distinct component types.
- Snapshot and binding queries remain read-only. Returning items mutates inventory
  and grid in one player query. No system introduces a conflicting `Changed` or
  `Added` filter beside a mutable access to the same component.

Static scenarios reviewed: insufficient/wrong tools, water plus airborne penalties,
zero/negative hardness, final durability use, stale voxel/stack proposals, paired
blocks and chest halves, full destinations, overlapping tags, shifted/mirrored
patterns, repeated-recipe exhaustion, remainder placement, session invalidation,
and save/restore with an occupied grid or damaged tool.

Verification is limited to `cargo check` and `cargo check --features dev`. The game,
tests, and benchmarks are not run; runtime behavior and speed are not claimed to
have been measured.

## Production-line accounting

Counts exclude blank lines, lexical Rust comments, test-only items, and test
directories. Both columns use the same lexer and the formatted pre-change
baseline. The initial rough prefix counter was replaced because it incorrectly
excluded production statements starting with a dereference operator.

| Production Rust | Before | After | Change |
| --- | ---: | ---: | ---: |
| Workspace production Rust | 1,518,572 | 1,519,494 | +922 |
| Application src/ | 20,985 | 21,910 | +925 |
| Shared block identities | 202 | 199 | -3 |

The shared block-identity cleanup removes obsolete mining-duration overrides
and reduces production code by three lines. The feature as a whole increases
production code; the global code-reduction target is **not met**. Formatting,
renaming, and changes to comments or spacing are not credited as reductions.

