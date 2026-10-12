# Shadow Windows desktop / Edge / D3D12

STATUS: MEASURED, 2026-10-12. Exact runtime d8afb1e3ca9440580c227ee218ce0d1b3d06a7ef,
RTX4070/host595.91.07/Windows580.88, production19flags + shadow1 only.
Linux9+10/fast30/desktop already passed on this exact runtime.

1610 boots/signs in/NVIDIA ready. Edge H264 PASS: rc0/40s, played14.986s/457frames/
19dropped, hardware video-decode6.54%. Three nvlddmkm153 events, display4101=0,
livekernel/WER/reboot=0, nohostXid. Harness calls them guest_tdr3/GSPcycles1 and
restarts; retain those original metrics, but they do not establish dxgkrnl141 or
display4101. This is playback PASS with driver events, not clean stability PASS.

1611 D3D12 FAIL: rc1/19s, no benchmarkimage/noGPUactivity proof, three153,
display4101/livekernel/WER/reboot0, nohostXid. No specific offending control or
causal source change established. Prior matching stock baseline PASS. Do not
call this newroute compatible or infer CPU scanner limits are the only defect.

Vulkan g3 BOOT_FAIL before launch because /var/lib available fell below100GiB
preflight; this is a harness-resource result, not a Vulkan application FAIL.
CollectionEXIT0 only means rows recorded, not allPASS. Cleanstop/DMAFQ/noQEMU
20:46:33UTC. Completed own1610/1611 disks and eight older own successful Linux
kernel/initrd copies removed after allqcow/protected audit (cleanup.json), freeing
11121049600bytes; original/personal1380 preserved. Same-source shadow Vulkan
resumed in separate unused1620 after restoring disk reserve; no code/oracle/media
change. Its results pending, not silently substituted into original matrix.

Observer first samples:1610 UnknownSuffix32/Source2;1611 UnknownSuffix26/Source2.
Early publication retry/history loss and software-identity Retry limit coverage.
This does NOT enumerate all scanner refusals; suffix unknown means no more CPU
PB reads. partial-observation-corpus.json contains actual raw GP/sample log lines,
source revision and compressed-log digest. These are partial mutable observations,
not full immutable PB corpora, execution proof, or interface/constant sources.
Earlier1510/1540/1570/1580/1590 historical records remain partial as audit describes.
