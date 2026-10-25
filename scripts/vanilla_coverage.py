"""Import-time coverage against the consuming workspace's native state registry.

Read the generated registry as data, without compiling/running a second program.
Reject changed generator layouts rather than silently skipping property families.
"""
from itertools import product
from pathlib import Path
import math
import re


def native_states():
    source = (Path(__file__).resolve().parents[2] /
              'worldgen/crates/pumpkin-data/src/generated/block.rs').read_text()
    blocks = {}
    declarations = list(re.finditer(r'^    pub const (\w+): Self = Block \{', source, re.M))
    for index, declaration in enumerate(declarations):
        end = declarations[index + 1].start() if index + 1 < len(declarations) else source.index('\n    pub fn', declaration.end())
        body = source[declaration.end():end]
        name = re.search(r'name: "([^"]+)"', body).group(1)
        state_ids = set(map(int, re.findall(r'id: BlockStateId::new\((\d+)u16\)', body)))
        blocks[declaration.group(1)] = (name, len(state_ids))
    if not blocks:
        raise ValueError('native block registry declarations could not be read')
    enums = {}
    for match in re.finditer(r'pub enum (\w+) \{[^}]+\}\nimpl \1 \{(.*?)\n\}', source, re.S):
        values = re.findall(r'Self::\w+ => "([^"]+)"', match.group(2))
        if values:
            enums[match.group(1)] = values
    profiles = {}
    structs = list(re.finditer(r'pub struct (\w+Properties) \{(.*?)\n\}', source, re.S))
    for index, match in enumerate(structs):
        end = structs[index + 1].start() if index + 1 < len(structs) else len(source)
        body = source[match.end():end]
        factory = body.split('pub fn from_state_id', 1)[0]
        fields = re.findall(r'r#(\w+): \{(.*?)\n            \}', factory, re.S)
        expected = re.findall(r'pub r#(\w+):', match.group(2))
        if sorted(name for name, _ in fields) != sorted(expected):
            raise ValueError(f'cannot read native property layout {match.group(1)}')
        domains = {}
        for name, field in fields:
            enum = re.search(r'index % (\w+)::variant_count\(\)', field)
            if enum:
                domains[name] = enums[enum.group(1)]
            elif 'value == 0' in field:
                domains[name] = ['true', 'false']
            else:
                count = int(re.search(r'index % (\d+)', field).group(1))
                offset = re.search(r'value \+ (\d+)u8', field)
                start = int(offset.group(1)) if offset else 0
                domains[name] = [str(value) for value in range(start, start + count)]
        handles = re.search(r'fn handles_block_id\(.*?\n    \}', body, re.S)
        if not handles:
            raise ValueError(f'cannot read native family membership {match.group(1)}')
        for key in re.findall(r'BlockId::(\w+)', handles.group(0)):
            profiles[key] = domains
    for key, (name, count) in blocks.items():
        domains = profiles.get(key, {})
        if math.prod(map(len, domains.values())) != count:
            raise ValueError(f'native property/state count mismatch for {name}')
        yield name, domains, count


def matches(condition, props):
    return all(any(matches(value, props) for value in choices) if key == 'OR'
               else all(matches(value, props) for value in choices) if key == 'AND'
               else props.get(key) in choices.split('|')
               for key, choices in condition.items())


def coverage(models):
    totals = dict(blocks=0, states=0, visible_states=0, invisible_states=0, missing_states=0)
    invisible = set(models['invisible_models'])
    for block, domains, count in native_states():
        if block not in models['blocks']:
            raise ValueError(f'native block {block} has no imported definition')
        data = models['blocks'][block]
        totals['blocks'] += 1
        for values in product(*domains.values()):
            props = dict(zip(domains, values))
            applies = []
            for key, apply in data.get('variants', {}).items():
                if not key or matches(dict(pair.split('=') for pair in key.split(',')), props):
                    applies.append(apply)
                    break
            applies.extend(part['apply'] for part in data.get('multipart', [])
                           if 'when' not in part or matches(part['when'], props))
            references = [variant['model'] for apply in applies
                          for variant in (apply if isinstance(apply, list) else [apply])]
            if not applies and 'variants' in data:
                raise ValueError(f'native state {block}{props} has no matching variant')
            visible = any(models['models'][name] for name in references)
            if not visible and references and not all(name in invisible for name in references):
                raise ValueError(f'native visible state {block}{props} has no geometry')
            totals['states'] += 1
            totals['visible_states' if visible else 'invisible_states'] += 1
    return totals
