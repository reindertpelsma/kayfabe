import re,glob,sys,collections,os
norm=lambda s: re.sub(r'\d+','N',re.sub(r'0x[0-9a-fA-F]+','H',s))[:200]
def lines(path, pat):
    out=collections.Counter()
    for l in open(path,errors='replace'):
        if re.search(pat,l): out[norm(l.strip())]+=1
    return out
pat=r'REFUSED|refused by|kf-host: channel birth|PRIVILEGED|INSUFFICIENT|permission|EPERM|EACCES|CAP_|cap_sys_admin|host tier'
arms=sorted(set(re.sub(r'fast_rs1_|_qemu.log','',os.path.basename(p)) for p in glob.glob('/data/claude-rootless/bench/fast_rs1_*_qemu.log')))
tot=collections.Counter()
for a in arms:
    r=lines(f'/data/claude-rootless/bench/fast_rs1_{a}_qemu.log',pat)
    u=lines(f'/home/kfrootless/bench/fast_us1_{a}_qemu.log',pat)
    rk=set(r); uk=set(u)
    d_r=rk-uk; d_u=uk-rk
    # ignore posture cleared/not-held difference
    d_r={x for x in d_r if 'cap_sys_admin' not in x}; d_u={x for x in d_u if 'cap_sys_admin' not in x}
    births_r=sum(v for k,v in r.items() if 'channel birth' in k); births_u=sum(v for k,v in u.items() if 'channel birth' in k)
    usr_u=sum(1 for l in open(f'/home/kfrootless/bench/fast_us1_{a}_qemu.log',errors='replace') if 'channel birth' in l and 'PRIVILEGED_CHANNEL=0 privilege=USER' in l)
    print(f'{a:24s} births root={births_r} user={births_u} user_USER={usr_u} only_root={len(d_r)} only_user={len(d_u)}')
    for x in sorted(d_r): print('   ONLY_ROOT',x[:160])
    for x in sorted(d_u): print('   ONLY_USER',x[:160])
