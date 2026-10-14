# Diagnosis helper, host-only, never committed: contended ERESOURCEs and their owner threads.
from typing import List
from volatility3.framework import interfaces, renderers, exceptions, constants
from volatility3.framework.configuration import requirements

class ResLocks(interfaces.plugins.PluginInterface):
    _required_framework_version = (2, 0, 0)
    _version = (1, 0, 0)

    @classmethod
    def get_requirements(cls) -> List[interfaces.configuration.RequirementInterface]:
        return [requirements.ModuleRequirement(name="kernel", description="Windows kernel", architectures=["Intel32", "Intel64"])]

    def _gen(self):
        ctx = self.context; kname = self.config["kernel"]; kernel = ctx.modules[kname]
        tn = kernel.symbol_table_name
        head = kernel.object_from_symbol("ExpSystemResourcesList", object_type=tn + constants.BANG + "_LIST_ENTRY")
        n = 0
        for r in head.to_list(tn + constants.BANG + "_ERESOURCE", "SystemResourcesList"):
            n += 1
            if n > 200000: break
            try:
                ex = int(r.NumberOfExclusiveWaiters); sh = int(r.NumberOfSharedWaiters)
                if ex == 0 and sh == 0: continue
                owner = int(r.OwnerEntry.OwnerThread) & ~7
                pid = tid = -1; pname = ""
                if owner:
                    try:
                        th = ctx.object(tn + constants.BANG + "_ETHREAD", kernel.layer_name, offset=owner | (0xFFFF << 48) if owner & (1 << 47) else owner)
                        tid = int(th.Cid.UniqueThread); pid = int(th.Cid.UniqueProcess)
                    except Exception: pass
                tbl = []
                if not owner:
                    try:
                        ot = int(r.OwnerTable)
                        if ot:
                            ot = ot | (0xFFFF << 48) if ot & (1 << 47) else ot
                            lay = ctx.layers[kernel.layer_name]
                            import struct
                            raw = lay.read(ot, 16 * 64, pad=True)
                            for k in range(1, 64):
                                th_, cnt = struct.unpack_from("<QI", raw, 16 * k)
                                th_ &= ~7
                                if th_:
                                    try:
                                        t2 = ctx.object(tn + constants.BANG + "_ETHREAD", kernel.layer_name, offset=th_ | (0xFFFF << 48) if th_ & (1 << 47) else th_)
                                        tbl.append("%d.%d(cnt%d,st%d,wr%d,exit%d,%x)" % (int(t2.Cid.UniqueProcess), int(t2.Cid.UniqueThread), cnt, int(t2.Tcb.State), int(t2.Tcb.WaitReason), 1 if t2.ExitTime.QuadPart > 0 else 0, th_))
                                    except Exception: tbl.append(hex(th_))
                    except Exception as e: tbl.append("err")
                yield (0, (hex(r.vol.offset), ex, sh, int(r.ActiveCount), int(r.Flag), hex(owner), pid, tid, 0, ",".join(tbl)))
            except (exceptions.InvalidAddressException, AttributeError, ValueError):
                continue

    def run(self):
        return renderers.TreeGrid([("Resource", str), ("ExclWaiters", int), ("SharedWaiters", int), ("Active", int), ("Flag", int), ("OwnerThread", str), ("OwnerPID", int), ("OwnerTID", int), ("x", int), ("Shared", str)], self._gen())
