"""Resolve state palettes to shared texture variants at import time.

No per-cell tint interpretation, wider model vertices, or extra fragment lookup
is needed for redstone power, crop age, or distinct tint indices on one model.
"""
from copy import deepcopy
from io import BytesIO
from pathlib import Path
import json
from PIL import Image


def apply_tints(jar, models):
    rules = json.loads(Path(__file__).with_name('block_tints.json').read_text())
    baked = {}
    def recolor(apply, indices):
        variants = deepcopy(apply if isinstance(apply, list) else [apply])
        for variant in variants:
            original = variant['model']
            key = (original, tuple(sorted(indices.items())))
            if key not in baked:
                quads = deepcopy(models['models'][original])
                for q in quads:
                    if q['tinted']:
                        color = indices.get(str(q.get('tintindex', 0)), 'ffffff')
                        if color != 'ffffff':
                            q['texture'] += '#' + color
                        q['tinted'] = False
                name = f'hollow:tinted/{len(baked)}'
                models['models'][name] = quads
                baked[key] = name
            variant['model'] = baked[key]
        return variants if isinstance(apply, list) else variants[0]
    for rule in rules:
        indices = rule.get('indices')
        if 'colormap' in rule:
            image = Image.open(BytesIO(jar.read('assets/minecraft/textures/colormap/' + rule['colormap'] + '.png'))).convert('RGB')
            temperature, rainfall = rule['temperature'], rule['rainfall']
            color = image.getpixel((int((1-temperature)*255), int((1-rainfall*temperature)*255)))
            indices = {'0': ''.join(f'{channel:02x}' for channel in color)}
        palette = rule.get('palette', {'': None})
        for block in rule['blocks']:
            data = models['blocks'][block]
            if 'variants' in data:
                output = {}
                for key, apply in data['variants'].items():
                    props = dict(pair.split('=') for pair in key.split(',')) if key else {}
                    for value, color in palette.items():
                        next_props = dict(props)
                        if 'property' in rule:
                            prop = rule['property']
                            if prop in props and props[prop] != value:
                                continue
                            next_props[prop] = value
                        new_key = ','.join(f'{name}={value}' for name, value in sorted(next_props.items()))
                        output[new_key] = recolor(apply, indices or {'0': color})
                data['variants'] = output
            else:
                parts = []
                for part in data['multipart']:
                    for value, color in palette.items():
                        result = deepcopy(part)
                        result['apply'] = recolor(result['apply'], indices or {'0': color})
                        if 'property' in rule:
                            when = {rule['property']: value}
                            result['when'] = {'AND': [result['when'], when]} if 'when' in result else when
                        parts.append(result)
                data['multipart'] = parts
