import sys,struct
for p in sys.argv[1:]:
    b=open(p,'rb').read()
    hz,t0=struct.unpack_from('<QQ',b,40)
    off=128; n=0; kinds={}; out=[]; last=None
    while off+32<=len(b):
        assert b[off:off+4]==b'KREC',(p,off)
        kind,d,qid,pl,cnt,ts,fe,ec=struct.unpack_from('<BBHIIQII',b,off+4)
        n+=1; kinds[kind]=kinds.get(kind,0)+1
        if kind in (1,3,4,5,6): out.append((kind,d,qid,'t=%.3fs'%((ts-t0)/hz)))
        if kind==2: last=(ts-t0)/hz
        off+=32+pl
        off=(off+7)&~7 if False else off
    print(p.split('/')[-2],'records',n,kinds,'end_off',off,'len',len(b),'last_elements_t=%.1f'%(last or 0))
    for o in out: print('  ',o)
