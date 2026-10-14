# Diagnosis helper, host-only, never committed: waiting threads of a Windows memory image with a stack-scan of module return addresses.
import logging, struct, bisect
from typing import List
from volatility3.framework import interfaces, renderers, exceptions
from volatility3.framework.configuration import requirements
from volatility3.plugins.windows import thrdscan, modules, pslist
from volatility3.framework import constants

def canon(v):
    return v | (0xFFFF << 48) if v & (1 << 47) else v

class ModOf(interfaces.plugins.PluginInterface):
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
                mods.append((canon(int(m.DllBase)), int(m.SizeOfImage), m.BaseDllName.get_string()))
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
        def threads():
            for proc in pslist.PsList.list_processes(ctx, kname):
                try:
                    for t in proc.ThreadListHead.to_list(kernel.symbol_table_name + constants.BANG + "_ETHREAD", "ThreadListEntry"):
                        yield t
                except exceptions.InvalidAddressException:
                    continue
        import os
        for a in os.environ["KF_ADDRS"].split(","):
            v = int(a, 16)
            yield (0, (0, "", 0, 0, 0, 0.0, modof(v) or "unknown"))

    def run(self):
        return renderers.TreeGrid([("PID", int), ("Process", str), ("TID", int), ("State", int), ("WaitReason", int), ("WaitedSec", float), ("Stack", str)], self._gen())
