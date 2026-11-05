# GB202 Linux 580 desktop baseline

STATUS: LIVE, 2026-10-19. MEASURED FAIL presentation at product24906bf33,
stock host595.91.07, actual guest580.159.04/kernel6.8.0-147-generic.

Second/final B2 arm (2026-10-19),22:07:32..22:10:53UTC: isolated codec45a4d91b5,
QEMU SHA4e57223b50c474718c90b7939110a7e064ce74aad207a9a0b901235182b2590d,
unchanged broker24906bf33. Native candidate gates9/9+10/12USER PASS.
Guest580.159.04 again produced the same1920x1080 one-color black frame
(pixelSHA788ae0147bdf979a6575938ca2d7d4403788588f7be2010f03776c968fd1ab49),
one flip timeout,12nvAssert,0guestXid; host Xid5->5.
This falsifies this candidate as a sufficient B2 fix in this run. The public
format defect is CPU-proved but the black-frame cause remains unresolved.
No further B2 hardware attempts this lease. Disk-state confound: this candidate
used a new child of the stopped post-baseline parent, rather than the exact
pre-baseline snapshot. Evidence: b2_codec_candidate/; draftPR51 is unmerged.

Cinnamon1/Xorg1, NVIDIA GL renderer. Mode validation passed; the actual X11
broker window is1920x1080, one black color, pixelSHA788ae0147bdf979a6575938ca2d7d4403788588f7be2010f03776c968fd1ab49.
Guest:1flip timeout,12nvAssert,0Xid. Host Xid count5before/5after (no increase).
This is not a working desktop. Current source-built broker SHA001791413c1ab10c4eed8fbd60cd6bf7d55842774ab4060e5c573fdccefd25b8,
actual process/binary/flags receipt adjacent. Legacy broker built by prep was
not used at runtime. Real seat0/sessionc1/kfbench/Xorg; no session/pgrep stubs.

Preparation refusals are retained: missing pinned public nvkvm-pv reference
was fetched atbadf2d707da47a625e0ce69172a84785d20f81b8, and first broker
launch was NOTRUN because its user lacked X11 authorization (no QEMU started).
Installed the missing xhost tool, authorized only the actual local seat user
with X11's SI:localuser:kfbench, verified xdpyinfo as that user, then reran the
same binary. No launcher/no-QEMU/session/owned-receipt guard was bypassed.
The actual baseline (2026-10-19) ran21:24:54..21:27:51UTC; owned QEMU stop used documented
ACPI/TERM fallback. Original image is stopped; the second measured B2 arm is reported above.

SUPERSEDED candidate-status below (2026-10-19): the candidate is now complete and measured FAIL.
CODE READ independent lead: CA7D/CA7E DMA method/jump fields end at bit15;
current decoder and worker use legacy masks/4KiB clipping. Standalone test
branche3cc4570c records6runtime failures against public headers, with2legacy
bounds tests passing. This establishes a format bug, not yet the cause of this
black frame. A separate format-only candidate is in preparation; no second caps
group, private reply guess, pacing/clock/credit/lifecycle change is authorized
by this evidence. Future comparisons must record disk-state differences: this
baseline booted the prepared image directly, so its runtime writes are present
in the stopped parent. A later candidate PASS is functional evidence, not a
perfect same-snapshot causal comparison.
