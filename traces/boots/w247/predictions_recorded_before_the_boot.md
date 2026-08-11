# w247 — PREDICTIONS, recorded BEFORE the boot

Control = `w246d_acbb9a3_witon_rbon` (committed): witness ON, route B ON, FB_BACKING **unset**.
Arm     = `w247_acbb9a3_all3`: identical + `KAYFABE_FB_BACKING=on`. ce_executor=host throughout.

1. `GR-FB-BACKING … → BACKED … placed_as_asked=true` lines APPEAR (w228a had 32; control has 0).
2. The three framebuffer operands change `Framebuffer{}` → `HostBackedFb{}`.
3. `SET_REPORT_SEMAPHORE` stays `GuestRam{gpa:…}` — it is not an FB leaf.
4. ★★★ `COMPLETION-WATCH … → NOT-OBSERVED samples=88` is **UNCHANGED**, and `CE-SUBMIT` stays
   **0** — because the GR doorbell is still refused at `Route::NotACopyEngineChannel` before any
   execution, so no engine ever runs the pushbuffer. Backing the operands does not execute them.
5. `GR-ADDRESS-CENSUS operands=5 bound=4 unbound=1 mme_dwords=39` unchanged — the census counts
   BINDING, not BACKING.

⇒ If (4) is FALSE — if the semaphore is observed — that is enormous and gets reported first.
⇒ If (1) or (2) is false, the crossing has regressed since `82f9aa5` and that is the finding.
