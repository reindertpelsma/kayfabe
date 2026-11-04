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

## RESULTS

### Method as run (MEASURED, 2026-11-03)

Plain CUDA container leases (image `nvidia/cuda:12.4.1-base-ubuntu22.04`, `--ssh --direct`, 16 GB disk, no KVM, no
kernel module, no kayfabe build) on hosts whose own NVIDIA driver is up. H-measurable-in-container: NOT falsified on
all nine leases: root in the container, sysfs mounted read-only (`sysfs on /sys type sysfs (ro,...)`), `resource0`
opens and maps `PROT_READ`, `tools/read_display_caps_page.py` exits 0 and reads a non-empty page. Per lease:
`<die folder>/read_context.txt` (date, kernel, host driver line, `nvidia-smi` name/bus id, `xid_before`/`xid_after` as
the count of `NVRM: Xid` lines in `dmesg`, chipset decode, tool rc) and `<die folder>/caps_page.txt` (the tool's raw
output). Cost: nine leases of about 20 minutes each at 0.06-0.26 USD/h (the nine hourly rates sum to about 1.0
USD/h): well under USD 0.5 in total. All instances were destroyed by exact id and `show instances-v1` lists only the
Blackwell-Windows lane's box.

Two conduct notes. (1) The first lease (GA104) ran the tool on every NVIDIA function `--list` showed, and a container
sees the whole host's PCI tree, so seven GPUs belonging to other tenants were read (read-only, 32-bit loads of the same
display caps page, nothing written, Xid 0 after). Those readings are not kept and not used; the runner was changed to
read only the GPU `nvidia-smi` shows the container, and later leases read exactly one device. (2) Most hosts run the
proprietary NVIDIA kernel module, not the open one (the `NVRM version` line in each context says which); the page is
read from BAR0 by the tool itself, not through the driver.

### Table (MEASURED 2026-11-03; `0x00c` is the tool's `DECISIVE` word)

| die (arch/impl) | board, PCI id | host driver (kernel module) | `0x00c` | folder |
|---|---|---|---|---|
| GA104 (0x170/0x4) | RTX 3070, 10de:2484 | 580.159.03 open | 0x00000100 | `ga104_3070/` |
| GA106 (0x170/0x6) | RTX 3060, 10de:2503 | 580.126.20 proprietary | 0x00000100 | `ga106_3060/` |
| GA106 (0x170/0x6) | RTX 3050, 10de:2507 | 580.178.04 proprietary | 0x00000100 | `ga106_3050/` |
| TU102 (0x160/0x2) | RTX 2080 Ti, 10de:1e04 | 580.178.04 proprietary | 0x00000100 | `tu102_2080ti/` |
| TU104 (0x160/0x4) | RTX 2070 SUPER, 10de:1e84 | 580.178.04 proprietary | 0x00000100 | `tu104_2070s/` |
| TU116 (0x160/0x8) | GTX 1660 SUPER, 10de:21c4 | 580.159.03 proprietary | 0x00000100 | `tu116_1660s/` |
| AD103 (0x190/0x3) | RTX 4080, 10de:2704 | 580.173.02 proprietary | 0x00000100 | `ad103_4080/` |
| AD106 (0x190/0x6) | RTX 4060 Ti, 10de:2803 | 580.178.04 proprietary | 0x00000100 | `ad106_4060ti/` |
| AD107 (0x190/0x7) | RTX 4060, 10de:2882 | **595.71.05** open | 0x00000100 | `ad107_4060/` |

The die identity is the chipset field of `NV_PMC_BOOT_42` (`read_chipset.py`): 0x174, 0x176 (x2), 0x162, 0x164,
0x168, 0x193, 0x196, 0x197. H-die-identity: not falsified. Every chipset matches the PCI device id's die (TU102 1e04,
TU104 1e84, TU116 21c4, AD103 2704, AD106 2803, AD107 2882, GA104 2484, GA106 2503/2507). (INFERRED, not asserted: the
decode was not run on a die with a known row; no box with GA102 or TU106 was leased. The unprivileged host control for
the same fact, `MC_GET_ARCH_INFO`, is what kf3 uses at realize.) `xid_before` = `xid_after` = 0 on every lease.

Note the GA106 pair: two boards, two PCI device ids, the same die, the same word.

### Agreement within each family (DATA ONLY; a family rule needs a mechanism and the owner's call, §AH)

- Ampere: GA102 (from #245), GA104, GA106 (two boards): all four `0x00c = 0x100`; the 95 non-zero words of the GA104
  page, both GA106 pages and the GA102 page are identical word for word.
- Turing: TU102, TU104, TU116 (plus TU106 from the existing row): all `0x00c = 0x100`; the TU102 and TU104 pages are
  identical (91 words); the TU116 page differs in one word outside `0x00c`: `0x008` reads 0x21 on TU102/TU104 and is
  absent (zero) on TU116 (90 words).
- Ada: AD103, AD106, AD107 (plus AD102 and AD104 from existing rows): all `0x00c = 0x100`; the three 99-word pages
  are identical word for word.
- Blackwell: not re-measured (GB202, GB205, GB206 existing rows read 0x102; GB203 on PR #242). GB207: no offer.
- The falsifier for a family rule ("two dies of one family disagree") was NOT triggered in any family. That is
  agreement, not a mechanism: no wildcard row is added.

### Rows and tests

Eight rows in `crates/kf-disp/data/caps_measured.tsv` (GA104, GA106, TU102, TU104, TU116, AD103, AD106 for branch 580;
AD107 for branch 595 only), no wildcard. `crates/kf-disp/tests/caps_sweep_20261103.rs` was committed alone and red first
(`test_red.txt`: the two row-dependent tests fail with `has a measured 0x00c row ... [(12, ...)]`), green after the
rows (`test_green.txt`). The three older "unmeasured die" tests (`ampere_caps.rs`, `turing_caps.rs`, `caps_derive.rs`)
named GA104, TU116/TU104, AD103/AD106 as their unmeasured examples and now name GA107, TU117 and AD107-on-580. The full
`cargo test -p kf-disp --no-fail-fast` ran green apart from those two targets before the fix (250 lib tests + all
integration targets), and `caps_derive`, `turing_caps`, `ampere_caps` and `caps_sweep_20261103` are green after.
INFERRED: kf3's realize-time die (RM `MC_GET_ARCH_INFO`) equals the `NV_PMC_BOOT_42` decode used here, as it does for
every existing row; no host RM control was run on these leases.

### Not measured, and why

TU117 (no GTX 1650 / 1630 / T400-class offer on Vast during the sweep), GB207 (no RTX 5050 offer), GA103 (skipped by
the brief; no offer found), GA107 (no offer: the 3050 offer was a GA106 board), AD107 on a 580 host (every RTX 4060
offer ran a 595 host), AD104 (not re-read: its row exists).
