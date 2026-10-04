#!/usr/bin/env python3
"""Research-only textual timer-map source census, not a C parser or product generator.

Checks exact source statements at every measured tag and emits source hashes for review.
The product layout is compiler-generated separately by tools/drivermatrix/host.spec.
"""
import argparse
import hashlib
import re
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ogkm', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    tags = [t for t in (root / 'tools/drivermatrix/tags.txt').read_text().splitlines()
            if t and not t.startswith('#')]

    def git(*argv):
        return subprocess.check_output(['git', '-C', str(args.ogkm), *argv], text=True)

    print('tag\tcommit\tquery_flags\tquery_access\tunprivileged_timer_range_permission\tregister_header_count\tregister_header_sha256')
    for tag in tags:
        path = 'src/nvidia/generated/g_subdevice_nvoc.c'
        nvoc = git('show', tag + ':' + path)
        found = re.search(r'/\*flags=\*/\s*(0x[0-9a-f]+)u,\s*/\*accessRight=\*/\s*(0x[0-9a-f]+)u,\s*/\*methodId=\*/\s*0x20800404u,', nvoc)
        assert found and found[2] == '0x0', (tag, 'query NVOC access changed')
        control = git('show', tag + ':src/nvidia/inc/kernel/rmapi/control.h')
        privilege = re.search(r'#define RMCTRL_FLAGS_NON_PRIVILEGED\s+(0x[0-9a-fA-F]+)', control)
        assert privilege and int(found[1], 16) & int(privilege[1], 16), (tag, 'query not nonprivileged')
        ctrl = git('show', tag + ':src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c')
        body = ctrl.split('NV_STATUS subdeviceCtrlCmdValidateMemMapRequest_IMPL', 1)[1]
        timer = body.split('tmrGetTimerBar0MapInfo_HAL', 1)[1].split('KernelFifo', 1)[0]
        for text in ('isAddressWithinLimits(start, length, bar0MapOffset, bar0MapSize)',
                     'pParams->protection = NV_PROTECT_READABLE;', 'return NV_OK;'):
            assert text in timer, (tag, text)
        hal = git('show', tag + ':src/nvidia/src/kernel/gpu/timer/timer_ptimer.c')
        for text in ('*pBar0MapOffset = DRF_BASE(NV_PTIMER);', '*pBar0MapSize   = DRF_SIZE(NV_PTIMER);'):
            assert text in hal, (tag, text)
        allpaths = git('ls-tree', '-r', '--name-only', tag, 'src/common/inc/swref/published').splitlines()
        headers = [p for p in allpaths if p.endswith('/dev_timer.h') and '/nvswitch/' not in p]
        assert headers, tag
        # Published register definitions including their access semantics; comments/licensing
        # outside macro lines do not affect this textual census hash. No values enter product.
        h = hashlib.sha256()
        for path in headers:
            content = git('show', tag + ':' + path)
            h.update(path.encode())
            h.update('\n'.join(l for l in content.splitlines() if l.startswith('#define NV_PTIMER')).encode())
        commit = git('rev-parse', tag + '^{commit}').strip()
        print(f'{tag}\t{commit}\t{found[1]}\t0x0\tNV_PROTECT_READABLE\t{len(headers)}\t{h.hexdigest()}')


if __name__ == '__main__':
    main()
