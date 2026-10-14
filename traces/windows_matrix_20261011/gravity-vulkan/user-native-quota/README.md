# Windows native-attempt quota lifetime

STATUS: GPU-FREE TESTED, 2026-10-12. Runtime relay remains unconditionally disabled.

Hypothesis: native channel budget remains charged from BEFORE the native attempt
through confirmed cleanup, including missing native results and guest token Free.
Falsifier: another birth can pass the advertised cap while an old native attempt
may still exist. RED test reintroduced the old early free_channel release;
token_free_retains_native_quota_until_confirmed_cleanup failed deterministically.
Corrected ticket reserves the actual per-runlist VmCaps budget before VA/native
birth, transfers the same charge at token publication under one caps lock,
retains it across token retirement, and releases only for never-attempted birth
or confirmed native cleanup. Ticket Drop grants no cleanup credit. Publication
collision restores the reserved charge; duplicate attachment is refused.

Independent review found token retirement CAS and kernel ownership-marker removal
needed the SAME shared caps lock as allocation; fixed. Deterministic core test
pauses at that exact gap and checks replacement kernel ownership survives.
Six ticket tests cover uncertain birth, publication collision, quota retention,
duplicate attachment, BUSY worker and token reuse. Full core/qemu/rm/chan suite
PASS. Review cleared this quota scope. Runtime notifier/submission/migration and
native cutover remain open; this is not a GravityMark or latest-revision Linux
hardware pass. Busy cleanup conservatively quarantines the draft until session
teardown; no timer or unconfirmed native error releases its storage/counts.
