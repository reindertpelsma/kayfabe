# GB202 Linux 580 desktop baseline

STATUS: LIVE, 2026-10-19. MEASURED FAIL presentation at product24906bf33,
stock host595.91.07, actual guest580.159.04/kernel6.8.0-147-generic.
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
The actual baseline ran21:24:54..21:27:51UTC; owned QEMU stop used documented
ACPI/TERM fallback. Original image is stopped; no second measured B2arm yet.

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
