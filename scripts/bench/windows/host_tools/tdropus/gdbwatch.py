# gdbwatch.py — loaded by `gdb -batch -x gdbwatch.py`; env GW_PORT, GW_ADDRS (comma list of "tag:addr"), GW_LOG.
# Hardware ACCESS watchpoints (8 bytes each; x86 DR) on guest kernel VAs through QEMU's gdbstub; every hit is logged
# (host time, vCPU, pc, rsp, 8 stack qwords, the 16-byte slot) and the guest continues (stop() returns False).
# Ends on SIGINT (the host driver sends it): removes the watchpoints and detaches, so the VM keeps running.
import gdb, os, time

LOG = open(os.environ["GW_LOG"], "a", buffering=1)
gdb.execute("set pagination off")
gdb.execute("set confirm off")
gdb.execute("set architecture i386:x86-64")
gdb.execute("set can-use-hw-watchpoints 1")
gdb.execute(f"target remote 127.0.0.1:{os.environ['GW_PORT']}")
inf = gdb.selected_inferior()


def rd(addr, n):
    try:
        return inf.read_memory(addr, n).tobytes()
    except gdb.MemoryError:
        return None


class W(gdb.Breakpoint):
    def __init__(self, tag, addr):
        super().__init__(f"*(unsigned long long *){addr:#x}", gdb.BP_WATCHPOINT, gdb.WP_ACCESS)
        self.tag, self.addr = tag, addr
        self.silent = True

    def stop(self):
        t = time.time()
        pc = int(gdb.parse_and_eval("$pc")) & 0xFFFFFFFFFFFFFFFF
        rsp = int(gdb.parse_and_eval("$rsp")) & 0xFFFFFFFFFFFFFFFF
        slot = rd(self.addr & ~0xF, 16)
        stk = rd(rsp, 64)
        th = gdb.selected_thread().num
        LOG.write("HIT t=%.6f cpu=%d tag=%s addr=%#x pc=%#x rsp=%#x slot=%s stack=%s\n" % (
            t, th, self.tag, self.addr, pc, rsp, slot.hex() if slot else "?",
            stk.hex() if stk else "?"))
        return False


ws = []
for item in os.environ["GW_ADDRS"].split(","):
    tag, a = item.split(":")
    try:
        ws.append(W(tag, int(a, 0)))
        LOG.write("WATCH %s %s at t=%.6f\n" % (tag, a, time.time()))
    except gdb.error as e:
        LOG.write("WATCH-REFUSED %s %s: %s\n" % (tag, a, e))
try:
    gdb.execute("continue")
except gdb.error as e:
    LOG.write("CONTINUE-ERR %s\n" % e)
LOG.write("STOPPED t=%.6f\n" % time.time())
for w in ws:
    w.delete()
gdb.execute("detach")
LOG.write("DETACHED t=%.6f\n" % time.time())
