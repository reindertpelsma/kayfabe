# Diagnosis helper, host-only: read _KEVENT objects at given kernel addresses (public ntoskrnl types) and their waiters.
import struct
from typing import List
from volatility3.framework import interfaces, renderers, exceptions, constants
from volatility3.framework.configuration import requirements

class KEvents(interfaces.plugins.PluginInterface):
    _required_framework_version = (2, 0, 0)
    _version = (1, 0, 0)

    @classmethod
    def get_requirements(cls) -> List[interfaces.configuration.RequirementInterface]:
        return [requirements.ModuleRequirement(name="kernel", description="Windows kernel", architectures=["Intel64"]),
                requirements.ListRequirement(name="addrs", element_type=str, description="KEVENT addresses (hex)", optional=False)]

    def _gen(self):
        kernel = self.context.modules[self.config["kernel"]]
        st = kernel.symbol_table_name
        for a in self.config["addrs"]:
            addr = int(a, 16) & ((1 << 48) - 1)
            try:
                ev = self.context.object(st + constants.BANG + "_KEVENT", layer_name=kernel.layer_name, offset=addr)
                h = ev.Header
                typ = int(h.Type)
                sig = int(h.SignalState)
                head = h.WaitListHead
                head_addr = int(head.vol.offset)
                wbt = self.context.symbol_space.get_type(st + constants.BANG + "_KWAIT_BLOCK")
                wle = wbt.relative_child_offset("WaitListEntry")
                waiters = []
                flink = int(head.Flink)
                n = 0
                while flink and flink != head_addr and n < 16:
                    wb = self.context.object(st + constants.BANG + "_KWAIT_BLOCK", layer_name=kernel.layer_name, offset=flink - wle)
                    try:
                        th = wb.Thread.dereference().cast(st + constants.BANG + "_ETHREAD")
                        waiters.append("tid=%d pid=%d wr=%d" % (int(th.Cid.UniqueThread), int(th.Cid.UniqueProcess), int(th.Tcb.WaitReason)))
                    except Exception as e:
                        waiters.append("wb@%x(%s)" % (flink, type(e).__name__))
                    try:
                        flink = int(wb.WaitListEntry.Flink)
                    except Exception:
                        waiters.append("walk-broken@%x" % flink)
                        break
                    n += 1
                try:
                    rawh = self.context.layers[kernel.layer_name].read(addr, 0x40, pad=True).hex()
                except Exception:
                    rawh = "?"
                yield (0, (a, typ, sig, ("; ".join(waiters) or "-") + " raw=" + rawh))
            except exceptions.InvalidAddressException as e:
                lay = self.context.layers[kernel.layer_name]
                try:
                    pa = lay.translate(addr)
                except Exception as e2:
                    pa = repr(e2)[:120]
                yield (0, (a, -1, -1, "unreadable: %s; translate=%s; layer=%s" % (repr(e)[:80], pa, kernel.layer_name)))
                continue
            # The address may name a structure that EMBEDS the event: scan 0x100 bytes for dispatcher headers
            # (Type 0/1 = notification/synchronization event, Size 6 = sizeof(KEVENT)/4, WaitListHead = canonical pointers).
            layer = self.context.layers[kernel.layer_name]
            try:
                raw = layer.read(addr, 0x100, pad=True)
            except Exception:
                continue
            # also follow the pointer at +8 (one level) and scan there
            try:
                p8 = struct.unpack_from("<Q", raw, 8)[0] & ((1 << 48) - 1)
                raw2 = layer.read(p8, 0x100, pad=True)
                yield (1, ("%s->%x" % (a, p8), -2, -2, "raw=" + raw2[:0x60].hex()))
                for off in range(0, 0x100 - 24, 8):
                    t, sz = raw2[off], raw2[off+2]
                    sigst = struct.unpack_from("<i", raw2, off + 4)[0]
                    fl, bl = struct.unpack_from("<QQ", raw2, off + 8)
                    if t in (0, 1) and sz == 6 and fl >> 47 in (0x1ffff, 1) and bl >> 47 in (0x1ffff, 1):
                        self2 = p8 + off + 8
                        yield (2, ("%x+%#x" % (p8, off), t, sigst, "followed event: waitlist %s" % ("EMPTY" if (fl & ((1<<48)-1)) == self2 else "flink=%x" % fl)))
            except Exception as e:
                yield (1, ("%s->?" % a, -3, -3, repr(e)[:80]))
            for off in range(0, 0x100 - 24, 8):
                t, absl, sz, inr = raw[off], raw[off+1], raw[off+2], raw[off+3]
                sigst = struct.unpack_from("<i", raw, off + 4)[0]
                fl, bl = struct.unpack_from("<QQ", raw, off + 8)
                if t in (0, 1) and sz == 6 and (fl >> 47 in (0x1ffff, 1)) and (bl >> 47 in (0x1ffff, 1)):
                    self_ = (addr + off + 8)
                    yield (1, ("%s+%#x" % (a, off), t, sigst, "embedded event: waitlist %s" % ("EMPTY" if (fl & ((1<<48)-1)) == self_ else "flink=%x blink=%x" % (fl, bl))))

    def run(self):
        return renderers.TreeGrid([("Addr", str), ("Type", int), ("SignalState", int), ("Waiters", str)], self._gen())
