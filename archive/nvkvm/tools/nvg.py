# Reusable gdb helpers for nvkvm closed-lib RE (#84). Source from a driver:
#   gdb> source tools/nvg.py
import gdb
gdb.execute("set pagination off")

def libbase(name):
    out = gdb.execute("info sharedlibrary " + name, to_string=True)
    for ln in out.splitlines():
        p = ln.split()
        if p and p[0].startswith("0x") and name in ln:
            return int(p[0], 16)
    return None

def pcval(): return int(gdb.parse_and_eval("$pc"))
def reg(r):  return int(gdb.parse_and_eval("$" + r)) & 0xffffffffffffffff
def eax():   return reg("rax") & 0xffffffff
def insn(a=None):
    return gdb.execute("x/i 0x%x" % a if a else "x/i $pc", to_string=True).strip()

def call_before(pc):
    """Nearest `call` instruction with address < pc (for finding what just returned)."""
    dis = gdb.execute("x/14i 0x%x" % (pc - 28), to_string=True)
    cs = []
    for l in dis.splitlines():
        if "\tcall" in l:
            try: a = int(l.split(":")[0].replace("=>", "").strip(), 16)
            except: continue
            if a < pc: cs.append(a)
    return max(cs) if cs else None

def step_until_eax(target, maxs=4000):
    """ni-step (step over calls) until eax==target. Returns info dict."""
    prev = None
    for i in range(maxs):
        if eax() == target:
            return {"i": i, "producer_pc": prev,
                    "producer": insn(prev) if prev else "(start)", "now": insn(),
                    "rdi": reg("rdi"), "rsi": reg("rsi"), "rdx": reg("rdx"), "rcx": reg("rcx")}
        c = insn()
        if c.split("\t")[-1].strip().startswith("ret"):
            return {"ret": True, "eax": eax(), "producer_pc": prev,
                    "producer": insn(prev) if prev else "?"}
        prev = pcval()
        gdb.execute("ni")
    return {"timeout": True}
