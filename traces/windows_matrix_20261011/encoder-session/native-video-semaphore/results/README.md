# Authored native video semaphore result

**STATUS: ANSWERED, 2026-10-11.** Bench source35f97433, binarySHA
81f695d518606def466684ee0cae8d64e0cbb55f66bfde752b9f42bdc029eb94.
Trusted AD104/RTX4070, stock host595.91.07, user nobody, NVENC1/C9B7.

Hypothesis/falsifier: each authored release or release+trap sequence must produce
its GPU-written0x61fe0001 payload and an actual GPU tail fence1; wrong payload,
absent tail, failure to clean up, or a host Xid disproves success.
Measured: release PASS, trap PASS; both USER births, all owned resources freed,
zero bounded host Xids. First observed tail latency1us/release and1057us/trap
begins after doorbell returns; it is not total submission latency.

This confirms execution/completion of these owned authored sequences. OS-event
and guest trap-notification delivery were not measured. Production HostRing still
refuses nonempty video work; Windows encoder recovery remains open. No codec work,
foreign address, guest word forwarding or CPU completion was used.
