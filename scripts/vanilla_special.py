"""Convert official Java model factories to the shared block quad format.

Only import-time family bindings live here. The engine consumes immutable recipes;
there are no per-block render systems, Java dependencies, or model parsers in game.
"""
from copy import deepcopy
from pathlib import Path
import json
import math
import os
import shutil
import subprocess
import tempfile

COLORS = dict(zip(
    'white orange magenta light_blue yellow lime pink gray light_gray cyan purple blue brown green red black'.split(),
    ['f9fffe', 'f9801d', 'c74ebd', '3ab3da', 'fed83d', '80c71f', 'f38baa', '474f52',
     '9d9d97', '169c9c', '8932b8', '3c44aa', '835432', '5e7c16', 'b02e26', '1d1d21']))


def extract_factories(jar):
    """Bake local official model factories headlessly, never initialize the game."""
    scripts = Path(__file__).resolve().parent
    minecraft = Path.home() / '.minecraft'
    java = os.environ.get('JAVA_HOME')
    java = str(Path(java) / 'bin/java') if java else None
    if not java:
        candidates = sorted((minecraft / 'runtime/java-runtime-epsilon').glob('**/bin/java'))
        java = next((str(p) for p in candidates), shutil.which('java'))
    if not java:
        raise ValueError('Java with source-file support is required to import official special models')
    libraries = sorted((minecraft / 'libraries').glob('**/*.jar'))
    if not libraries:
        raise ValueError('Minecraft client libraries are required under ~/.minecraft/libraries')
    with tempfile.TemporaryDirectory(prefix='hollow-model-import-') as temp:
        client = Path(temp) / 'client.jar'
        client.write_bytes(jar.fp.getvalue() if hasattr(jar.fp, 'getvalue') else Path(jar.filename).read_bytes())
        output = Path(temp) / 'models.json'
        classpath = os.pathsep.join(map(str, [client, *libraries]))
        result = subprocess.run([
            java, '-Djava.awt.headless=true', '--class-path', classpath,
            str(scripts / 'ExtractBlockModels.java'),
            str(scripts / 'block_model_factories.json'), str(output),
        ], cwd=temp, capture_output=True, text=True)
        if result.returncode:
            raise ValueError(f'Official model extraction failed:\n{result.stderr[-5000:]}')
        return json.loads(output.read_text())


def transform(quads, scale=(1, 1, 1), translate=(0, 0, 0), rotations=()):
    """One affine assembly path for all model parts and attachment families."""
    out = deepcopy(quads)
    def apply(vector, normal=False):
        value = [vector[i] / scale[i] if normal else vector[i] * scale[i] for i in range(3)]
        for axis, degrees in rotations:
            u, v = ((axis + 1) % 3, (axis + 2) % 3)
            c, s = math.cos(math.radians(degrees)), math.sin(math.radians(degrees))
            value[u], value[v] = c * value[u] - s * value[v], s * value[u] + c * value[v]
        if normal:
            length = math.sqrt(sum(x*x for x in value))
            return [round(x / length, 7) for x in value]
        return [round(value[i] + translate[i], 7) for i in range(3)]
    for q in out:
        q['positions'] = [apply(p) for p in q['positions']]
        q['normal'] = apply(q['normal'], True)
        if math.prod(scale) < 0:
            q['positions'].reverse()
            q['uv'].reverse()
    return out


def import_special_models(jar, models, states, textures):
    factories = extract_factories(jar)
    chest_pivots = []
    def geometry(factory, texture, keep_parts=False, **transformation):
        quads = transform(factories[factory], **transformation)
        for q in quads:
            if not keep_parts:
                q.pop('part', None)
                q.pop('pivot', None)
            q.update(texture='@' + texture, tinted=False, cull=[0, 0, 0],
                     translucent=False, shade='', effect=0, light_emission=0)
        textures.add('@' + texture)
        return quads
    def bind(block, variants, append=False):
        if block not in states:
            return
        if append:
            old = states[block]
            parts = [{'apply': value, **({'when': dict(x.split('=') for x in key.split(','))} if key else {})}
                     for key, value in old.get('variants', {}).items()]
            parts.extend(old.get('multipart', []))
            for key, quads in variants.items():
                name = f'hollow:special/{block}/{key or "default"}'
                models[name] = quads
                parts.append({'apply': {'model': name}, **({'when': dict(x.split('=') for x in key.split(','))} if key else {})})
            states[block] = {'multipart': parts}
        else:
            states[block] = {'variants': {}}
            for key, quads in variants.items():
                name = f'hollow:special/{block}/{key or "default"}'
                models[name] = quads
                states[block]['variants'][key] = {'model': name}
    def oriented(quads, angles=None):
        angles = angles or {'south': 0, 'west': -90, 'north': 180, 'east': 90}
        centered = transform(quads, translate=(-0.5, -0.5, -0.5))
        return {f'facing={face}': transform(centered, rotations=[(1, angle)], translate=(0.5, 0.5, 0.5))
                for face, angle in angles.items()}

    # Shared closed-chest assembly; material and double-half are just recipe inputs.
    chest_materials = {'chest': 'normal', 'trapped_chest': 'trapped', 'ender_chest': 'ender'}
    for stage in ['', 'exposed_', 'weathered_', 'oxidized_']:
        material = 'copper' + ('_' + stage[:-1] if stage else '')
        for wax in ['', 'waxed_']:
            chest_materials[wax + stage + 'copper_chest'] = material
    for block, material in chest_materials.items():
        variants = {}
        for half in (['single'] if block == 'ender_chest' else ['single', 'left', 'right']):
            texture = f'entity/chest/{material}' + (f'_{half}' if half != 'single' else '')
            for key, quads in oriented(geometry('chest_' + half, texture)).items():
                variants[key + (f',type={half}' if block != 'ender_chest' else '')] = quads
        bind(block, variants)

    # Animated world chests share canonical local body/lid geometry. Closed
    # assemblies remain available to icons and drops through the normal recipes.
    for half in ['single', 'left', 'right']:
        texture = 'entity/chest/normal' + ('_' + half if half != 'single' else '')
        quads = geometry('chest_' + half, texture, keep_parts=True)
        moving = [q for q in quads if q['part'].endswith(('/lid', '/lock'))]
        pivot = next(q['pivot'] for q in moving if q['part'].endswith('/lid'))
        chest_pivots.append(pivot)
        for part, selected in [('body', [q for q in quads if q not in moving]), ('lid', moving)]:
            for q in selected:
                q.pop('part')
                q.pop('pivot')
                if part == 'lid':
                    q['positions'] = [[round(p[i] - pivot[i], 7) for i in range(3)] for p in q['positions']]
            models[f'hollow:chest/{half}/{part}'] = selected
    bind.chest_pivots = chest_pivots

    # The same axis transforms serve every shulker color.
    for color in ['', *COLORS]:
        block = (color + '_' if color else '') + 'shulker_box'
        texture = 'entity/shulker/shulker' + ('_' + color if color else '')
        shell = geometry('shulker', texture, scale=(1, -1, -1), translate=(0.5, 1.5, 0.5))
        centered = transform(shell, translate=(-0.5, -0.5, -0.5))
        rotations = {'up': [], 'down': [(0, 180)], 'north': [(0, -90)],
                     'south': [(0, 90)], 'west': [(2, 90)], 'east': [(2, -90)]}
        bind(block, {f'facing={face}': transform(centered, rotations=rotation, translate=(0.5, 0.5, 0.5))
                     for face, rotation in rotations.items()})

    for color in COLORS:
        for attachment in ['standing', 'wall']:
            hardware = geometry('banner_' + attachment, 'entity/banner/banner_base')
            cloth = geometry('flag_' + attachment, f'entity/banner/base#{COLORS[color]}')
            quads = transform(hardware + cloth, scale=(2/3, -2/3, -2/3),
                               translate=(0.5, 0, 0.5))
            if attachment == 'standing':
                centered = transform(quads, translate=(-0.5, 0, -0.5))
                variants = {f'rotation={i}': transform(centered, rotations=[(1, -i * 22.5)], translate=(0.5, 0, 0.5)) for i in range(16)}
            else:
                variants = oriented(quads)
            bind(color + ('_banner' if attachment == 'standing' else '_wall_banner'), variants)

    heads = {
        'skeleton_skull': ('skull', 'entity/skeleton/skeleton'),
        'wither_skeleton_skull': ('skull', 'entity/skeleton/wither_skeleton'),
        'zombie_head': ('humanoid_head', 'entity/zombie/zombie'),
        'creeper_head': ('skull', 'entity/creeper/creeper'),
        'player_head': ('humanoid_head', 'entity/player/wide/steve'),
        'piglin_head': ('piglin_head', 'entity/piglin/piglin'),
        'dragon_head': ('dragon_head', 'entity/enderdragon/dragon'),
    }
    for block, (factory, texture) in heads.items():
        quads = geometry(factory, texture, scale=(-1, -1, 1))
        bind(block, {f'rotation={i}': transform(quads, rotations=[(1, -i * 22.5)], translate=(0.5, 0, 0.5)) for i in range(16)})
        variants = {}
        for face, angle, dx, dz in [('south', 180, 0, 1), ('west', 90, -1, 0), ('north', 0, 0, -1), ('east', -90, 1, 0)]:
            variants['facing=' + face] = transform(quads, rotations=[(1, angle)], translate=(0.5-dx*0.25, 0.25, 0.5-dz*0.25))
        wall = block.replace('_skull', '_wall_skull') if block.endswith('_skull') else block.replace('_head', '_wall_head')
        bind(wall, variants)

    for stage in ['', 'exposed_', 'weathered_', 'oxidized_']:
        texture = 'entity/copper_golem/copper_golem' + ('_' + stage[:-1] if stage else '')
        variants = {}
        for pose in ['standing', 'running', 'sitting', 'star']:
            quads = geometry('golem_' + pose, texture, translate=(0.5, 0, 0.5))
            for key, value in oriented(quads, {'north': 0, 'east': -90, 'south': 180, 'west': 90}).items():
                variants[key + ',copper_golem_pose=' + pose] = value
        for wax in ['', 'waxed_']:
            bind(wax + stage + 'copper_golem_statue', variants)

    pot = geometry('pot_base', 'entity/decorated_pot/decorated_pot_base')
    pot += geometry('pot_sides', 'entity/decorated_pot/decorated_pot_side')
    bind('decorated_pot', oriented(pot))
    bind('conduit', {'': geometry('conduit', 'entity/conduit/base', translate=(0.5, 0.5, 0.5))})
    book = geometry('book', 'entity/enchantment/enchanting_table_book', rotations=[(2, 80)], translate=(0.5, 0.85, 0.5))
    bind('enchanting_table', {'': book}, append=True)

    return bind
