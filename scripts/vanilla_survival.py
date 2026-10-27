"""Import HUD sprites for survival gameplay."""
from io import BytesIO
from pathlib import Path
from PIL import Image
import json


def import_survival(jar, destination):
    root = destination / 'survival'
    root.mkdir(parents=True, exist_ok=True)
    prefix = 'assets/minecraft/textures/'
    names = sorted(name for name in jar.namelist() if name.endswith('.png') and (
        name.startswith(prefix + 'gui/sprites/hud/heart/') or
        name.startswith(prefix + 'mob_effect/') or
        (name.startswith(prefix + 'gui/sprites/hud/') and
         Path(name).stem.startswith(('food_', 'armor_', 'air', 'crosshair')))))
    columns, tile = 16, 32
    rows = (len(names) + columns - 1) // columns
    atlas = Image.new('RGBA', (columns * tile, rows * tile))
    sprites = {}
    for index, name in enumerate(names):
        sprite = Image.open(BytesIO(jar.read(name))).convert('RGBA')
        if max(sprite.size) > tile:
            continue
        x, y = index % columns * tile, index // columns * tile
        atlas.paste(sprite, (x, y))
        key = name.removeprefix(prefix).removesuffix('.png')
        sprites[key] = [x, y, sprite.width, sprite.height]
    atlas.save(root / 'hud.png')
    (root / 'hud.json').write_text(json.dumps({
        'sprites': sprites,
    }, separators=(',', ':')) + '\n')
    (root / 'source.json').write_text(json.dumps({
        'minecraft': '26.3', 'hud_sprites': len(sprites),
    }, indent=2) + '\n')
    print(f'Survival: {len(sprites)} HUD/effect sprites')
