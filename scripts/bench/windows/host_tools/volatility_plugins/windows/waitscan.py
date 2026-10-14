# Diagnosis helper, host-only, never committed: waiting threads of a Windows memory image with a stack-scan of module return addresses.
import logging, struct, bisect
from typing import List
from volatility3.framework import interfaces, renderers, exceptions
from volatility3.framework.configuration import requirements
from volatility3.plugins.windows import thrdscan, modules

class WaitScan(interfaces.plugins.PluginInterface):
    _required_framework_version = (2, 0, 0)
    _version = (1, 0, 0)

    @classmethod
    def get_requirements(cls) -> List[interfaces.configuration.RequirementInterface]:
        return [requirements.ModuleRequirement(name="kernel", description="Windows kernel", architectures=["Intel32", "Intel64"]),
                requirements.VersionRequirement(name="thrdscan", component=thrdscan.ThrdScan, version=(2, 0, 0)),
                requirements.VersionRequirement(name="modules", component=modules.Modules, version=(3, 0, 0))]

    def _gen(self):
        ctx = self.context
        kname = self.config["kernel"]
        kernel = ctx.modules[kname]
        layer = ctx.layers[kernel.layer_name]
        mods = []
        for m in modules.Modules.list_modules(ctx, kname):
            try:
                mods.append((int(m.DllBase), int(m.SizeOfImage), m.BaseDllName.get_string()))
            except Exception:
                pass
        mods.sort()
        bases = [m[0] for m in mods]
        def modof(a):
            i = bisect.bisect_right(bases, a) - 1
            if i >= 0 and a < mods[i][0] + mods[i][1]:
                return "%s+0x%x" % (mods[i][2], a - mods[i][0])
        try:
            now = struct.unpack("<Q", layer.read(0xFFFFF78000000320, 8))[0]
        except Exception:
            now = 0
        for t in thrdscan.ThrdScan.scan_threads(ctx, kname):
            try:
                if t.ExitTime.QuadPart > 0 or t.Tcb.State == 4:
                    continue
                proc = t.owning_process()
                pid = int(proc.UniqueProcessId); name = proc.ImageFileName.cast("string", max_length=15, errors="replace")
                state = int(t.Tcb.State); wr = int(t.Tcb.WaitReason)
                wt = int(t.Tcb.WaitTime)
                waited = ((now - wt) & 0xFFFFFFFF) * 15.625 / 1000.0 if now else -1
                ks = int(t.Tcb.KernelStack); ist = int(t.Tcb.InitialStack)
                hits = []
                if state == 5 and ist > ks and ist - ks < 0x20000:
                    raw = layer.read(ks, ist - ks, pad=True)
                    for off in range(0, len(raw) - 7, 8):
                        v = struct.unpack_from("<Q", raw, off)[0]
                        if v >= 0xFFFF800000000000:
                            s = modof(v)
                            if s:
                                hits.append(s)
                yield (0, (pid, str(name), int(t.Cid.UniqueThread), state, wr, round(waited, 1), " < ".join(hits[:16])))
            except (exceptions.InvalidAddressException, AttributeError, ValueError):
                continue

    def run(self):
        return renderers.TreeGrid([("PID", int), ("Process", str), ("TID", int), ("State", int), ("WaitReason", int), ("WaitedSec", float), ("Stack", str)], self._gen())
