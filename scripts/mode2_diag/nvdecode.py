#!/usr/bin/env python3
"""nvdecode.py — decode + diff nvtrace.c output (semantic NVIDIA ioctl traces).

Decodes the raw ptrace trace from nvtrace.c into named ops with:
  - escape/opcode + control-cmd -> enum name (parsed from the open SDK headers),
  - the NVOS struct fields, AND the inner PARAMS struct decoded FIELD-BY-FIELD
    (struct typedefs parsed from the SDK headers, array sizes resolved from #defines),
  - fd->path + handle canonicalization,
  - a host-vs-guest DIFF that compares decoded fields (not hex), so only semantic
    divergences surface (e.g. gpuId g=0x10000 h=0x7).

Usage:
  nvdecode.py decode <trace.txt> [--ogkm DIR]
  nvdecode.py diff <host.txt> <guest.txt> [--ogkm DIR]
"""
import sys, os, re, struct, glob, functools

ESC = {0x27:"RM_ALLOC_MEMORY",0x28:"RM_ALLOC_OBJECT",0x29:"RM_FREE",0x2A:"RM_CONTROL",
       0x2B:"RM_ALLOC",0x4A:"RM_VID_HEAP_CONTROL",0x4E:"RM_MAP_MEMORY",0x57:"RM_MAP_MEMORY_DMA",
       0x4D:"RM_UNMAP_MEMORY",0x5E:"RM_DUP_OBJECT"}

# base type -> (size, align, signed, is_ptr)
TYPES = {
 "NvU8":(1,1),"NvS8":(1,1),"NvBool":(1,1),"NvV8":(1,1),"char":(1,1),
 "NvU16":(2,2),"NvS16":(2,2),"NvV16":(2,2),
 "NvU32":(4,4),"NvS32":(4,4),"NvV32":(4,4),"NvHandle":(4,4),"NvBool32":(4,4),
 "NvU64":(8,8),"NvS64":(8,8),"NvP64":(8,8),"NvLength":(8,8),"void*":(8,8),
}

@functools.lru_cache(maxsize=1)
def _hdr_text(ogkm):
    base = os.path.join(ogkm, "src/common/sdk/nvidia/inc")
    txt = []
    for f in glob.glob(base + "/**/*.h", recursive=True):
        try: txt.append(open(f, errors='replace').read())
        except OSError: pass
    return "\n".join(txt)

@functools.lru_cache(maxsize=1)
def build_cmd_names(ogkm):
    names = {}
    pat = re.compile(r'#define\s+(NV[0-9A-Za-z_]+)\s+\(?(0x[0-9a-fA-F]+)')
    for ln in _hdr_text(ogkm).splitlines():
        m = pat.search(ln)
        if m and "_CTRL_CMD_" in m.group(1):
            try: names[int(m.group(2),16)] = m.group(1)
            except ValueError: pass
    return names

@functools.lru_cache(maxsize=1)
def build_defines(ogkm):
    d = {}
    pat = re.compile(r'#define\s+([A-Z0-9_]+)\s+\(?(0x[0-9a-fA-F]+|\d+)\)?\s*$')
    for ln in _hdr_text(ogkm).splitlines():
        m = pat.search(ln)
        if m:
            try: d[m.group(1)] = int(m.group(2),0)
            except ValueError: pass
    return d

@functools.lru_cache(maxsize=1)
def build_structs(ogkm):
    """name -> list[(ctype, fieldname, count)] for typedef struct NAME {..} NAME;"""
    txt = _hdr_text(ogkm); structs = {}
    for m in re.finditer(r'typedef\s+struct\s+(\w+)?\s*\{(.*?)\}\s*(\w+)\s*;', txt, re.S):
        name = m.group(3); body = m.group(2)
        fields = []
        for raw in body.split(';'):
            s = raw.strip()
            if not s or s.startswith('//') or s.startswith('#'): continue
            s = re.sub(r'/\*.*?\*/', '', s, flags=re.S).strip()
            s = s.replace("NV_DECLARE_ALIGNED(", "").rstrip(")")
            s = re.sub(r',\s*\d+\s*$', '', s)            # drop NV_DECLARE_ALIGNED align arg
            s = re.sub(r'NV_ALIGN_BYTES\(\d+\)', '', s).strip()
            if not s: continue
            mm = re.match(r'(?:const\s+)?([A-Za-z_]\w*)\s*\*?\s*([A-Za-z_]\w*)\s*(\[[^\]]*\])*$', s)
            if not mm: continue
            ctype, fname = mm.group(1), mm.group(2)
            count = 1
            for ar in re.findall(r'\[([^\]]+)\]', s):
                ar = ar.strip()
                count *= (int(ar,0) if re.match(r'^(0x[0-9a-fA-F]+|\d+)$', ar) else -1)
            fields.append((ctype, fname, count))
        if name: structs[name] = fields
    return structs

def _align(off, a): return (off + a - 1) & ~(a - 1)

def sizeof_struct(structs, name, _seen=None):
    _seen = _seen or set()
    if name in _seen or name not in structs: return None
    _seen = _seen | {name}; off = 0; maxa = 1
    for ctype, fname, count in structs[name]:
        if ctype in TYPES: sz, al = TYPES[ctype]
        elif ctype in structs:
            sz = sizeof_struct(structs, ctype, _seen); al = 8
            if sz is None: return None
        else: return None
        if count < 0: return None
        off = _align(off, al) + sz * count; maxa = max(maxa, al)
    return _align(off, maxa)

def decode_struct(structs, defines, name, data, ogkm, _off=0):
    """return list of (fieldname, valuestr) decoding `data` as struct `name`."""
    if name not in structs: return None
    out = []; off = _off
    for ctype, fname, count in structs[name]:
        if ctype in TYPES:
            sz, al = TYPES[ctype]; off = _align(off, al)
            if count == 1:
                if off + sz <= len(data):
                    v = int.from_bytes(data[off:off+sz], 'little')
                    out.append((fname, f"0x{v:x}" if v>9 else str(v)))
                off += sz
            else:
                n = count if count>0 else 0
                if 0 < n <= 16 and off + sz*n <= len(data):
                    vals = [int.from_bytes(data[off+i*sz:off+(i+1)*sz],'little') for i in range(n)]
                    out.append((fname+"[]", "["+",".join(hex(v) for v in vals[:8])+("..." if n>8 else "")+"]"))
                off += sz * (n if n>0 else 0)
        elif ctype in structs:
            sub = sizeof_struct(structs, ctype)
            if sub is None: return out  # bail on unknown nested
            off = _align(off, 8)
            out.append((fname, f"<{ctype}>"))
            off += sub * (count if count>0 else 1)
        else:
            return out
    return out

def cmd_to_struct(cmdname):
    # NV0000_CTRL_CMD_GPU_GET_ID_INFO -> NV0000_CTRL_GPU_GET_ID_INFO_PARAMS
    return cmdname.replace("_CTRL_CMD_", "_CTRL_") + "_PARAMS"

def parse(path):
    evs = []
    for ln in open(path, errors='replace'):
        ln = ln.rstrip("\n")
        if ln.startswith("OPEN"):
            m = re.search(r'fd=(\d+) path=(\S+)', ln)
            if m: evs.append(("OPEN", int(m.group(1)), m.group(2)))
        elif ln.startswith("MMAP"):
            evs.append(("MMAP", dict(re.findall(r'(\w+)=(\S+)', ln))))
        elif ln.startswith("IOCTL"):
            evs.append(("IOCTL", dict(re.findall(r'(\w+)=(\S+)', ln))))
    return evs

def u32(b,o): return struct.unpack_from('<I', b, o)[0] if len(b)>=o+4 else 0

def decode_ioctl(d, ogkm):
    names = build_cmd_names(ogkm); structs = build_structs(ogkm); defines = build_defines(ogkm)
    nr = int(d.get("nr","0x0"),16); hdr = bytes.fromhex(d.get("hdr","")); pb = bytes.fromhex(d.get("p",""))
    rec = {"esc":ESC.get(nr,f"nr_0x{nr:02x}"),"nr":nr,"ret":d.get("ret"),"fd":d.get("fd"),
           "path":d.get("path","?"),"psz":int(d.get("psz","0")),"params":d.get("p","")}
    if nr == 0x2A and len(hdr)>=32:
        cmd=u32(hdr,8); cn=names.get(cmd,f"0x{cmd:08x}")
        rec.update(kind="CTRL",hClient=u32(hdr,0),hObject=u32(hdr,4),cmd=cmd,cmdname=cn,
                   paramsSize=u32(hdr,24),status=u32(hdr,28))
        sn = cmd_to_struct(cn) if cn.startswith("NV") else None
        rec["fields"] = decode_struct(structs,defines,sn,pb,ogkm) if sn else None
        rec["struct"] = sn
    elif nr == 0x2B and len(hdr)>=32:
        rec.update(kind="ALLOC",hParent=u32(hdr,4),hObjectNew=u32(hdr,8),hClass=u32(hdr,12),
                   paramsSize=u32(hdr,24),status=u32(hdr,28))
    else:
        rec.update(kind=rec["esc"], hdr=hdr.hex())
    return rec

def canon():
    m={}; n=[1]
    def c(h):
        if h in (0,0xffffffff): return str(h)
        if h not in m: m[h]=n[0]; n[0]+=1
        return f"H{m[h]}"
    return c

def fmt(r,c):
    if r.get("kind")=="CTRL":
        base=f"CTRL {r['cmdname']:<46} cli={c(r['hClient'])} obj={c(r['hObject'])} psz={r['paramsSize']} st=0x{r['status']:x}"
        if r.get("fields"):
            base += "\n      " + " ".join(f"{n}={v}" for n,v in r["fields"][:12])
        return base
    if r.get("kind")=="ALLOC":
        return f"ALLOC class=0x{r['hClass']:08x} parent={c(r['hParent'])} new={c(r['hObjectNew'])} psz={r['paramsSize']} st=0x{r['status']:x}"
    return f"{r.get('esc')} fd={r.get('fd')} ret={r.get('ret')}"

def cmd_decode(path, ogkm):
    c=canon()
    for e in parse(path):
        if e[0]=="OPEN": print(f"OPEN  fd={e[1]} {e[2]}")
        elif e[0]=="MMAP":
            d=e[1]; print(f"MMAP  {d.get('path','?')} addr={d.get('addr')} len={d.get('len')} off={d.get('off')} ret={d.get('ret')}")
        else: print(fmt(decode_ioctl(e[1], ogkm), c))

def cmd_diff(hp, gp, ogkm):
    def load(p):
        seq=[]
        for e in parse(p):
            if e[0]=="IOCTL": seq.append(decode_ioctl(e[1], ogkm))
        return seq
    H,G=load(hp),load(gp)
    print(f"host ioctls={len(H)} guest ioctls={len(G)}")
    from collections import defaultdict
    hidx=defaultdict(list)
    for r in H: hidx[(r.get("kind"),r.get("cmd",r.get("hClass",r.get("esc"))))].append(r)
    seen=defaultdict(int); ndiff=0
    print("\n=== FIELD-LEVEL divergences (host vs guest) ===")
    for r in G:
        k=(r.get("kind"),r.get("cmd",r.get("hClass",r.get("esc")))); occ=seen[k]; seen[k]+=1
        hl=hidx.get(k,[])
        if occ>=len(hl): continue
        h=hl[occ]; diffs=[]
        if r.get("status")!=h.get("status"): diffs.append(f"status g=0x{r.get('status',0):x} h=0x{h.get('status',0):x}")
        if r.get("paramsSize")!=h.get("paramsSize"): diffs.append(f"psz g={r.get('paramsSize')} h={h.get('paramsSize')}")
        # field-by-field (decoded), skip pointer-looking values
        gf=dict(r.get("fields") or []); hf=dict(h.get("fields") or [])
        for fn in gf:
            gv,hv=gf[fn],hf.get(fn)
            if hv is not None and gv!=hv:
                try:
                    if int(gv,16)>>40==0x7f or int(hv,16)>>40==0x7f: continue  # ptr
                except (ValueError,TypeError): pass
                diffs.append(f"{fn} g={gv} h={hv}")
        if diffs:
            ndiff+=1
            if ndiff<=50:
                nm=r.get("cmdname", f"class=0x{r.get('hClass',0):08x}" if r.get('kind')=='ALLOC' else r.get('esc'))
                print(f"  {r.get('kind'):5} {nm:<44} " + " | ".join(diffs[:6]))
    gcnt=defaultdict(int)
    for r in G: gcnt[(r.get('kind'),r.get('cmd',r.get('hClass',r.get('esc'))))]+=1
    ho=[r for k,l in hidx.items() for j,r in enumerate(l) if j>=gcnt[k]]
    print(f"\n{ndiff} field divergences; {len(ho)} host-only ioctls (guest stopped early). first host-only:")
    cc=canon()
    for r in ho[:8]: print("  "+fmt(r,cc).split(chr(10))[0])

if __name__=="__main__":
    ogkm="/workspace/nvidia-gpu-passthrough/research_clones/ogkm"
    for i,x in enumerate(sys.argv):
        if x=="--ogkm": ogkm=sys.argv[i+1]
    a=[x for x in sys.argv[1:] if not x.startswith("--") and sys.argv[sys.argv.index(x)-1]!="--ogkm"]
    if not a: print(__doc__); sys.exit(1)
    if a[0]=="decode": cmd_decode(a[1], ogkm)
    elif a[0]=="diff" and len(a)>=3: cmd_diff(a[1], a[2], ogkm)
    else: print(__doc__)
