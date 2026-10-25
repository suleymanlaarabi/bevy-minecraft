# Breaking overlay textures

## Minecraft (used when imported)

`vanilla/destroy_stage_0.png` through `vanilla/destroy_stage_9.png` are the
unmodified official Minecraft textures, © Mojang Studios. They are generated
locally by `python3 scripts/import_vanilla.py` from Mojang's client download
(SHA-1 verified), and excluded from git, like the vanilla block textures.
`vanilla/source.json` records the source version and client SHA-1.

All ten stages must be present to use this set. Otherwise the bundled textures
below are used.

## Bundled fallback

The `stage_0.png` through `stage_9.png` textures are derived from
[`misc/crack_anylength.png`](https://github.com/minetest-texture-packs/Pixel-Perfection/blob/master/misc/crack_anylength.png)
from the [Pixel Perfection](https://github.com/minetest-texture-packs/Pixel-Perfection)
texture pack.

- Author: Hugh "XSSheep" Rutland
- Contributors: Toby109tt, tacotexmex, devurandom
- License: [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)
- Changes: the ten source frames were split into individual files for this
  game's ten-stage API.

These derived textures are distributed under CC BY-SA 4.0.
