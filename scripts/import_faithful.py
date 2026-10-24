#!/usr/bin/env python3
"""Build the block-layer texture array from a pinned Faithful 32x revision (Pillow)."""
from io import BytesIO
from pathlib import Path
from urllib.request import urlopen
import hashlib
import json
from PIL import Image

REVISION = 'ee415a4109f341098ad7cd162af1be8307a7082a'
BASE = f'https://raw.githubusercontent.com/Faithful-Resource-Pack/Faithful-32x-Java/{REVISION}/'
DEST = Path(__file__).resolve().parents[1] / 'assets/voxel/packs/faithful'
NAMES = ['grass_block_top', 'grass_block_side', 'dirt', 'stone', 'sand', 'snow', 'oak_log', 'oak_log_top', 'oak_leaves', 'deepslate', 'gravel', 'coal_ore', 'iron_ore', 'copper_ore', 'gold_ore', 'redstone_ore', 'diamond_ore', 'emerald_ore', 'lapis_ore']

def download(path):
    with urlopen(BASE + path, timeout=60) as response:
        return response.read()

def main():
    DEST.mkdir(parents=True, exist_ok=True)
    atlas = Image.new('RGBA', (32, 32 * len(NAMES)))
    hashes = {}
    for layer, name in enumerate(NAMES):
        data = download(f'assets/minecraft/textures/block/{name}.png')
        hashes[name] = hashlib.sha256(data).hexdigest()
        tile = Image.open(BytesIO(data)).convert('RGBA')
        if tile.size != (32, 32):
            raise ValueError(f'{name}: expected 32x32, got {tile.size}')
        # The engine uses opaque leaves; close the cutout with a dark leaf tone.
        if name == 'oak_leaves':
            background = Image.new('RGBA', tile.size, (82, 82, 82, 255))
            background.alpha_composite(tile)
            tile = background
        atlas.paste(tile, (0, layer * 32))
    atlas.save(DEST / 'blocks_array.png')
    (DEST / 'LICENSE.txt').write_bytes(download('LICENSE.txt'))
    (DEST / 'source.json').write_text(json.dumps({'repository': BASE.split('/' + REVISION)[0], 'revision': REVISION, 'sha256': hashes}, indent=2) + '\n')

if __name__ == '__main__':
    main()
