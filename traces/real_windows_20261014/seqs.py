import sys,struct
# independent check of transport seqNum continuity per queue instance (580 element layout: seqNum @+36, elemCount @+40); no message bytes printed
for p in sys.argv[1:]:
    b=open(p,'rb').read(); off=128
    ents={}   # (qid,dir) -> list of bytes of entries, in order
    while off+32<=len(b):
        kind,d,qid,pl=struct.unpack_from('<BBHI',b,off+4)
        if kind==2:
            ents.setdefault((qid,d),[]).append(b[off+32:off+32+pl])
        off+=32+pl
    print(p.split('/')[-2])
    for k in sorted(ents):
        data=b''.join(ents[k]); n=len(data)//4096; i=0; seqs=[]
        while i<n:
            e=data[i*4096:i*4096+64]
            seq,ec=struct.unpack_from('<II',e,36)
            seqs.append(seq); i+=max(ec,1)
        gaps=[(seqs[j],seqs[j+1]) for j in range(len(seqs)-1) if seqs[j+1]!=seqs[j]+1]
        print('  queue_instance',k[0],'dir','cmd' if k[1]==0 else 'status','messages',len(seqs),'first_seq',seqs[0],'last_seq',seqs[-1],'seq_gaps',len(gaps),gaps[:3])
