"""Resolve vanilla block model inheritance into reusable local quad geometry."""
import json
import math
from vanilla_special import import_special_models

DIRECTIONS = {'west':(-1,0,0),'east':(1,0,0),'down':(0,-1,0),
              'up':(0,1,0),'north':(0,0,-1),'south':(0,0,1)}

def deduplicate_models(data):
    """Share identical assemblies across waxed variants and inherited aliases."""
    fingerprints = {}
    remap = {}
    output = {}
    def application(apply):
        for variant in apply if isinstance(apply, list) else [apply]:
            name = variant['model']
            if name not in remap:
                quads = data['models'][name]
                fingerprint = json.dumps(quads, sort_keys=True, separators=(',', ':'))
                canonical = fingerprints.setdefault(fingerprint, name)
                output[canonical] = quads
                remap[name] = canonical
            variant['model'] = remap[name]
    for block in data['blocks'].values():
        for apply in block.get('variants', {}).values():application(apply)
        for part in block.get('multipart', []):application(part['apply'])
    data['models'] = output
    data['invisible_models'] = sorted({remap[name] for name in data['invisible_models'] if name in remap})

def rotate(point, origin, axis, angle, rescale=False):
    p = [point[i]-origin[i] for i in range(3)]
    a = 'xyz'.index(axis); u=(a+1)%3; v=(a+2)%3
    c=math.cos(math.radians(angle)); s=math.sin(math.radians(angle))
    p[u],p[v]=c*p[u]-s*p[v],s*p[u]+c*p[v]
    if rescale:
        p[u]/=abs(c); p[v]/=abs(c)
    return [round(p[i]+origin[i],7) for i in range(3)]

def import_models(jar):
    cache={}; models={}; states={}; textures=set()
    names=set(jar.namelist())
    def model(name):
        name=name.removeprefix('minecraft:')
        if name in cache:return cache[name]
        path=f'assets/minecraft/models/{name}.json'
        if path not in names:return {}
        own=json.loads(jar.read(path)); parent=model(own.get('parent','')) if own.get('parent') else {}
        resolved={**parent,**own,'textures':{**parent.get('textures',{}),**own.get('textures',{})}}
        cache[name]=resolved
        return resolved
    def bake(name):
        if name in models:return
        data=model(name); quads=[]
        for element in data.get('elements',[]):
            lo=element['from']; hi=element['to']
            for direction,face in element.get('faces',{}).items():
                normal=list(DIRECTIONS[direction]); axis=next(i for i,x in enumerate(normal) if x)
                u=(axis+1)%3; v=(axis+2)%3; pos=[*lo]; pos[axis]=hi[axis] if normal[axis]>0 else lo[axis]
                points=[]
                for cu,cv in [(0,0),(1,0),(1,1),(0,1)]:
                    p=pos.copy();p[u]=hi[u] if cu else lo[u];p[v]=hi[v] if cv else lo[v];points.append(p)
                # The quad order above is positive along the face axis.
                if normal[axis]<0:points=[points[i] for i in [0,3,2,1]]
                uv=face.get('uv')
                if uv is None:
                    uv={'down':[lo[0],16-hi[2],hi[0],16-lo[2]],'up':[lo[0],lo[2],hi[0],hi[2]],
                        'north':[16-hi[0],16-hi[1],16-lo[0],16-lo[1]],'south':[lo[0],16-hi[1],hi[0],16-lo[1]],
                        'west':[lo[2],16-hi[1],hi[2],16-lo[1]],'east':[16-hi[2],16-hi[1],16-lo[2],16-lo[1]]}[direction]
                # Derive the canonical UV at each physical corner before rotations.
                def corner_uv(p):
                    if direction=='down':s,t=p[0],16-p[2];bounds=(lo[0],hi[0],16-hi[2],16-lo[2])
                    elif direction=='up':s,t=p[0],p[2];bounds=(lo[0],hi[0],lo[2],hi[2])
                    elif direction=='north':s,t=16-p[0],16-p[1];bounds=(16-hi[0],16-lo[0],16-hi[1],16-lo[1])
                    elif direction=='south':s,t=p[0],16-p[1];bounds=(lo[0],hi[0],16-hi[1],16-lo[1])
                    elif direction=='west':s,t=p[2],16-p[1];bounds=(lo[2],hi[2],16-hi[1],16-lo[1])
                    else:s,t=16-p[2],16-p[1];bounds=(16-hi[2],16-lo[2],16-hi[1],16-lo[1])
                    a,b,c,d=bounds
                    return [(uv[0]+(uv[2]-uv[0])*((s-a)/(b-a) if b!=a else 0))/16,
                            (uv[1]+(uv[3]-uv[1])*((t-c)/(d-c) if d!=c else 0))/16]
                coords=[corner_uv(p) for p in points]
                n=face.get('rotation',0)//90;coords=coords[n:]+coords[:n]
                if 'rotation' in element:
                    r=element['rotation']
                    rotations=[(r['axis'],r['angle'])] if 'axis' in r else [(a,r.get(a,0)) for a in 'xyz']
                    for axis,angle in rotations:
                        points=[rotate(p,r['origin'],axis,angle,r.get('rescale',False)) for p in points]
                        normal=rotate(normal,[0,0,0],axis,angle)
                ref=face['texture'];seen=set();translucent=False
                if isinstance(ref,str) and ref in data.get('textures',{}):ref='#'+ref
                while isinstance(ref,str) and ref.startswith('#'):
                    if ref in seen:raise ValueError(f'cyclic texture in {name}')
                    seen.add(ref)
                    if ref[1:] not in data.get('textures',{}):raise ValueError(f'unresolved texture {ref} in {name}')
                    ref=data['textures'][ref[1:]]
                    if isinstance(ref,dict):translucent|=ref.get('force_translucent',False);ref=ref['sprite']
                if isinstance(ref,dict):translucent|=ref.get('force_translucent',False);ref=ref['sprite']
                texture=ref.removeprefix('minecraft:').removeprefix('block/')
                if f'assets/minecraft/textures/block/{texture}.png' not in names:
                    raise ValueError(f'missing model texture {texture} in {name}')
                textures.add(texture)
                quads.append({'positions':[[round(c/16,7) for c in p] for p in points], 'normal':normal,
                              'uv':coords,'texture':texture,'tinted':'tintindex' in face,
                              'tintindex':face.get('tintindex',0),
                              'cull':list(DIRECTIONS.get(face.get('cullface'),(0,0,0))),
                              'translucent':translucent or texture in ('ice','frosted_ice_0','frosted_ice_1','frosted_ice_2','frosted_ice_3'),
                              'shade':element.get('shade_direction_override',''),
                              'light_emission':element.get('light_emission',0), 'effect':0})
        models[name]=quads
    for path in sorted(names):
        if not path.startswith('assets/minecraft/blockstates/') or not path.endswith('.json'):continue
        block=path.rsplit('/',1)[1][:-5]; data=json.loads(jar.read(path));states[block]=data
        applies=list(data.get('variants',{}).values())+[p['apply'] for p in data.get('multipart',[])]
        for apply in applies:
            for variant in apply if isinstance(apply,list) else [apply]:bake(variant['model'])
    bind = import_special_models(jar, models, states, textures)
    # Fluids and effects use the same surface format; the worker adjusts fluid
    # heights and culls interfaces against the sampled section neighborhood.
    def box_model(name, lo, hi, texture, effect=0, emission=0, translucent=False):
        cache[name] = {'elements': [{'from': lo, 'to': hi, 'light_emission': emission,
            'faces': {direction: {'texture': texture, 'cullface': direction}
                      for direction in DIRECTIONS}}]}
        bake(name)
        for q in models[name]:
            q['effect'] = effect
            q['translucent'] = translucent
        return models[name]
    bind('lava', {'': box_model('hollow/fluid/lava', [0,0,0], [16,16,16], 'minecraft:block/lava_still', emission=15)})
    # Retain a particle/default surface for icon consumers; terrain water is
    # emitted by the shared fluid path, independently of block model recipes.
    bind('water', {'': box_model('hollow/fluid/water', [0,0,0], [16,16,16], 'minecraft:block/water_still', translucent=True)})
    for block, bottom, height in [('end_portal', 6, 12), ('end_gateway', 0, 16)]:
        quads = box_model('hollow/effect/' + block, [0,bottom,0], [16,height,16], 'minecraft:block/obsidian', effect=10, emission=15)
        for q in quads:
            q['texture'] = '@entity/end_portal/end_portal'
        textures.add('@entity/end_portal/end_portal')
        bind(block, {'': quads})
    # Empty models are legal only when deliberately invisible. In particular,
    # pitcher upper halves at ages 0-2 have no visible upper geometry in vanilla.
    invisible = {'minecraft:block/' + name for name in
                 ['air', 'barrier', 'structure_void', 'moving_piston',
                  'pitcher_crop_top_stage_0', 'pitcher_crop_top_stage_1', 'pitcher_crop_top_stage_2']}
    invisible.update(f'minecraft:block/light_{i:02}' for i in range(16))
    invisible.add('minecraft:block/water')  # bubble columns use the fluid surface.
    used = set()
    for block, data in states.items():
        applies = list(data.get('variants', {}).values()) + [p['apply'] for p in data.get('multipart', [])]
        for apply in applies:
            for variant in apply if isinstance(apply, list) else [apply]:
                name = variant['model']
                if name not in models or (not models[name] and name not in invisible):
                    raise ValueError(f'unresolved visible model {name} for {block}')
                used.add(name)
    models = {name: models[name] for name in sorted(used)}
    textures = {q['texture'] for quads in models.values() for q in quads}
    return {'version':2,'blocks':states,'models':models,
            'invisible_models':sorted(invisible & used)},textures
