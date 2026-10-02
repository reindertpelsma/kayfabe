# V3 sweep and install — every driver pair on every GPU family, tested on the binary users install

**STATUS: DESIGN, 2026-10-02.** Nothing in this document is built. Branch `v3-sweep`, cut from master
`d88639f9`; not merged. This document answers the owner's plan of 2026-10-02, which is not yet recorded in
`docs/OWNER_RULINGS.md`. The plan has three parts:

1. Adopt nvkvm-pv's unattended coverage sweep so that it tests kayfabe.
2. Support every GSP driver we want on both driver axes, and test host/guest driver mismatch "vGPU style".
3. Make kayfabe installable as one binary, QEMU with kf3 built in, with the optional guest module shipped
   separately.

Later items appear only where they constrain this design: the display broker, a small UEFI GOP option ROM,
Windows, UVM and a security review.

The inputs were three read-only studies of 2026-10-02: nvkvm-pv's sweep, kayfabe's test infrastructure and
the install artifact. Everything load-bearing was re-checked against the files cited here. Related documents:

- `V3_DRIVER_MATRIX.md` §5.0 describes the per-box runner. This design wraps that runner; it does not
  replace it.
- `scripts/bench/box/README.md` holds the box rules, which this design inherits unchanged.
- `OWNER_RULINGS.md` C.5 is the ruling on GPU-family coverage.

**Citation conventions:**

- A path without a prefix is in this repository.
- `nvkvm-pv:` means the sibling repository `reindertpelsma/nvkvm-pv` at `368d2db`.
- `qemu-10.2.4/` means the upstream QEMU 10.2.4 release tarball.
- Estimates are labelled as estimates.

## 0. One screen

- **Shape.** The sweep has two layers.
  - **The coordinator** runs on a trusted machine and is adopted from nvkvm-pv. It writes the plan, rents
    one vast KVM box per GPU family, guards the money and keeps the ledger. The families are Turing,
    Ampere, Ada and Blackwell.
  - **Kayfabe's existing per-box runner** does the work inside each box: `scripts/drivermatrix/sweep.sh`
    swaps host drivers, runs bare metal first, runs the gates, and runs the thin and fat guests.
  - Hopper, GA100 and GB10x stay source-derived until a VM host exists (`OWNER_RULINGS.md:63-67`).
- **Which drivers.** Kayfabe's own ABI evidence chooses them, not nvkvm-pv's profiles.
  - The 29 measured tags fall into 22 guest-axis classes and 17 host-axis classes of identical consumed
    layouts (§1.4).
  - The sweep takes one representative per class on each axis, plus the behaviour boundaries that layouts
    cannot show.
  - It adds the band of mismatch cells: per host, the exact guest, a same-branch minor and n−1 (§1.5).
- **No false passes.**
  - Every unit is listed before the first rental, and a unit without a verdict is UNTESTED.
  - Driver versions are checked on content on both sides.
  - The exit code ranks a failure above an untested unit (§1.8).
- **Install.** One relocatable tarball holds QEMU 10.2.4 with kf3 compiled in, a launcher, a preflight
  check and a manifest. One script builds it, in a pinned container in GitHub CI. The sweep installs that
  same tarball by sha256 (§2.8), so a support claim is a claim about the bytes users download.
- **Cost (estimates, §1.12).**

  | tier | box time | compute cost | wall time |
  |---|---|---|---|
  | smoke | 30–36 min per family (pace taken from the 2026-09-30 chain logs), about 2.4 box-hours | $0.40–1.20 | about 40 min on 4 boxes |
  | full | about 60 box-hours | $9–30 | 8–10 h on 8 boxes |
- **Owner decisions.** Several are needed before a public binary or a full sweep; they are listed in §4.

## 1. The sweep

### 1.1 Two harnesses, one layer apart

| | nvkvm-pv `scripts/sweep.sh` (3229 lines) | kayfabe `scripts/drivermatrix/sweep.sh` (538 lines) |
|---|---|---|
| runs on | the operator's trusted machine, driving boxes over ssh/scp (`nvkvm-pv:scripts/sweep.sh:875-948`) | one READY box, as one detached job (`V3_DRIVER_MATRIX.md:392-397`) |
| rents and destroys boxes | yes: registry, spend cap, auto-destroy timer (`nvkvm-pv:scripts/sweep.sh:650-691`, `:2257-2292`) | no |
| driver axes | host only; the guest uses the host's userspace (`nvkvm-pv:scripts/sweep.sh:1853-1872`) | host × guest (`scripts/drivermatrix/sweep.sh:111-113`, `:323-331`) |
| verdict | `tests/validate.sh`'s exit code, kept verbatim | queue-log lines that `scripts/drivermatrix/matrix_table.py` turns into one grid per die |
| exit code | 0–4, computed from ledger rows (`nvkvm-pv:scripts/sweep.sh:96-105`, `:3191-3228`) | `finish 0` after any normal run, whatever the rows said (`scripts/drivermatrix/sweep.sh:538`); 3 on a dead GPU or failed build (`:272`) |
| run on hardware | yes (`nvkvm-pv:sweep-runs/verify-main-arch/`) | not yet (`V3_DRIVER_MATRIX.md:385-390`) |

They complement each other. nvkvm-pv has the outer loop that kayfabe lacks: renting, money safety, the
ledger and the exit codes. Kayfabe has a box-side runner whose shape nvkvm-pv cannot provide, because it
has two driver axes, the 580-only grader and fat-guest ladders. The adoption is therefore nvkvm-pv's outer
loop around kayfabe's inner runner. Nothing on the box comes from nvkvm-pv's Mode-1 path.

### 1.2 What is copied, adapted and dropped

The coordinator is vendored into `scripts/sweep/` from `nvkvm-pv@368d2db`. A `VENDORED_FROM` file records
the commit, and every fix is made in both repositories (task S9). Both repositories belong to the same
owner, and the box steps differ, so vendoring is cheaper than a shared library. Extract a shared library
only if the copies drift.

**Copied as-is:**

- **Auto-destroy timer** (`nvkvm-pv:scripts/sweep_autodestroy.sh`, launched by `nvkvm-pv:scripts/sweep.sh:657-691`).
  - A separate process, started with `setsid nohup` from its own file.
  - Its pid is checked with `ps` before the first rent; the sweep refuses to rent without it.
  - Its fail-open listing must be fixed first (§1.3, item 1).
- **Registry.** An instance id is appended and synced before anything touches the box
  (`nvkvm-pv:scripts/sweep.sh:650`, `:2292`). The spend cap is checked before every create
  (`nvkvm-pv:scripts/sweep.sh:2257-2267`). A STOP file is honoured between rows, hosts and boxes.
- **Offer choice and box liveness.**
  - Offers are ranked by what the job will cost (compute, disk and transfer). Marketplace numbers are
    coerced to numbers, and the advertised driver is ignored (`nvkvm-pv:scripts/sweep.sh:951-1115`), once
    the duplicate block in §1.3 item 7 is deleted.
  - Dead and slow boxes are told apart by the frozen `Domain not found` log signature, which must be
    byte-identical twice at least 120 s apart (`nvkvm-pv:scripts/sweep.sh:1200-1257`).
  - The committed known-bad machine list stays (`nvkvm-pv:scripts/sweep-known-bad-machines.txt`).
- **VM check:** `systemd-detect-virt` must report `kvm` or `qemu`, and `/dev/kvm` must exist
  (`nvkvm-pv:scripts/sweep.sh:1259-1268`).
- **Untrusted-endpoint discipline** (`nvkvm-pv:scripts/sweep.sh:875-948`, `:1498-1550`,
  `nvkvm-pv:scripts/sweep_parse_steamos.py`):
  - host and port are checked against allowlists, and ssh is built as an argv array with no `eval`;
  - an empty target is refused and stdin is `</dev/null`;
  - cached files are relayed by sha256 with an atomic rename;
  - box-produced text is parsed against allowlists, kept to printable characters and truncated.
- **Ledger and renderer.**
  - The JSONL ledger is fsynced per row, and `jrec` embeds child JSON safely (`nvkvm-pv:scripts/sweep.sh:466-484`, `:2208`).
  - `nvkvm-pv:scripts/sweep_matrix_md.py` merges ledgers; every non-verdict cell becomes
    `UNTESTED — <reason>` and is never blank.
- **Detached long stages.** A long stage runs detached with a DONE sentinel, and only logs and allowlisted
  verdicts come back (`nvkvm-pv:scripts/sweep.sh:2697-2767`). This fits the later display, broker and Windows stages.
- **Offline tests in library mode.** The coordinator is sourced with a stub `vastai` on `PATH`
  (`nvkvm-pv:tests/sweep_offline_test.sh`).
- **Driver availability.** The rows of `nvkvm-pv:scripts/sweep-driver-availability.tsv` for kayfabe's
  tags are copied, together with its warning that a 403 is a geo-redirect to the `.cn` CDN, not a missing
  file (`:16-24`).

**Adapted:**

| piece | nvkvm-pv today | kayfabe |
|---|---|---|
| transport | ssh/scp only | one interface with two backends. ssh is for the dev host. `vx` is for attended cloud sessions, which have no SSH egress (`scripts/bench/box/README.md:84-99`). |
| code on the box | the working tree is tarred and scp'd, then QEMU is built per box (`nvkvm-pv:scripts/sweep.sh:1552-1683`) | the release artifact, by sha256 (§2.8). Test instruments (raw client, thin initrd) are still built from the same revision. |
| host driver install | `sweep_matrix.install_driver` | `scripts/bench/provision_host_driver.sh`, plus nvkvm-pv's `.run --check`, a per-tag sha256 pin and the 404-versus-transport classification (`nvkvm-pv:scripts/sweep_matrix.py:524-728`). The kayfabe script already uses the open module and the kernel's own compiler, and checks the result on content (`provision_host_driver.sh:124-165`). |
| work on the box | `boot_and_validate`, Mode 1 (`nvkvm-pv:scripts/sweep.sh:1845-2079`) | kayfabe's per-box runner and the lanes of §1.6 |
| driver choice | profiles and same-profile alternates from `nvkvm_abi.h` (`nvkvm-pv:scripts/sweep.sh:293-299`, `:486-537`) | kayfabe's measured classes (§1.4). There are no alternates across tags, because NVIDIA moves ABI inside a branch (`V3_DRIVER_MATRIX.md:129-134`). |
| planner | `sweep_plan.py`, which `sweep.sh` never reads | `scripts/sweep/plan.py`, whose `plan.json` is the coordinator's only input |
| GPU family | marketing-name map | the name is an advisory filter before renting. The die derived on the box from the PCI id decides (`scripts/drivermatrix/sweep.sh:214-243`). |
| keep-on-error, deadline, exit code, resume key | global; computed from rows; keyed on (arch, driver) | per box; computed from planned units; keyed on the full unit identity (§1.8) |
| control run | the preinstalled driver, run first and not counted | the same, as bare `cup2` plus gates on the template's preinstalled driver before the purge. That driver is the closed 575.51.03 module (`V3_DRIVER_MATRIX.md:857`; `provision_host_driver.sh:6-7`). |

**Dropped:**

- **Reaping by label**, including the `--reconcile` prefix mode, which destroys other running sweeps'
  boxes (`nvkvm-pv:scripts/sweep.sh:742-791`, `:2930-2944`). Kayfabe tears down by id only, on a shared
  account (`scripts/bench/box/README.md:25-26`).
- **The hard-coded protected-id list** (`nvkvm-pv:scripts/sweep.sh:261`). It is unnecessary when only
  this run's own ids are ever touched.
- **All of Mode 1:** the host-libs bundle, the 9p mount, `nvkvm-guest.service`, the ABI-profile journal
  grep, the DENY/AUDIT counters and validate.sh's Mode-1 checks.
- **The SteamOS and Kata stages.**
- **Recording raw `vastai create` output** (§1.3, item 4).

### 1.3 Defects to fix in the vendored copy, and upstream to nvkvm-pv

The 2026-10-02 study reproduced items 1–3 offline with a stub `vastai` in a scratch directory. It touched
neither repository nor vast. The code paths named here were re-read on 2026-10-02.

| # | defect | where | consequence | fix |
|---|---|---|---|---|
| 1 | **Fail-open listing.** `live_ids` prints nothing when the listing fails ("say nothing rather than lie"), and every caller reads nothing as "gone". | `nvkvm-pv:scripts/sweep_autodestroy.sh:84-96`, `:128-137`, `:145-151`; `nvkvm-pv:scripts/sweep.sh:702-737`, `:794-844`. The same shape exists in kayfabe's own `scripts/bench/box/vast_reaper.sh:28-34`, `:48-52`. | At the deadline the timer destroys nothing, despite its "unconditionally" contract (`:31-35`). The early stand-down fires, `destroy_verified` reports "verified absent", and exit 4 can never fire. | The listing returns ALIVE, GONE or UNKNOWN, and UNKNOWN counts as alive. At a deadline, `destroy` is issued for every registered id whatever the listing says; destroy is idempotent and ids are never reused (`:31-35`). A run that ends with any UNKNOWN exits 4. |
| 2 | **`BOX_FAILED` is never reset.** | `nvkvm-pv:scripts/sweep.sh:2239` (global), `:2368` | A healthy box rented after a failing one is kept. In `verify-main-arch`, box 49460345 passed 4/4 and was kept "because 5 failed". | Count per box. |
| 3 | **The exit code counts rows, not units.** | `nvkvm-pv:scripts/sweep.sh:3191-3228`; a malformed line is skipped silently (`:3200-3201`) | A family skipped by STOP produces no row, so the run exits 0. A box failure that a retry box fully covered still exits 2. | Units are planned before renting (§1.8). |
| 4 | **Raw create output is printed and recorded** when the id parse fails. | `nvkvm-pv:scripts/sweep.sh:2285-2289` | That output can carry the `instance_api_key`, which must never be printed or recorded (`scripts/bench/box/README.md:23-24`). | Reduce the output to `{success, new_contract}` before any print or record. On a parse failure, stop with exit 4 and list label-matched candidate ids for a human; never destroy them automatically, because they might not be ours. |
| 5 | **`arch_of` misfiles Turing Quadros.** | `nvkvm-pv:scripts/sweep_matrix.py:188-214` | Checked 2026-10-02: `Q RTX 4000` maps to ada, `Q RTX 5000` to blackwell, and `Q RTX 6000` and `TITAN RTX` to nothing. | The die derived on the box decides; a name only filters offers. |
| 6 | **One global deadline**, never consulted before renting a later box. | `nvkvm-pv:scripts/sweep.sh:663`, `:3159-3178` | A late box can be destroyed mid-run. | A deadline per box, sized from its plan shard. |
| 7 | **A leftover uncoerced duplicate of the offer filter.** | `nvkvm-pv:scripts/sweep.sh:1056-1069` | One price sent as a string crashes the offer search, and every family reports no offer. | Delete the duplicate. |
| 8 | **The open-module "forced install" forces nothing**, because the helper's fast path checks only the version. | `nvkvm-pv:scripts/sweep.sh:1753-1760`; `nvkvm-pv:scripts/sweep_matrix.py:560-563` | A closed module of the right version passes the fast path. | Kayfabe's installer checks both version and flavour on content (`provision_host_driver.sh:152-165`). |
| 9 | **Auto-blacklisting writes a tracked file**, so the next run refuses the dirty tree. | `nvkvm-pv:scripts/sweep.sh:635-641`, `:2981-2989` | — | Write to the run directory; a human commits it. |

### 1.4 Driver selection: kayfabe's own ABI boundaries

**The rule.** Version-specific behaviour is keyed on the exact measured tag (`V3_DRIVER_MATRIX.md:129-134`,
`:307-310`). An unmeasured version is refused by name: `AbiError::Unmeasured` for a guest and
`HostAbiError::Unmeasured` for a host (`crates/kf-abi/src/hostabi.rs:184-197`; `V3_DRIVER_MATRIX.md:3-8`).
So a boundary is wherever a consumed item changes. A sweep that crosses every boundary on an axis needs at
least one representative of each class of tags that are identical in every item that axis consumes.

**Method.** This derivation was run on 2026-10-02 at `d88639f9` as a read-only script; committing it is
task S6.

- The axis of a consumed item is its section of `tools/drivermatrix/consumed.txt`. Lines 6–45 are the
  guest axis, 46–233 both axes and 234–313 the host axis. A "both axes" item counts on both.
- For every item, `traces/driver_matrix/ranges.tsv` lists the runs of tags over which it is identical. It
  holds only consumed items (`V3_DRIVER_MATRIX.md:270-271`). The runs tile all 29 tags with no gaps.
- A run that starts after the first tag is a boundary at that tag. A class is a maximal stretch of
  consecutive tags with no boundary on that axis.

**Result:** 22 classes on the guest axis and 17 on the host axis. `.run` availability comes from
`nvkvm-pv:scripts/sweep-driver-availability.tsv`.

| tag | guest class | host class | public `.run` | note |
|---|---|---|---|---|
| 535.309.01 | G1 | H1 | yes | guest stops at `_gpuInitChipInfo`, carried since; re-run queued (`V3_DRIVER_MATRIX.md:532`). ISA 8.2 JIT floor (`:89`) |
| 545.23.08 | G2 | H2 | **no** | guest firmware from a CUDA-repo deb (`:483-485`); a 545 host does not build on Linux 6.8 (`:740`) |
| 550.40.07 | G3 | H3 | yes | range unmap begins here on the host (`:194`) |
| 550.54.14 | G4 | H4 | yes | fat guest not staged on the image kernel 6.8.0-139 (`:529`) |
| 550.90.07 | G5 | H5 | yes | |
| 555.42.02 | G6 | H6 | yes | |
| 560.28.03 | G7 | H7 | yes | |
| 565.57.01 | G8 | H8 | yes | ladder 0/4 (`:528`) |
| 570.86.15 | G9 | H9 | yes (`tesla/`) | |
| 570.124.06 | G10 | H9 | yes (`tesla/`) | |
| 570.148.08 | G11 | H9 | yes (`tesla/`) | guest ladder 0/4, blocked by the UVM first-channel wall (`:602-610`) |
| 575.51.02 | G12 | H10 | yes | |
| 575.51.03 | G12 | H10 | **no** | |
| 575.57.08 | G12 | H10 | yes | |
| 575.64.05 | G12 | H10 | yes | last tag with the fn 54/79 carrier (`:178`) |
| 580.65.06 | G13 | H11 | yes | |
| 580.82.07 | G14 | H12 | yes | |
| 580.94.02 | G15 | H13 | **no** | |
| 580.95.05 | G15 | H13 | yes | |
| 580.105.08 | G16 | H13 | yes | |
| 580.126.09 | G16 | H13 | yes | |
| 580.159.04 | G17 | H13 | yes | the reference pair |
| 580.173.02 | G18 | H13 | yes | |
| 580.178.04 | G18 | H13 | yes | |
| 590.48.01 | G19 | H14 | yes | |
| 595.84 | G20 | H15 | yes | |
| 610.43.02 | G21 | H16 | yes | |
| 610.57.04 | G21 | H16 | yes | |
| 615.71.09 | G22 | H17 | not in the table | out of the asked range by ruling (`V3_DRIVER_MATRIX.md:852-853`) |

Note: a "both axes" item counts on both, so some host classes may be finer than the host strictly needs.
That costs extra rows, never a missed boundary.

**Behaviour boundaries that layouts do not show.** Each one must be crossed by a lane that exercises it,
not only by a layout check:

- **575.64.05 → 580.65.06.** Up to 575.64.05, page-directory statements arrive as the dedicated RPCs fn 54
  and fn 79; from 580.65.06 they are a control (G11; `V3_DRIVER_MATRIX.md:178`, `:650-664`). This was the
  `cuInit` wall, so a ladder must cross it.
- **580.65.06.** Four things start here:
  - the per-map PTE kind on the host (`:218-227`);
  - the interrupt subtree-map control (`:737-738`);
  - the CDP floor (`:15-16`);
  - the lower end of the 30-arm grader's interval (`:777-783`).
- **595.84.** The guest reads two GSP heartbeats after every RPC (`:718-730`).
- **610.43.02 on GA10x.** Channel ids are per runlist (`:530`).
- **570.x.**
  - The CeUtils self-test opens with the sysmem copy (`:526`).
  - nvidia-uvm reads its first channel before the mirror places it (`:602-610`).
- **Blackwell on vast's KVM template.** Hosts below 580 hang in `RmInitAdapter` on the missing PCI
  function 1. nvkvm-pv measured this on 2026-08-30 on GB205 and GB203
  (`nvkvm-pv:scripts/sweep_matrix.py:152-176`).
  - kf3 presents a single PCI function, so a 570 *guest* on an emulated GB20x may take the same path inside
    the guest.
  - That is unmeasured, so the plan carries the cell.

**Representatives.** The representative of each class is its latest tag with a public `.run`, unless an
older tag of the class is already measured. In that case the measured one is kept, for continuity with
`V3_DRIVER_MATRIX.md` §6.0.

- **Guest axis, 21 in range:** 535.309.01, 545.23.08 (thin guest only), 550.40.07, 550.54.14, 550.90.07,
  555.42.02, 560.28.03, 565.57.01, 570.86.15, 570.124.06, 570.148.08, 575.57.08, 580.65.06, 580.82.07,
  580.95.05, 580.105.08, 580.159.04, 580.173.02, 590.48.01, 595.84, 610.57.04.
- **Host axis, 15 obtainable:** 535.309.01, 550.40.07, 550.54.14, 550.90.07, 555.42.02, 560.28.03,
  565.57.01, 570.148.08, 575.57.08, 580.65.06, 580.82.07, 580.159.04 (reference), 590.48.01, 595.84,
  610.57.04.

**Exclusions.** These are recorded with their reason, never dropped:

- **Unobtainable**, with no public `.run`: 545.23.08, 575.51.03 and 580.94.02.
  - On the host axis they cannot be installed.
  - On the guest axis the thin guest can still be staged, with firmware from the CUDA repository
    (`scripts/drivermatrix/stage_guest_driver.sh:92-133`). The fat guest needs the `.run`
    (`scripts/drivermatrix/stage_fat_guest.sh:33`).
  - The classes of 575.51.03 and 580.94.02 are covered by 575.57.08 and 580.95.05. Class G2/H2 (545.23.08)
    has no substitute: its host cell and its guest ladder are `UNTESTED(unobtainable)`.
- **Negative control:** 615.71.09.
  - A 615 guest must be refused by name (`NoEncoding`) before any work runs, so the cell passes when the
    refusal fires.
  - It runs only if its firmware is obtainable; otherwise it is `UNTESTED(unobtainable)`.
- **Environment:**
  - Blackwell hosts below 580 on vast's template (above).
  - Pre-550 hosts may need nvkvm-pv's 5.15 host-kernel switch (`nvkvm-pv:scripts/sweep.sh:2081-2175`).
  - A 545 host does not build on 6.8 (`V3_DRIVER_MATRIX.md:740`).
- **Driver predates the GPU.** This is decided on the box, from `nvidia-smi` after a clean install
  (`nvkvm-pv:scripts/sweep.sh:2521-2559`). It is listed, never counted.

A driver release newer than tag 29 enters by `tools/drivermatrix/regen.sh <tag>` (`V3_DRIVER_MATRIX.md:3-6`).
The sweep's plan then reclassifies it, and `plan.json` gains its cells; no list is edited by hand.

### 1.5 The band — host/guest mismatch, vGPU style

The band comes from `V3_DRIVER_MATRIX.md:155-157`, the support asymmetry ruled by the owner on 2026-08-09:
an exact match, same-branch minors and n−1 must work; more mismatch is an extra. The host and guest
versions are read from two different places and never assumed equal (`:151-154`).

**Definitions used:**

- Majors are ordered as they occur in `tools/drivermatrix/tags.txt`: 535, 545, 550, 555, 560, 565, 570,
  575, 580, 590, 595, 610.
- n−1 means the previous major in that list, and the cell takes that major's guest representative.
  Whether feature branches count is §4 Q4.

**Every host H in the plan gets these guest cells:**

| cell | guest | lane | required? |
|---|---|---|---|
| E | H itself | thin + ladder if H is 580.x, else ladder | band, must pass |
| M | one other measured tag of H's major with a fat guest, preferring another guest class | as for E | band, must pass |
| P | the n−1 representative | ladder | band, must pass |
| D | 580.159.04 | thin + ladder | needed anyway: the thin grader accepts only `[580.65.06, 581)` guests (`V3_DRIVER_MATRIX.md:777-783`). For hosts below 580 this guest is *newer* than the host; for 590/595/610 it is n−1 to n−3. Recorded as band or extra by that rule. |
| X | the 2026-09-26 walk's mixed guests, 590.48.01 and 575.57.08 (`scripts/drivermatrix/sweep.sh:113`) | ladder | extra, for continuity; band tier only |

Per host, this gives:

| host | E | M | P |
|---|---|---|---|
| 535.309.01 | itself | — | — (nothing below 535 is in range) |
| 550.40.07 / 550.54.14 / 550.90.07 | itself | another 550 | 545.23.08: `init` only, no fat guest |
| 555.42.02 | itself | — | 550.90.07 |
| 560.28.03 | itself | — | 555.42.02 |
| 565.57.01 | itself | — | 560.28.03 |
| 570.148.08 | itself | 570.124.06 | 565.57.01 |
| 575.57.08 | itself | 575.64.05 | 570.148.08 |
| 580.65.06 / 580.82.07 | itself (thin + ladder) | 580.159.04 (= D) | 575.57.08 |
| 580.159.04 (reference) | every guest representative (21) | — | — |
| 590.48.01 | itself | — | 580.159.04 (= D) |
| 595.84 | itself | — | 590.48.01 |
| 610.57.04 | itself | 610.43.02 | 595.84 |

**A gap this design does not close.** `V3_DRIVER_MATRIX.md:156-157` says out-of-band pairs must be refused
by name. No pair policy exists in code: a grep of `crates/kf-*` and `qemu/hw/misc/kf3` on 2026-10-02 found
none. Any measured host is accepted with any measured guest. Until the owner fixes the band's definition
(§4 Q4) and a policy is built, the sweep records out-of-band pairs as extras. It does not expect a
refusal.

### 1.6 The per-box test

Every box runs the lanes below in this order, one GPU job at a time (`scripts/bench/box/README.md:27`).
"Expected" figures come from the 2026-09-30 logs in `traces/v3_families_master/{ada1,bw1}.tgz`,
`traces/v3_turing_master/tu116_1915bd71.tgz` and `traces/v3_mc23/mc23_afb552ea.tgz`, and from
`traces/driver_matrix/walk/kfh/summary/hostwalk2.log` (2026-09-26), unless marked as an estimate. Bounds
are the runner's `T_*` defaults (`scripts/drivermatrix/sweep.sh:120-126`).

| # | lane | runs for | grades | expected | bound |
|---|---|---|---|---|---|
| 0 | box acceptance | every box | VM and `/dev/kvm`; root disk ≥ 60 GB for smoke and ≥ 100 GB for full shards (the 100 is an estimate: about 20 fat overlays and 15 `.run` files); die family = the requested family; GPU health (`scripts/drivermatrix/sweep.sh:251-260`) | < 2 min (estimate) | 10 min |
| 1 | provision + artifact | every box | `provision_full.sh` steps for the instruments (raw client, thin initrd); the artifact checked by sha256 and by `-device help` listing `kf3-gpu` (§2.8) | READY in 7.1 min (TU116), 8.3 min (GB205), 11.8 min (AD104) on 2026-09-30 | 60 min |
| 2 | control | every box | bare `cup2` + gates on the preinstalled driver, before the purge; not counted | ~2 min (estimate) | 30 min |
| 3 | host swap | each non-reference host | open module and exact version, on content (`provision_host_driver.sh:152-165`); `.run` sha256 pin | part of ~37 min per host (hostwalk2.log, 2026-09-26: 21:25:32 → 23:17:54 for three full hosts plus three failed swaps) | `T_SWAP` 2400 s |
| 4 | bare metal | every host | `cuda_ladder.sh host`: all four rungs on the reference host, `cup2` elsewhere. A failure excludes the host; it is not a kayfabe failure (`OWNER_RULINGS.md:32-33`). | 9 s for four rungs (2026-09-30) | `T_BARE` 1800 s |
| 5 | gates | every host | `scripts/bench/v3_gates.sh` 9/9 | 40–61 s (2026-09-30) | `T_GATES` 3600 s |
| 6 | bare 30-arm suite | 580.x hosts in `[580.65.06, 581)` only (grader limit) | `BARE_SUITE_PASS=30` (`scripts/bench/box/merge_check.sh:49-50`) | 177 s of arm time (mc23, 2026-09-30) | 1800 s |
| 7 | canary | every host | one `--timer` arm; a failure skips that host's thin rows (`V3_DRIVER_MATRIX.md:398-405`) | ~3 min (estimate) | `T_CANARY` 1200 s |
| 8 | thin 30-arm | 580.x guests | `fast_suite.sh` at budget 180, `p/n` with n > 0 | 10.4–18 min wall (2026-09-30) | `T_THIN` 10800 s |
| 9 | fat ladder | every guest in the host's set | `cup2` CE `0xabcd1234`, `cup3` 43, `cup8` `bad=0 maxerr=0`, `cup8bench` PASS (`scripts/bench/cuda_ladder.sh:39-53`), `reps=1`, denominator = planned | 3.8–4.4 min (2026-09-30) plus about 6 min of staging per new guest (estimate) | `T_LADDER` = 4 × 1800 + 600 s |
| 10 | init | non-580 guests, reference host | `failure_point.sh`: how far the guest's RM got. This is diagnostic; the ladder is the verdict (`V3_DRIVER_MATRIX.md:501-503`). | ~3 min (estimate) | 1200 s |
| 11 | display (optional) | reference pair, guest 580.159.04 only | `scripts/bench/display/lane.sh` `DISPLAY_*` lines | M1/M2 1–2 min, M3 4.5 min (2026-09-30) | 1800 s |
| 12 | apps subset (optional) | reference pair, Ampere only for now | the app matrix minus `vkpeak`, `clpeak`, `gpu_burn` and `geekbench` (the last passes on an upload, `scripts/apps/run_apps.sh:96`) | guest about 370 s, plus 20–40 min of app provisioning (estimate) | 3 h |

Notes on scope:

- **Crate tests are not a sweep lane.** GitHub CI and the merge bar run them. Every ledger row records the
  CI run that passed for its revision instead.
- **The apps lane is held to Ampere** because the app bundle is compiled for `sm_86` only
  (`scripts/apps/build_bundle.sh:34`, `:60`, `:69`, `:77`).
- **The display lane is held to guest 580.159.04** because display layouts exist only for that version
  (`crates/kf-disp/src/layout.rs:78-87`).

### 1.7 Tiers, the plan and shards

`plan.py` writes `plan.json` before anything is rented. It lists every unit, where a unit is
`(family, host, guest, lane)`, together with the tier, the shard and the expected minutes. The coordinator
reads nothing else.

| tier | for | hosts | guests | estimated box time per family |
|---|---|---|---|---|
| **smoke** | every merge candidate | 580.159.04 | 580.159.04 | 28–36 min: the 2026-09-30 chain of provision, merge bar and ladder took 27.7 min on GB205 and 35.5 min on AD104 |
| **band** | every release | 570.148.08, 575.57.08, 580.65.06, 580.159.04, 590.48.01, 595.84, 610.57.04 | E/M/P/D/X per host; the reference host carries 580.159.04, 580.105.08, 575.57.08, 570.148.08, 590.48.01, 595.84 and 610.57.04 | about 6 h (estimate) |
| **full** | each new NVIDIA tag, or on owner request | the 15 host representatives | the reference host carries all 21 guest representatives plus the init rows; every other host gets E/M/P/D | about 16 h on one box (estimate); about 9.5 h for Blackwell, where hosts below 580 are environment exclusions |

**How the full-tier estimate is built** (estimate):

- The reference host takes about 5.7 h: 6 thin suites at 15 min, 20 ladders at 10 min including staging,
  15 init rows at 3 min, and bare, gates and canary.
- Each of the other 14 hosts takes about 44 min: a swap of about 8 min, bare 1, gates 1, canary 3, thin 15,
  and about 4 ladders at 4 min, with the fat guests already staged on that box.

**Shards.** A family's full plan runs as two shards on two boxes of the same family. Each shard is its own
per-box runner `TAG`, resumable by row (`V3_DRIVER_MATRIX.md:424-430`).

- Shard A takes the reference host plus 580.65.06, 580.82.07, 590.48.01, 595.84 and 610.57.04.
- Shard B takes the hosts from 535 to 575.

That brings wall time to about 8–9.5 h per family. A shorter shard also loses less when vast destroys a box
on its own (`scripts/bench/box/README.md:9-11`).

**Dies.** Within a family the plan prefers a die not yet measured at the current release, for example
GA102, GA104 and GA106, or GB205, GB203 and GB202. It records the die on every row, so per-die coverage
grows across sweeps. This follows "derive per die, maintain per family" (`OWNER_RULINGS.md:23-25`).

### 1.8 Status taxonomy, coverage and exit codes

Every planned unit ends in exactly one status:

- **PASS / FAIL / INCOMPLETE** are the only verdicts. INCOMPLETE means kayfabe ran but the grader could not
  complete its count: a ladder `k/4 CUT`, or a thin `NO_RESULT` (`V3_DRIVER_MATRIX.md:409-416`).
- **UNTESTED(reason)** means no verdict, with the reason named:
  - box: `box-dead`, `box-slow`, `box-not-vm`, `box-arch-mismatch`, `box-disk`;
  - provisioning: `provision-failed`, `artifact-mismatch`;
  - host: `host-unobtainable`, `host-install-failed`, `host-flavour-wrong`, `host-version-mismatch`,
    `host-cuda-broken` (bare metal failed on this host);
  - guest: `guest-stage-failed`, `guest-unobtainable`, `guest-version-mismatch`;
  - run: `gpu-dead` (after an FLR, `scripts/drivermatrix/sweep.sh:245-260`), `canary-skipped`,
    `timer-killed`, `stopped`, `no-row`.
- **EXCLUDED(reason)** is listed in the matrix but never counted: `driver-predates-gpu`,
  `env-blackwell-fn1`, `out-of-range`.
- **CONTROL** rows are informational only.
- **EXPECTED-REFUSAL** cells, the negative controls, pass only if the named refusal fired and nothing ran.

**Why an untested driver can never count as a pass:**

1. **The plan comes first.** Coverage is computed over `plan.json`'s units, never over the rows that happen
   to exist. A unit with no row is `UNTESTED(no-row)`, which fixes §1.3 item 3.
2. **The host version is checked on content before and after every row.** `/proc/driver/nvidia/version`
   must contain the requested version and `Open Kernel Module` (`provision_host_driver.sh:152-165`). Any
   mismatch makes the row `UNTESTED(host-version-mismatch)`, whatever its result.
3. **The guest version is checked on content.** Every lane declares `guest-driver=<g>`
   (`scripts/drivermatrix/guest_walk.sh:55`, `:66`; `scripts/drivermatrix/sweep.sh:513`;
   `scripts/drivermatrix/failure_point.sh:36`). At fn 1 the device refuses by name a guest whose own
   `NV_VERSION_STRING` differs (`crates/kf-rm/src/guestsysinfo.rs:112-125`, `:141-151`).
   - Today agreement is silent. Task S8 adds a positive `fn 1: guest says <v>; MATCH` line, and a row
     without it is `UNTESTED(guest-version-mismatch)`. An absent refusal is not evidence of agreement.
   - Staging also checks the built module's version (`stage_guest_driver.sh:86-90`).
4. **The artifact is checked.** The binary's sha256 must equal the plan's, and its embedded git sha must
   equal the plan's revision (§2.4).
5. **The denominators are planned.** A ladder is `k/4` against four planned rungs, and a thin row needs
   `n > 0` (`V3_DRIVER_MATRIX.md:409-416`).
6. **The grader's scope is enforced.** A non-580 guest can only earn a verdict from the ladder. Its init
   row is never a pass of any lane (`V3_DRIVER_MATRIX.md:501-503`, `:777-783`).
7. **Bare metal comes first.** A host whose own `cup2` fails is excluded. It is never charged to kayfabe and
   never passes (`OWNER_RULINGS.md:32-33`).

**Coverage.** A cell is covered when it has a PASS at the plan's revision and artifact, on a die of the
requested family, with both loaded versions equal to the requested ones. Box events form a separate
channel. A box that died does not block coverage if a retry box produced the verdict.

**Exit codes** keep nvkvm-pv's numbers, with a different precedence:

| code | meaning | precedence |
|---|---|---|
| 4 | possible leak: any registered id not GONE in a *successful* final listing, any UNKNOWN, or an unparsed create | 1 (highest) |
| 3 | could not start: no timer, dirty tree, or the artifact failed its checks | 2 |
| 1 | at least one FAIL or INCOMPLETE among planned units | 3 |
| 2 | at least one planned unit UNTESTED | 4 |
| 0 | every planned unit PASS (EXCLUDED units listed) | 5 |

Here a real failure outranks missing coverage, the opposite of `nvkvm-pv:scripts/sweep.sh:3215-3228`. A
kayfabe defect must not hide behind box noise. nvkvm-pv's own `tests/validate.sh` already ranks FAIL first
(`nvkvm-pv:tests/validate.sh:184-196`).

### 1.9 What every row records

The coordinator appends one JSONL row per unit attempt to `ledger.jsonl`. The per-box runner writes the
box half in a new `rows.jsonl`, beside its queue log (task S7).

| field | source |
|---|---|
| `unit` (family, host, guest, lane), `tier`, `shard`, `run` | `plan.json` |
| `rev` (full sha), `artifact_sha256`, `artifact_git_sha`, `kf3_abi` | plan; the artifact's `MANIFEST.json`; kf3's realize line (task S8, I2) |
| `host_driver_loaded`, `host_flavour`, before and after the row | `/proc/driver/nvidia/version`; kf-host's own reading, printed at realize (task S8; today realize logs the guest table but not the host version, `crates/kf-qemu/src/device.rs:376-384`) |
| `guest_driver_declared`, `guest_driver_said` | the device property; the fn-1 line (task S8) |
| `guest_image` (sha256), `guest_kernel`, `host_kernel` | the staged image; `uname -r` on both sides |
| `vast_id`, `machine_id`, `dph`, `nested` (`systemd-detect-virt`) | coordinator; box |
| `gpu_name`, `pci_id`, `die`, `family`, `vram_mib`, `host_bar1_mib`, `bdf` | the runner's `SWEEP_ARCH` line (`scripts/drivermatrix/sweep.sh:214-243`); sysfs |
| `status`, `verdict`, `counts` (`p/n`, `k/planned`), `failed_arms`, `refusals` (top named refusals, `scripts/drivermatrix/sweep.sh:201`) | the lane's output |
| `seconds`, `ts`, `ci_run` (the GitHub run that passed at `rev`) | coordinator |
| `evidence` (paths and sha256 of the committed tarballs) | coordinator |

**Never recorded:** IP addresses, ports, `EXECD_URL` lines, raw `vastai create` output, or any key. Box
text passes the allowlisted, printable-only parser before it reaches the ledger.

### 1.10 Where evidence lands in git

Evidence goes to the branch `sweep/<run>`, never to master or v3. `<run>` is `<UTC date>-<tier>-<rev8>`.

- **Per shard: `traces/driver_matrix/walk/<TAG>/`**, where `TAG` is `<run>_<family>_<shard>`.
  - This is the runner's own text-only tarball, unpacked: `summary/sweep_<TAG>.log`, `gates/`, `swaps/` and
    `ARCH`, plus `rows.jsonl`.
  - `scripts/drivermatrix/matrix_table.py` already reads this layout and prints one grid per die
    (`V3_DRIVER_MATRIX.md:417-423`; `traces/driver_matrix/walk/README.md`).
- **Per run: `traces/sweep/<run>/`** holds:
  - `plan.json` and `ledger.jsonl`;
  - `MATRIX.md`, rendered by the vendored `sweep_matrix_md.py`;
  - `COST.md`;
  - `boxes.tsv` (vast id, machine id, GPU, die, rate, start and end);
  - `autodestroy.log`, ids only.

**When it is pushed.** The coordinator pulls each shard's tarball after every host, since the runner packs
one after each host (`scripts/drivermatrix/sweep.sh:530-531`). It commits it and pushes `sweep/<run>`
then, and again at each box's end. That follows `OWNER_RULINGS.md:155-158` and `:164-167`, and the push
happens even when vast destroys the box mid-run.

**Afterwards.** A docs-only commit regenerates `V3_DRIVER_MATRIX.md` §6.0 from the ledger, and writes the
release's `SUPPORT.md` (§2.4).

**Size estimate.** The committed walk evidence of one box is 752 KB (`kfh`) and 972 KB (`kfd`), from `du`
on 2026-10-02. So a full sweep adds about 10–20 MB (estimate).

### 1.11 Safety

- **Money.**
  - The timer is armed and ps-checked before the first rent (§1.2).
  - Each box has its own deadline, sized from its shard's estimate with headroom, never a single +8 h.
  - The listing is tri-state, and a deadline destroys every registered id unconditionally (§1.3 item 1).
  - The spend cap is checked before every create, and STOP is honoured between rows, hosts and boxes.
  - At the end, every registered id must be GONE in a successful listing; otherwise the run exits 4.
- **Teardown scope.**
  - Only ids in this run's registry are destroyed. Destroy runs `-y` and is checked against the listing
    (`scripts/bench/box/README.md:25-29`).
  - The per-run label is a human-visible marker only. Nothing is destroyed by label, and no protected list
    is needed.
  - A create whose id cannot be parsed stops the run with exit 4 and a list of candidates (§1.3 item 4).
- **Keep on error.**
  - A box with failures of its own may be kept for inspection. Its id goes on record, it is still bounded
    by its deadline, and the run's summary names it.
  - Healthy boxes are always destroyed (§1.3 item 2).
- **No secrets on boxes** (`scripts/bench/box/README.md:21-22`; `OWNER_RULINGS.md:155-158`):
  - code comes from public GitHub;
  - the artifact is pushed to the box by the coordinator and checked by sha256 on arrival;
  - NVIDIA `.run` files come from NVIDIA's public paths, or by a coordinator relay checked by sha256 when
    the box geolocates to the `.cn` CDN (`nvkvm-pv:scripts/sweep-driver-availability.tsv:16-24`);
  - the vast API key, git credentials and any SSH agent stay on the coordinator.
  - Note: `vx` boxes carry execd's PSK, which is an open owner question (`STATUS_AND_HANDOFF.md:340`).
    Unattended sweeps therefore use ssh from the dev host (§4 Q7).
- **Box output is data.** Nothing executable comes back (`scripts/bench/box/README.md:163-166`). Text is
  parsed against allowlists, kept to printable characters and size-limited. A box can lie about a result,
  so the strongest checks (versions, artifact sha256) are re-read in kf3's own log lines, and the evidence
  tarball is committed for a human to audit.
- **Resumability.** A vanished box is re-rented within the same family. The last pulled log goes back to
  the new box, and rows with an EXIT are skipped (`V3_DRIVER_MATRIX.md:424-430`, `:480-481`).

### 1.12 Cost and time per sweep (estimates)

| tier | boxes | box-hours | wall time | compute cost at $0.15–0.50 per box-hour |
|---|---|---|---|---|
| smoke | 4 (one per family) | about 2.4 | about 40 min after the rent | about $0.40–1.20 |
| band | 4 | about 22 | about 6 h | about $3–11 |
| full | 8 (two shards per family) | about 60: ~16 h each for Turing, Ampere and Ada, ~9.5 h for Blackwell, plus provisioning | about 8–10 h | about $9–30 |

The rate range has two ends. The lower end is the retained RTX 3060's listed rate of about $0.1661/hour
(`STATUS_AND_HANDOFF.md:131-132`, recorded 2026-09-29). The upper end is nvkvm-pv's default price cap
(`nvkvm-pv:scripts/sweep.sh:200`).

Not included: storage while a box exists, transfer, and the cost of re-rents. Ada and Blackwell offers cost
more than the RTX 3060 by an amount not recorded here.

The per-unit minutes behind these totals are in §1.6 and §1.7. The sweep's own `COST.md` replaces these
estimates with recorded ones after its first run.

## 2. The install artifact

### 2.1 Why "one binary" means a rebuilt QEMU, linked dynamically

- **The integration ladder.** The owner's 2026-08-09 ladder (`docs/archive/vmm_integration_and_support_matrix.md:55-60`)
  prefers rank 1: an extension loaded into an already-compiled VMM.
  - kf3 cannot be rank 1. It requires QEMU ≥ 10.2 (`qemu/hw/misc/kf3/kf3.c:50-52`) and uses QEMU-internal
    APIs, and QEMU has no external device-plugin interface.
  - The route to rank 1 is a vfio-user frontend, which is design-only (`docs/design/V3_VFIO_USER_FRONTEND.md:3-9`).
  - So the artifact is rank 2 with the reason named: a stock QEMU plus our additive overlay, compiled for
    the user.
  - That archived file is archived for its architecture. Its rulings are the ones `V3_DRIVER_MATRIX.md:155`
    still cites.
- **The version floor.** The VMM is the one support axis where a version floor is acceptable
  (`docs/archive/vmm_integration_and_support_matrix.md:31-36`). So shipping one pinned QEMU does not narrow
  the driver, kernel or GPU-family axes.
- **Dynamic linking.** kf-cuda `dlopen`s `libcuda.so.1` with `RTLD_NOW|RTLD_GLOBAL`
  (`crates/kf-cuda/src/driver_unsafe.rs:1-26`, `:44`, `:69-70`), and a static binary cannot `dlopen`.
  QEMU's `prefer_static` links `-static` or `-static-pie` (`qemu-10.2.4/meson.build:458-459`). The binary
  therefore stays a glibc dynamic executable, and its glibc floor becomes part of the artifact's identity.
- **One dependency closure.** The Rust half is `libkf_qemu.a`. Its closure is 16 in-tree crates and one
  external crate, `libc 0.2.189`. That count comes from `cargo tree --locked --offline -p kf-qemu -e normal,build`,
  run 2026-10-02 at `d88639f9`. `kf-crec` and `kf-harness` are outside the closure.
  - The only `build.rs` in the workspace is `crates/kayfabe-isolate-host/build.rs`, the frozen grader's
    static helper, which is not on the product path.
  - The PTX kernels, the TSV tables, the 29-tag matrix and the VBIOS synthesis are all compiled in, and no
    data file ships beside the binary for them (`crates/kf-cuda/src/walk.rs:59`;
    `crates/kf-abi/src/generated/matrix.rs:30-61`).

### 2.2 Exact contents

```text
kayfabe-<version>-x86_64-linux-gnu.tar.xz
└── kayfabe-<version>/
    ├── bin/
    │   ├── qemu-system-x86_64     QEMU 10.2.4 with kf3 compiled in (KF3_ABI 10), stripped
    │   ├── qemu-img               QEMU's image tool (built with --enable-tools; recommended, §2.3)
    │   ├── kayfabe-run            launcher (POSIX sh): runs kayfabe-preflight, derives the device and
    │   │                          machine arguments, execs qemu-system-x86_64
    │   └── kayfabe-preflight      host checks (§2.6), each failure naming its fix
    └── share/
        ├── qemu/                  QEMU's relocatable data: BIOS, keymaps, edk2-x86_64-code.fd and
        │                          edk2-x86_64-secure-code.fd (installed by default)
        ├── kayfabe/MANIFEST.json  build identity and pinned inputs (§2.4), per-file sha256
        ├── kayfabe/rom/           (later) kf3-gop.rom, the boot-display option ROM, own version + sha256
        └── doc/kayfabe/           README.install.md, LICENSES/ (QEMU's COPYING and LICENSE, edk2's
                                   licenses, kayfabe's LICENSE, the libc crate's)
kayfabe-<version>-x86_64-linux-gnu.tar.xz.sha256
kayfabe-<version>-x86_64-linux-gnu.debug.tar.xz    split debuginfo, for crash reports
SUPPORT.md                                          the sweep's matrix for exactly this tarball (§2.4)
```

Basis for the contents:

- QEMU 10.2.4 installs relocatably by default (`qemu-10.2.4/meson_options.txt:114-115`).
- It installs its firmware blobs by default, decompressing the x86_64 edk2 images
  (`qemu-10.2.4/meson_options.txt:56-57`; `qemu-10.2.4/pc-bios/meson.build:12-13`, `:24-26`).
- So `ninja install` into a staging prefix yields a tree that works wherever it is extracted.
- Today's bench binary is not that tree: it symlinks `pc-bios` and `qemu-bundle` into a shared build
  directory (`scripts/bench/build_kf3.sh:62-70`).

### 2.3 The build script and its pinned inputs

There is one script, `scripts/release/build_artifact.sh`. CI runs it, a user's one-command source install
runs it, and the sweep's fallback for an unreleased revision runs it. It reads `scripts/release/inputs.lock`:

| input | pin |
|---|---|
| QEMU | 10.2.4 from `download.qemu.org`, sha256 recorded in the lock after checking the release signature on a trusted network. Today the tarball is fetched with no check (`scripts/bench/provision_bench_tree.sh:81-82`). |
| Rust | 1.99.0, minimal profile, from `rust-toolchain.toml:5-10`. The script refuses a different `rustc --version`. |
| crates | `cargo build --release --locked -p kf-qemu`; the lock records `Cargo.lock`'s sha256. `--locked` is new: `build_kf3.sh:24` lacks it. |
| build container | `ubuntu:22.04` by digest, so the glibc floor is 2.35 and matches vast's Ubuntu 22.04 KVM template (`scripts/bench/box/README.md:37`, `:101-102`). The study's proxy build on the 26.04 dev host required GLIBC_2.42. |
| configure flags | the product line below; recorded verbatim in the manifest |

**The product configure line.** It starts from the bench line in `scripts/bench/build_kf3.sh:49-52`.

- Unchanged: `--target-list=x86_64-softmmu`, `--without-default-features`, `--enable-kvm`,
  `--enable-system`, `--enable-pixman`, `--enable-vnc`, `--disable-docs`, `--disable-guest-agent`,
  `--disable-werror`, `--disable-gtk`, `--disable-sdl`, `--disable-curses`, `--disable-libssh` and
  `--disable-vde`.
- Recommended changes, pending §4 Q2:
  - `--enable-slirp`, for rootless user networking. The bench uses a root-created tap
    (`scripts/bench/boot_nvkvm.sh:87`), and the product's rootless story is slirp
    (`docs/PRODUCT_POSITIONING.md:171-172`).
  - `--enable-tpm`, for Windows guests.
  - `--enable-tools`, for `qemu-img`.
  - VNC kept to a unix socket in the launcher. A build with no crypto backend cannot password- or
    TLS-protect VNC (the install study, `qemu-10.2.4/ui/vnc.c:4123-4129`).

**Steps,** each one failing closed:

1. **Refuse a dirty tree.** A dirty build is possible, but its manifest says so and it is never
   published.
2. **Fetch and check QEMU.** Download into a fresh temporary directory and check the sha256.
3. **Build the Rust half.** Build `kf-qemu` with `KF_BUILD_ID=<git sha>[-dirty]` in the environment. It is
   read by `option_env!`, so no `build.rs` is added to the product closure (task I2).
4. **Apply the overlay.** Copy the three files from `qemu/hw/misc/kf3/` and the archive into the tree, as
   `build_kf3.sh:35-39` does.
   - The Kconfig stanza becomes an unconditional `default y` with `depends on PCI && KVM`.
   - Today it is `default y if TEST_DEVICES` (`scripts/bench/build_kf3.sh:39`). TEST_DEVICES is on only
     because `config PC` implies it (`qemu-10.2.4/hw/i386/Kconfig:35`). A build with
     `--without-default-devices` runs `minikconf --allnoconfig` (`qemu-10.2.4/meson.build:3447`), and that
     drops kf3 with no error.
5. **Configure and install.** Configure with the product line and `--prefix=/`, run `ninja`, then
   `DESTDIR=<stage> ninja install`.
6. **Strip.** Strip the binaries and split out the debuginfo with `objcopy --only-keep-debug`.
7. **Check the result.**
   - `bin/qemu-system-x86_64 -device help` must list `kf3-gpu`, written to a file first and then grepped,
     as `nvkvm-pv:.github/workflows/release.yml:221` does. A pipe into `grep -q` is not used.
   - The glibc floor from `objdump -T` must be ≤ 2.35 (`nvkvm-pv:.github/workflows/release.yml:249-250`).
   - The `ldd` closure must fall within an allowlist kept in the repository.
8. **Add kayfabe's own files.** Install `kayfabe-run`, `kayfabe-preflight`, `LICENSES/` and
   `README.install.md`.
9. **Write the manifest.** Write `MANIFEST.json`. Its accepted-tag lists are computed at build time by
   asking `kf_abi::versions::table_for` and the host-ABI gate for every measured tag, not typed by hand.
10. **Pack.** Make a deterministic tar (`--sort=name --owner=0 --group=0 --numeric-owner`,
    `nvkvm-pv:.github/workflows/release.yml:316-317`), with `SOURCE_DATE_EPOCH` set from the commit, then
    write its `.sha256`.

**Reproducibility (stretch goal).** Use `-ffile-prefix-map` and a fixed build path. Then two CI builds of
the same commit must produce the same sha256, or the differences are listed in the release notes.

### 2.4 Versioning, checksums and provenance

**Versioning.**

- Releases are tagged `v0.<minor>.<patch>` until a 1.0. Every tarball carries the full git sha.
- A per-commit CI build is versioned `0.0.0+g<sha>` and is never published as a release.
- The C and Rust halves are locked by `KF3_ABI`. Realize refuses an archive whose
  `kf3_abi_version()` differs (`qemu/hw/misc/kf3/kf3.h:14`; `qemu/hw/misc/kf3/kf3.c:724-727`), so a
  release always ships both halves from one commit.

**Fields of `MANIFEST.json`:**

- `name`, `version`, `git.sha`, `git.dirty`, `git.tag`, `kf3_abi`;
- `qemu.version`, `qemu.tarball_sha256`, `qemu.configure`;
- `rust.rustc`, `rust.cargo_lock_sha256`;
- `build.container_digest`, `build.glibc_floor`, `build.needed` (the dynamic libraries),
  `build.source_date_epoch`;
- `ptx.walk` and `ptx.scanout`: ISA and target, 8.2 and 6.4 on `sm_75`
  (`cuda/walk/kf_walk.ptx:9-10`; `cuda/display/kf_scanout.ptx:20-21`);
- `host_drivers_accepted` and `guest_drivers_accepted`, each a tag with "accepted" or a named refusal,
  derived as in §2.3 step 9;
- `display_guest_drivers`: `["580.159.04"]` today (`crates/kf-disp/src/layout.rs:78-87`);
- `files`: every file's path and sha256.

**The support statement is not in the manifest.** The manifest says what the binary will accept, while a
support claim needs a sweep. So the release process is:

1. Build in CI.
2. Run the band (or full) sweep on that exact tarball.
3. Render `SUPPORT.md` from the ledger. It lists only covered cells, with family, die, host, guest and
   artifact sha256, and every other accepted tag as "accepted, untested".
4. Publish the tarball, its `.sha256`, `SUPPORT.md` and a build-provenance attestation, following
   `nvkvm-pv:.github/workflows/release.yml:326-334`. A user checks the download with
   `gh attestation verify`.

`SUPPORT.md` is a separate release asset, so adding it does not change the tarball's sha256.

### 2.5 How a user installs and runs it

```sh
v=0.1.0; t=kayfabe-$v-x86_64-linux-gnu.tar.xz
curl -fLO https://github.com/reindertpelsma/kayfabe/releases/download/v$v/$t
curl -fLO https://github.com/reindertpelsma/kayfabe/releases/download/v$v/$t.sha256
sha256sum -c $t.sha256
gh attestation verify $t --repo reindertpelsma/kayfabe          # optional, needs gh
mkdir -p ~/.local/opt && tar -xJf $t -C ~/.local/opt             # rootless; or /opt as root
~/.local/opt/kayfabe-$v/bin/kayfabe-preflight
~/.local/opt/kayfabe-$v/bin/kayfabe-run --image guest.qcow2 --ram 8G [--gpu-minor 0] [--guest-driver <v>]
```

The source install is one command, `scripts/release/build_artifact.sh --install ~/.local/opt`, which is
the same script CI runs.

**What `kayfabe-run` writes,** so that no user has to know it (from `scripts/fastguest/run_fast_guest.sh:200-237`):

- guest RAM as `memory-backend-memfd,share=on` and `-machine q35,accel=kvm,memory-backend=…`;
- `bar1-size` set to the largest guest BAR1 that fits the host card (§2.6);
- `fb-mb` set to the card's memory minus 2 GiB, capped at 8192;
- `bar2-size=32M`;
- `guest-driver=` only when the user declares one. Left unset, the device re-selects at fn 1
  (`V3_DRIVER_MATRIX.md:348-357`).

Guest images are the user's. The guest needs the stock NVIDIA driver of an accepted guest tag. The optional
guest helper module is separate (§2.7).

### 2.6 Host requirements checked at start

`kayfabe-preflight` runs before QEMU. The device repeats the hard checks at realize, so a user who skips the
launcher still gets a named refusal.

| check | preflight | realize, today |
|---|---|---|
| x86_64 Linux and a writable `/dev/kvm` | yes | `-accel kvm` required (`qemu/hw/misc/kf3/kf3.c:720-723`) |
| KVM MSI-via-irqfd (in-kernel irqchip) | yes, by kernel config | refused without it (`kf3.c:789-793`) |
| host driver loaded, with a version among the accepted host tags | yes; reads `/proc/driver/nvidia/version` and warns when the tag is accepted but untested in `SUPPORT.md` | R2 gate: unreadable, unparsable and unmeasured versions are refused by name (`crates/kf-host/src/lib.rs:159-191`; `crates/kf-abi/src/hostabi.rs:184-197`) |
| open or closed kernel module | reported; closed is a warning (unmeasured, §4 Q5) | no check (none in `crates/`; only provisioning checks, `scripts/bench/provision_host_driver.sh:152-158`) |
| `libcuda.so.1` and `libnvidia-ptxjitcompiler` loadable | yes | no walker means no device (`crates/kf-qemu/src/device.rs:281-283`) |
| `/dev/nvidiactl`, `/dev/nvidia<minor>` and `/dev/nvidia-uvm` openable by the invoking user | yes | the host RM open fails, by name |
| host BAR1 ≥ guest BAR1 + BAR2 + 1 MiB + 16 MiB | yes; computes the largest guest BAR1 that fits | summed per card, refused by name (`crates/kf-qemu/src/cardbudget.rs:5-14`, `:24-27`; `device.rs:268-273`) |
| card memory for `fb-mb` | yes, from `nvidia-smi` | `store of N MiB refused: NoMemory` |
| guest RAM is a shared memfd | the launcher always passes one | **not checked at realize.** It fails at the first sysmem placement (`crates/kf-qemu/src/mem.rs:362-369`); task I5 moves it to realize |
| host RAM ≥ guest RAM | yes | all guest RAM is pinned for the VM's life (`crates/kf-qemu/src/mem.rs:1609-1614`) |
| glibc ≥ the manifest's floor | yes | the loader refuses |

**The device's own defaults fail on common cards.** Those defaults are `bar1-size` 256 MiB, `bar2-size`
32 MiB and `fb-mb` 8192 (`qemu/hw/misc/kf3/kf3.c:884-890`). They need 305 MiB of host BAR1 under the
budget above. Both harnesses therefore override them: a 128 MiB guest BAR1, and `fb-mb` sized from the card
(`scripts/fastguest/run_fast_guest.sh:205-237`). Task I5 makes the device derive them, or refuse with the
value that fits.

### 2.7 What ships separately

- **The guest doorbell helper module.** It is design-only (`docs/design/V3_GUEST_DOORBELL_MODULE.md:3-13`).
  When built it becomes a guest-side DKMS source package, because it compiles against the guest's kernel.
  It is never part of the host tarball, and stock guests keep the trapped path without it.
- **The UVM EFS patch for the host's nvidia-uvm.** This is the one privileged host piece, allowed for UVM
  only (`OWNER_RULINGS.md:125-132`). It stays unmerged until the per-fault GR hold is bounded
  (`OWNER_RULINGS.md:133-142`).
  - It is built against `/usr/src/nvidia-580.159.04` only (`tools/uvm_efs/box/build_efs.sh:12`), and every
    host version needs a port (`V3_DRIVER_MATRIX.md:17-20`).
  - Ship it as one DKMS source package per host tag, plus a CI check that runs `patch --dry-run` against
    every accepted host tag's nvidia-uvm source.
- **The boot-display GOP option ROM.** It is a design (`docs/design/V3_DISPLAY.md:12-26`): an EFI GOP driver
  built with EDK2 or `uefi-rs`, loaded through `romfile=`.
  - It needs its own UEFI toolchain, so it is built in its own CI job.
  - It ships inside the tarball as data, under `share/kayfabe/rom/`, with its own version and sha256 in the
    manifest.
  - It must agree with the fake GSP's `uefiScanoutSurfaceSizeInMB` answer (`V3_DISPLAY.md:23-26`).
- **An optional host BAR1-resize helper.** nvkvm-pv's helper and systemd unit (`nvkvm-pv:scripts/nvkvm-bar1-resize.sh`)
  can raise host BAR1 so that larger guest BAR1s fit. It needs root and briefly unloads the NVIDIA stack, so
  it is opt-in and separate.
- **An OCI image comes later,** adapted from nvkvm-pv's `Dockerfile`. Distro packages come after that. There
  is no AppImage: libcuda is `dlopen`ed and must use the host's own glibc and loader.

### 2.8 The sweep consumes the same artifact

1. **The plan names the artifact.** `plan.json` names the artifact by sha256: a release asset, or the
   per-commit CI artifact for the revision (task I3).
2. **The coordinator fetches and checks it.** It fetches the artifact on the trusted machine (`gh`, which
   is authenticated there). It checks the sha256, the attestation, and that `MANIFEST.json`'s `git.sha`
   equals the plan's revision. Then it pushes the artifact to the box.
   - Boxes need no GitHub credentials. Pushing a binary *to* an untrusted box is allowed; copying one
     *back* is not (`scripts/bench/box/README.md:166`).
3. **The box checks it again.** It re-checks the sha256, extracts to `/opt/kayfabe-<sha8>`, and confirms
   that `-device help` lists `kf3-gpu`. Only then does any row run.
4. **The runner uses the installed binary.** The runner and both harnesses take `QEMU_BIN`
   (`scripts/fastguest/run_fast_guest.sh:34-36`; `scripts/bench/boot_nvkvm.sh:24`;
   `scripts/bench/boot_capture.sh:90-92`).
   - Today the per-box runner unsets it on purpose (`scripts/drivermatrix/sweep.sh:100`) and builds kf3
     itself (`:395`).
   - A new `KF3_ARTIFACT=<prefix>` mode skips `build_kf3.sh`, sets `QEMU_BIN` to the artifact's binary
     after the checks above, and writes the artifact's sha256 into every row (task I4).
   - `merge_check.sh` gains the same mode, so a promotion to master is also measured on the artifact
     (`merge_check.sh:44` builds kf3 from source today).
5. **The instruments still build on the box from the same revision.** These are the raw client
   (`scripts/drivermatrix/sweep.sh:393`) and the thin initrd. They are test instruments and not what users
   install. Their sha256 is recorded per row, as `failure_point.sh:25-33` already does for the client.
6. **The source fallback.** A revision with no CI artifact yet can be built on the box by the same
   `build_artifact.sh`. Its rows carry `artifact=source-build` and never feed `SUPPORT.md`.

### 2.9 Gaps to close before any binary is distributed

- **The license (§4 Q1).**
  - kayfabe is Apache-2.0 (`LICENSE:1-2`; `Cargo.toml:46`).
  - `libkf_qemu.a` is linked statically into QEMU (`qemu/hw/misc/kf3/meson.build:4-8`).
  - QEMU says the emulator as a whole is GPLv2 (`qemu-10.2.4/LICENSE:8-9`).
  - The three overlay files carry no license header (none found by grep on 2026-10-02).
  - The FSF treats Apache-2.0 as incompatible with GPLv2. That is general knowledge, not a fact in this
    repository, and it needs the owner's legal reading.
  - Building is not blocked; publishing is.
- **The Kconfig gate,** which can drop kf3 silently (§2.3 step 4).
- **The device defaults and the late memfd failure** (§2.6).
- **No build stamp inside the binary.** Today the harness can only print the binary's age, because "a
  check would need a provenance stamp inside the binary" (`scripts/fastguest/run_fast_guest.sh:138-143`).
  `KAYFABE_REV.txt` is stale and must not feed the manifest.
- **The bench configure flags,** which suit a bench rather than a product (§2.3).
- **Unmeasured install facts.** A rootless end-to-end boot has never been recorded
  (`docs/PRODUCT_POSITIONING.md:43`). The first real kf3 artifact's `ldd`/`readelf` output has never been
  taken either; the study's dependency list comes from a proxy build. Task I1's first CI run records both.

## 3. Implementation tasks, in order

"Box" means the task needs a rented GPU box or a physical GPU host. Every task lands on a sub-branch, and
evidence lands on its branch after each run.

| # | task | acceptance | box |
|---|---|---|---|
| 1 | **S1** Vendor nvkvm-pv's coordinator into `scripts/sweep/` at `368d2db` with `VENDORED_FROM`: timer, registry, offer ranking, dead-vs-slow check, VM check, endpoint helpers, ledger and renderer, known-bad list, offline test. Remove every Mode-1 path. | The ported offline test passes in CI with a stub `vastai`. No executable file under `scripts/sweep/` mentions `host-libs`, `nvkvm_guest`, `/mnt/nvkvm` or `validate.sh`. | no |
| 2 | **S2** Tri-state listing (ALIVE, GONE, UNKNOWN) in the timer, `destroy_verified`, reconcile and `scripts/bench/box/vast_reaper.sh`. Deadline destroys every registered id; UNKNOWN at the end gives exit 4. | Offline tests with a stub listing that returns HTTP 502 or garbage: at the deadline a destroy is issued for every registered id, the early stand-down never fires, and the sweep exits 4. | no |
| 3 | **S3** Teardown by id only: delete label reaping, the `--reconcile` prefix mode and the protected list. Reduce create output to `{success, new_contract}`. A parse failure stops with exit 4 and a candidate list. | An offline stub account holding foreign instances, labelled, prefix-labelled and unlabelled, never sees a destroy for them. `instance_api_key` appears in no log or ledger line, including on the parse-failure path. | no |
| 4 | **S9** Port S2 and S3, plus the `BOX_FAILED` reset, to nvkvm-pv itself. | An nvkvm-pv PR with the same offline tests is green. | no |
| 5 | **S4** Plan-first units: `plan.json`, a per-box keep decision and deadline, unit-based exit codes with the §1.8 precedence, a resume key over the full unit identity, the ledger schema of §1.9. | Offline tests reproduce the five mis-scorings measured in the 2026-10-02 study and show the fixed outcomes: a healthy box after a failing one is destroyed; a family skipped by STOP exits 2 and is named; a retry-recovered box is covered; a product-stage all-pass exits 0; a malformed ledger line is a counted parse error. | no |
| 6 | **S5** GPU family: the name is an advisory pre-rent filter; the on-box PCI-derived die decides. A mismatch gives `box-arch-mismatch`, then destroy and re-rent. | A unit test maps `Q RTX 4000`, `Q RTX 5000`, `Q RTX 6000`, `Q RTX 8000` and `TITAN RTX` to Turing. An offline coordinator test re-rents on a mismatched die. | no |
| 7 | **S6** Plan inputs: commit the §1.4 class derivation (`plan.py classes`); `scripts/sweep/availability.tsv` for kayfabe's tags with each `.run`'s sha256, plus its generator, which runs from a trusted network only and refuses to write on a 403 or `.cn` redirect; band cells per §1.5; tiers and shards per §1.7. | `plan.py classes` reproduces §1.4's table from the committed `ranges.tsv`. `plan.py --tier full` lists 545.23.08, 575.51.03 and 580.94.02 as `host-unobtainable`. Adding a tag with `regen.sh` changes the plan with no hand edit. | no |
| 8 | **S7** Make the per-box runner sweep-grade. | `bash -n`, shellcheck, the committed stub test and `matrix_table.py --selftest` all run in CI. The stub test asserts the CUT, NO_RESULT, killed and swap-failed cases and their exit codes. | no |
| 9 | **S8** kf3 identity lines: at realize, the host driver version and module flavour as kf-host read them, plus the build stamp; at fn 1, `guest says <v>; MATCH|RESELECTED|REFUSED`. | Unit tests on the line formats. The live lines appear in the first H1 box run (task 15). | no |
| 10 | **I1** `scripts/release/build_artifact.sh` and `inputs.lock` as in §2.3. | Produces the tarball in a clean `ubuntu:22.04` container. `-device help` lists `kf3-gpu`, the glibc floor is ≤ 2.35, the `ldd` closure is within the allowlist, and `MANIFEST.json` holds the derived tag lists and per-file sha256. A `--without-default-devices` build still contains `kf3-gpu`. | no |
| 11 | **I2** Build stamp: `option_env!("KF_BUILD_ID")` in kf-qemu, printed at realize. | A unit test covers the formatter. The sweep refuses an unstamped binary. | no |
| 12 | **I3** CI: a per-commit artifact job, and a release workflow with attestation, adapted from `nvkvm-pv:.github/workflows/release.yml`, with actions pinned by sha. | A test tag produces a tarball, its `.sha256` and an attestation that `gh attestation verify` accepts. No vast involvement. Publishing stays disabled until task 13. | no |
| 13 | **I7** License, owner-gated (§4 Q1): SPDX headers on the overlay files, `LICENSES/` in the tarball, the owner's ruling recorded in `OWNER_RULINGS.md`. | The ruling is recorded, and the license files listed in the manifest match the ruling. This must land before any public release. | no |
| 14 | **I4** Artifact mode for the per-box runner and `merge_check.sh`: the coordinator fetches, checks and pushes; the box checks again; `QEMU_BIN` is set; the artifact's sha256 is on every row. | On one box, the smoke tier passes with the artifact binary and every row carries its sha256. A tampered tarball is refused before any boot. | yes |
| 15 | **H1** Smoke tier on all four families, with the artifact. | Each family is covered for (580.159.04, 580.159.04) at one artifact sha256, and the run exits 0. The final listing shows every box gone. `COST.md` is recorded. | yes |
| 16 | **I5** Device defaults derived from host facts, or refused with the value that fits; a refusal at realize when guest RAM has no fd. | Unit tests cover the derivation. On a box, a bare `-device kf3-gpu` boots or names the fitting `bar1-size`, and a run without memfd is refused at realize, by name. | yes |
| 17 | **I6** `kayfabe-run` and `kayfabe-preflight` (§2.5, §2.6). | Offline tests with fake `/proc` and `/sys` trees cover every refusal. On a box, preflight passes on a READY box and names the closed module and a missing `/dev/kvm`. | yes |
| 18 | **H2** Band tier, on Ampere first, then the other three families. | Every planned unit has a verdict or a named UNTESTED reason. Hosts 590.48.01, 595.84 and 610.57.04 are measured for the first time. `MATRIX.md` is rendered. | yes |
| 19 | **H3** Full tier, two shards per family. | Coverage is reported over `plan.json`. `V3_DRIVER_MATRIX.md` §6.0 is regenerated from the ledger, and `SUPPORT.md` is generated for the artifact. | yes |
| 20 | **H4** A non-nested smoke run on an owner-provided physical host, through the manual `--ssh` path (no rent, no destroy). Owner-gated, §4 Q6. | One non-nested smoke result per release, recorded with `nested=none`. | yes |
| 21 | **H5** Optional lanes on the reference pair: the display lane; the app subset once the bundle is built for every SM from `sm_75` to `sm_120`. | Display `DISPLAY_*` verdicts on each family. The app subset runs on a non-Ampere family. | yes |
| 22 | **I8** UVM EFS as one DKMS package per host tag, plus the CI `patch --dry-run` check, after UVM merges (`OWNER_RULINGS.md:133-142`). | CI lists the accepted host tags the patch applies to. One box builds and loads the DKMS module at 580.159.04. | yes |

**Detail of S7 (task 8).** The runner gains:

- a nonzero exit, with distinct codes, when any planned row failed, was cut, was unstaged or failed its
  swap;
- `rows.jsonl` beside its queue log, with the §1.9 fields;
- a firmware check for the box's own die. Today it checks `gsp_ga10x.bin` only
  (`scripts/drivermatrix/stage_guest_driver.sh:111`, `:115`, `:134`).

Around the runner:

- `provision_host_driver.sh` gains `.run --check` and the sha256 pin.
- `host_preflight.sh` accepts every family from Turing on (today it accepts only GA10x,
  `scripts/bench/host_preflight.sh:43-52`), with a disk floor per tier (`:69` vs `README.md:43-45`).
- The stale README lines are fixed (`scripts/bench/box/README.md:154-155`).
- The 2026-09-28 stubbed end-to-end test is committed (`V3_DRIVER_MATRIX.md:449-452`).

## 4. Open questions for the owner

- **Q1 — License for distributing binaries.**
  - kf-qemu (Apache-2.0) is linked statically into QEMU (GPLv2). Every crate in the product closure is
    kayfabe's except `libc` 0.2.189, which is `MIT OR Apache-2.0` (its own `Cargo.toml`).
  - Options: (a) dual-license the closure as `Apache-2.0 OR MIT`, or add `GPL-2.0-or-later`, and give the
    three kf3 overlay files `GPL-2.0-or-later` headers; or (b) ship no binaries and offer the source build
    only.
  - Recommendation: (a). This needs the owner's legal reading.
- **Q2 — Shipping our own QEMU.**
  - A shipped QEMU 10.2.4 makes kayfabe responsible for its security fixes. Following the 10.2.x stable
    releases means a smoke sweep per bump.
  - Accept that, and the product feature set of §2.3? Recommended: slirp on, TPM on, `qemu-img` included,
    VNC on a unix socket only.
- **Q3 — Spend and cadence.**
  - The estimates of §1.12 are: smoke about $0.40–1.20, band about $3–11, full about $9–30 per sweep.
  - Proposed: smoke on every merge candidate, band on every release, full on each new NVIDIA tag. Up to 8
    boxes at once. What per-run spend cap?
- **Q4 — The band.**
  - Does n−1 count every branch in the measured list (including the feature branches 555, 560 and 565) or
    production branches only?
  - Is a guest newer than its host required or extra?
  - Should out-of-band pairs be refused by name in code, as `V3_DRIVER_MATRIX.md:156-157` asks? No such
    policy exists today.
- **Q5 — Closed kernel modules.**
  - vast's template ships the closed 575.51.03 module (`V3_DRIVER_MATRIX.md:857`). Only provisioning
    insists on the open module; no kayfabe code checks it.
  - Should closed-module hosts, and closed-module guests in GSP mode, be a sweep axis, or stay out of
    scope?
- **Q6 — A non-nested host.**
  - Every vast box is a VM, so its guests are nested, and nested boxes cannot show bare-host coherency
    bugs (`V3_DRIVER_MATRIX.md:808-818`).
  - Which physical host or hosts may the sweep drive through the manual `--ssh` path, and may it do so
    unattended? `172.22.1.20` was unreachable on 2026-09-30, and the RTX 3050 kiosk's address is not in the
    repository (`STATUS_AND_HANDOFF.md:97-99`).
- **Q7 — Where unattended sweeps run.**
  - Proposed: only from the dev host, which is durable and has the vast CLI and ssh.
  - Cloud sessions would also need the execd PSK question answered (`STATUS_AND_HANDOFF.md:340`), and a
    coordinator and timer that outlive the session.
- **Q8 — What "supported" means.**
  - The binary accepts every measured host tag that passes the ABI gate (fail-closed by layout). Should the
    published support statement list only cells the sweep has covered for that artifact, with every other
    accepted tag shown as "accepted, untested"?
  - Or should a release's binary refuse host tags that no sweep has run?

## 5. Sources read for this design

- **The three studies of 2026-10-02.**
  - nvkvm-pv's sweep: `nvkvm-pv:scripts/sweep.sh`, `sweep_autodestroy.sh`, `sweep_matrix.py`,
    `sweep_plan.py`, `sweep-driver-availability.tsv`, `tests/validate.sh` and `sweep-runs/`.
  - kayfabe's test infrastructure: `scripts/bench/box/`, `scripts/drivermatrix/`, `scripts/fastguest/`,
    `scripts/apps/` and `scripts/bench/display/`.
  - The install artifact: `scripts/bench/build_kf3.sh`, `qemu/hw/misc/kf3/`, `crates/kf-qemu/` and the
    QEMU 10.2.4 tree.
- **Re-checked here on 2026-10-02:**
  - every `file:line` above;
  - the class derivation of §1.4;
  - the `cargo tree` closure of §2.1;
  - the timing lines of §1.6, read from the committed tarballs;
  - `arch_of` on the Turing Quadro names (§1.3 item 5).
- **Not checked here:**
  - the exact format of `vastai create` output (`scripts/bench/box/README.md:23-24` is relied on);
  - the claim that a powered-off vast VM stops GPU billing. It is not used by this design.
  - any run of the vendored code. Nothing was rented or executed on a box.
