"""Extract static first-person item transforms and generated sprite sources."""
from io import BytesIO
import json
from PIL import Image


def import_held_items(jar, destination):
    names = set(jar.namelist())
    cache = {}
    sprites = set()

    def resolve(name):
        name = name.removeprefix('minecraft:')
        if name in cache:
            return cache[name]
        path = f'assets/minecraft/models/{name}.json'
        if path not in names:
            return {}
        own = json.loads(jar.read(path))
        parent = resolve(own['parent']) if 'parent' in own else {}
        result = {**parent, **own,
                  'textures': {**parent.get('textures', {}), **own.get('textures', {})},
                  'display': {**parent.get('display', {}), **own.get('display', {})},
                  'generated': own.get('parent') in ('builtin/generated', 'minecraft:builtin/generated')
                               or parent.get('generated', False)}
        cache[name] = result
        return result

    items = {}
    for path in sorted(names):
        if not path.startswith('assets/minecraft/items/') or not path.endswith('.json'):
            continue
        definition = json.loads(jar.read(path)).get('model', {})
        if definition.get('type') != 'minecraft:model':
            continue  # Dynamic and special renderers need their own behavior.
        model = resolve(definition['model'])
        entry = {'display': model.get('display', {}).get('firstperson_righthand', {})}
        if model.get('generated'):
            texture = model.get('textures', {}).get('layer0')
            seen = set()
            while isinstance(texture, str) and texture.startswith('#'):
                if texture in seen:
                    raise ValueError(f'Cyclic held-item texture in {path}')
                seen.add(texture)
                texture = model['textures'].get(texture[1:])
            if not isinstance(texture, str):
                continue
            texture = texture.removeprefix('minecraft:')
            if f'assets/minecraft/textures/{texture}.png' not in names:
                raise ValueError(f'Missing held-item sprite: {texture}')
            entry['sprite'] = texture
            sprites.add(texture)
        items[path.rsplit('/', 1)[1][:-5]] = entry

    for texture in sorted(sprites):
        image = Image.open(BytesIO(jar.read(f'assets/minecraft/textures/{texture}.png'))).convert('RGBA')
        image = image.crop((0, 0, image.width, image.width))
        target = destination / 'held' / f'{texture}.png'
        target.parent.mkdir(parents=True, exist_ok=True)
        image.save(target)
    (destination / 'held_items.json').write_text(json.dumps(
        {'version': 1, 'items': items}, separators=(',', ':')) + '\n')
