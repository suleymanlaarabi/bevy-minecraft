#!/usr/bin/env python3
"""Import gameplay sound events, variants and block sound types from a local client."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CLIENT = Path.home() / '.minecraft'
DEST = ROOT / 'assets/audio/vanilla'
CUES = [
    'block.chest.open', 'block.chest.close', 'entity.item.pickup',
    'entity.player.swim', 'entity.player.splash', 'entity.player.small_fall',
    'entity.player.big_fall', 'entity.player.hurt', 'entity.player.hurt_drown',
    'entity.player.hurt_on_fire', 'entity.player.death', 'entity.generic.eat',
    'entity.generic.drink', 'entity.player.burp', 'item.chorus_fruit.teleport',
    'entity.item.break', 'ui.button.click',
    'entity.cow.ambient', 'entity.cow.hurt', 'entity.cow.death', 'entity.cow.step',
    'entity.pig.ambient', 'entity.pig.hurt', 'entity.pig.death', 'entity.pig.step',
]


def import_sounds():
    version = json.loads((CLIENT / 'versions/26.3/26.3.json').read_text())
    objects = json.loads((CLIENT / f"assets/indexes/{version['assetIndex']['id']}.json").read_text())['objects']

    def source(key):
        digest = objects[key]['hash']
        return CLIENT / 'assets/objects' / digest[:2] / digest

    events = json.loads(source('minecraft/sounds.json').read_text())
    java_home = os.environ.get('JAVA_HOME')
    java = str(Path(java_home) / 'bin/java') if java_home else next(
        (str(p) for p in sorted((CLIENT / 'runtime/java-runtime-epsilon').glob('**/bin/java'))),
        shutil.which('java'))
    if not java:
        raise ValueError('A local Java runtime is required to extract block sound types')
    classpath = os.pathsep.join(map(str, [CLIENT / 'versions/26.3/26.3.jar',
                                        *sorted((CLIENT / 'libraries').glob('**/*.jar'))]))
    with tempfile.TemporaryDirectory(prefix='hollow-sound-import-') as temp:
        output = Path(temp) / 'blocks.json'
        result = subprocess.run([java, '-Djava.awt.headless=true', '--class-path', classpath,
                                 str(ROOT / 'scripts/ExtractBlockSounds.java'), str(output)],
                                cwd=temp, capture_output=True, text=True)
        if result.returncode:
            raise ValueError(result.stderr[-6000:])
        blocks = json.loads(output.read_text())

    def resolve(name, ancestors=()):
        name = name.removeprefix('minecraft:')
        if name in ancestors:
            raise ValueError(f'Cyclic sound event: {name}')
        if name == 'intentionally_empty':
            return []
        variants = []
        for entry in events[name].get('sounds', []):
            entry = {'name': entry} if isinstance(entry, str) else entry
            volume, pitch, weight = (entry.get(key, 1) for key in ('volume', 'pitch', 'weight'))
            if entry.get('type') == 'event':
                children = resolve(entry['name'], (*ancestors, name))
                total = sum(child['weight'] for child in children)
                for child in children:
                    variants.append(dict(child, volume=child['volume'] * volume,
                                         pitch=child['pitch'] * pitch,
                                         weight=child['weight'] / total * weight))
            else:
                path = entry['name'].removeprefix('minecraft:') + '.ogg'
                target = DEST / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(source('minecraft/sounds/' + path).read_bytes())
                variants.append(dict(path=path, volume=volume, pitch=pitch, weight=weight))
        return variants

    needed = set(CUES)
    needed.update(name for name in events if name.startswith("item.armor.equip_"))
    for block in blocks.values():
        needed.update(value for value in block.values() if isinstance(value, str))
    bank = {name: resolve(name) for name in sorted(needed)}
    DEST.mkdir(parents=True, exist_ok=True)
    (DEST / 'sounds.json').write_text(json.dumps(dict(blocks=blocks, events=bank), separators=(',', ':')) + '\n')
    print(f'Imported {len(blocks)} block sound profiles, {len(bank)} events, '
          f'{len(list(DEST.rglob("*.ogg")))} sound files')


if __name__ == '__main__':
    import_sounds()
