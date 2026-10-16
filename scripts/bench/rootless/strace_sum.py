import re,sys,collections
f=sys.argv[1]
opens=collections.Counter(); ioc=collections.Counter(); socks=collections.Counter(); other=collections.Counter(); mm=collections.Counter(); fails=collections.Counter()
for l in open(f,errors='replace'):
    m=re.match(r'\d+ \S+ (\w+)\((.*)',l)
    if not m: continue
    sc,rest=m.groups()
    err=re.search(r'= -1 (E\w+)',l)
    if sc in('openat','open'):
        p=re.search(r'"([^"]*)"',rest)
        if p:
            path=p.group(1)
            if path.startswith(('/lib','/usr/lib','/etc/ld','/proc/self','/sys/devices/system/cpu','/usr/share/locale')) : continue
            opens[(path,err.group(1) if err else 'ok')]+=1
    elif sc=='ioctl':
        m2=re.match(r'(\d+)<([^>]*)>, (\w+|_IO[CRW]*\([^)]*\))',rest)
        if m2:
            fd,path,req=m2.groups()
            if 'kvm' in path and req.startswith('KVM_RUN'): req='KVM_RUN'
            if path.startswith('/dev/nvidia') or 'udmabuf' in path or 'dri' in path or 'vfio' in path:
                ioc[(path,req if not req.startswith('_IOC') else req)]+=1
            elif 'kvm' in path or 'anon_inode' in path:
                ioc[(re.sub(r'\[.*','',path) if 'anon' in path else path, req.split('(')[0] if not req.startswith('KVM') else req)]+=1
            else: ioc[(path[:30],req[:50])]+=1
        if err: fails[('ioctl',err.group(1),rest[:90])]+=1
    elif sc in('socket','connect','bind'):
        socks[(sc,rest[:100],err.group(1) if err else 'ok')]+=1
    elif sc=='mmap':
        if 'MAP_ANONYMOUS' not in rest:
            p=re.search(r'(\d+)<([^>]*)>',rest)
            if p and p.group(2).startswith('/dev'): mm[(p.group(2),re.search(r'PROT_\S+',rest).group(0),re.search(r'MAP_\w+',rest).group(0))]+=1
        if 'MAP_LOCKED' in rest or 'MAP_HUGETLB' in rest: other[('mmap-flag',rest[:120])]+=1
    else:
        other[(sc,rest[:110])]+=1
print("== OPENS (path,result): count")
for k,v in sorted(opens.items()): print(v,k)
print("== IOCTLS (fd path, request): count")
for k,v in sorted(ioc.items(),key=lambda x:(x[0][0],-x[1])): print(v,k)
print("== /dev mmaps")
for k,v in sorted(mm.items()): print(v,k)
print("== sockets"); 
for k,v in sorted(socks.items()): print(v,k)
print("== other syscalls")
for k,v in sorted(other.items()): print(v,k)
print("== failing ioctls")
for k,v in sorted(fails.items(),key=lambda x:-x[1])[:30]: print(v,k)
