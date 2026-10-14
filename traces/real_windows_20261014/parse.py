import sys,json
t=open(sys.argv[1]).read()
d=json.loads(t[t.find('{'):t.rfind('}')+1])
print('discovery',d['discovery']); print('stats',d['stats']); print('capture',d['capture'])
st={0:'OK',1:'UNREADABLE',2:'ZERO',3:'SUSPENDED',4:'BAD_ADDR',5:'NO_RMARGS',6:'ARGS_BAD',7:'REGION_BAD',8:'QUEUE_BAD'}
t0=None
for e in d['events']:
    h=bytes.fromhex(e['data_hex']); c=e['code']
    if c in (5,6):
        ts=int.from_bytes(h[16:24],'little')/3579545
        if t0 is None: t0=ts
        if c==5: print('ev5 gen',e['generation'],'mb_status',st[int.from_bytes(h[0:4],'little')],'pa',hex(int.from_bytes(h[8:16],'little')),'t=%.3f'%(ts-t0))
        else: print('ev6 gen',e['generation'],'readfail',int.from_bytes(h[0:4],'little'),'pair',hex(int.from_bytes(h[8:16],'little')),'t=%.3f'%(ts-t0))
    else: print('ev',c,'gen',e['generation'],h[:48].hex())
