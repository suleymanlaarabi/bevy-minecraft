#!/usr/bin/env python3
"""Build the block-layer texture array from the official Minecraft client jar (Pillow).

Mojang's textures may not be redistributed: the generated PNG is git-ignored and
must be rebuilt locally from Mojang's own download servers.
"""
from io import BytesIO
from pathlib import Path
from urllib.request import urlopen
import hashlib
import json
import sys
import zipfile
from PIL import Image
from vanilla_models import import_models, deduplicate_models
from vanilla_textures import texture_banks
from vanilla_coverage import coverage
from vanilla_tints import apply_tints

VERSION = sys.argv[1] if len(sys.argv) > 1 else '26.3'
MANIFEST = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
DEST = Path(__file__).resolve().parents[1] / 'assets/voxel/packs/vanilla'
GRASS = (0x91, 0xBD, 0x59)  # plains grass colour
FOLIAGE = (0x77, 0xAB, 0x2F)  # plains foliage colour
SPRUCE = (0x61, 0x99, 0x61)
BIRCH = (0x80, 0xA7, 0x55)

def cube(texture, tint=None):
    return (texture, texture, texture, tint)

def column(top, side, bottom=None, tint=None):
    return (top, side, bottom or top, tint)

# VoxelKind key -> (top, side, bottom, tint). Keys match `VoxelKind::key`.
BLOCKS = {
    'grass_block': ('grass_block_top', 'grass_block_side', 'dirt', None),
    'dirt': cube('dirt'), 'stone': cube('stone'), 'sand': cube('sand'), 'snow': cube('snow'),
    'oak_log': column('oak_log_top', 'oak_log'), 'oak_leaves': cube('oak_leaves', FOLIAGE),
    'deepslate': column('deepslate_top', 'deepslate'), 'gravel': cube('gravel'),
    **{ore: cube(ore) for kind in ['coal', 'iron', 'copper', 'gold', 'redstone', 'diamond', 'emerald', 'lapis'] for ore in [f'{kind}_ore', f'deepslate_{kind}_ore']},
    **{name: cube(name) for name in ['granite', 'diorite', 'andesite', 'tuff', 'calcite', 'bedrock', 'cobblestone', 'mossy_cobblestone', 'cobbled_deepslate', 'stone_bricks', 'mossy_stone_bricks', 'cracked_stone_bricks', 'obsidian', 'smooth_basalt', 'dripstone_block', 'amethyst_block', 'budding_amethyst', 'magma', 'coarse_dirt', 'rooted_dirt', 'mud', 'clay', 'moss_block', 'red_sand', 'terracotta', 'powder_snow', 'ice', 'packed_ice', 'blue_ice', 'bricks', 'prismarine', 'prismarine_bricks', 'dark_prismarine', 'sea_lantern', 'brown_mushroom_block', 'red_mushroom_block', 'mushroom_stem', 'cherry_leaves', 'pale_oak_leaves', 'azalea_leaves', 'flowering_azalea_leaves']},
    **{f'{color}_terracotta': cube(f'{color}_terracotta') for color in ['white', 'orange', 'yellow', 'brown', 'red', 'light_gray']},
    **{f'{wood}_log': column(f'{wood}_log_top', f'{wood}_log') for wood in ['spruce', 'birch', 'jungle', 'acacia', 'dark_oak', 'mangrove', 'cherry', 'pale_oak']},
    **{f'{wood}_planks': cube(f'{wood}_planks') for wood in ['oak', 'spruce', 'birch', 'jungle', 'acacia', 'dark_oak']},
    'spruce_leaves': cube('spruce_leaves', SPRUCE), 'birch_leaves': cube('birch_leaves', BIRCH),
    **{f'{wood}_leaves': cube(f'{wood}_leaves', FOLIAGE) for wood in ['jungle', 'acacia', 'dark_oak', 'mangrove']},
    'magma_block': cube('magma'), 'lava': cube('lava_still'), 'snow_block': cube('snow'),
    'podzol': column('podzol_top', 'podzol_side', 'dirt'), 'mycelium': column('mycelium_top', 'mycelium_side', 'dirt'),
    'dirt_path': column('dirt_path_top', 'dirt_path_side', 'dirt'), 'farmland': column('farmland', 'dirt', 'dirt'),
    'sandstone': column('sandstone_top', 'sandstone', 'sandstone_bottom'),
    'red_sandstone': column('red_sandstone_top', 'red_sandstone', 'red_sandstone_bottom'),
    'pumpkin': column('pumpkin_top', 'pumpkin_side'), 'melon': column('melon_top', 'melon_side'),
    'hay_block': column('hay_block_top', 'hay_block_side'), 'bookshelf': column('oak_planks', 'bookshelf'),
}
del BLOCKS['magma']

def download(url):
    with urlopen(url, timeout=120) as response:
        return response.read()

def client_jar():
    local = Path.home() / '.minecraft' / 'versions' / VERSION
    manifest = local / f'{VERSION}.json'
    archive = local / f'{VERSION}.jar'
    if manifest.is_file() and archive.is_file():
        client = json.loads(manifest.read_text())['downloads']['client']
        data = archive.read_bytes()
    else:
        versions = json.loads(download(MANIFEST))['versions']
        entry = next(v for v in versions if v['id'] == VERSION)
        client = json.loads(download(entry['url']))['downloads']['client']
        data = download(client['url'])
    if hashlib.sha1(data).hexdigest() != client['sha1']:
        raise ValueError('client jar SHA-1 mismatch')
    return zipfile.ZipFile(BytesIO(data)), client['sha1']

def import_breaking_textures(jar, sha1):
    dest = DEST.parents[1] / 'breaking/vanilla'
    # Read all ten stages before replacing any files. Keep Mojang's PNGs unchanged.
    stages = [jar.read(f'assets/minecraft/textures/block/destroy_stage_{stage}.png')
              for stage in range(10)]
    dest.mkdir(parents=True, exist_ok=True)
    for stage, data in enumerate(stages):
        (dest / f'destroy_stage_{stage}.png').write_bytes(data)
    (dest / 'source.json').write_text(json.dumps(
        {'minecraft': VERSION, 'client_sha1': sha1}, indent=2) + '\n')

def main():
    jar, sha1 = client_jar()
    import_breaking_textures(jar, sha1)
    from import_player_assets import import_player_assets
    import_player_assets(jar)
    DEST.mkdir(parents=True, exist_ok=True)
    models, _ = import_models(jar)
    apply_tints(jar, models)
    deduplicate_models(models)
    models['coverage'] = coverage(models)
    models['client_sha1'] = sha1
    required = {name for top, side, bottom, _ in BLOCKS.values() for name in (top, side, bottom)}
    required.update(["water_flow", "lava_flow"])
    atlas, special_atlas, layers = texture_banks(jar, models, required, GRASS)
    # All model families and timelines are resolved before publishing the cache.
    atlas.save(DEST / 'blocks_array.png')
    special_atlas.save(DEST / 'special_array.png')
    (DEST / 'models.json').write_text(json.dumps(models, separators=(',', ':')))
    layer_count = atlas.height // 16
    print(f"Imported {len(models['blocks'])} block definitions, {len(models['models'])} models, "
          f"{layer_count} block texture layers, {models['special_layers']} special atlas pages, "
          f"and {len(models['animations'])} GPU animation timelines")
    print('Native registry coverage:', models['coverage'])
    # Still water keeps all its flipbook frames: a 16x512 strip of grey frames
    # tinted in the shader and cross-faded at the game's 10 frames per second.
    Image.open(BytesIO(jar.read('assets/minecraft/textures/block/water_still.png'))).convert('RGBA').save(DEST / 'water_still.png')
    def tint(color):
        return '(1.0, 1.0, 1.0)' if color is None else '(' + ', '.join(f'{c / 255:.3f}' for c in color) + ')'
    entries = []
    for key, (top, side, bottom, color) in sorted(BLOCKS.items()):
        top_tint = tint(GRASS if key == 'grass_block' else color)
        entries.append(f'        "{key}": (top: {layers[top]}, side: {layers[side]}, bottom: {layers[bottom]}, tint_top: {top_tint}, tint_side: {tint(color)}, tint_bottom: {tint(color)}),')
    (DEST / 'pack.ron').write_text(f'(\n    name: "Minecraft {VERSION}",\n    layers: {layer_count},\n    texture: "blocks_array.png",\n    blocks: {{\n' + '\n'.join(entries) + '\n    },\n)\n')
    (DEST / 'source.json').write_text(json.dumps({'minecraft': VERSION, 'client_sha1': sha1}, indent=2) + '\n')

if __name__ == '__main__':
    main()
