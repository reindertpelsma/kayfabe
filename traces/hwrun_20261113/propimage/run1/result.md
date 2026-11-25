| step | verdict | detail |
|---|---|---|
| unpack | PASS | unpacked directory used; seed check |
| clock | PASS | rtc base 2026-11-12T17:49:48+00:00 (stop + gap, never earlier) |
| overlay | PASS | throwaway overlay of the unpacked base |
| boot | PASS | qemu pid 129047, offline, display=broker, flags=kf3_flags.sample.env |
| ssh | PASS | up after 141 s |
| verify | FAIL | gpu_uuid |
| gpu | FAIL | ["NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver. Make sure that the latest NVIDIA driver is installed and running. This can also be happening if non-NVIDIA GPU is running as primary display, and NVIDIA GPU is in WDDM mode."] |
| app:3dmark | LAUNCH_ONLY | evidence=SCREEN(launch only; the app did no work) alive=True window=True procs=3DMark,3DMark titles=3DMark done  |
| app:solidworks | FAIL | evidence=SCREEN(launch only; the app did no work) alive=True window=False procs=SLDWORKS,sldworks_fs titles= done  |
| app:resolve | FAIL | evidence=SCREEN(launch only; the app did no work) alive=True window=False procs=Resolve titles=Project Manager done  |
| app:superposition | LAUNCH_ONLY | evidence=SCREEN(launch only; the app did no work) alive=True window=True procs=launcher titles=UNIGINE Superposition v1.1 done  |
| app:blackmyth | LAUNCH_ONLY | evidence=SCREEN(launch only; the app did no work) alive=True window=True procs=steam,steamservice,steamwebhelper,steamwebhelper,steamwebhelper,steamwebhelper,steamwebhelper,steamwebhelper titles=Black Myth: Wukong Benchmark Tool done |
| stop | FAIL | qemu rc=None |
