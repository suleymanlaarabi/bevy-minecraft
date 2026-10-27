# Gameplay audio

Run `python scripts/import_sounds.py` after installing Minecraft 26.3 locally.
The importer reads the official block registry headlessly and copies sounds from
`~/.minecraft/assets`. It does not start Minecraft or the renderer. The player
asset importer also rebuilds this bank. Generated Mojang audio lives in the
ignored `assets/audio/vanilla` directory.

The shared bank contains each block's break, step, placement, hit and landing
sound, volume and pitch, plus the official door, trapdoor and fence gate events.
Event references resolve to weighted file variants with their own volume and
pitch. Silent vanilla events remain silent. Imported assets are required; a
missing bank logs an actionable warning rather than substituting unrelated sounds.

`GameAudioPlugin` loads and indexes the bank once. Gameplay emits `Sound` events;
Bevy owns playback, spatial listeners and automatic source cleanup. World sounds
use the player camera listener and distance culling. The settings screen's
**Sound Volume** controls gameplay and button audio, including survival feedback.

Current hooks cover:

- Distance-based footsteps, faster cadence when sprinting, climbing, landing,
  swimming and entering water. Sneaking and creative flight suppress footsteps;
  teleports reset cadence. Footsteps sample coverings and elevated support cells.
- Mining hits every 0.25 seconds while breaking, accepted block removal and
  placement, and broken mining tools. Rejected edits emit no completion sound.
  Paired door edits and multi-block placement emit once per committed action.
- Opening and closing doors, trapdoors and fence gates using their native wood
  or metal sound type; chest sounds follow lid target changes, with a single
  source for a double chest and closure when the viewer session ends.
- Successful item pickup, armor equipment, food, drinks, burps, chorus teleport,
  damage by source, fall damage, death, and Feathers button activation.

There is no creature combat system to attach attack sounds to yet. Hitting a
block uses its mining hit sound; swinging into empty air remains silent.

Validation follows the repository rules: `cargo check` and explicit ECS access
review only. Sound balance, device output and timing need confirmation in game
by the user; compilation does not validate audible playback.
