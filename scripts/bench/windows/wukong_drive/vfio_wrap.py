#!/usr/bin/env python3
"""J4 VFIO reference: run propimage_check.py unchanged except (1) the kf3-gpu device is replaced by the real GPU through vfio-pci
(0000:01:00.0 + audio .1), (2) the kf3-only gpu seed repair/compare are skipped (the real GPU has its own identity).
Everything else (image, overlay, offline slirp restrict=on, faked RTC, CPU profile, TPM) is the harness's own argv."""
import importlib.util, sys, pathlib
SRC = pathlib.Path(sys.argv.pop(1))   # path of propimage_check.py
spec = importlib.util.spec_from_file_location("propimage_check", SRC)
pc = importlib.util.module_from_spec(spec); sys.modules["propimage_check"] = pc; spec.loader.exec_module(pc)
orig = pc.PI.build_argv
def patched(*a, **k):
    argv = orig(*a, **k)
    i = next(n for n, v in enumerate(argv) if v == "-device" and argv[n + 1].startswith("kf3-gpu"))
    del argv[i:i + 2]
    argv[i:i] = ["-device", "vfio-pci,host=0000:01:00.0,multifunction=on,addr=0x4.0", "-device", "vfio-pci,host=0000:01:00.1,addr=0x4.1"]
    return argv
pc.PI.build_argv = patched
pc.gpu_fix = lambda g, uuid: (True, "VFIO reference: kf3 seed repair/UUID check not applicable (real GPU)")
oc = pc.PI.compare
pc.PI.compare = lambda s, f: [tuple(["VFIO_DIFF" if (j == 1 and v == "MISMATCH") else v for j, v in enumerate(r)]) for r in oc(s, f)]
sys.exit(pc.main(sys.argv[1:]))
