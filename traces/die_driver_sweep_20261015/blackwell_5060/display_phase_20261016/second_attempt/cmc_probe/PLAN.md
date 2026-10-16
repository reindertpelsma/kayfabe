# B1 rank2: Windows CMC log-buffer refusal sensitivity

**STATUS: LIVE, 2026-10-16 21:02UTC.** Hardware UNRUN; next serial experiment on only own56286918.

INFERRED H-CMC: refusing pure-[In] a89 contributes to Windows580.88/8088 Code43.
FALSIFIER before run: a89 is actually acknowledged NV_OK but the first teardown boundary
remains277 (and explicit driver unload384), compared to same-binary probe OFF.
If the boundary stays, reject rank2; no CCU probe. CBCB remains NOT DERIVABLE120B/43distinct.
Public595.91.07 kernel_gsplite.c handles NOT_SUPPORTED nonfatally; this weakens H-CMC but
does not establish closed Windows behavior. Guest CPU polling of its buffer is UNVERIFIED;
QGA responsiveness alone is insufficient evidence that polling never happens.

Clean draftPR30: testd7be19b8f then bounded echo73af410b6, based on owner topic1cb048610.
Two behavior assertions fail first. All791kf-rm CPU tests pass after the fix.
Test setup negatives preserved: nonexistent cmd_off field; serialization fixture used bit0
instead of documented bit1. C compiler against public580.65.06 tag307159f2623d3bf45feb9177bd2da52ffbc5ddf9:
command0x20800a89, sizeof24, PublicId0/logBufferSize8/logBufferAddr16.
The link is constructed only with host KF3_CMC_LOG_BUFFER_PROBE=1; default profile is unchanged.
Only this verified ABI, matching RmControl,24 bytes, nonserialized, exact bounded packet length.
Input addresses/sizes are never dereferenced, mapped, retained, forwarded or logged; no firmware
logger and no GPU completion is implemented. This measures reachability, not product correctness.

Integrate onto36d134bc5, native gates→private build, then fresh same-parent580 siblings with
identical8vCPU/8GiB/4GiBFB/UUID/VARS/displayON. First21 Windows flags (P19+two bounded trace flags),
second adds only CMC diagnostic flag. Assert actual process environment/source/SHA/host595/build8088/GSP1.
Export bounded metadata/public display bodies plus a89 STATUS/SIZE ONLY; never a89 address bytes.
Clean owned shutdown after each. Stop at21:50UTC for teardown margin; original timer22:06:28,
hard22:11:25. No owner GPU/VM or third lease. Current campaign ceiling9.23USD plus transfers.
