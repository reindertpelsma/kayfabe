# Q4 K4 GPU-side attribution

STATUS: RESEARCH IN PROGRESS, 2026-10-23. Candidate correctness remains PARTIAL/HOLD.

CODE READ ROUND_K4.md / plan.json: explicit host return for a four-hour
investigation, deadline11:00:58UTC and restore cutoff10:45:58UTC. G1–G5
falsifiers precede the runs. Exact candidate714ec29d3, no product optimization.

MEASURED 2026-10-23, first-clean-pair.json: kayfabe4642 and VFIO4648 completed
CATIA/Creo and12 own proxy phases each, with successful sampler export,
normal shutdown/restore and zero153/4101/reboot/Budget/new Xid. The original second
kayfabe run4645 finished both viewsets, then failed the proxy installer at
the OS argv byte limit and restored cleanly. Its FAIL remains excluded.
Replacement4649 and native4643 are running; final comparisons await them.

MEASURED 2026-10-23, frame-backing-4642b.json: the own broker retained an
8294400-byte frame exported by udmabuf. CODE READ at714ec29d3,
crates/kf-qemu/src/broker.rs::BrokerSeat::frame: this rung pins a memfd mapping
for D2H and wraps those host pages in udmabuf. A dma-buf accepted by the broker
does not establish GPU-resident backing. The initial directory-only process
filter found no broker and remains inconclusive in frame-backing-4642.json.

INFERRED windowed-proxy-plan.json: the primary proxy's borderless window
fills the virtual1920x1080 desktop but only part of the physical desktop.
An additional1280x720 visible-window control, with occlusion/status logging,
is predeclared for the second pair only. The original12 phases remain intact.
The generated D3D11 grid is a proxy; CATIA/Creo use OpenGL.

MEASURED 2026-10-23, analysis-controls.json / clock-envelope-controls.json:
17+10 GPU-free evidence-integrity tests pass. Four original mutants and two
clock-envelope mutants each lose at least two assertions. Expected red
24aa08d53 /6985bf181 were missing new APIs/imports with zero behavioral
assertions executed, not evidence of production runtime defects.

MEASURED 2026-10-23, identity-controls-4642.json: canonical creation/image
identity from held process handles passes six positive/negative checks;
starttime/image/old-array mutants lose two assertions each. The original
array wrapper failed two positive assertions. The earlier4646 decoded
control passed once, then repeat4647 failed positive image-identity checks;
that original failure is retained in identity-controls-4647-original-failure.json.
All field matches remain strict, and no own control child remains.

MEASURED 2026-10-23, native-progress-control-4648.json: the native supervisor
received an agent signature field before benchmarking, with an empty
pre-workload signature. Only subsequent real vendor-log changes establish
progress. The prior4646 controller lacked its per-run agent environment and
hit the unchanged900s watchdog after viewsets/proxy completed during export;
its original failed result remains excluded from the clean repetitions.

MEASURED 2026-10-23, aux4644-recovery.json: the earlier kayfabe viewsets
completed but sampler identity/export failed. Raw artifacts were recovered
read-only from its stopped overlay. Auxiliary4646 essential sampler files
were likewise extracted read-only and matched source SHA256s; a recursive
folder copy failed on other files, so no complete-folder recovery is claimed.
Neither auxiliary run is counted as a clean repetition.

CODE READ controls/clock_envelope.py: the first fixed-offset analysis model
failed its3s uncertainty check for4642; that failure is retained. The
conservative analysis accepts a physical sample only for every offset in the
full observed host/guest response-bracket union, plus vendor-boundary exclusion.
This removes more edge data. Final cohort statistics use the same maximum
exclusion margin across all four clean boots; no workload timer changes.

MEASURED 2026-10-23, host-preflight.json: CUDA0, NVIDIA bound,
GDM/Ubuntu/autologin and two monitors before VM work; idle link Gen1x16,
capability Gen4x16. Prelaunch4640 never created a VM; its missing staged
receipt failure remains in prelaunch-4640.json.

MEASURED 2026-10-23, transport-controls.json: three script-transport tests pass;
two mutants each fail two assertions. EXPECTED RED f29d3e7cd was a missing
new API/import, with zero behavioral assertions. The212922-byte installer
is supplied via stdin to the same agent queue, preserving exact UTF-8 and
newlines. This fixes the mechanical argv limit without changing a timer or
a guard. No product source or production behavior changed.
