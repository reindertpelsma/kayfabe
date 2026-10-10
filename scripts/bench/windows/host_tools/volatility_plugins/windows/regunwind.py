# Diagnosis helper, host-only, never committed: real x64 unwind of waiting kernel threads from a memory image.
import struct, bisect
from typing import List
from volatility3.framework import interfaces, renderers, exceptions, constants
from volatility3.framework.configuration import requirements
from volatility3.plugins.windows import modules, pslist, ssdt

def canon(v):
    return v | (0xFFFF << 48) if v & (1 << 47) else v

class Mod:
    def __init__(self, layer, base, size, name):
        self.base, self.size, self.name = base, size, name
        self.rf = None
        try:
            hdr = layer.read(base, 0x1000, pad=True)
            nt = struct.unpack_from("<I", hdr, 0x3C)[0]
            opt = nt + 24
            dd = opt + 112
            rva, sz = struct.unpack_from("<II", hdr, dd + 3 * 8)
            if rva and sz:
                raw = layer.read(base + rva, sz, pad=True)
                n = sz // 12
                ent = struct.unpack("<%dI" % (n * 3), raw[: n * 12])
                self.rf = [(ent[3*i], ent[3*i+1], ent[3*i+2]) for i in range(n) if ent[3*i]]
                self.begins = [e[0] for e in self.rf]
        except Exception:
            self.rf = None
    def find(self, rva):
        if not self.rf: return None
        i = bisect.bisect_right(self.begins, rva) - 1
        if i >= 0 and rva < self.rf[i][1]: return self.rf[i]

class RegUnwind(interfaces.plugins.PluginInterface):
    _required_framework_version = (2, 0, 0)
    _version = (1, 0, 0)

    @classmethod
    def get_requirements(cls) -> List[interfaces.configuration.RequirementInterface]:
        return [requirements.ModuleRequirement(name="kernel", description="Windows kernel", architectures=["Intel32", "Intel64"]),
                requirements.VersionRequirement(name="modules", component=modules.Modules, version=(3, 0, 0)),
                requirements.VersionRequirement(name="pslist", component=pslist.PsList, version=(3, 0, 0)),
                requirements.VersionRequirement(name="ssdt", component=ssdt.SSDT, version=(2, 0, 0))]

    def _gen(self):
        ctx = self.context; kname = self.config["kernel"]; kernel = ctx.modules[kname]
        layer = ctx.layers[kernel.layer_name]
        coll = ssdt.SSDT.build_module_collection(ctx, kname)
        mods = []
        for m in modules.Modules.list_modules(ctx, kname):
            try: mods.append(Mod(layer, canon(int(m.DllBase)), int(m.SizeOfImage), m.BaseDllName.get_string()))
            except Exception: pass
        mods.sort(key=lambda m: m.base); bases = [m.base for m in mods]
        def modof(a):
            i = bisect.bisect_right(bases, a) - 1
            if i >= 0 and a < mods[i].base + mods[i].size: return mods[i]
        symcache = {}
        def sym(a, m):
            if a in symcache: return symcache[a]
            s = ""
            try:
                for mn, sl in coll.get_module_symbols_by_absolute_location(a):
                    best = None
                    for x in sl:
                        if best is None or x[1] > best[1]: best = x
                    if best: s = best[0].split("!")[-1]; break
            except Exception: pass
            symcache[a] = s; return s
        def u64(addr):
            return struct.unpack("<Q", layer.read(addr, 8, pad=True))[0]
        def unwind_one(rip, rsp, regs):
            m = modof(rip)
            if m is None: return None
            rf = m.find(rip - 1 - m.base)
            if rf is None:
                return u64(rsp), rsp + 8
            while True:
                uw = layer.read(m.base + rf[2] & ~3 if False else m.base + (rf[2] & ~1), 4, pad=True)
                ver_flags, prolog, ncodes, frame = struct.unpack("<BBBB", uw)
                flags = ver_flags >> 3
                codes = layer.read(m.base + (rf[2] & ~1) + 4, ncodes * 2, pad=True)
                i = 0; fp_off = None
                while i < ncodes:
                    off, opinfo = codes[2*i], codes[2*i+1]
                    op, info = opinfo & 0xF, opinfo >> 4
                    if op == 0: regs[info] = u64(rsp); rsp += 8; i += 1
                    elif op == 1:
                        if info == 0: rsp += struct.unpack_from("<H", codes, 2*i+2)[0] * 8; i += 2
                        else: rsp += struct.unpack_from("<I", codes, 2*i+2)[0]; i += 3
                    elif op == 2: rsp += info * 8 + 8; i += 1
                    elif op == 3:
                        fr = frame & 0xF; fo = (frame >> 4) * 16
                        if regs.get(fr) is None: return None
                        rsp = regs[fr] - fo; i += 1
                    elif op == 4: regs[info] = u64(rsp + struct.unpack_from("<H", codes, 2*i+2)[0] * 8); i += 2
                    elif op == 5: regs[info] = u64(rsp + struct.unpack_from("<I", codes, 2*i+2)[0]); i += 3
                    elif op in (6, 7): i += 1
                    elif op == 8: i += 2
                    elif op == 9: i += 3
                    elif op == 10:
                        rsp = u64(rsp + 24) if info else u64(rsp + 24); return u64(rsp), rsp + 8
                    else: return None
                if flags & 4:
                    n = ncodes + (ncodes & 1)
                    rfx = struct.unpack("<III", layer.read(m.base + (rf[2] & ~1) + 4 + n * 2, 12, pad=True)); rf = rfx; continue
                break
            return u64(rsp), rsp + 8
        import os
        pairs = [tuple(int(x, 16) for x in p.split(":")) for p in os.environ["UNW"].split(",")]
        thr = []
        for proc in pslist.PsList.list_processes(ctx, kname):
            try:
                pid = int(proc.UniqueProcessId); name = str(proc.ImageFileName.cast("string", max_length=15, errors="replace"))
                for t in proc.ThreadListHead.to_list(kernel.symbol_table_name + constants.BANG + "_ETHREAD", "ThreadListEntry"):
                    try: thr.append((canon(int(t.Tcb.StackLimit)), canon(int(t.Tcb.InitialStack)), pid, name, int(t.Cid.UniqueThread), int(t.Tcb.State)))
                    except Exception: pass
            except exceptions.InvalidAddressException: pass
        for rip, rsp in pairs:
            own = [x for x in thr if x[0] <= rsp < x[1]]
            frames = []; regs = {}
            for _ in range(48):
                m = modof(rip)
                if m is None: frames.append("?%x" % rip); break
                frames.append("%s+0x%x" % (m.name, rip - m.base))
                r = unwind_one(rip, rsp, regs)
                if r is None: frames.append("?unwind"); break
                rip, rsp = r
                if rip < 0xFFFF800000000000: break
            yield (0, (own[0][2] if own else -1, own[0][3] if own else "", own[0][4] if own else -1, own[0][5] if own else -1, " < ".join(frames)))

    def run(self):
        return renderers.TreeGrid([("PID", int), ("Process", str), ("TID", int), ("State", int), ("Stack", str)], self._gen())
