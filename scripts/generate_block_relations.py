"""Bake native block relationships without runtime name construction.

Run after updating the integrated worldgen registry. This reads native metadata;
Minecraft names remain an import-time concern.
"""
from pathlib import Path
import re
import struct

ROOT = Path(__file__).resolve().parents[1]
source = (ROOT / "crates/worldgen/crates/pumpkin-data/src/generated/block.rs").read_text()
blocks = {}
for match in re.finditer(r'pub const (\w+): Self = Block \{(.*?)\n    \};', source, re.S):
    body = match[2]
    name = re.search(r'name: "([^"]+)"', body)[1]
    item = int(re.search(r'item_id: (\d+)', body)[1])
    raw = int(re.search(r'id: BlockId::new\((\d+)u16\)', body)[1])
    assert raw == len(blocks), "Block declarations must follow native ID order"
    blocks[name] = (match[1], item, raw)
assert len(blocks) == 1286, "Review the updated block registry before regenerating"

def itemizable(name):
    return blocks[name][1] != 0 and name not in {"air", "water", "lava"}

canonical = {}
walls = {}
materials = {}
for name in blocks:
    candidates = [name.replace("_wall_", "_"), name.removeprefix("wall_"), name.removesuffix("_plant")]
    canonical[name] = next((value for value in candidates if value != name and value in blocks and itemizable(value)), name)
    item = canonical[name]
    candidates = ["wall_" + item]
    if "_" in item:
        base, suffix = item.rsplit("_", 1)
        candidates.append(base + "_wall_" + suffix)
    if item.endswith("_hanging_sign"):
        candidates.append(item.removesuffix("_hanging_sign") + "_wall_hanging_sign")
    walls[name] = next((value for value in candidates if value in blocks), None)
    material = None
    for suffix in ["_stairs", "_slab", "_wall", "_fence", "_fence_gate", "_door", "_trapdoor", "_pane"]:
        if name.endswith(suffix):
            base = name.removesuffix(suffix)
            material = next((value for value in [base, base + "s", base + "_planks"] if value in blocks), None)
            if material:
                break
    materials[name] = material

fingerprint = 0xCBF29CE484222325
for name in blocks:
    for byte in name.encode() + b"\0":
        fingerprint = ((fingerprint ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
output = bytearray(b"HBR\x01")
output.extend(struct.pack("<HQ", len(blocks), fingerprint))
for name in blocks:
    for mapping in [canonical, walls, materials]:
        target = mapping[name]
        output.extend(struct.pack("<H", blocks[target][2] if target is not None else 0xFFFF))
(ROOT / "crates/worldgen/src/block_relations.bin").write_bytes(output)
