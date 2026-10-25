STATUS: LIVE, 2026-10-23 — PRECOMP source coverage; application validation is separate.

| Date | Die | Host branch | PRECOMP source | Reason/evidence | Windows candidate validation |
|---|---|---|---|---|---|
|2026-10-23|GB202|595|measured:GB202,595.91.07,2026-10-23|57310491 privileged public READ32/GLOBAL,48 words/328 fields|UNRUN; old derived6e3fd3fd6 failed0x50|
|2026-10-23|GB206|595|measured:GB206,595.91.07,2026-10-23|57302449 privileged public READ32/GLOBAL,48 words/328 fields|UNRUN|
|2026-10-23|GB203|595|inherited-with-reason:GB203<-GB202|Owner-approved provisional DISPv0502/CA layout; all328 fields agree between GB202/GB206|UNRUN; not a measurement ofGB203|
|2026-10-23|GB205|595|inherited-with-reason:GB205<-GB202|Same explicit provisional reason; donor source tag retained|UNRUN; not a measurement ofGB205|
|2026-10-23|GB207|595|inherited-with-reason:GB207<-GB202|Same explicit provisional reason; donor source tag retained|UNRUN; not a measurement ofGB207|

2026-10-23 The table covers all48 generated PRECOMP addresses per donor, with no missing named word. Generic consttab lookup still refuses unmeasured neighbours; the RM policy contains only these three named inheritance approvals. An exact future measurement takes precedence, a partial own page refuses, and unlisted dies/host branches refuse. The inheritance is not a proved whole-generation rule.

2026-10-23 Other capabilities retain their existing sources: enabled/floorswept/topology facts are not overwritten by this48-word policy. In particular, the older unnamed caps0x00c is outside this measurement; GB203/GB207 still have their existing named unmeasured warning/fallback there. This work supplies no invented0x00c value and claims no full-die acceptance. Physical declarations formerly left unclaimed now have a T3 source for this scope; colour-processing/FVLUT implementation coverage remains unverified. The T1 family report remains a report of the lower-tier virtual derivation, not the final T3 page.
