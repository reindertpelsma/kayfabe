# ★★★★★ WHICH GUEST TOKENS WERE RUNG AND NEVER REACHED HARDWARE.
#
# Reads `DOORBELL-LEDGER tok=… passthrough=… emulated=… other=… forwarded=…` rows on stdin
# (with or without the `kayfabe: ` prefix) and prints the `tok=…` field of every row that was
# RUNG and never FORWARDED. That is the ledger's own rule, in one place:
#
#   > "forwarded=0 with emulated>0 means it never went to hardware; forwarded>0 means it did"
#
# ⊘⊘⊘ **THIS IS A SEPARATE FILE BECAUSE THE INLINE VERSION ROTTED SILENTLY.** `[measured
# w813d]` it printed `$1` — the literal word `DOORBELL-LEDGER`, since the rows are grepped WITH
# their prefix — so the caller's `grep -c 'tok='` counted zero, the gate stopped firing, and the
# suite "improved" from 16/30 to 23/30. ⚠ The score moving the RIGHT WAY was the symptom.
# ⇒ One statement of the rule, exercised by `gate_selftest.sh` with no GPU and no guest, so the
# failure mode "passes everything" is caught by a test instead of by a suite score.
#
# ⊘ Fields are matched BY NAME, never by position: a column added to the ledger tomorrow must
# not silently shift what this reads.
{
    e = 0; f = 0; tokf = ""
    for (i = 1; i <= NF; i++) {
        split($i, a, "=")
        if (a[1] == "tok")       tokf = $i
        if (a[1] == "emulated")  e = a[2]
        if (a[1] == "forwarded") f = a[2]
    }
    if (tokf != "" && e + 0 > 0 && f + 0 == 0) print tokf
}
