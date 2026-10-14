# Bounded source-budget correction

STATUS: RESEARCH, 2026-10-12. Source revision is this containing commit.
Windows1580 measured~100KiB ordinary entries exceed the64KiB prototype cap.
Trusted policy now matches896KiB private staging region,225 public4KiB pieces,
and16x64KiB prepared store CPUviews (1MiB per channel). More fragmented store
views clean-refuse before host mutation. This is a resource policy, no per-die fact.

GPU-free2152PASS/0failed: kf-abi/chan/rm/host/gsp/qemu suite1875,
kf-mem/trap suite271, native probe6. Actual storage/snapshot/Cache cases
failed first; retained logs. Cache red initially had a compile fixture error,
then reproduced real Bounds after correction; final tests use backend-relative
RAM offsets and Missing(lastVA) when last page is removed. Tests include full
unaligned896KiB through225rows, RAM and contiguous store15windows, cap/bounds
rejection before host effects, revoked final page and unchanged real GET/PUT.

Independent source review found no runtime blocker. Rare private write_authored
now occurs before shared Pt/source/RAM publication guards. Record excludes Free,
prior staging still waits for real HOST witness; retry never advances PUT and
final authority revalidation is unchanged. No host verbs or PB work on vCPU.
Bounded snapshot CPUcopy still holds row/RAM guards: latency at larger sizes
remains unmeasured, and no12ms compatibility claim is made. Native100KiB padded
barrier mode added; exact native/Linux/Windows hardware results are PENDING.
