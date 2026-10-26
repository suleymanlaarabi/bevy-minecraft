"""Import HUD sprites and a small local sound bank for survival gameplay."""
from io import BytesIO
from pathlib import Path
from PIL import Image
import json


SOUNDS = {
    'hurt': 'damage/hit1',
    'eat': 'random/eat1',
    'drink': 'random/drink',
    'burp': 'random/burp',
    'teleport': 'mob/endermen/portal',
}


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
    client = Path.home() / '.minecraft'
    index_name = json.loads((client / 'versions/26.3/26.3.json').read_text())['assetIndex']['id']
    index = json.loads((client / f'assets/indexes/{index_name}.json').read_text())['objects']
    audio = root / 'audio'
    audio.mkdir(exist_ok=True)
    for cue, name in SOUNDS.items():
        key = f'minecraft/sounds/{name}.ogg'
        if key not in index:
            raise ValueError(f'Missing sound in client asset index: {key}')
        digest = index[key]['hash']
        source = client / 'assets/objects' / digest[:2] / digest
        if not source.is_file():
            raise ValueError(f'Missing local client sound: {source}')
        (audio / f'{cue}.ogg').write_bytes(source.read_bytes())
    (root / 'source.json').write_text(json.dumps({
        'minecraft': '26.3', 'hud_sprites': len(sprites), 'sounds': SOUNDS,
    }, indent=2) + '\n')
    print(f'Survival: {len(sprites)} HUD/effect sprites, {len(SOUNDS)} sound cues')
