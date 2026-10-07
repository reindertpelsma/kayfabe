#!/bin/bash
# mode2_build_dbg2.sh — runs INSIDE the Mode-2 guest. Builds an instrumented
# open nvidia.ko from the 9p DKMS source (/mnt/ogkm) with:
#   1. GR-allocparams-null shim (nv.c) — past the cuCtxCreate SIGSEGV
#   2. CE-caps inject shim (nv.c)      — past cuCtxCreate=999
#   3. nv_post_event printk (nv.c)     — log every os-event fd wakeup (handle,
#      index, data_valid) so we see whether POST_EVENT->osNotifyEvent->postEvent
#      actually reaches nv_post_event for libcuda's events (3a/48/56).
# Stashes to ~/nvmods-dbg2 and loads it.  Fast: links the precompiled RM core.
set -u
NVVER=580.159.04
DST=/home/ubuntu/nvmods-dbg2
sudo mkdir -p /mnt/ogkm /mnt/build2
mountpoint -q /mnt/ogkm  || sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576,ro ogkm /mnt/ogkm
sudo umount /mnt/build2 2>/dev/null || true
sudo mount -t tmpfs -o size=6G tmpfs /mnt/build2
echo "copying source..."
sudo cp -aL /mnt/ogkm/. /mnt/build2/ 2>/dev/null

sudo python3 - <<'PYEOF'
f="/mnt/build2/nvidia/nv.c"
s=open(f).read()
# 1. GR shim — after the copy-in failure block
a1='        nv_printf(NV_DBG_ERRORS, "NVRM: failed to copy in ioctl data!\\n");\n        status = -EFAULT;\n        goto done_early;\n    }\n'
gr='''
    if (arg_cmd == 0x2b && arg_copy != NULL && (arg_size == 48 || arg_size == 40)) {
        NvU32 _hc = *(NvU32 *)((char *)arg_copy + 12);
        if (((_hc >> 8) & 0xff) >= 0xb0 && (((_hc & 0xff) == 0xc0) || ((_hc & 0xff) == 0x97))) {
            *(NvU64 *)((char *)arg_copy + 16) = 0;
        }
    }
'''
assert a1 in s, "GR anchor not found"; s=s.replace(a1, a1+gr, 1)
# 2. CE shim — after the default-case rm_ioctl
a2='            rmStatus = rm_ioctl(sp, nv, &nvlfp->nvfp, arg_cmd, arg_copy, arg_size);\n'
ce='''            if (arg_cmd == 0x2a && arg_copy != NULL && arg_size >= 32 &&
                *(NvU32 *)((char *)arg_copy + 8) == 0x20802a0a) {
                void __user *_up = (void __user *)(*(NvU64 *)((char *)arg_copy + 16));
                if (_up) {
                    NvU8 _c[136]; memset(_c, 0, sizeof(_c));
                    _c[0]=0xe3;_c[1]=0x03;_c[2]=0xe3;_c[3]=0x03;
                    _c[4]=0xe2;_c[5]=0x03;_c[6]=0xe2;_c[7]=0x03;
                    *(NvU64 *)(_c + 128) = 0x0fULL;
                    (void)copy_to_user(_up, _c, sizeof(_c));
                }
            }
'''
assert a2 in s, "CE anchor not found"; s=s.replace(a2, a2+ce, 1)
# 3. nv_post_event printk
a3='    nv_linux_file_private_t *nvlfp = nv_get_nvlfp_from_nvfp(event->nvfp);\n    unsigned long eflags;\n    nvidia_event_t *nvet;\n'
pe='    printk(KERN_INFO "NVKVMDBG nv_post_event handle=0x%x index=0x%x info32=0x%x dv=%d\\n", handle, index, info32, data_valid);\n'
assert a3 in s, "nv_post_event anchor not found"; s=s.replace(a3, a3+pe, 1)
open(f,"w").write(s)
print("patched nv.c OK")
PYEOF

echo "building (links precompiled core)..."
( cd /mnt/build2 && sudo make modules -j2 >/tmp/dbg2_build.log 2>&1 )
KO=$(find /mnt/build2 -name nvidia.ko | head -1)
if [ -z "$KO" ]; then echo "BUILD FAILED:"; tail -25 /tmp/dbg2_build.log; exit 1; fi
mkdir -p "$DST"; cp "$KO" "$DST/nvidia.ko"
# uvm from the existing dbg stash (unchanged)
cp /home/ubuntu/nvmods-dbg/nvidia-uvm.ko "$DST/" 2>/dev/null
echo "built + stashed $(ls -la $DST/nvidia.ko)"
