# Player survival

The survival implementation uses the existing 20 Hz character schedule. `Living`
requires health, timed effects, damage state, armor statistics, and environmental
conditions. Only players require nutrition, item use, and a respawn point. Future
creatures can use damage and effects without carrying player inventory or hunger.

## Simulation and ownership

`SurvivalSystems` orders preparation before `CharacterSystems`, then environmental
sampling, consumption, metabolism, and damage resolution. World loading, saving,
and developer simulation suspension gate these fixed systems. Character outcomes
record actual travel, jumps, sprinting, and climbing; hunger costs use those
outcomes rather than render-frame input. Successful mining commits contribute
exhaustion through a targeted observer.

Immutable item profiles resolve food, use duration, remainders, slowdown, and
armor attributes once from the bundled registry. Equipment statistics update on
inventory changes. Independent locomotion updates and damage-window advancement
use parallel queries. Environmental sampling stays within each character's bounds;
teleport and respawn searches have explicit limits and use loaded voxel data.

All damage enters through `DamageRequest`. The resolver handles invulnerability
windows, stronger overlapping hits, armor/toughness, resistance, absorption, armor
wear, exhaustion, velocity impulses, feedback, and death. Attackers can supply
their entity and impulse; strength and weakness apply to direct attacks. Future
mob/projectile systems should emit these requests rather than mutate health.

Fall damage, drowning, suffocation, lava, fire, cactus, berry bushes, hot floors,
lit campfires, poison, wither, starvation, and the void share this pipeline.
The default starvation floor is one health point, matching the Normal profile.
Natural regeneration spends nutrition through the exhaustion system.

## Consumption and effects

Hold the secondary mouse button to eat or drink. Usable targeted blocks take
priority. The selected item is used first, with offhand consumption available
when the main hand does not perform a block action. Opening inventory/developer
UI, releasing use, changing the selected item, leaving the session, or dying
cancels the action. Picking up more of the same stack preserves its progress.

Completion validates the stack and commits its removal, nutrition, effects, and
container remainder while borrowing the same inventory owner. Remainders fill
the emptied slot, enter inventory, or become dropped items. No partial use is
saved. Potion and stew identities participate in stacking and persistence.

Registry food actions support applying/removing/clearing effects and bounded safe
teleportation. The active-effect mechanism covers the effects carried by current
foods plus regeneration, poison, wither, absorption, resistance, fire resistance,
hunger, nausea, blindness, speed/slowness, jump boost, slow falling, water breathing,
instant health/damage, saturation, health boost, strength/weakness, haste/fatigue,
and night vision. Stronger effects take precedence; equal-strength applications
refresh duration. This is intentionally a single active entry per effect rather
than Vanilla's hidden weaker-effect queue.

## UI, assets, and respawn

`scripts/import_player_assets.py` imports the survival atlas and the shared vanilla
sound bank. `scripts/import_sounds.py` can rebuild only the sounds from the local
Minecraft 26.3 client. See [gameplay audio](AUDIO.md) for coverage and ownership. HUD entities
are persistent, reuse the atlas, and change only affected image rectangles, text,
or visibility. Hearts include half values, damage blinking, poison, wither, and
absorption. Food, armor, air, active-effect durations, and inventory nutrition
details use the same presentation path. The crosshair also uses its Minecraft
sprite.

Noto Sans replaces the default text font, with Noto Sans Symbols 2 as an explicit
fallback for UI symbols. Both fonts and their OFL licenses are bundled. Health
and food icons are textures. Damage camera motion and survival sound volume are
exposed in settings.

Death stops player actions and movement. Click Respawn or press Enter to return;
inventory is retained. Respawn moves the dead player's existing camera toward
the saved spawn, waits for loaded terrain and meshes, finds collision-free
standing space, and then restores health/nutrition. The camera remains the single
`VoxelViewer` required by the streaming pipeline. The button activates on press
and displays `Loading spawn…` while waiting. Save/error screens hide survival
overlays so their controls remain accessible.

Saved data contains current statistics, effect durations, damage windows,
environmental conditions, spawn coordinates, and consumable identities directly.
Saved data and generated caches are disposable between updates. No format-version
fields/checks, migrations, or old-data preservation paths are maintained.

## Verification and limits

`cargo check` passes. The game was not launched by the agent. A user-reported
respawn failure exposed a second streaming viewer; the corrected path moves the
existing camera and creates no additional viewer. Its runtime result, visual
layout, sound playback, and animation feel still require user confirmation.

Affected systems were inspected for ECS access conflicts. Damage resolution uses
one actor query, including sequential attacker reads before target mutation.
Consumption has one mutable owner query; voxel data is read separately. Respawn
loader and player transform queries are explicitly disjoint. Camera follow keeps
its existing player/camera disjoint filters. HUD mutation uses one view query;
font styling has one mutable font-component query. No new overlapping change
filter/mutable-component query pairs were introduced.

Freezing, experience, beds, full death consequences, brewing/food acquisition,
and creature/projectile combat remain separate roadmap work. No benchmark or
performance claim is made.

## Production-code accounting

Both snapshots were formatted identically before counting Rust production lines
in application `src/`, excluding blanks and comment-only lines, with
`scripts/measure_rust_production.py`.

| Scope | Before | After | Change |
| --- | ---: | ---: | ---: |
| Application Rust | 21,820 | 24,476 | +2,656 |
| Player spawn, including replacement of the custom crosshair | 215 | 203 | -12 |
| World save model, including removal of version metadata/checks | 262 | 238 | -24 |

This feature adds production code overall: a total-code-reduction target is not
met. The obsolete crosshair construction and save compatibility metadata were
removed rather than retained alongside replacements. Counting does not attribute
formatting or naming changes to code reduction.
