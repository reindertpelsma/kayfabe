#!/usr/bin/env python3
"""inband.py -- decode the IN-BAND RM status of every record in an nvdiff capture."""
import io, json, os, subprocess, sys
from collections import Counter

ESC = {0x27:"NV_ESC_RM_ALLOC_MEMORY",0x28:"NV_ESC_RM_ALLOC_OBJECT",0x29:"NV_ESC_RM_FREE",
 0x2A:"NV_ESC_RM_CONTROL",0x2B:"NV_ESC_RM_ALLOC",0x32:"NV_ESC_RM_CONFIG_GET",
 0x33:"NV_ESC_RM_CONFIG_SET",0x34:"NV_ESC_RM_DUP_OBJECT",0x35:"NV_ESC_RM_SHARE",
 0x37:"NV_ESC_RM_CONFIG_GET_EX",0x38:"NV_ESC_RM_CONFIG_SET_EX",0x39:"NV_ESC_RM_I2C_ACCESS",
 0x41:"NV_ESC_RM_IDLE_CHANNELS",0x4A:"NV_ESC_RM_VID_HEAP_CONTROL",0x4D:"NV_ESC_RM_ACCESS_REGISTRY",
 0x4E:"NV_ESC_RM_MAP_MEMORY",0x4F:"NV_ESC_RM_UNMAP_MEMORY",0x52:"NV_ESC_RM_GET_EVENT_DATA",
 0x54:"NV_ESC_RM_ALLOC_CONTEXT_DMA2",0x56:"NV_ESC_RM_ADD_VBLANK_CALLBACK",
 0x57:"NV_ESC_RM_MAP_MEMORY_DMA",0x58:"NV_ESC_RM_UNMAP_MEMORY_DMA",0x59:"NV_ESC_RM_BIND_CONTEXT_DMA",
 0x5C:"NV_ESC_RM_EXPORT_OBJECT_TO_FD",0x5D:"NV_ESC_RM_IMPORT_OBJECT_FROM_FD",
 0x5E:"NV_ESC_RM_UPDATE_DEVICE_MAPPING_INFO",0x5F:"NV_ESC_RM_LOCKLESS_DIAGNOSTIC",
 0x7F:"NVDMARK (nvd_prog phase marker)",0xC8:"NV_ESC_CARD_INFO",0xC9:"NV_ESC_REGISTER_FD",
 0xCA:"NV_ESC_ALLOC_OS_EVENT_OLD",0xCE:"NV_ESC_ALLOC_OS_EVENT",0xCF:"NV_ESC_FREE_OS_EVENT",
 0xD2:"NV_ESC_CHECK_VERSION_STR",0xD3:"NV_ESC_IOCTL_XFER_CMD",0xD6:"NV_ESC_SYS_PARAMS",
 0xD8:"NV_ESC_NUMA_INFO",0xD9:"NV_ESC_SET_NUMA_STATUS",0xDA:"NV_ESC_EXPORT_TO_DMABUF_FD",
 0xDB:"NV_ESC_WAIT_OPEN_COMPLETE"}
NVSTATUS = {0x00:"NV_OK",0x1E:"NV_ERR_INSUFFICIENT_PERMISSIONS",0x30:"NV_ERR_INVALID_ARGUMENT",
 0x36:"NV_ERR_INVALID_CLASS",0x38:"NV_ERR_INVALID_DEVICE",0x3C:"NV_ERR_INVALID_LIMIT",
 0x41:"NV_ERR_INVALID_OBJECT_HANDLE",0x42:"NV_ERR_INVALID_OBJECT_NEW",0x43:"NV_ERR_INVALID_OBJECT_OLD",
 0x44:"NV_ERR_INVALID_OBJECT_PARENT",0x47:"NV_ERR_INVALID_PARAM_STRUCT",0x4A:"NV_ERR_INVALID_POINTER",
 0x51:"NV_ERR_INVALID_STATE",0x56:"NV_ERR_NOT_SUPPORTED",0x57:"NV_ERR_OBJECT_NOT_FOUND",
 0x58:"NV_ERR_OPERATING_SYSTEM",0x5F:"NV_ERR_TIMEOUT",0x65:"NV_ERR_GENERIC"}

def status_slot(nr, iocsize):
    if nr==0x27: return ("NVOS02_PARAMETERS",40)
    if nr==0x29: return ("NVOS00_PARAMETERS",12)
    if nr==0x2A: return ("NVOS54_PARAMETERS",28)
    if nr==0x2B: return ("NVOS64_PARAMETERS",40) if iocsize>=48 else ("NVOS21_PARAMETERS",28)
    if nr==0x34: return ("NVOS55_PARAMETERS",24)
    if nr==0x35: return ("NVOS57_PARAMETERS",20)
    if nr==0x4A: return ("NVOS32_PARAMETERS",20)
    if nr==0x4E: return ("NVOS33_PARAMETERS_WITH_FD",40)
    if nr==0x4F: return ("NVOS34_PARAMETERS",24)
    if nr==0x57: return ("NVOS46_PARAMETERS_V580",56) if iocsize>=64 else ("NVOS46_PARAMETERS",48)
    if nr==0x58: return ("NVOS47_PARAMETERS",40)
    if nr==0x5E: return ("NVOS56_PARAMETERS",32)
    return (None,None)

def open_text(path):
    if path.endswith(".zst"):
        try:
            import zstandard
            return io.TextIOWrapper(zstandard.ZstdDecompressor().stream_reader(open(path,"rb")),errors="replace")
        except ImportError:
            p=subprocess.run(["zstd","-dc",path],stdout=subprocess.PIPE,check=True)
            return io.StringIO(p.stdout.decode("utf-8","replace"))
    if path.endswith(".gz"):
        import gzip; return gzip.open(path,"rt",errors="replace")
    return open(path,"r",errors="replace")

def u32le(h,off): return int.from_bytes(bytes.fromhex(h[2*off:2*off+8]),"little")

def classify(rec):
    out={"state":"NoStatusField","struct":None,"off":None,"status":None,"cmd":None,
         "psize":rec.get("psize"),"pgot":None,"hdr_got":0}
    hpre=rec.get("hpre") or ""; hpost=rec.get("hpost") or ""; ppost=rec.get("ppost") or ""
    if not isinstance(hpre,str): hpre=""
    if not isinstance(hpost,str): hpost=""
    if not isinstance(ppost,str): ppost=""
    out["hdr_got"]=len(hpost)//2
    pgot=rec.get("pgot"); pb=len(ppost)//2
    out["pgot"]=(min(pgot,pb) if pb else pgot) if isinstance(pgot,int) else pb
    if rec.get("t")!="ioctl": return out
    dev=rec.get("dev");  dev=dev if isinstance(dev,str) else ""
    if dev.startswith("nvidia-uvm"): return out
    nr=rec.get("nr")
    if not isinstance(nr,int): return out
    if nr==0x2A and len(hpre)>=24:
        try: out["cmd"]=u32le(hpre,8)
        except ValueError: out["cmd"]=None
    iocsize=rec.get("iocsize"); iocsize=iocsize if isinstance(iocsize,int) else 0
    name,off=status_slot(nr,iocsize)
    if off is None: return out
    out["struct"],out["off"]=name,off
    if out["hdr_got"]<off+4:
        out["state"]="Truncated"; return out
    try: st=u32le(hpost,off)
    except ValueError:
        out["state"]="Truncated"; return out
    out["status"]=st; out["state"]="Ok" if st==0 else "Refused"
    return out

def render(rec,info):
    nr=rec.get("nr"); esc=ESC.get(nr,("nr_0x%02x"%nr) if isinstance(nr,int) else "?")
    nrs=("0x%02x"%nr) if isinstance(nr,int) else "?"; st=info["status"] or 0
    return ("seq=%-6s dev=%-12s %s (nr=%s)  %s.status@+%s\n"
      "         RM_CONTROL cmd = %s\n"
      "         paramsSize(declared) = %s   param bytes captured = %s   hdr bytes captured = %s\n"
      "         status = 0x%08x  (%d)  %s     [rc=%s errno=%s trunc=%s]")%(
      rec.get("i"),rec.get("dev"),esc,nrs,info["struct"],info["off"],
      ("0x%08x"%info["cmd"]) if info["cmd"] is not None else "n/a (not RM_CONTROL)",
      info["psize"],info["pgot"],info["hdr_got"],st,st,
      NVSTATUS.get(st,"unknown NV_STATUS"),rec.get("rc"),rec.get("errno"),rec.get("trunc",0))

def main(argv):
    if len(argv)!=2:
        sys.stderr.write("usage: inband.py <capture.jsonl[.gz|.zst]>\n"); return 2
    path=argv[1]
    if not os.path.exists(path):
        sys.stderr.write("no such capture: %s\n"%path); return 2
    try: fh=open_text(path)
    except Exception as exc:
        sys.stderr.write("cannot open %s: %r\n"%(path,exc)); return 2
    total=0; bad=0; hist=Counter(); refused=[]
    with fh:
        while True:
            try: line=fh.readline()
            except Exception: bad+=1; break
            if not line: break
            line=line.strip()
            if not line: continue
            total+=1
            try:
                rec=json.loads(line)
                if not isinstance(rec,dict): raise ValueError("not an object")
                info=classify(rec)
            except Exception:
                bad+=1; hist["BadLine"]+=1; continue
            hist[info["state"]]+=1
            if info["state"]=="Refused": refused.append((rec,info))
    print("capture : %s"%path); print("total records : %d"%total)
    print("bad/unparsable lines : %d"%bad); print("")
    print("STATE HISTOGRAM  (four states, never a bool)")
    for k in ("Ok","Refused","NoStatusField","Truncated","BadLine"):
        print("   %-14s %d"%(k,hist.get(k,0)))
    if hist.get("Truncated"):
        print("   !! Truncated records are UNMEASURED, not Ok.")
    print("")
    print("REFUSED RECORDS  (in-band non-zero NV status): %d"%len(refused))
    if not refused: print("   (none)")
    for rec,info in refused: print("   "+render(rec,info))
    print(""); print("="*78); print("LAST-REFUSED"); print("="*78)
    if refused: print("   "+render(*refused[-1]))
    else: print("   (none -- no record decoded to Refused)")
    return 0

if __name__=="__main__": sys.exit(main(sys.argv))
