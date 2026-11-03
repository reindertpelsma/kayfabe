# Display caps word 0x00c sweep over the unmeasured consumer dies (2026-11-03)

STATUS: RESEARCH, 2026-11-03 (pre-registration written and pushed BEFORE the first lease; results are appended under
"RESULTS"). MEASURED = a line in a committed raw log in a per-die folder next to this file; INFERRED = reasoning.

Stacked on PR #245 (`claude/ampere-hw-20261103`, the GA102 row). Tier 3 of OWNER_RULINGS §AH: the display caps page
(BAR0 0x640000) is reachable only through a privileged display object, so the value is a privileged measurement
in a sweep. Tool: `tools/read_display_caps_page.py` (root, read-only `PROT_READ` loads through sysfs `resource0`;
nothing written, no ioctl, no unbind). A die with no row gets the clean page and a notice; a family rule needs
every measured die to agree plus a mechanism (§AH). This lane produces rows per die and reports agreement as data
only; it adds no family wildcard.

## Dies already covered at the base (skipped)

`caps_measured.tsv` at #245: AD104 (0x190/4), AD102 (0x190/2), TU106 (0x160/6), GA102 (0x170/2), GB202, GB205,
GB206. PR #243 (Turing, TU106) and PR #242 (GB203, 580 and 595) add no other die. Targets here: GA104, GA106,
GA107, GA103; TU102, TU104, TU116, TU117; AD103, AD106, AD107; GB207. AD104 is not re-read: its row exists
(the owner's PC would add nothing).

## Question and falsifier (pre-registered)

- **Q: does `0x00c` differ per die within a family?** No value is predicted. Every measured die so far reads
  `0x100` (Turing, Ada, Ampere) or `0x102` (Blackwell).
  - Data only: the family values are reported as measured. FALSIFIED for a family rule (the rule is not
    available) if two measured dies of one family disagree. Agreement of all measured dies does NOT establish a
    rule: that needs a mechanism and the owner's call.
- **H-measurable-in-container**: the tool runs on a plain CUDA container lease (host driver up, root in the
  container, sysfs `resource0` readable). FALSIFIER: the tool exits non-zero, or the page reads empty/all-ones.
  Then a KVM lease with the host driver installed is tried (cheap ones only).
- **H-die-identity**: the die's (architecture, implementation) is read from the same BAR0 mapping,
  `NV_PMC_BOOT_42` (0xa00) bits 28:20 = chipset, architecture = chipset & 0x1f0, implementation = chipset & 0xf
  (read-only, PROT_READ, script `read_chipset.py` in this folder). Check: the same read on a die with a known row
  must give its row's (architecture, implementation) (GA102 0x172, TU106 0x166 in the existing rows). FALSIFIER:
  a chipset that does not match the PCI device id family, or a decode that contradicts a known die. Then the die is
  not claimed.
- **Driver branch**: the host driver version is recorded exactly (`nvidia-smi`, `/proc/driver/nvidia/version`). The
  row's branch column is the guest driver branch. A row is written for branch 580 only from a host on 580.x;
  a host on another branch yields a row for that branch only if kayfabe ships class tables for it (580 and 595
  are the supported pair), otherwise the die is reported as read-but-not-rowed (the value is recorded in the
  trace, not in the table). INFERRED, not assumed: a 595 host read stands in for a 580 guest only where an
  existing 580 oracle for the same family already agrees (AD104, GB202); nothing else gets a cross-branch row.
- **Rules**: one short lease per die, at most 30 min, at most two failed attempts per die, total spend at most
  about USD 3, boxes destroyed by exact own id. GPU UUIDs are never committed (`scripts/ci/no_gpu_uuids.py`).
  Xid count before/after is recorded where `dmesg` is readable (a container may not allow it: noted if so).
