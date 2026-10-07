# w249 — PREDICTIONS, recorded BEFORE the boot

Boot `w249_acbb9a3_hostdmesg` — same configuration as `w247` (all three flags armed,
`ce_executor=host`), the only change being `boot_capture.sh`'s new host-dmesg watermark+delta.

1. `run_w249_*_hostdmesg.log` exists and the probe log states `HOST_DMESG_LINES=<n>` explicitly.
2. `n > 0` — the boot provokes host-driver output. ⚠ If `n == 0` that is a legitimate result and
   the harness must still exit 0 and say so.
3. ★★★ **THE JOIN**: the count of `chandesConstruct_IMPL: Invalid object allocation request`
   lines in **this boot's** host delta EQUALS the count of `ENGINE-OBJECT … REFUSED Rm(Other(64))`
   lines in **this boot's** qemu log. Predicted **12 = 12** (w247's number, same configuration).
   ⇒ this upgrades 241-across-the-campaign to an N-for-this-boot join.
4. `kfifoRunlistSetId_GM107` count equals the `chandesConstruct_IMPL` count in the same delta
   (they are printed at two levels of one failure).
5. Every `Rm(Other(64))` is class `0xc7b5` (a **copy-engine** object) and **none** is graphics —
   as measured in w247. ⊘ The 8 `Route::NotACopyEngineChannel` doorbell refusals are OUR router
   and must produce **no** host-driver line at all.

⇒ If (3) fails, the correlation does not survive contact with a per-boot count and that is the
bigger finding. If (5) fails, the population changed between boots.

---

# SCORING (added after the runs — the predictions above are unedited)

| # | prediction | outcome |
|---|---|---|
| 1 | `hostdmesg.log` exists, `HOST_DMESG_LINES=<n>` stated | ✅ |
| 2 | `n > 0` | ✅ 42 (w250) |
| 3 | ★ **join closes: 12 = 12** | ⊘⊘ **FALSIFIED — 12 ours, 14 the host's** |
| 4 | `kfifoRunlistSetId` count == `chandesConstruct` count | ✅ 14 == 14 |
| 5 | all refusals class `0xc7b5` (CE), none graphics; doorbell refusals produce no host line | ✅ 12/12 `0xc7b5`; 8 `Route::NotACopyEngineChannel` produce nothing host-side |

⇒ **Prediction 3 is the one that mattered and it was wrong.** Our own counters make 12 exact
(`seen=32 forwarded=18` ⇒ 14 refused, of which 2 are `NoVas` and issued no host verb), so **two
host-side engine-object failures come from a path that is not `forward_engine_object`.**
Reproduced identically on two independent boots (`w249`, `w250`).

⚠ A defect the validating boot caught, recorded because it is the same class as everything else:
the first placement of the capture sat **before** the workload hook and collected **3 of 53**
lines. Moved to phase 3c. **The instrument was placed where it could not see the event.**
