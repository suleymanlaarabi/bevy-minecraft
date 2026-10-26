import json,re,sys
from pathlib import Path

def lines(path):
    # Generator test modules are separate files gated by their parent modules.
    if path.name in {'tests.rs', 'test.rs', 'test_deserializer.rs', 'parity_fingerprint_test.rs'} or 'proto_chunk_test' in path.parts:
        return 0
    source=path.read_text()
    # Test items in this project are trailing modules or standalone helpers.
    while (match:=re.search(r'#\[cfg\(test\)\]',source)):
        start=match.start()
        brace=source.find('{',match.end())
        semicolon=source.find(';',match.end())
        if semicolon >= 0 and (brace < 0 or semicolon < brace):
            end=semicolon+1
        else:
            depth=1
            end=brace+1
            token=re.compile(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/|[{}]',re.S)
            for part in token.finditer(source,end):
                if part[0]=='{': depth+=1
                if part[0]=='}': depth-=1
                if depth==0:
                    end=part.end()
                    break
        source=source[:start]+source[end:]
    source=re.sub(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/',lambda m: '\n'*m[0].count('\n') if m[0].startswith(('//','/*')) else m[0],source,flags=re.S)
    return sum(bool(line.strip()) for line in source.splitlines())
root=Path(sys.argv[1])
counts={str(p.relative_to(root)):lines(p) for p in sorted(root.rglob('*.rs'))}
scope=[p for p in counts if p.startswith(('block_behavior/','inventory/')) or p in ['player/interaction.rs','player/skin/held_item.rs']]
print(json.dumps({'total':sum(counts.values()),'gameplay':sum(counts[p] for p in scope),'scope':scope,'files':counts},indent=2))
