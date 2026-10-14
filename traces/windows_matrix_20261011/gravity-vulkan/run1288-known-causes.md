# GravityMark Vulkan run1288: existing-log checks

**STATUS: RESEARCH, 2026-10-11.** No new workload run for this screen.

Benchmark starts18:16:06.9349013UTC. Earliest recorded guest nvlddmkm153 is
18:17:20.4160251UTC,73.481124s later; other153 records follow at18:17:21.4413667
and18:17:24.3446958. Host Xid32 precedes them at18:17:10UTC, approximately63s
after app start (original journal stored whole seconds). This is not an
immediate <2s startup refusal. Timing alone does not establish a TDR or cause.
Guest display4101=0, live_kernel=0, WER1001=0, unchanged boot time.153 alone is
not TDR proof. One GSPcycle and the device-lost dialog are independently measured.

The complete stopped-run QEMU log has birth_refused_cap maximum0, mem-held0,
unreconciled0; no OverDeclaredCap, birth REFUSED or HELD-BY-HOST/hole diagnostic.
There are other refusals:18 memory diagnostics (firmware carve-out protection,
misaligned leaves, late boot-framebuffer overlap), unavailable/unmeasured host
controls, guest controls answered NOT_SUPPORTED and I2C object denials. The
misaligned-leaf records name other address spaces than the faulting GR twin;
this does not establish they are harmless or causal. Bounded raw leaf samples
are included, not reinterpreted as a valid mapping. The reported channel budget is1789 per runlist from host limit2045; no policy
change follows. See run1288-known-causes.json for counts and bounded examples.

Arrival-to-clear invalidate maximum75560us violates the required12ms bound.
This maximum is cumulative across boot (VaTiming::since retains the maximum),
including earlier setup; it does not locate a slow invalidate at the app fault.
Its existence warrants investigation, not an attribution of the Vulkan fault.
No maplog timeline or complete fault-time page-table/backing audit was enabled.
Zero held/unreconciled counters do not prove all mappings or PT reads correct.
The faulting GR host channel4e has a real software object but no authored host
registrations; exact trigger remains unmeasured. No speculative fix admitted.

Existing Vulkan compute/microbench passes do not test every large transfer,
queue topology or synchronization case. The translated512KiB copy-slice path
is a candidate to test, not established as the cause of this GR channel fault.
Same-GPU VFIO comparison is queued before further product iteration.
