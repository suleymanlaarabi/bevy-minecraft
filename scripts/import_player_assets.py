#!/usr/bin/env python3
"""Extract local item sprites and humanoid equipment from the official client.

Generated Mojang textures are ignored by Git, like the existing vanilla blocks.
Use a local client jar or the same verified download as import_vanilla.py.
"""
from io import BytesIO
from PIL import Image
import json
import sys
import zipfile
from pathlib import Path
from vanilla_held_items import import_held_items
from vanilla_survival import import_survival
from import_sounds import import_sounds

DEST = Path(__file__).resolve().parents[1] / 'assets/player/vanilla'

def import_player_assets(jar):
    prefix = 'assets/minecraft/textures/'
    sprites = sorted(name for name in jar.namelist()
                     if name.startswith(prefix + 'item/') and name.endswith('.png'))
    columns = 32
    tile = 16
    rows = (len(sprites) + columns - 1) // columns
    atlas = Image.new('RGBA', (columns * tile, rows * tile))
    indices = {}
    for index, name in enumerate(sprites):
        sprite = Image.open(BytesIO(jar.read(name))).convert('RGBA')
        # Animated item sheets display their first square frame.
        sprite = sprite.crop((0, 0, sprite.width, sprite.width)).resize((tile, tile), Image.Resampling.NEAREST)
        atlas.paste(sprite, ((index % columns) * tile, (index // columns) * tile))
        indices[Path(name).stem] = index
    DEST.mkdir(parents=True, exist_ok=True)
    atlas.save(DEST / 'items.png')
    (DEST / 'items.json').write_text(json.dumps({
        'columns': columns, 'rows': rows, 'tile': tile, 'items': indices,
    }, separators=(',', ':')) + '\n')
    for name in jar.namelist():
        if (name.startswith(prefix + 'entity/equipment/') and
            any('/' + layer + '/' in name for layer in ['humanoid', 'humanoid_leggings'])
            and name.endswith('.png')):
            target = DEST / name.removeprefix(prefix)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(jar.read(name))
    import_held_items(jar, DEST)
    import_survival(jar, DEST)
    import_sounds()

if __name__ == '__main__':
    if len(sys.argv) > 1:
        with zipfile.ZipFile(sys.argv[1]) as jar:
            import_player_assets(jar)
    else:
        from import_vanilla import client_jar
        jar, _ = client_jar()
        import_player_assets(jar)
