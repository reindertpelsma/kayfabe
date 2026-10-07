#!/usr/bin/env python3
"""Bite-check §14.43's rung: plant each defect shape someone could plausibly write, run the
tests that name it, and report every NON-BITER."""
import subprocess, shutil, sys, os, time

ROOT = "/workspace/nvkvm-rs"
ABI = f"{ROOT}/crates/kayfabe-abi/src/fmbpromote.rs"
DEV = f"{ROOT}/crates/kayfabe-device/src/inittables.rs"

BITES = [
    # (name, file, old, new, target packages/tests)
    ("D1-clamp: clamp the runqueue count instead of refusing it (the C's literal shape)",
     ABI,
     """    if declared as usize > MAX_RUNQUEUES {
        return Err(FaultMethodBufferError::TooManyRunqueues { declared });
    }""",
     """    let declared = declared.min(MAX_RUNQUEUES as u32);""",
     [("kayfabe-abi", "--lib"), ("kayfabe-device", "--test promote_fault_method_buffers")]),

    ("aperture: fold an unnameable address space into sysmem instead of refusing",
     ABI,
     """        if buf.address_space != ADDR_SYSMEM && buf.address_space != ADDR_FBMEM {
            return Err(FaultMethodBufferError::UnknownAddressSpace {
                runqueue,
                address_space: buf.address_space,
            });
        }""",
     """        let buf = MethodBuffer { address_space: ADDR_SYSMEM, ..buf };""",
     [("kayfabe-abi", "--lib"), ("kayfabe-device", "--test promote_fault_method_buffers")]),

    ("bar2: index bar2Addr inside the 32-byte memdesc stride instead of its own array",
     ABI,
     "            bar2_addr: u64_at(params, BAR2_ADDR_OFF + runqueue * 8),",
     "            bar2_addr: u64_at(params, at + 24 - 24),",
     [("kayfabe-abi", "--lib")]),

    ("size: read the memdesc stride as 24 bytes instead of 32",
     ABI,
     "pub const MEMDESC_INFO_SIZE: usize = 32;",
     "pub const MEMDESC_INFO_SIZE: usize = 24;",
     [("kayfabe-abi", "--lib"), ("kayfabe-device", "--test promote_fault_method_buffers")]),

    ("addr-zero: accept a sized buffer at physical address 0",
     ABI,
     """        if buf.base == 0 && !buf.is_destroy() {
            return Err(FaultMethodBufferError::SizedAtAddressZero {
                runqueue,
                size: buf.size,
            });
        }""",
     "",
     [("kayfabe-abi", "--lib")]),

    ("destroy: treat the SDK's size==0 destroy form as malformed",
     ABI,
     "        if buf.base == 0 && !buf.is_destroy() {",
     "        if buf.base == 0 {",
     [("kayfabe-abi", "--lib"), ("kayfabe-device", "--test promote_fault_method_buffers")]),

    ("echo: reply with the raw request slice instead of re-encoding what was accepted",
     DEV,
     """                match kayfabe_abi::fmbpromote::decode_promote_fault_method_buffers(raw) {
                    Ok(req) => kayfabe_abi::fmbpromote::encode_promote_fault_method_buffers(&req),
                    Err(_) => return refuse(),
                }""",
     """                match kayfabe_abi::fmbpromote::decode_promote_fault_method_buffers(raw) {
                    Ok(_) => raw.to_vec(),
                    Err(_) => return refuse(),
                }""",
     [("kayfabe-device", "--test promote_fault_method_buffers")]),

    ("serve: answer NV_OK with zeros when the decode refuses (the silent-wall shape)",
     DEV,
     """                match kayfabe_abi::fmbpromote::decode_promote_fault_method_buffers(raw) {
                    Ok(req) => kayfabe_abi::fmbpromote::encode_promote_fault_method_buffers(&req),
                    Err(_) => return refuse(),
                }""",
     """                match kayfabe_abi::fmbpromote::decode_promote_fault_method_buffers(raw) {
                    Ok(req) => kayfabe_abi::fmbpromote::encode_promote_fault_method_buffers(&req),
                    Err(_) => vec![0u8; want.params_size()],
                }""",
     [("kayfabe-device", "--test promote_fault_method_buffers")]),
]


def run(pkg, sel):
    cmd = ["cargo", "test", "-p", pkg] + sel.split() + ["--no-fail-fast"]
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    return r.returncode


def main():
    nonbiters = []
    for name, path, old, new, targets in BITES:
        src = open(path).read()
        if src.count(old) != 1:
            print(f"SKIP (anchor {src.count(old)}x): {name}")
            continue
        bak = src
        open(path, "w").write(src.replace(old, new, 1))
        os.utime(path, None)
        bit = False
        detail = []
        for pkg, sel in targets:
            rc = run(pkg, sel)
            detail.append(f"{pkg} {sel} rc={rc}")
            if rc != 0:
                bit = True
        open(path, "w").write(bak)
        os.utime(path, None)
        status = "BITES" if bit else "*** NON-BITER ***"
        print(f"{status}: {name}   [{'; '.join(detail)}]", flush=True)
        if not bit:
            nonbiters.append(name)
    print()
    print(f"non-biters: {len(nonbiters)}/{len(BITES)}")
    for n in nonbiters:
        print("  -", n)
    # statefulness canary: the tree must be green again after every restore
    rc = run("kayfabe-device", "--test promote_fault_method_buffers")
    print(f"canary (restored tree) rc={rc}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
