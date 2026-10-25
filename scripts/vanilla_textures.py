"""Pack block flipbooks and arbitrary-size special sprites into shared banks."""
from io import BytesIO
import json
from PIL import Image, ImageChops

SPECIAL_LAYER = 4096
PAGE_SIZE = 1024


def texture_banks(jar, models, required, grass):
    names = sorted(required | {q['texture'] for qs in models['models'].values() for q in qs})
    ordinary = [name for name in names if not name.startswith('@')]
    special = [name for name in names if name.startswith('@')]
    slots = {name: index for index, name in enumerate(names)}
    layers = {name: index for index, name in enumerate(ordinary)}
    images = {}
    animations = []
    frames = []
    alpha = {}
    entries = set(jar.namelist())
    for name in ordinary:
        source, _, color = name.partition('#')
        path = f'assets/minecraft/textures/block/{source}.png'
        image = Image.open(BytesIO(jar.read(path))).convert('RGBA')
        meta = json.loads(jar.read(path + '.mcmeta')).get('animation') if path + '.mcmeta' in entries else None
        meta = meta if meta is not None else None
        width, height = image.size
        fw = meta.get('width', min(width, height)) if meta is not None else width
        fh = meta.get('height', min(width, height)) if meta is not None else height
        if not fw or not fh or width % fw or height % fh:
            raise ValueError(f'invalid animation frame dimensions in {name}')
        tiles = [image.crop((x, y, x + fw, y + fh)).resize((16, 16), Image.Resampling.NEAREST)
                 for y in range(0, height, fh) for x in range(0, width, fw)]
        if color:
            tint = Image.new('RGBA', (16, 16), (*bytes.fromhex(color), 255))
            tiles = [ImageChops.multiply(tile, tint) for tile in tiles]
        if name == 'grass_block_side':
            overlay = Image.open(BytesIO(jar.read('assets/minecraft/textures/block/grass_block_side_overlay.png'))).convert('RGBA')
            tint = Image.new('RGBA', overlay.size, (*grass, 255))
            tiles[0].alpha_composite(ImageChops.multiply(overlay, tint))
        alpha[name] = all(tile.getextrema()[3][0] == 255 for tile in tiles)
        images[name] = tiles[0]
        if meta is not None and len(tiles) > 1:
            sequence = meta.get('frames', list(range(len(tiles))))
            if not sequence:
                raise ValueError(f'empty animation timeline in {name}')
            frame_map = {0: layers[name]}
            for index, tile in enumerate(tiles[1:], 1):
                frame_map[index] = len(ordinary) + len(frames)
                frames.append(tile)
            timeline = []
            for entry in sequence:
                index = entry['index'] if isinstance(entry, dict) else entry
                duration = entry.get('time', meta.get('frametime', 1)) if isinstance(entry, dict) else meta.get('frametime', 1)
                if index not in frame_map or duration <= 0:
                    raise ValueError(f'invalid animation timeline in {name}')
                timeline.append([frame_map[index], duration])
            animations.append({'layer': layers[name], 'frames': timeline, 'interpolate': meta.get('interpolate', False)})
    physical = [images[name] for name in ordinary] + frames
    if len(physical) > 2048:
        raise ValueError(f'block texture array requires {len(physical)} layers; maximum is 2048')
    atlas = Image.new('RGBA', (16, 16 * len(physical)))
    for index, image in enumerate(physical):
        atlas.paste(image, (0, index * 16))

    # Shelf-pack the source-resolution entity textures. One shared array of atlas
    # pages avoids both upscaling tiny sprites to dragon-skin size and one material
    # per block. UVs are resolved once here, not in mesh workers or fragments.
    pages = [Image.new('RGBA', (PAGE_SIZE, PAGE_SIZE))]
    regions = {}
    x, y, row_height = 0, 0, 0
    for name in sorted(special, key=lambda n: (-len(n), n)):
        source, _, color = name[1:].partition('#')
        image = Image.open(BytesIO(jar.read(f'assets/minecraft/textures/{source}.png'))).convert('RGBA')
        if color:
            rgb = tuple(bytes.fromhex(color))
            image = ImageChops.multiply(image, Image.new('RGBA', image.size, (*rgb, 255)))
        width, height = image.size
        if width + 2 > PAGE_SIZE or height + 2 > PAGE_SIZE:
            raise ValueError(f'special texture {source} exceeds atlas page dimensions')
        if x + width + 2 > PAGE_SIZE:
            x, y, row_height = 0, y + row_height, 0
        if y + height + 2 > PAGE_SIZE:
            pages.append(Image.new('RGBA', (PAGE_SIZE, PAGE_SIZE)))
            x, y, row_height = 0, 0, 0
        # Replicated one-pixel borders keep derivatives and filtering inside sprites.
        page = pages[-1]
        padded = image.resize((width + 2, height + 2), Image.Resampling.NEAREST)
        padded.paste(image, (1, 1))
        padded.paste(image.crop((0, 0, 1, height)), (0, 1))
        padded.paste(image.crop((width-1, 0, width, height)), (width+1, 1))
        padded.paste(image.crop((0, 0, width, 1)), (1, 0))
        padded.paste(image.crop((0, height-1, width, height)), (1, height+1))
        page.paste(padded, (x, y))
        regions[name] = (len(pages)-1, x+1, y+1, width, height)
        alpha[name] = image.getextrema()[3][0] == 255
        x += width + 2
        row_height = max(row_height, height + 2)
    if len(pages) > 2048:
        raise ValueError('special texture array exceeds 2048 pages')
    special_atlas = Image.new('RGBA', (PAGE_SIZE, PAGE_SIZE * len(pages)))
    for index, page in enumerate(pages):
        special_atlas.paste(page, (0, PAGE_SIZE * index))
    packed_quads = set()
    for quads in models['models'].values():
        for q in quads:
            # Waxed/material aliases can intentionally share the same quad data.
            if id(q) in packed_quads:
                continue
            packed_quads.add(id(q))
            name = q['texture']
            if name in regions:
                _, rx, ry, width, height = regions[name]
                q['uv'] = [[(rx + u * width) / PAGE_SIZE, (ry + v * height) / PAGE_SIZE] for u, v in q['uv']]
            q['texture'] = slots[name]
    models.update(textures=names,
                  texture_layers=[SPECIAL_LAYER + regions[name][0] if name in regions else layers[name] for name in names],
                  opaque_textures=[alpha[name] for name in names],
                  special_layers=len(pages), animations=animations)
    # The consuming renderer reserves 13 bits for model slots, including the
    # three fallback face slots of each native block kind.
    block_count = models['coverage']['blocks']
    if len(names) + block_count * 3 > 8192:
        raise ValueError('model texture slots exceed the packed 13-bit range')
    for name, quads in models['models'].items():
        for q in quads:
            if any(not -2 <= value < 15 for p in q['positions'] for value in p):
                raise ValueError(f'model {name} exceeds section vertex position bounds')
            if any(not -8 <= value < 8 for uv in q['uv'] for value in uv):
                raise ValueError(f'model {name} exceeds packed UV bounds')
    return atlas, special_atlas, layers
