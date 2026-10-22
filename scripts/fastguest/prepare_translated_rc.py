#!/usr/bin/env python3
"""Prepare a guest-only probe from pinned public ogkm source."""
import argparse
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

TAG = "580.159.04"
REV = "b81d58ee0224d1d290bef1c080592b619e184042"
API_NAMES = ("RegisterGpu", "UnregisterGpu", "SessionCreate", "SessionDestroy",
             "GetGpuInfo", "DeviceCreate", "DeviceDestroy", "AddressSpaceCreate",
             "AddressSpaceDestroy", "TsgAllocate", "TsgDestroy", "ChannelAllocate",
             "ChannelDestroy", "MemoryAllocSys", "MemoryCpuMap", "MemoryCpuUnMap",
             "MemoryFree", "QueryCopyEnginesCaps")


def patch_guest(source):
    """Add an observation after the real guest RC notification operation."""
    function = "static NV_STATUS\n_kgspRpcRCTriggered\n("
    pattern = r"    return krcErrorSendEventNotifications_HAL\(pGpu, pKernelRc,.*?\n        \);"
    matches = list(re.finditer(pattern, source, re.S))
    if source.count(function) != 1 or len(matches) != 1 or "KF_RC_EVENT" in source:
        raise ValueError("unrecognized/already instrumented public guest RC body")
    match = matches[0]
    if match.start() < source.index(function):
        raise ValueError("RC notification is outside the expected handler")
    replacement = '    extern void nvKfRcControlEvent\n        (NvU32, NvU32, NvU32, NvU32);\n' + match.group().replace("return krc", "status = krc", 1) + '''
    if (status == NV_OK)
    {
        NV_PRINTF(LEVEL_ERROR, "KF_RC_EVENT chid=0x%x engine=0x%x xid=%u scope=%u\\n",
                  rpc_params->chid, rpc_params->nv2080EngineType,
                  rpc_params->exceptType, rpc_params->scope);
        nvKfRcControlEvent(rpc_params->chid, rpc_params->nv2080EngineType,
                           rpc_params->exceptType, rpc_params->scope);
    }
    return status;'''
    return source[:match.start()] + replacement + source[match.end():]


BRIDGE = '''
/* Guest bench only: handshake AFTER real RM event notification succeeded. */
static atomic_t kf_rc_control_count = ATOMIC_INIT(0);
static NvU32 kf_rc_control_chid, kf_rc_control_engine;
static NvU32 kf_rc_control_xid, kf_rc_control_scope;
void nvKfRcControlEvent(NvU32 chid, NvU32 engine, NvU32 xid, NvU32 scope);
NvU32 nvKfRcControlEventCount(NvU32 chid, NvU32 engine);
void nvKfRcControlEvent(NvU32 chid, NvU32 engine, NvU32 xid, NvU32 scope)
{
    WRITE_ONCE(kf_rc_control_chid, chid);
    WRITE_ONCE(kf_rc_control_engine, engine);
    WRITE_ONCE(kf_rc_control_xid, xid);
    WRITE_ONCE(kf_rc_control_scope, scope);
    smp_wmb();
    atomic_inc(&kf_rc_control_count);
}
NvU32 nvKfRcControlEventCount(NvU32 chid, NvU32 engine)
{
    NvU32 count = atomic_read(&kf_rc_control_count);
    smp_rmb();
    if (READ_ONCE(kf_rc_control_chid) != chid ||
        READ_ONCE(kf_rc_control_engine) != engine ||
        READ_ONCE(kf_rc_control_xid) != 32 || READ_ONCE(kf_rc_control_scope) != 0)
        return 0;
    return count;
}
EXPORT_SYMBOL(nvKfRcControlEventCount);
'''


def derive_api(source):
    """Keep public ABI declarations/types; avoid the driver's nv-linux.h build wrapper."""
    end = source.index("} UvmGpuPlatformInfo;") + len("} UvmGpuPlatformInfo;")
    start = source.rfind("typedef struct", 0, end)
    declarations = []
    for name in API_NAMES:
        pattern = rf"(?:NV_STATUS|void)\s+nvUvmInterface{name}\s*\([^;]+\);"
        found = re.findall(pattern, source)
        if len(found) != 1:
            raise ValueError(f"public declaration unavailable: {name}")
        declarations.extend(found)
    license_end = source.index("*/")+2
    return (source[:license_end] + '\n/* Derived from public nv_uvm_interface.h at '+REV+' */\n'
            '#include "nv_uvm_types.h"\n' + source[start:end] + '\n'
            + '\n'.join(declarations) + '\n'
            + 'NvU32 nvKfRcControlEventCount(NvU32 chid, NvU32 engine);\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ogkm", type=Path, required=True, help="public source Git repository")
    parser.add_argument("--out", type=Path, required=True, help="new private build directory")
    parser.add_argument("--kernel", type=Path, help="matching kernel headers; compile object only")
    args = parser.parse_args()
    actual = subprocess.check_output(["git", "-C", str(args.ogkm), "rev-parse", TAG+"^{commit}"], text=True).strip()
    if actual != REV:
        parser.error("ogkm tag differs from pinned public source")
    args.out.mkdir(parents=True, exist_ok=False)
    root = args.out.resolve()
    ogkm = root / "ogkm"
    ogkm.mkdir()
    process = subprocess.Popen(["git", "-C", str(args.ogkm), "archive", REV], stdout=subprocess.PIPE)
    try:
        with tarfile.open(fileobj=process.stdout, mode="r|") as archive:
            for member in archive:
                target = (ogkm/member.name).resolve()
                if not target.is_relative_to(ogkm) or member.issym() or member.islnk():
                    raise ValueError("unexpected archive path/link")
                archive.extract(member, ogkm, filter="data")
    finally:
        process.stdout.close()
        if process.wait() != 0:
            raise ValueError("public source archive failed")
    interface = (ogkm/"kernel-open/common/inc/nv_uvm_interface.h").read_text()
    (root/"kf_rc_api.h").write_text(derive_api(interface))
    handler = ogkm/"src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c"
    handler.write_text(patch_guest(handler.read_text()))
    bridge = ogkm/"kernel-open/nvidia/nv_uvm_interface.c"
    bridge.write_text(bridge.read_text()+BRIDGE)
    shutil.copyfile(Path(__file__).with_name("kf_rc_probe.c"), root/"kf_rc_probe.c")
    (root/"Makefile").write_text('obj-m := kf_rc_probe.o\nccflags-y += -I$(src)/ogkm/kernel-open/common/inc -I$(src)/ogkm/src/common/sdk/nvidia/inc -I$(src)/ogkm/src/common/inc\n')
    (root/"SOURCE").write_text(REV+"\n")
    if args.kernel:
        subprocess.run(["nice", "-n", "19", "make", "-C", str(args.kernel.resolve()),
                        "M="+str(root), "-j2", "kf_rc_probe.o"], check=True)
    print(f"PREPARED public_guest_source={REV} out={root}; no module installed or loaded")


if __name__ == "__main__":
    main()
