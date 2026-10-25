"""Resolve vanilla block model inheritance into reusable local quad geometry."""
import json
import math

DIRECTIONS = {'west':(-1,0,0),'east':(1,0,0),'down':(0,-1,0),
              'up':(0,1,0),'north':(0,0,-1),'south':(0,0,1)}

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
                while isinstance(ref,str) and ref.startswith('#'):
                    if ref in seen:raise ValueError(f'cyclic texture in {name}')
                    seen.add(ref);ref=data.get('textures',{}).get(ref[1:],'minecraft:block/stone')
                    if isinstance(ref,dict):translucent|=ref.get('force_translucent',False);ref=ref['sprite']
                if isinstance(ref,dict):translucent|=ref.get('force_translucent',False);ref=ref['sprite']
                texture=ref.removeprefix('minecraft:').removeprefix('block/')
                if f'assets/minecraft/textures/block/{texture}.png' not in names: texture='stone'
                textures.add(texture)
                quads.append({'positions':[[round(c/16,7) for c in p] for p in points], 'normal':normal,
                              'uv':coords,'texture':texture,'tinted':'tintindex' in face,
                              'cull':list(DIRECTIONS.get(face.get('cullface'),(0,0,0))),
                              'translucent':translucent or texture in ('ice','frosted_ice_0','frosted_ice_1','frosted_ice_2','frosted_ice_3'),
                              'shade':element.get('shade_direction_override','')})
        models[name]=quads
    for path in sorted(names):
        if not path.startswith('assets/minecraft/blockstates/') or not path.endswith('.json'):continue
        block=path.rsplit('/',1)[1][:-5]; data=json.loads(jar.read(path));states[block]=data
        applies=list(data.get('variants',{}).values())+[p['apply'] for p in data.get('multipart',[])]
        for apply in applies:
            for variant in apply if isinstance(apply,list) else [apply]:bake(variant['model'])
    return {'version':1,'blocks':states,'models':models},textures
