STATUS: ANSWERED, 2026-10-17. Native measurement/comparison complete; Windows cause unproven.

| Die | OS/driver | Flags | Result | Source/evidence |
|---|---|---|---|---|
| GA104 arch0x170/impl4 | Native Linux595.91.07 | Root read-only fixed page; no KF3 | PASS caps measurement,95 nonzero words | Own56326220; preflight/ampere595_native_caps.txt |
| GA104 | Public595.91.07 derived page versus native | SDR production derivation | FAIL exact equality:97 differing words |9d6f210ba; caps595_diff.tsv/equality.txt/receipt.json |
| GA104 | Windows580.88 binding/desktop/hold | OwnerP19 required if run | Still UNVERIFIED/UNRUN in this wave | No Windows result inferred from Linux measurement |

Hypothesis H-M1 was stated in PLAN.md before rental. Falsifier: derived page equals
the native page byte for byte but Windows still fails, ruling caps out as the sole
cause. MEASURED: pages differ at97 words, so the equality part did not fire and the
Windows-cause conclusion remains untested. A difference is not proof that a word
causes a defect, nor that a physical capability belongs in the virtual device.

MEASURED: root PROT_READ one-page tool reads native word0x00c=0x00000100; current
GA104/595 page leaves it0 and names it unmeasured. The native page has95 nonzero
words. The full4096-byte comparison includes native zero words and prints every
changed word with public header fields or explicit unknown labels plus the current
derivation basis. Source9d supplies the derived page; diagnostic comparator source
is pinned886f885c5ffaa9cb49df6058b1b05af61c32371c, compiled locally against9d libraries.
Native measurement is an oracle; no captured page, adjacent-die row or branch wildcard
is added. This595 measurement does not establish a Windows580 branch value.

MEASURED identity: UID65534/CapEff0 MC_GET_ARCH_INFO givesGA104arch0x170/impl4;
host open595.91.07, native gates9/9+gate10PASS/birthsOK. Guest apps/fullraw remain
pending and are not counted here. The genuine production/caps behavior of this
candidate must be checked in the next serial application batch.

Only owned56326220 active.175minute timer2714225 target04:25:28UTC; manual retirement
target03:55UTC and absolute API-absence deadline05:00UTC/07:00CEST. Rate0.1555555556/h
incl150GB; fulltimer compute ceiling0.46USD, campaign ceiling10.96USD+transfers.
No shared-host access, no master merge, D1-D6 remain open. Continue M3 native-first.
