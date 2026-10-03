# V3 sweep and install — every driver pair on every GPU family, tested on the binary users install

**STATUS: DESIGN, 2026-10-02; revised the same day after an adversarial review.** Nothing in this
document is built. Branch `v3-sweep`, cut from master `d88639f9`; not merged. The review raised 2 blockers,
10 majors and 14 minors; each was re-read against the cited files before acting, none was rejected, and
the fixes are folded into the sections they correct. The main changes: written PASS rules for every row
and a first-verdict rule for retries (§1.8); exclusions decided from source per die, never from box output
(§1.4); an exit 0 that every tier can reach (§1.8); per-box deadlines and a backstop off the dev host
(§1.11); a Kconfig fix that survives `--without-default-devices` and bundled host libraries (§2.3); the
running binary's sha256 measured on every boot (§2.8); a launcher lane and an undeclared-guest lane
(§1.6). This document answers the owner's plan of 2026-10-02, which is not yet recorded in
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
  - Every row has a written PASS rule. A row that completes with a failing count is FAIL with a nonzero
    rc, never an rc=0 row (§1.8).
  - A unit's status is its first verdict. Only units without a verdict are retried; a later PASS of a
    failed unit is FLAKY and counts as FAIL (§1.8).
  - Exclusions are decided by `plan.py` from source and committed tables before renting, per die, never
    from what a box prints (§1.4). So every tier can exit 0; only what goes wrong in a run, or a harness
    gap still open, can stop it (§1.8).
  - Driver versions are checked on content on both sides.
  - The exit code ranks a failure above an untested unit (§1.8).
- **Install.** One relocatable tarball holds QEMU 10.2.4 with kf3 compiled in, the non-glibc libraries it
  needs, a launcher, a preflight check and a manifest. One script builds it, in a pinned container in
  GitHub CI. The sweep installs that same tarball by sha256 and measures the running binary's sha256 on
  every boot (§2.8), so a support claim is a claim about the bytes users download.
  - The control, bare-metal, gates and bare-suite lanes do not run the artifact by design: they grade the
    box and the host, not the product binary (§1.6).
  - A launcher lane boots through `kayfabe-run` itself, and the harnesses take the launcher's derived
    device arguments (§1.6).
- **Cost (estimates, §1.12).**

  | tier | box time | compute cost | wall time |
  |---|---|---|---|
  | smoke | 33–41 min per family (pace taken from the 2026-09-30 chain logs, plus one launcher ladder), about 2.7 box-hours | $0.40–1.40 | about 45 min on 4 boxes |
  | full | about 60 box-hours | $9–30 | about 9 h on 7 boxes |
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

**Copied as-is, except where noted:**

- **Auto-destroy timer** (`nvkvm-pv:scripts/sweep_autodestroy.sh`, launched by `nvkvm-pv:scripts/sweep.sh:657-691`).
  - A separate process, started with `setsid nohup` from its own file.
  - Its pid is checked with `ps` before the first rent; the sweep refuses to rent without it.
  - Copied, then changed in two places before first use. Its fail-open listing is fixed (§1.3, item 1).
    And it gains per-id deadlines: as written it takes one deadline for the whole registry
    (`nvkvm-pv:scripts/sweep_autodestroy.sh:23`, `:56`, `:119-124`), so the per-box deadlines of §1.11
    cannot be expressed in it (§1.3, item 6).
- **Registry.** An instance id is appended and synced before anything touches the box
  (`nvkvm-pv:scripts/sweep.sh:650`, `:2292`). The spend cap is checked before every create
  (`nvkvm-pv:scripts/sweep.sh:2257-2267`). A STOP file is honoured between rows, hosts and boxes.
  - Each line is `<id> <deadline-epoch>`. The timer's id parse, `grep -oE '^[0-9]+'`
    (`nvkvm-pv:scripts/sweep_autodestroy.sh:78-81`), already ignores the second field.
  - The registry belongs to one run, in its run directory, as in nvkvm-pv (`nvkvm-pv:scripts/sweep.sh:3132`).
    It is never the shared `~/.kayfabe/vast_boxes.reg` that every session appends to
    (`scripts/bench/box/README.md:55-56`).
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
| keep-on-error, deadline, exit code, resume key | global; computed from rows; keyed on (arch, driver) | per box, with kept ids in a per-run kept file (§1.11); computed from planned units; keyed on the full unit identity (§1.8) |
| control run | the preinstalled driver, run first and not counted | the same, as bare `cup2` plus gates on the template's preinstalled driver before the purge. That driver is the closed 575.51.03 module (`V3_DRIVER_MATRIX.md:857`; `provision_host_driver.sh:6-7`). `provision_full.sh` swaps the driver before it builds anything (`scripts/bench/box/provision_full.sh:17`, `:20-23`), so it is split into a box-and-tree phase and a driver phase, and the control runs between them (task S7). On Blackwell the control is n/a: the closed 575 cannot initialise the GPU at all (`nvkvm-pv:scripts/sweep_matrix.py:175-178`). |

**Dropped:**

- **Reaping by label**, including the `--reconcile` prefix mode, which destroys other running sweeps'
  boxes (`nvkvm-pv:scripts/sweep.sh:742-791`, `:2930-2944`). Kayfabe tears down by id only, on a shared
  account (`scripts/bench/box/README.md:25-26`).
- **The hard-coded protected-id list** (`nvkvm-pv:scripts/sweep.sh:261`). It is unnecessary when only
  this run's own ids are ever touched. Boxes kept for inspection go in a per-run kept file instead
  (§1.11).
- **All of Mode 1:** the host-libs bundle, the 9p mount, `nvkvm-guest.service`, the ABI-profile journal
  grep, the DENY/AUDIT counters and validate.sh's Mode-1 checks.
- **The SteamOS and Kata stages.**
- **Recording raw `vastai create` output** (§1.3, item 4).
- **Excluding a driver because of what the box printed** (§1.3, item 10). Exclusions come from source
  (§1.4).

### 1.3 Defects to fix in the vendored copy, and upstream to nvkvm-pv

The 2026-10-02 study reproduced items 1–3 offline with a stub `vastai` in a scratch directory. It touched
neither repository nor vast. The code paths named here were re-read on 2026-10-02. Items 10 and 11 come
from the review of the same day and were read from the code, not reproduced.

| # | defect | where | consequence | fix |
|---|---|---|---|---|
| 1 | **Fail-open listing.** `live_ids` prints nothing when the listing fails ("say nothing rather than lie"), and every caller reads nothing as "gone". | `nvkvm-pv:scripts/sweep_autodestroy.sh:84-96`, `:128-137`, `:145-151`; `nvkvm-pv:scripts/sweep.sh:702-737`, `:794-844`. The same shape exists in kayfabe's own `scripts/bench/box/vast_reaper.sh:28-34`, `:48-52`. | At the deadline the timer destroys nothing, despite its "unconditionally" contract (`:31-35`). The early stand-down fires, `destroy_verified` reports "verified absent", and exit 4 can never fire. | The listing returns ALIVE, GONE or UNKNOWN, and UNKNOWN counts as alive. At a deadline, `destroy` is issued for every registered id whatever the listing says; destroy is idempotent and ids are never reused (`:31-35`). A run that ends with any UNKNOWN exits 4. |
| 2 | **`BOX_FAILED` is never reset.** | `nvkvm-pv:scripts/sweep.sh:2239` (global), `:2368` | A healthy box rented after a failing one is kept. In `verify-main-arch`, box 49460345 passed 4/4 and was kept "because 5 failed". | Count per box. |
| 3 | **The exit code counts rows, not units.** | `nvkvm-pv:scripts/sweep.sh:3191-3228`; a malformed line is skipped silently (`:3200-3201`) | A family skipped by STOP produces no row, so the run exits 0. A box failure that a retry box fully covered still exits 2. | Units are planned before renting (§1.8). |
| 4 | **Raw create output is printed and recorded** when the id parse fails. | `nvkvm-pv:scripts/sweep.sh:2285-2289` | That output can carry the `instance_api_key`, which must never be printed or recorded (`scripts/bench/box/README.md:23-24`). A create that prints no parsable id can still leave a billing contract (`:2280-2283`). | Reduce the output to `{success, new_contract}` before any print or record. Every create carries a label unique to the attempt (`<run>-<attempt>-<nonce>`). On a parse failure, query the listing for an instance with exactly that label, created after the call, and register the single match with its deadline; registering is not destroying, and teardown stays by id. No match or several matches stop the run with exit 4 and a loud alert. |
| 5 | **`arch_of` misfiles Turing Quadros.** | `nvkvm-pv:scripts/sweep_matrix.py:188-214` | Checked 2026-10-02: `Q RTX 4000` maps to ada, `Q RTX 5000` to blackwell, and `Q RTX 6000` and `TITAN RTX` to nothing. | The die derived on the box decides; a name only filters offers. |
| 6 | **One global deadline**, never consulted before renting a later box. The timer itself takes one deadline for the whole registry. | `nvkvm-pv:scripts/sweep.sh:663`, `:3159-3178`; `nvkvm-pv:scripts/sweep_autodestroy.sh:23`, `:56`, `:119-124` | A late box can be destroyed mid-run, and an early box outlives its need until the global deadline. | A deadline per registry line (`<id> <deadline-epoch>`), sized from the box's plan shard. The timer destroys each id once its own deadline passes, and the run deadline stays a ceiling. |
| 7 | **A leftover uncoerced duplicate of the offer filter.** | `nvkvm-pv:scripts/sweep.sh:1056-1069` | One price sent as a string crashes the offer search, and every family reports no offer. | Delete the duplicate. |
| 8 | **The open-module "forced install" forces nothing**, because the helper's fast path checks only the version. | `nvkvm-pv:scripts/sweep.sh:1753-1760`; `nvkvm-pv:scripts/sweep_matrix.py:560-563` | A closed module of the right version passes the fast path. | Kayfabe's installer checks both version and flavour on content (`provision_host_driver.sh:152-165`). |
| 9 | **Auto-blacklisting writes a tracked file**, so the next run refuses the dirty tree. | `nvkvm-pv:scripts/sweep.sh:635-641`, `:2981-2989` | — | Write to the run directory; a human commits it. |
| 10 | **`driver-predates-gpu` is decided from the box's `nvidia-smi`.** Any answer without a GPU name, other than the open-module message, gets that status, and the exit count skips it. | `nvkvm-pv:scripts/sweep.sh:2525-2558`, `:3203` | A wedged GPU or an `RmInitAdapter` failure prints the same "No devices were found", so it leaves the denominator and the run can exit 0. nvkvm-pv's own notes record one such reading as a misdiagnosis (`nvkvm-pv:scripts/sweep_matrix.py:173-175`). | Exclusions come from source, per die, before renting (§1.4). A post-install "no GPU" is UNTESTED (§1.8). |
| 11 | **A kept box's address is written into the ledger.** | `nvkvm-pv:scripts/sweep.sh:2396-2400`; it happened in `nvkvm-pv:sweep-runs/verify-main-arch/sweep.jsonl:30` | A committed ledger names a reachable root login on a live box. | Record the id only. The box-text parser scrubs IPv4 addresses and `:<port>` patterns (§1.9). |

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

- **Guest axis, 21 in range:** 535.309.01, 545.23.08 (diagnostic init row only; its ladder is excluded
  below), 550.40.07, 550.54.14, 550.90.07, 555.42.02, 560.28.03, 565.57.01, 570.86.15, 570.124.06,
  570.148.08, 575.57.08, 580.65.06, 580.82.07, 580.95.05, 580.105.08, 580.159.04, 580.173.02, 590.48.01,
  595.84, 610.57.04.
- **Host axis, 15 obtainable:** 535.309.01, 550.40.07, 550.54.14, 550.90.07, 555.42.02, 560.28.03,
  565.57.01, 570.148.08, 575.57.08, 580.65.06, 580.82.07, 580.159.04 (reference), 590.48.01, 595.84,
  610.57.04.

**Exclusions.** `plan.py` decides every exclusion before renting, from source and committed tables, and
writes it into `plan.json` as `EXCLUDED(<reason>)` (§1.8). Each is recorded with its reason and never
dropped. Nothing a box prints creates an exclusion.

- **Unobtainable** (`EXCLUDED(unobtainable)`), with no public `.run` in the availability table: 545.23.08,
  575.51.03 and 580.94.02.
  - On the host axis they cannot be installed.
  - On the guest axis the thin guest can still be staged, with firmware from the CUDA repository
    (`scripts/drivermatrix/stage_guest_driver.sh:92-133`). The fat guest needs the `.run`
    (`scripts/drivermatrix/stage_fat_guest.sh:32`), so the ladder of 545.23.08 is excluded, and with it the
    P cell of every 550 host (§1.5).
  - The classes of 575.51.03 and 580.94.02 are covered by 575.57.08 and 580.95.05. Class G2/H2 (545.23.08)
    has no substitute. Only its diagnostic init row runs, on the staged 6.5 guest kernel (task S7).
  - A `.run` that the table lists as available but that fails to download on the box is
    `UNTESTED(host-install-failed)` or `UNTESTED(guest-stage-failed)`, never an exclusion.
- **Out of range** (`EXCLUDED(out-of-range)`): 615.71.09, by ruling (`V3_DRIVER_MATRIX.md:852-853`).
  - It gets no box lane. With `guest-driver=615.71.09` declared, realize refuses before any guest boots
    (`crates/kf-qemu/src/device.rs:365-373`). That refusal is a pure function of the version string, and a
    unit test already pins it (`crates/kf-abi/src/versions.rs:2480-2491`). 615.71.09 also has no row in
    `nvkvm-pv:scripts/sweep-driver-availability.tsv`, so a box cell would have been vacuous or never run.
  - What a box could add is the undeclared case. There a 615 guest fails before fn 1 on the element shape,
    with no named refusal (`V3_DRIVER_MATRIX.md:344-346`). That is a refusal-by-name gap, put to the owner
    in §4 Q4, and the undeclared lane grades the same shape of failure across the 610 break (§1.6).
- **Driver predates the die** (`EXCLUDED(driver-predates-die)`), on both axes. A tag whose ogkm source has
  no HAL for a die can neither drive it as a host nor run on it as a guest.
  - Read on 2026-10-02 from the ogkm git tags (`src/common/inc/swref/published/nv_arch.h`, the
    `GPU_IMPLEMENTATION_<die>` defines, and `src/nvidia/generated/g_chips2halspec_nvoc.h`). Each of the 28
    measured tags from 535.309.01 to 610.57.04 carries TU102, TU104, TU106, TU116, TU117, GA102, GA103,
    GA104, GA106, GA107, AD102, AD103, AD104, AD106 and AD107. GB202, GB203, GB205, GB206 and GB207 first
    appear at 570.86.15. So on Blackwell the host and guest tags 535.309.01 to 565.57.01 are excluded; on
    Turing, Ampere and Ada nothing is.
  - `plan.py` commits this table per die (task S6), following "derive per die, maintain per family"
    (`OWNER_RULINGS.md:23-25`). The coordinator applies it to the die the box derives from its PCI id
    (`scripts/drivermatrix/sweep.sh:214-243`).
  - It matters on the guest axis because kf3 presents the host's own boot registers and PCI identity to
    the guest (`crates/kf-qemu/src/device.rs:240`, `:347-358`, `:386-394`). A guest older than the die
    therefore meets a GPU it has no HAL for. The runner's firmware check cannot see that: every family from
    Ampere to Blackwell loads `gsp_ga10x.bin` (ogkm 595.84 `kernel-open/common/inc/nv-firmware.h:95-102`).
  - nvkvm-pv's on-box rule is not adopted (§1.3, item 10). A post-install "No devices were found" is
    `UNTESTED(host-install-failed)`, or `UNTESTED(gpu-dead)` after a failed FLR.
- **Environment** (`EXCLUDED(env-blackwell-fn1)`): Blackwell hosts 570.x and 575.x on vast's template
  (above). Two further facts are requirements, not exclusions:
  - Pre-550 hosts may need nvkvm-pv's 5.15 host-kernel switch (`nvkvm-pv:scripts/sweep.sh:2081-2175`).
  - A 545 host does not build on 6.8 (`V3_DRIVER_MATRIX.md:740`); 545.23.08 is unobtainable anyway.

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
| 550.40.07 / 550.54.14 / 550.90.07 | itself | another 550 | 545.23.08: `EXCLUDED(unobtainable)`, no fat guest (§1.4); its init row is diagnostic only |
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

On Blackwell the exclusions of §1.4 remove the hosts below 580 and the guests 535.309.01 to 565.57.01, so
the reference host carries 13 guest representatives there, and no remaining P cell names an excluded
guest.

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
| 1 | provision + artifact | every box | `provision_full.sh`'s box-and-tree phase, then the instruments (raw client, thin initrd). In artifact mode kf3 is not built and `kf3-bins/` is deleted, so no fallback binary exists on the box (§2.8). The artifact is checked by sha256 and by `-device help` listing `kf3-gpu`. | READY in 7.1 min (TU116), 8.3 min (GB205), 11.8 min (AD104) on 2026-09-30 | 60 min |
| 2 | control | every box but Blackwell | bare `cup2` + gates on the template's preinstalled closed 575.51.03, between the box-and-tree phase and the driver phase (§1.2); not counted. n/a on Blackwell, where that driver cannot initialise the GPU (`nvkvm-pv:scripts/sweep_matrix.py:175-178`). | ~2 min (estimate) | 30 min |
| 3 | host swap | each non-reference host | open module and exact version, on content (`provision_host_driver.sh:152-165`); `.run` sha256 pin | part of ~37 min per host (hostwalk2.log, 2026-09-26: 21:25:32 → 23:17:54 for three full hosts plus three failed swaps) | `T_SWAP` 2400 s |
| 4 | bare metal | every host | `cuda_ladder.sh host`: all four rungs on the reference host, `cup2` elsewhere. PASS iff every planned rung reads `verdict=PASS`. Otherwise the host's units are `UNTESTED(host-cuda-broken)`: never charged to kayfabe and never a pass (`OWNER_RULINGS.md:32-33`). | 9 s for four rungs (2026-09-30) | `T_BARE` 1800 s |
| 5 | gates | every host | `scripts/bench/v3_gates.sh`: PASS iff `V3_GATES_SUMMARY pass=9 fail=0` | 40–61 s (2026-09-30) | `T_GATES` 3600 s |
| 6 | bare 30-arm suite | 580.x hosts in `[580.65.06, 581)` only (grader limit) | PASS iff `BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30` (`scripts/bench/box/merge_check.sh:49-50`); otherwise the host's thin rows are `UNTESTED(host-cuda-broken)` | 177 s of arm time (mc23, 2026-09-30) | 1800 s |
| 7 | canary | every host with thin rows | one `--timer` arm, graded as a planned unit (PASS or FAIL, §1.8). A canary that is not PASS leaves that host's thin rows `UNTESTED(canary-skipped)` (`V3_DRIVER_MATRIX.md:398-405`). | ~3 min (estimate) | `T_CANARY` 1200 s |
| 8 | thin 30-arm | 580.x guests | `fast_suite.sh` at budget 180. PASS iff 30/30 with no failed, crashed or unrun arm, over the arm list pinned in `plan.json` (§1.8) | 10.4–18 min wall (2026-09-30) | `T_THIN` 10800 s |
| 9 | fat ladder | every guest in the host's set | `cup2` CE `0xabcd1234`, `cup3` 43, `cup8` `bad=0 maxerr=0`, `cup8bench` PASS (`scripts/bench/cuda_ladder.sh:39-53`), `reps=1`. PASS iff k = planned = 4 (§1.8) | 3.8–4.4 min (2026-09-30) plus about 6 min of staging per new guest (estimate); a failing ladder took 5–18 min in the committed walk | `T_LADDER` = 4 × 1800 + 600 s |
| 10 | launcher | reference pair, every tier | the fat guest booted by `kayfabe-run --image <fat guest> --ram <n> --guest-driver 580.159.04` with nothing else, so preflight and the device arguments the launcher derives are what runs (§2.5); graded as the ladder | about 5 min (estimate) | `T_LADDER` |
| 11 | undeclared | band and full tiers: per pre-fn-1 surface break among the planned pairs | booted with `guest-driver` unset, as a user who runs QEMU without the launcher would. `plan.py` lists the breaks with `kf_abi::versions::pre_fn1_surface_differs` (`crates/kf-abi/src/versions.rs:511-550`). Today the init-args shape changes at 555.42.02 and 595.84, and the queue element at 610.43.02 (`traces/driver_matrix/ranges.tsv:132-134`, `:1561-1563`). A same-surface pair is graded as the ladder and its fn-1 line must read `RESELECTED`. A cross-surface pair, such as host 595.84 with guest 590.48.01 or host 610.57.04 with guest 595.84, is EXPECTED-REFUSAL (§1.8). | 4–15 min per cell (estimate) | `T_LADDER` |
| 12 | init | non-580 guests, reference host | `failure_point.sh`: how far the guest's RM got. DIAGNOSTIC, never counted; the ladder is the verdict (`V3_DRIVER_MATRIX.md:501-503`). | ~3 min (estimate) | 1200 s |
| 13 | display (optional) | reference pair, guest 580.159.04 only | `scripts/bench/display/lane.sh` `DISPLAY_*` lines | M1/M2 1–2 min, M3 4.5 min (2026-09-30) | 1800 s |
| 14 | apps subset (optional) | reference pair, Ampere only for now | the app matrix minus `vkpeak`, `clpeak`, `gpu_burn` and `geekbench` (the last passes on an upload, `scripts/apps/run_apps.sh:96`) | guest about 370 s, plus 20–40 min of app provisioning (estimate) | 3 h |

Notes on scope:

- **Crate tests are not a sweep lane.** GitHub CI and the merge bar run them. Every ledger row records the
  CI run that passed for its revision instead.
- **Four lanes do not run the artifact, by design.** The control (2), bare metal (4), gates (5) and the
  bare suite (6) grade the box and the host. The gates are built by cargo on the box
  (`scripts/bench/v3_gates.sh:22`). Lanes 7 to 14 run the artifact's QEMU.
- **The harnesses run the launcher's configuration.** Today they pin a 128 MiB guest BAR1
  (`scripts/bench/boot_nvkvm.sh:27`, `:41`; `scripts/fastguest/run_fast_guest.sh:214`), while
  `kayfabe-run` derives the largest BAR1 that fits (§2.5). In artifact mode they take `bar1-size`, `fb-mb`
  and `bar2-size` from `kayfabe-preflight --device-args`, the same derivation the launcher uses (task I6).
- **Why the launcher and undeclared lanes exist.** Every other lane declares `guest-driver=<g>`
  (`scripts/drivermatrix/sweep.sh:513`; `scripts/drivermatrix/guest_walk.sh:55`). With it unset, the device
  re-selects at fn 1 only when the pair's pre-fn-1 surface is identical, and refuses otherwise
  (`crates/kf-rm/src/lib.rs:287-301`; `V3_DRIVER_MATRIX.md:348-357`). So a band cell that passes with a
  declared guest can fail without one. `kayfabe-run` therefore requires `--guest-driver` (§2.5), and the
  undeclared lane covers users who run QEMU directly.
- **The apps lane is held to Ampere** because the app bundle is compiled for `sm_86` only
  (`scripts/apps/build_bundle.sh:34`, `:60`, `:69`, `:77`).
- **The display lane is held to guest 580.159.04** because display layouts exist only for that version
  (`crates/kf-disp/src/layout.rs:78-87`).

### 1.7 Tiers, the plan and shards

`plan.py` writes `plan.json` before anything is rented. It lists every unit, where a unit is
`(family, host, guest, lane)`, together with the tier, the shard, the expected minutes and, for an excluded
unit, its `EXCLUDED` reason (§1.4). The coordinator reads nothing else.

| tier | for | hosts | guests | estimated box time per family |
|---|---|---|---|---|
| **smoke** | every merge candidate | 580.159.04 | 580.159.04, plus the launcher lane | 33–41 min: the 2026-09-30 chain of provision, merge bar and ladder took 27.7 min on GB205 and 35.5 min on AD104; the launcher ladder adds about 5 min (estimate) |
| **band** | every release | 570.148.08, 575.57.08, 580.65.06, 580.159.04, 590.48.01, 595.84, 610.57.04 | E/M/P/D/X per host; the reference host carries 580.159.04, 580.105.08, 575.57.08, 570.148.08, 590.48.01, 595.84 and 610.57.04; the launcher lane; the undeclared lane at the 595.84 and 610.43.02 breaks | about 9 h (estimate, built below); about 6.5 h for Blackwell, where the 570.x and 575.x hosts are environment exclusions |
| **full** | each new NVIDIA tag, or on owner request | the 15 host representatives | the reference host carries all 21 guest representatives plus the init rows; every other host gets E/M/P/D; the launcher lane; the undeclared lane at every break | about 17 h on one box (estimate); about 8.5 h for Blackwell, where the hosts below 580 and the guests 535.309.01 to 565.57.01 are excluded (§1.4) |

**How the band estimate is built** (estimate). A passing ladder takes about 5 min: 3.8–4.4 min on
2026-09-30, about 5–6 min in the 2026-09-26 walk. A failing one took 5–18 min in that walk (the
`CUDA_LADDER_STARTED`/`DONE` stamps of `traces/driver_matrix/walk/*/summary/cl_*_guest.out`), and the 565
and 570 guests' ladders are 0/4 today (`V3_DRIVER_MATRIX.md:526-528`), so they are counted at 10 min.
Staging a new fat guest is about 6 min, a swap about 8 min, a thin suite 15 min.

- The reference host takes about 2.2 h: 2 thin suites, 7 ladders, 6 fat stagings, 5 init rows, the launcher,
  bare, the bare suite, gates and canary.
- The six other hosts take 43–85 min each, about 6.3 h together: a swap, bare, gates, canary, the D thin
  suite, 3–6 ladders with their stagings, and the E thin suite on 580.65.06. The 570.148.08 host is the
  longest because three of its guests (E, M and P) are 565/570 ladders.
- The undeclared lane adds about 40 min: a same-surface and a cross-surface pair at each of the two breaks.
- The review of 2026-10-02 put this tier at about 8–10 h per family, against this document's first figure
  of 6 h, which left out the stagings and the failing ladders.

**How the full-tier estimate is built** (estimate):

- The reference host takes about 5.8 h: 6 thin suites at 15 min, 20 ladders at 10 min including staging,
  15 init rows at 3 min, bare, gates and canary, and the launcher.
- Each of the other 14 hosts takes about 44 min: a swap of about 8 min, bare 1, gates 1, canary 3, thin 15,
  and about 4 ladders at 4 min, with the fat guests already staged on that box.
- The undeclared lane adds about 1 h: two pairs at each of the 555.42.02, 595.84 and 610.43.02 breaks.
- On Blackwell the reference host carries 13 guests (about 4.3 h), five other hosts remain (about 3.7 h),
  and the undeclared lane adds about 40 min.

**Shards.** A family's full plan runs as two shards on two boxes of the same family. Each shard is its own
per-box runner `TAG`, resumable by row (`V3_DRIVER_MATRIX.md:424-430`).

- Shard A takes the reference host plus 580.65.06, 580.82.07 and 610.57.04.
- Shard B takes the hosts from 535 to 575, plus 590.48.01 and 595.84.
- On Blackwell every shard-B host below 580 is excluded, so the family runs as one shard on one box.

That brings wall time to about 9 h per family: about 9 h for shard A with the undeclared lane, about 8 h
for shard B. A shorter shard also loses less when vast destroys a box on its own
(`scripts/bench/box/README.md:9-11`).

**Dies.** Within a family the plan prefers a die not yet measured at the current release, for example
GA102, GA104 and GA106, or GB205, GB203 and GB202. It records the die on every row, so per-die coverage
grows across sweeps. This follows "derive per die, maintain per family" (`OWNER_RULINGS.md:23-25`).

- A shard is bound to one die: the runner refuses a pulled log from another die, because one `TAG` is one
  revision on one die (`scripts/drivermatrix/sweep.sh:354-363`).
- So a vanished box is re-rented on the same die, by filtering offers on the PCI id. When no such offer
  exists, the shard continues under a new `TAG` on the new die, and the coordinator skips the units the
  ledger already holds a verdict for.

### 1.8 Status taxonomy, coverage and exit codes

Every planned unit ends in exactly one status. A unit can have several attempts; the rule that combines
them is under **Attempts** below, and every attempt stays in the ledger.

- **PASS / FAIL / INCOMPLETE** are the verdicts. INCOMPLETE means kayfabe ran but the grader could not
  complete its count: a ladder `k/4 CUT` or `NOTRUN`, a thin `NO_RESULT`, or a thin suite with an arm that
  never ran (`V3_DRIVER_MATRIX.md:409-416`; `scripts/fastguest/fast_suite.sh:97-102`).
- **FLAKY** is a FAIL or INCOMPLETE of a unit followed by a PASS of the same unit. Both attempts are kept,
  and it counts as FAIL.
- **UNTESTED(reason)** means no verdict, with the reason named:
  - box: `box-dead`, `box-slow`, `box-not-vm`, `box-arch-mismatch`, `box-disk`;
  - provisioning: `provision-failed`, `artifact-mismatch` (the running binary is not the manifest's, §2.8);
  - host: `host-install-failed` (including a post-install "No devices were found"), `host-flavour-wrong`,
    `host-version-mismatch`, `host-cuda-broken` (bare metal or the bare suite failed on this host);
  - guest: `guest-stage-failed`, `guest-version-mismatch`;
  - run: `gpu-dead` (after an FLR, `scripts/drivermatrix/sweep.sh:245-260`), `canary-skipped`,
    `timer-killed`, `stopped`, `no-row`.
- **EXCLUDED(reason)** is listed in the matrix but never counted. `plan.py` decides it before renting, from
  source and committed tables (§1.4), and nothing a box prints creates one: `driver-predates-die`,
  `env-blackwell-fn1`, `unobtainable`, `out-of-range`.
- **CONTROL** rows (lane 2) and **DIAGNOSTIC** rows (the init rows of lane 12) are informational only.
- **EXPECTED-REFUSAL** cells, the undeclared cross-surface pairs of lane 11, pass only if the guest booted
  and kf3's log names the refusal (`RefusedSurface`, `crates/kf-rm/src/lib.rs:299-301`). A guest that fails
  with no named refusal, for example before fn 1 at the 610 element break (`V3_DRIVER_MATRIX.md:344-346`),
  is FAIL: refusing by name is the requirement under test.
- ★ **2026-10-03 — the managed-memory rows of the apps lane (14)** carry a second class beside their
  verdict, from `scripts/apps/loud_verdict.sh` (`V3_APP_MATRIX.md` §R5.3). The sibling of EXPECTED-REFUSAL
  is **EXPECTED_LOUD**: managed memory is unsupported (`OWNER_RULINGS.md` §I), and such a row passes only
  if all of these hold:
  - the guest printed an `Xid 31` naming kayfabe;
  - kf3 named the fault (`UNSERVICED-GPU-FAULT`) and posted `RC_TRIGGERED`;
  - the app saw an error.
  
  **KF3_DEFECT** (a C′ / `RC-UNARMED` signature) and **SILENT** (anything else, a hang included) are
  FAIL, and both block the release. A twin whose notifier is unarmed or undeclared turns a fault into a
  silent hang, so a managed-memory row whose kf3 slice names an `RC-UNARMED` birth is KF3_DEFECT, and
  one naming an `RC-NONE` birth is SILENT. Every lane-14 row also records the boot's `rc_unarmed` and
  `rc_none` counts from kf3's status line, and its own `rc_silent_births`.
  - ⊘ CORRECTED 2026-10-03 (review of `5af7e644`): this bullet said *"the lane asserts both are 0"*
    while nothing asserted it — the counts were recorded and read by no consumer. The assertion is now
    built: after each guest boot `scripts/apps/apps_matrix.sh` runs `scripts/apps/boot_gate.sh` over the
    boot's whole kf3 log and records `APPS_BOOT_GATE … gate=PASS|FAIL|UNMEASURED`. Any boot that is not
    PASS — a nonzero `rc[unarmed=]`/`rc[none=]`, an `RC-UNARMED`/`RC-NONE` birth line, or a counter it
    cannot read — fails the lane: `apps_matrix.sh guest` exits 3 and `summarize.py` prints
    `BOOT_GATE … lane=FAIL` (`V3_APP_MATRIX.md` §R5.3). The gate is over every row of the boot, not
    only the managed-memory rows.

**Verdict rules.** The per-box runner applies these to parsed counts, never to a step's exit code
(task S7):

| lane | PASS iff | otherwise |
|---|---|---|
| bare metal (4) | every planned rung reads `verdict=PASS` | the host's units are `UNTESTED(host-cuda-broken)` |
| gates (5) | `V3_GATES_SUMMARY pass=9 fail=0`, the merge bar's line (`scripts/bench/box/merge_check.sh:43`) | FAIL; INCOMPLETE when no summary line exists |
| bare suite (6) | `BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30` (`merge_check.sh:50`) | the host's thin rows are `UNTESTED(host-cuda-broken)` |
| canary (7) | `verdict=PASS`, p = n = 1 | FAIL, or INCOMPLETE on `NO_RESULT`; either way the host's thin rows are `UNTESTED(canary-skipped)` |
| thin (8) | `FAST_SUITE_PASS` = `ARMS` = 30 with `FAST_SUITE_FAIL`, `FAST_SUITE_CRASH` and `NOTRUN` all 0 (the merge bar's line, `merge_check.sh:55`), and the sha256 of the arm names that ran equal to the arm-list hash in `plan.json` | FAIL if any arm failed, crashed or timed out, or if the hash differs; INCOMPLETE on `NO_RESULT` or an arm that never ran |
| ladder (9), launcher (10), undeclared same-surface pair (11) | k = planned = 4: four `CL_ROW` lines, every one `verdict=PASS` | FAIL for a full-length ladder with any failed rung, `0/4` included; INCOMPLETE for `CUT` or `NOTRUN` |

Today's runner records three of these failures as rc=0 rows, so `RETRY_FAILED` and `failed_before`
(`scripts/drivermatrix/sweep.sh:171-177`) treat them as successes:

- a completed thin row such as 25/30 (`sweep.sh:486-501`);
- a full-length `0/4` ladder, since only `CUT` and `NOTRUN` change the rc (`sweep.sh:515-523`);
- a `BARE … verdict=FAIL` row (`sweep.sh:428-436`).

The cause is that `guest_walk.sh` and `cuda_ladder.sh` always exit 0 (`guest_walk.sh:76-77`;
`cuda_ladder.sh:138-139`), and `guest_walk.sh` drops `fast_suite.sh`'s own exit code (`guest_walk.sh:55-56`).
Task S7 makes both scripts exit nonzero on a failure, and derives each row's rc and its `rows.jsonl` status
from the table above.

**The thin denominator is pinned, not taken from the revision.** `fast_suite.sh` prints the length of its
own list, `ARMS=${#ARMS[@]}` (`scripts/fastguest/fast_suite.sh:104`), and the runner accepts any n ≥ 1
(`sweep.sh:491`). So a revision that dropped failing arms would grade p = n < 30. Instead:

- the default arm list (`fast_suite.sh:24-30`) is pinned by its sha256 in a reviewed file,
  `scripts/sweep/thin_arms.sha256`;
- `plan.py` refuses, with exit 3, a revision whose list hashes differently, so changing the list is a
  reviewed commit;
- the runner hashes the arm names that actually ran (the `FAST_CELL_ARM` lines, `fast_suite.sh:89`), and a
  mismatch makes the row FAIL.

**Attempts.** A unit's status is its first verdict on a healthy box at the plan's revision and artifact.

- Only UNTESTED units are re-run, on the same box or a retry box. In sweep mode the runner's
  `RETRY_FAILED=1` (`scripts/drivermatrix/sweep.sh:29`, `:176-177`), which re-runs every row with a nonzero
  rc, is replaced by a re-run of UNTESTED-class rcs only (task S7).
- A verdict is never re-run automatically. When a human re-runs a FAIL or an INCOMPLETE and it passes,
  the unit is FLAKY.
- Why it matters: 580.x guests scored 27–30/30 at `47348e3b`, most of the reds being the adapter-init
  flake (`V3_DRIVER_MATRIX.md:517-521`). Its main mechanism was fixed at `7f271349`, with 0 of 300 opens
  failing afterwards, but its `NV_ERR_INVALID_STATE` variant stays open (`V3_DRIVER_MATRIX.md:578-596`). A
  retry-until-pass rule would hide exactly that kind of defect.

**Why an untested driver can never count as a pass:**

1. **The plan comes first.** Coverage is computed over `plan.json`'s units, never over the rows that happen
   to exist. A unit with no row is `UNTESTED(no-row)`, which fixes §1.3 item 3.
2. **The host version is checked on content before and after every row.** `/proc/driver/nvidia/version`
   must contain the requested version and `Open Kernel Module` (`provision_host_driver.sh:152-165`). Any
   mismatch makes the row `UNTESTED(host-version-mismatch)`, whatever its result.
3. **The guest version is checked on content.** Every lane but the undeclared one declares
   `guest-driver=<g>` (`scripts/drivermatrix/guest_walk.sh:55`, `:66`; `scripts/drivermatrix/sweep.sh:513`;
   `scripts/drivermatrix/failure_point.sh:36`). At fn 1 the device refuses by name a guest whose own
   `NV_VERSION_STRING` differs (`crates/kf-rm/src/guestsysinfo.rs:112-125`, `:141-151`).
   - Today agreement is silent. Task S8 adds a positive `fn 1: guest says <v>; MATCH` line, and a row
     without it is `UNTESTED(guest-version-mismatch)`. An absent refusal is not evidence of agreement.
   - In the undeclared lane the line must read `RESELECTED` to the planned guest, or name the refusal for
     an EXPECTED-REFUSAL cell.
   - Staging also checks the built module's version (`stage_guest_driver.sh:86-90`).
4. **The artifact is measured, not asserted.** The tarball's sha256 must equal the plan's and its
   `MANIFEST.json` `git.sha` the plan's revision. Every boot also measures the sha256 of the binary that
   actually ran, and it must equal the manifest's entry for `bin/qemu-system-x86_64` (§2.8). Otherwise the
   row is `UNTESTED(artifact-mismatch)`.
5. **The denominators are planned.** A ladder is `k/4` against four planned rungs, and a thin row is `p/30`
   over the pinned arm list (above).
6. **The grader's scope is enforced.** A non-580 guest can only earn a verdict from the ladder. Its init
   row is DIAGNOSTIC and never a pass of any lane (`V3_DRIVER_MATRIX.md:501-503`, `:777-783`).
7. **Bare metal comes first.** A host whose own `cup2` fails makes its units `UNTESTED(host-cuda-broken)`.
   That is never charged to kayfabe and never passes (`OWNER_RULINGS.md:32-33`).
8. **Exclusions never come from a box.** A box that prints "No devices were found" after an install makes
   that host `UNTESTED(host-install-failed)`. It cannot shrink the denominator (§1.4).

**Coverage.** A cell is covered when its first verdict is PASS at the plan's revision, with the sha256 of
the binary that ran equal to the manifest's (§2.8), on a die of the requested family, and with both loaded
versions equal to the requested ones. Box events form a separate channel: a unit whose box died has no
verdict and is re-run on a retry box.

**Exit codes** keep nvkvm-pv's numbers, with a different precedence:

| code | meaning | precedence |
|---|---|---|
| 4 | possible leak: a registered id not GONE in a *successful* final listing, other than a KEPT box that is alive within its own deadline (§1.11); any UNKNOWN; an unregistered instance carrying one of this run's labels | 1 (highest) |
| 3 | could not start: no timer, dirty tree, the artifact failed its checks, or the thin arm list differs from its pin | 2 |
| 1 | at least one FAIL, INCOMPLETE or FLAKY among planned units | 3 |
| 2 | at least one planned unit UNTESTED | 4 |
| 0 | every planned unit that is not EXCLUDED is PASS (EXCLUDED units listed) | 5 |

CONTROL and DIAGNOSTIC rows are not planned units for the exit code; they never change it.

Exit 0 is reachable at every tier: every unit that cannot be tested by construction is EXCLUDED by
`plan.py` before renting (§1.4). What remains UNTESTED is what went wrong during a run, or a harness gap
that is still open, such as the 550.54.14 fat guest, whose `.run` does not install on the fat image's
kernel (`V3_DRIVER_MATRIX.md:529`). Such gaps are fixed, not excluded.

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
| `qemu_exe_sha256`, `build_id`, per boot | measured: the sha256 of `readlink -f /proc/<qemu pid>/exe` while QEMU runs; the `KF_BUILD_ID` stamp kf3 prints at realize (§2.8, task I2) |
| `attempt`, `first_verdict` | coordinator: the attempt number, and the verdict that decides the unit's status (§1.8) |
| `host_driver_loaded`, `host_flavour`, before and after the row | `/proc/driver/nvidia/version`; kf-host's own reading, printed at realize (task S8; today realize logs the guest table but not the host version, `crates/kf-qemu/src/device.rs:376-384`) |
| `guest_driver_declared`, `guest_driver_said` | the device property; the fn-1 line (task S8) |
| `guest_image` (sha256), `guest_kernel`, `host_kernel` | the staged image; `uname -r` on both sides |
| `vast_id`, `machine_id`, `dph`, `nested` (`systemd-detect-virt`) | coordinator; box |
| `gpu_name`, `pci_id`, `die`, `family`, `vram_mib`, `host_bar1_mib`, `bdf` | the runner's `SWEEP_ARCH` line (`scripts/drivermatrix/sweep.sh:214-243`); sysfs |
| `status`, `verdict`, `counts` (`p/n`, `k/planned`), `failed_arms`, `refusals` (top named refusals, `scripts/drivermatrix/sweep.sh:201`) | the lane's output |
| `seconds`, `ts`, `ci_run` (the GitHub run that passed at `rev`) | coordinator |
| `evidence` (paths and sha256 of the committed tarballs) | coordinator |

**Never recorded:** IP addresses, ports, `EXECD_URL` lines, raw `vastai create` output, or any key. Box
text passes the allowlisted, printable-only parser before it reaches the ledger. The parser also scrubs
anything shaped like an IPv4 address or a `host:port` pair, because nvkvm-pv's keep-on-error line once
put a box's root login into a committed ledger (§1.3, item 11).

### 1.10 Where evidence lands in git

Evidence goes to the branch `sweep/<run>`, never to master or v3. `<run>` is `<UTC date>-<tier>-<rev8>`.

- **Per shard: `traces/driver_matrix/walk/<TAG>/`**, where `TAG` is `<run>_<family>_<shard>`, with the die
  appended when a re-rent moves the shard to another die (§1.7).
  - This is the runner's own text-only tarball, unpacked: `summary/sweep_<TAG>.log`, `gates/`, `swaps/` and
    `ARCH`, plus `rows.jsonl`.
  - The tarball is box output, so it is checked before it reaches git. The coordinator extracts it into a
    scratch directory and accepts only regular files under those paths: no symlinks, no `..`, no exec bits,
    a size cap per file and in total, printable text only, and nothing under `.github/`. It scrubs IPv4 and
    `host:port` patterns, then moves the result under `traces/`.
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
  - Each box has its own deadline, sized from its shard's estimate with headroom, never a single +8 h. The
    deadline is written beside the id in the registry (`<id> <deadline-epoch>`), and the timer destroys
    each id once its own deadline passes; the run's deadline stays a ceiling for all of them (§1.3
    item 6). The timer as copied could not do this: it takes one deadline for the whole registry
    (`nvkvm-pv:scripts/sweep_autodestroy.sh:23`, `:56`, `:119-124`).
  - The listing is tri-state, and a deadline destroys every registered id unconditionally (§1.3 item 1).
  - The spend cap is checked before every create, and STOP is honoured between rows, hosts and boxes.
  - At the end, every registered id must be GONE in a successful listing, or be a KEPT box that is alive
    within its own deadline; otherwise the run exits 4.
  - **A backstop that outlives the dev host.** The timer, the registry and the coordinator all run on the
    dev host, which is not durable (`OWNER_RULINGS.md:164-167`), and a timer there dies with the machine
    (`scripts/bench/box/README.md:57-58`). A full sweep holds up to 7 boxes for 8–9 h. So each box also
    arms its own `shutdown -P` at its deadline plus a margin, as the first step of provisioning. That only
    stops the spend if a powered-off vast VM stops GPU billing, which has not been measured (§5), so task
    H1 measures it on one box before the backstop is relied on; storage bills either way. A second timer on
    another machine is the alternative or the complement, and needs the vast API key there (§4 Q7).
- **Teardown scope.**
  - Only ids in this run's registry are destroyed. Destroy runs `-y` and is checked against the listing
    (`scripts/bench/box/README.md:25-29`).
  - The registry is per run (§1.2). The shared `~/.kayfabe/vast_boxes.reg` of `vast_reaper.sh`
    (`scripts/bench/box/vast_reaper.sh:18`) never gets deadline semantics: every session appends to it on a
    shared account (`scripts/bench/box/README.md:25-26`, `:55-56`).
  - The per-run label is a human-visible marker, plus a unique label per create attempt. Nothing is
    destroyed by label, and no protected list is needed.
  - A create whose id cannot be parsed is resolved through its unique label to at most one id, which is
    registered, never destroyed on the spot; no match or several stop the run with exit 4 (§1.3 item 4).
- **Keep on error.**
  - A box with failures of its own may be kept for inspection. Its id goes into the run's kept file, as in
    nvkvm-pv (`nvkvm-pv:scripts/sweep.sh:3131`, `:2393-2394`), and it is still bounded by its own deadline.
  - At the end, a kept id that is alive within its deadline is reported as KEPT, not as a leak. So a run
    with one FAIL and one kept box exits 1, not 4, and exit 4 keeps meaning a box nobody accounted for. A
    kept id still alive past its deadline is a leak.
  - Healthy boxes are always destroyed (§1.3 item 2).
- **No secrets on boxes** (`scripts/bench/box/README.md:21-22`; `OWNER_RULINGS.md:155-158`):
  - code comes from public GitHub;
  - the artifact is pushed to the box by the coordinator and checked by sha256 on arrival;
  - NVIDIA `.run` files, for the host and for the guest, and the CUDA-repo debs that stand in for a missing
    guest `.run`, come from NVIDIA's public paths, each checked against the sha256 that the availability
    table records (task S6). When the box geolocates to the `.cn` CDN they come by a coordinator relay,
    checked the same way (`nvkvm-pv:scripts/sweep-driver-availability.tsv:16-24`). The first draft pinned
    only the host `.run`; the guest downloads are unpinned today (`stage_guest_driver.sh:98-113`,
    `:117-121`);
  - the vast API key, git credentials and any SSH agent stay on the coordinator.
  - Note: `vx` boxes carry execd's PSK, which is an open owner question (`STATUS_AND_HANDOFF.md:340`).
    Unattended sweeps therefore use ssh from the dev host (§4 Q7).
- **Box output is data.** Nothing executable comes back (`scripts/bench/box/README.md:163-166`). Text is
  parsed against allowlists, kept to printable characters and size-limited, and the evidence tarball is
  checked member by member before it is committed (§1.10). A box can lie about a result, so the strongest
  checks are re-read at their source: the versions and the build stamp in kf3's own log lines, the running
  binary's sha256 from `/proc` (§2.8). The evidence tarball is committed for a human to audit.
- **Resumability.** A vanished box is re-rented on the same die (§1.7). The last pulled log goes back to
  the new box, and rows with an EXIT are skipped (`V3_DRIVER_MATRIX.md:424-430`, `:480-481`). Only rows
  whose rc is UNTESTED-class are re-run (§1.8).

### 1.12 Cost and time per sweep (estimates)

| tier | boxes | box-hours | wall time | compute cost at $0.15–0.50 per box-hour |
|---|---|---|---|---|
| smoke | 4 (one per family) | about 2.7 | about 45 min after the rent | about $0.40–1.40 |
| band | 4 | about 35: ~9 h each for Turing, Ampere and Ada, ~6.5 h for Blackwell | about 9–10 h | about $5–18 |
| full | 7 (two shards each for Turing, Ampere and Ada; one for Blackwell, §1.7) | about 60: ~17 h each for Turing, Ampere and Ada, ~8.5 h for Blackwell, plus provisioning | about 9 h | about $9–30 |

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
    │   ├── qemu-system-x86_64     QEMU 10.2.4 with kf3 compiled in (KF3_ABI 10), stripped,
    │   │                          RUNPATH=$ORIGIN/../lib
    │   ├── qemu-img               QEMU's image tool (built with --enable-tools; recommended, §2.3)
    │   ├── kayfabe-run            launcher (POSIX sh): runs kayfabe-preflight, derives the device and
    │   │                          machine arguments, execs qemu-system-x86_64
    │   └── kayfabe-preflight      host checks (§2.6), each failure naming its fix; --device-args
    │                              prints the derived device arguments
    ├── lib/                       every library the binaries NEED that is not host-provided (glib,
    │                              zlib, pixman, slirp and their own dependencies), from the build
    │                              container, each with RUNPATH=$ORIGIN
    └── share/
        ├── qemu/                  QEMU's relocatable data: BIOS, keymaps, edk2-x86_64-code.fd and
        │                          edk2-x86_64-secure-code.fd (installed by default)
        ├── kayfabe/MANIFEST.json  build identity and pinned inputs (§2.4), per-file sha256
        ├── kayfabe/rom/           (later) kf3-gop.rom, the boot-display option ROM, own version + sha256
        └── doc/kayfabe/           README.install.md, LICENSES/ (QEMU's COPYING and LICENSE, edk2's
                                   and the other installed firmware's licenses, each bundled
                                   library's license, kayfabe's LICENSE, the libc crate's)
kayfabe-<version>-x86_64-linux-gnu.tar.xz.sha256
kayfabe-<version>-x86_64-linux-gnu.debug.tar.xz    split debuginfo, for crash reports
kayfabe-<version>-sources.tar.xz                   the corresponding source: the exact QEMU tarball, the
                                                   kf3 overlay, kayfabe at the tag, and each bundled
                                                   LGPL library's source (§2.9)
SUPPORT.md                                          the sweep's matrix for exactly this tarball (§2.4)
```

Basis for the contents:

- QEMU 10.2.4 installs relocatably by default (`qemu-10.2.4/meson_options.txt:114-115`).
- It installs its firmware blobs by default, decompressing the x86_64 edk2 images
  (`qemu-10.2.4/meson_options.txt:56-57`; `qemu-10.2.4/pc-bios/meson.build:12-13`, `:24-26`).
- So `ninja install` into a staging prefix yields a tree that works wherever it is extracted.
- Today's bench binary is not that tree: it symlinks `pc-bios` and `qemu-bundle` into a shared build
  directory (`scripts/bench/build_kf3.sh:62-70`).
- **The binary needs libraries beyond glibc,** so the tree carries them.
  - A QEMU 10.2.4 build on the dev host with VNC, slirp and tools off already NEEDs `libz.so.1` and
    `libglib-2.0.so.0`, besides glibc and `libgcc_s.so.1` (`readelf -d`, 2026-10-02). The product line
    adds pixman and, if §4 Q2 agrees, slirp.
  - nvkvm-pv leaves these to the user's package manager (`nvkvm-pv:.github/workflows/release.yml:302-307`).
    Shipping them instead means the sweep's boxes, which have them from provisioning, and a user's host
    run the same libraries.
  - Host-provided, and never bundled: glibc's libraries, the loader and `libgcc_s.so.1`. libcuda is
    `dlopen`ed from the host as before, and its own dependencies are glibc's, so this is still not an
    AppImage (§2.7).
  - Each bundled library carries its own `RUNPATH=$ORIGIN`, set with `patchelf`, because the loader does
    not apply an executable's `RUNPATH` to the dependencies of its libraries.

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
- Added, not optional: `--with-devices-x86_64=kayfabe`, so kf3 is built in by assignment rather than by a
  default (step 4).
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
3. **Build the Rust half.** Build `kf-qemu` with `KF_BUILD_ID=<git sha>[-dirty]+<ci|local>` in the
   environment. It is read by `option_env!`, so no `build.rs` is added to the product closure (task I2).
   Only the CI jobs stamp `+ci`; the same script run anywhere else, including the sweep's source fallback
   and a user's source install, stamps `+local`, so a source build of the same revision never reads as the
   CI artifact.
4. **Apply the overlay.** Copy the three files from `qemu/hw/misc/kf3/` and the archive into the tree, as
   `build_kf3.sh:35-39` does.
   - Today the Kconfig stanza is `default y if TEST_DEVICES` (`scripts/bench/build_kf3.sh:39`).
     TEST_DEVICES is on only because `config PC` implies it (`qemu-10.2.4/hw/i386/Kconfig:35`). A build with
     `--without-default-devices` runs `minikconf --allnoconfig` (`qemu-10.2.4/meson.build:3447`), and that
     drops kf3 with no error.
   - An unconditional `default y` would not fix it. Under `--allnoconfig` every `default` and `imply` is
     off (`qemu-10.2.4/docs/devel/kconfig.rst:283-287`): `allnoconfig` maps every default value to false
     (`qemu-10.2.4/scripts/minikconf.py:45`, applied in `do_default`, `:282-284`). Q35 itself is a plain
     `default y` (`qemu-10.2.4/hw/i386/Kconfig:100-103`) and drops the same way.
   - So the stanza keeps `bool`, `default y` and `depends on PCI && KVM`, and the overlay also writes
     `configs/devices/x86_64-softmmu/kayfabe.mak`: `include default.mak`, `CONFIG_Q35=y`, `CONFIG_KF3=y`.
     An assignment in a `.mak` file survives `--allnoconfig`, and an assignment that contradicts an unmet
     `depends on` stops the build with an error instead of dropping the device
     (`qemu-10.2.4/scripts/minikconf.py:118`).
5. **Configure and install.** Configure with the product line, `--with-devices-x86_64=kayfabe` (which
   selects that `.mak`, `qemu-10.2.4/configure:695-700`) and `--prefix=/`, run `ninja`, then
   `DESTDIR=<stage> ninja install`.
6. **Strip.** Strip the binaries and split out the debuginfo with `objcopy --only-keep-debug`.
7. **Bundle the libraries and check the result.**
   - Copy every library that `readelf -d` lists as NEEDED by `bin/` and, transitively, by `lib/`, other
     than the host-provided set (glibc's libraries, the loader, `libgcc_s.so.1`), into `lib/`, and set the
     `RUNPATH`s of §2.2 with `patchelf`.
   - `bin/qemu-system-x86_64 -device help` must list `kf3-gpu`, written to a file first and then grepped,
     as `nvkvm-pv:.github/workflows/release.yml:221` does. A pipe into `grep -q` is not used.
   - The glibc floor from `objdump -T` must be ≤ 2.35 (`nvkvm-pv:.github/workflows/release.yml:249-250`).
   - Every NEEDED entry of every ELF file in the tree must resolve inside `lib/` or to the host-provided
     set; the list is read with `readelf -d`, not `ldd`, which would resolve from the build container.
   - The extracted tarball must run `-device help` in clean `ubuntu:22.04` and `ubuntu:24.04` containers
     with no QEMU or `-dev` packages installed (task I1). The sweep's boxes cannot show this: provisioning
     installs those libraries.
8. **Add kayfabe's own files.** Install `kayfabe-run`, `kayfabe-preflight`, `LICENSES/` and
   `README.install.md`.
9. **Write the manifest.** Write `MANIFEST.json`. Its accepted-tag lists are computed at build time by
   asking `kf_abi::versions::table_for` and the host-ABI gate for every measured tag, not typed by hand.
   Each tag is then annotated, also mechanically, with the owner's range (535 to 610,
   `OWNER_RULINGS.md:63`; 615.71.09 out of range, `V3_DRIVER_MATRIX.md:852-853`) and with known build
   limits (a 545 host does not build on Linux 6.8, `V3_DRIVER_MATRIX.md:740`). The annotation matters on
   the host axis: the host gate checks only that a tag is measured (`crates/kf-abi/src/hostabi.rs:189-197`),
   and the structs it passes through are identical at every measured tag up to 615.71.09
   (`crates/kf-host/src/lib.rs:193-196`), so it accepts 615.71.09 and 545.23.08 as hosts.
10. **Pack.** Make a deterministic tar (`--sort=name --owner=0 --group=0 --numeric-owner`,
    `nvkvm-pv:.github/workflows/release.yml:316-317`), with `SOURCE_DATE_EPOCH` set from the commit, then
    write its `.sha256`.

**Reproducibility (stretch goal).** Use `-ffile-prefix-map` and a fixed build path. Then two CI builds of
the same commit must produce the same sha256, or the differences are listed in the release notes.

### 2.4 Versioning, checksums and provenance

**Versioning.**

- Releases are tagged `v0.<minor>.<patch>` until a 1.0. Every tarball carries the full git sha, and every
  binary its `KF_BUILD_ID` stamp, `<sha>+ci` or `<sha>+local` (§2.3 step 3).
- A per-commit CI build is versioned `0.0.0+g<sha>` and is never published as a release. It is attested
  like a release (task I3), so the sweep can check every artifact it runs the same way (§2.8).
- The C and Rust halves are locked by `KF3_ABI`. Realize refuses an archive whose
  `kf3_abi_version()` differs (`qemu/hw/misc/kf3/kf3.h:14`; `qemu/hw/misc/kf3/kf3.c:724-727`), so a
  release always ships both halves from one commit.

**Fields of `MANIFEST.json`:**

- `name`, `version`, `git.sha`, `git.dirty`, `git.tag`, `kf3_abi`;
- `qemu.version`, `qemu.tarball_sha256`, `qemu.configure`;
- `rust.rustc`, `rust.cargo_lock_sha256`;
- `build.container_digest`, `build.glibc_floor`, `build.source_date_epoch`, `build.id` (the stamp);
- `build.needed`: every library a binary NEEDs, each marked bundled (with its `lib/` path, version and
  license) or host-provided;
- `ptx.walk` and `ptx.scanout`: ISA and target, 8.2 and 6.4 on `sm_75`
  (`cuda/walk/kf_walk.ptx:9-10`; `cuda/display/kf_scanout.ptx:20-21`);
- `host_drivers_accepted` and `guest_drivers_accepted`, each a tag with "accepted" or a named refusal,
  derived as in §2.3 step 9, plus that step's annotations: "out of range" (615.71.09) and "does not build
  on Linux 6.8" (a 545 host);
- `display_guest_drivers`: `["580.159.04"]` today (`crates/kf-disp/src/layout.rs:78-87`);
- `files`: every file's path and sha256.

**The support statement is not in the manifest.** The manifest says what the binary will accept, while a
support claim needs a sweep. So the release process is:

1. Build in CI.
2. Run the band (or full) sweep on that exact tarball.
3. Render `SUPPORT.md` from the ledger. It lists only covered cells, with family, die, host, guest,
   artifact sha256 and the configuration measured: a declared `guest-driver`, which `kayfabe-run` requires
   (§2.5), and the device arguments `kayfabe-preflight` derived. Every other in-range accepted tag is shown
   as "accepted, untested"; out-of-range tags are not shown as supported at all.
4. Publish the tarball, its `.sha256`, the sources tarball (§2.9), `SUPPORT.md` and a build-provenance
   attestation, following `nvkvm-pv:.github/workflows/release.yml:326-334`. A user checks the download
   with `gh attestation verify`.

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
~/.local/opt/kayfabe-$v/bin/kayfabe-run --image guest.qcow2 --ram 8G --guest-driver <v> [--gpu-minor 0]
```

The source install is one command, `scripts/release/build_artifact.sh --install ~/.local/opt`, which is
the same script CI runs.

**What `kayfabe-run` writes,** so that no user has to know it (from `scripts/fastguest/run_fast_guest.sh:200-237`):

- guest RAM as `memory-backend-memfd,share=on` and `-machine q35,accel=kvm,memory-backend=…`;
- `bar1-size` set to the largest guest BAR1 that fits the host card (§2.6);
- `fb-mb` set to the card's memory minus 2 GiB, capped at 8192;
- `bar2-size=32M`;
- the three values above come from `kayfabe-preflight --device-args`, the same derivation the sweep's
  harnesses use in artifact mode (§1.6);
- `guest-driver=<v>`, always. `kayfabe-run` refuses to start without `--guest-driver`, and names the flag
  and where to read the version (the guest's `/proc/driver/nvidia/version`, or its `.run` file name).

Why the launcher requires it. Left unset, the device answers as the host's version and re-selects at fn 1
only when the pair's pre-fn-1 surface is identical; otherwise it refuses
(`crates/kf-rm/src/lib.rs:287-301`; `V3_DRIVER_MATRIX.md:348-357`). Across the 610 element break the guest
fails before fn 1 with no named refusal at all (`V3_DRIVER_MATRIX.md:344-346`). Band cells such as host
595.84 with guest 590.48.01, or host 610.57.04 with guest 595.84, differ before fn 1
(`traces/driver_matrix/ranges.tsv:132-134`, `:1561-1563`), so they pass with a declared guest and fail
without one. Requiring the flag makes every user run the configuration the sweep measures. The device's
undeclared mode stays for users who run QEMU directly, and the undeclared lane grades it (§1.6).

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
| open or closed kernel module | reported, never refused (§4 Q5, answered 2026-10-03); a closed host shows as "accepted, untested" until a sweep covers it | no check (none in `crates/`; only provisioning checks, `scripts/bench/provision_host_driver.sh:152-158`) |
| `libcuda.so.1` and `libnvidia-ptxjitcompiler` loadable | yes | no walker means no device (`crates/kf-qemu/src/device.rs:281-283`) |
| `/dev/nvidiactl`, `/dev/nvidia<minor>` and `/dev/nvidia-uvm` openable by the invoking user | yes | the host RM open fails, by name |
| host BAR1 ≥ guest BAR1 + BAR2 + 1 MiB + 16 MiB | yes; computes the largest guest BAR1 that fits | summed per card, refused by name (`crates/kf-qemu/src/cardbudget.rs:5-14`, `:24-27`; `device.rs:268-273`) |
| card memory for `fb-mb` | yes, from `nvidia-smi` | `store of N MiB refused: NoMemory` |
| guest RAM is a shared memfd | the launcher always passes one | **not checked at realize.** It fails at the first sysmem placement (`crates/kf-qemu/src/mem.rs:362-369`); task I5 moves it to realize |
| host RAM ≥ guest RAM | yes | all guest RAM is pinned for the VM's life (`crates/kf-qemu/src/mem.rs:1609-1614`) |
| glibc ≥ the manifest's floor | yes | the loader refuses |
| every NEEDED library resolves (bundled in `lib/`, or host-provided per `build.needed`) | yes: runs `bin/qemu-system-x86_64 --version`, and on failure names the missing library and the package that provides it | the loader refuses |

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
  - It is built against `/usr/src/nvidia-580.159.04` only (`tools/uvm_efs/box/build_efs.sh:11`), and every
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
  is no AppImage: libcuda is `dlopen`ed and must use the host's own glibc and loader. The tarball's `lib/`
  holds only libraries outside that set (§2.2).

### 2.8 The sweep consumes the same artifact

1. **The plan names the artifact.** `plan.json` names the artifact by sha256: a release asset, or the
   per-commit CI artifact for the revision (task I3).
2. **The coordinator fetches and checks it.** It fetches the artifact on the trusted machine (`gh`, which
   is authenticated there). It checks the sha256, the attestation (per-commit artifacts are attested too,
   task I3), that `MANIFEST.json`'s `git.sha` equals the plan's revision, and that its `build.id` ends in
   `+ci`. Then it pushes the artifact to the box.
   - Boxes need no GitHub credentials. Pushing a binary *to* an untrusted box is allowed; copying one
     *back* is not (`scripts/bench/box/README.md:166`).
3. **The box checks it again.** It re-checks the sha256, extracts to `/opt/kayfabe-<sha8>`, and confirms
   that `-device help` lists `kf3-gpu`. Only then does any row run.
4. **The runner uses the installed binary, and nothing else can run.** The runner and both harnesses take
   `QEMU_BIN` (`scripts/fastguest/run_fast_guest.sh:34-36`; `scripts/bench/boot_nvkvm.sh:24`;
   `scripts/bench/boot_capture.sh:90-92`).
   - Today the per-box runner unsets it on purpose (`scripts/drivermatrix/sweep.sh:100`) and builds kf3
     itself (`:395`). Provisioning builds kf3 as well, and READY requires that build
     (`scripts/bench/box/provision_full.sh:23`, `:25`).
   - With `QEMU_BIN` empty, the harnesses fall back to `kf3-bins/<rev>/`
     (`scripts/bench/boot_capture.sh:92-96`; `scripts/bench/boot_nvkvm.sh:24`;
     `scripts/fastguest/run_fast_guest.sh:34`), and the apps harness takes `KF3_BIN` or the newest
     `kf3-bins/` binary (`scripts/apps/apps_matrix.sh:35`). A source-built kf3 on the box is therefore a
     silent fallback.
   - A new `KF3_ARTIFACT=<prefix>` mode closes that. Provisioning skips `build_kf3.sh` and deletes
     `kf3-bins/`; READY requires the artifact checks instead. The runner sets `QEMU_BIN`, and `KF3_BIN`
     for the apps harness, to the artifact's binary. Every harness refuses to boot with `QEMU_BIN` empty
     (task I4).
   - **Every boot measures the binary that ran.** While QEMU runs, the harness records the sha256 of
     `readlink -f /proc/<qemu pid>/exe`, and kf3 prints its `KF_BUILD_ID` at realize. A row whose measured
     sha256 differs from `MANIFEST.json`'s `files` entry for `bin/qemu-system-x86_64` is
     `UNTESTED(artifact-mismatch)`. The runner's own record of the sha256 it was given is not evidence.
   - `boot_capture.sh`'s stamp today is the binary's parent directory name
     (`scripts/bench/boot_capture.sh:103-106`), which for `<prefix>/bin/qemu-system-x86_64` would read
     `kf3-bin-rev:bin`. It is replaced by the measured sha256 and the build stamp.
   - `merge_check.sh` gains the same mode, so a promotion to master is also measured on the artifact
     (`merge_check.sh:44` builds kf3 from source today).
5. **The instruments still build on the box from the same revision.** These are the raw client
   (`scripts/drivermatrix/sweep.sh:393`) and the thin initrd. They are test instruments and not what users
   install. Their sha256 is recorded per row, as `failure_point.sh:25-33` already does for the client.
6. **The source fallback.** A revision with no CI artifact yet can be built on the box by the same
   `build_artifact.sh`. Its binary is stamped `+local`, its rows carry `artifact=source-build`, and they
   never feed `SUPPORT.md`.

### 2.9 Gaps to close before any binary is distributed

- **The license (§4 Q1).**
  - ⊘ **ANSWERED 2026-10-02:** the repository is now `Apache-2.0 OR GPL-2.0-or-later`; see `LICENSE`
    and `docs/OWNER_RULINGS.md` §G. The next line describes the state before that ruling.
    ⚠ This section missed one blocker, which the ruling found. `kf-abi`'s `capability.rs` is derived
    from gVisor's nvproxy and stays Apache-2.0 only. `kf-qemu` links `kf-abi`, so the QEMU binary is
    not yet GPL-distributable until that file is re-derived (I7).
  - kayfabe is Apache-2.0 (`LICENSE:1-2`; `Cargo.toml:46`).
  - `libkf_qemu.a` is linked statically into QEMU (`qemu/hw/misc/kf3/meson.build:4-8`).
  - QEMU says the emulator as a whole is GPLv2 (`qemu-10.2.4/LICENSE:8-9`).
  - The three overlay files carry no license header (none found by grep on 2026-10-02).
  - The FSF treats Apache-2.0 as incompatible with GPLv2. That is general knowledge, not a fact in this
    repository, and it needs the owner's legal reading.
  - Linkage is not the only obligation. Distributing a GPLv2 binary also obliges us to provide its
    corresponding source: the exact QEMU tarball, the kf3 overlay and kayfabe at the release tag. The
    tarball also ships firmware that QEMU installs (`qemu-10.2.4/pc-bios/meson.build:24`), each under its
    own license, and the bundled libraries of §2.2, of which glib is LGPL. So every release publishes the
    sources tarball of §2.2, and `LICENSES/` lists every firmware and library license (task I7).
  - Building is not blocked; publishing is.
- **The Kconfig gate,** which can drop kf3 silently; a `default y` alone does not survive
  `--without-default-devices`, so the fix is the `.mak` assignment (§2.3 step 4).
- **The host libraries,** which the bench boxes have and a user's host may not (§2.2).
- **The device defaults and the late memfd failure** (§2.6).
- **No build stamp inside the binary.** Today the harness can only print the binary's age, because "a
  check would need a provenance stamp inside the binary" (`scripts/fastguest/run_fast_guest.sh:138-143`).
  `KAYFABE_REV.txt` is stale and must not feed the manifest.
- **The bench configure flags,** which suit a bench rather than a product (§2.3).
- **Unmeasured install facts.** A rootless end-to-end boot has never been recorded
  (`docs/PRODUCT_POSITIONING.md:43`). The first real kf3 artifact's `readelf` output has never been taken
  either; the dependency list above comes from a QEMU build without kf3, VNC or slirp. Task I1's first CI
  run records it, and its clean-container runs record whether the tarball starts on a host with none of
  those libraries installed. No task here records the rootless boot; the launcher lane runs as root.

## 3. Implementation tasks, in order

"Box" means the task needs a rented GPU box or a physical GPU host. Every task lands on a sub-branch, and
evidence lands on its branch after each run.

| # | task | acceptance | box |
|---|---|---|---|
| 1 | **S1** Vendor nvkvm-pv's coordinator into `scripts/sweep/` at `368d2db` with `VENDORED_FROM`: timer, registry, offer ranking, dead-vs-slow check, VM check, endpoint helpers, ledger and renderer, known-bad list, offline test. Remove every Mode-1 path. | The ported offline test passes in CI with a stub `vastai`. No executable file under `scripts/sweep/` mentions `host-libs`, `nvkvm_guest`, `/mnt/nvkvm` or `validate.sh`. | no |
| 2 | **S2** Tri-state listing (ALIVE, GONE, UNKNOWN) in the timer, `destroy_verified` and reconcile. Per-id deadlines: registry lines `<id> <deadline-epoch>`, each id destroyed once its own deadline passes, the run deadline a ceiling. A deadline destroys every registered id whatever the listing says; UNKNOWN at the end gives exit 4. `scripts/bench/box/vast_reaper.sh` gets the tri-state listing only: its registry is shared by every session (`vast_reaper.sh:18`; `README.md:55-56`), so it never gets deadline semantics. | Offline tests with a stub listing that returns HTTP 502 or garbage: at the deadline a destroy is issued for every registered id, the early stand-down never fires, and the sweep exits 4. With two ids whose deadlines differ, the earlier id is destroyed at its own deadline and the later one is left alone until its own. | no |
| 3 | **S3** Teardown by id only: delete label reaping, the `--reconcile` prefix mode and the protected list. Reduce create output to `{success, new_contract}`. A label unique to each create attempt; on a parse failure the single listing match of that label is registered, and none or several stop the run with exit 4 and an alert. | An offline stub account holding foreign instances, labelled, prefix-labelled and unlabelled, never sees a destroy for them. A parse failure with one matching instance registers it; with none or two, the run exits 4 and destroys nothing. No log or ledger line contains `instance_api_key`, an IPv4 address, a `host:port` pair or an `EXECD_URL` line, including on the parse-failure and keep-on-error paths. | no |
| 4 | **S9** Port S2 and S3, plus the `BOX_FAILED` reset, the address scrub (§1.3 item 11) and the source-derived exclusions (§1.3 item 10), to nvkvm-pv itself. | An nvkvm-pv PR with the same offline tests is green. | no |
| 5 | **S4** Plan-first units: `plan.json` with its exclusions, a per-box keep decision and deadline, kept ids in a per-run kept file, unit-based exit codes with the §1.8 precedence, first-verdict aggregation and FLAKY, a resume key over the full unit identity, the ledger schema of §1.9. | Offline tests reproduce the five mis-scorings measured in the 2026-10-02 study and show the fixed outcomes: a healthy box after a failing one is destroyed; a family skipped by STOP exits 2 and is named; a retry-recovered box is covered; a product-stage all-pass exits 0; a malformed ledger line is a counted parse error. They also show: a FAIL followed by a PASS of the same unit is FLAKY and exits 1; one FAIL plus one UNTESTED exits 1; one FAIL plus one kept box alive within its deadline exits 1, with the box reported KEPT; a box that prints "No devices were found" after an install leaves the denominator unchanged and cannot exit 0; a plan whose only non-PASS units are EXCLUDED exits 0. | no |
| 6 | **S5** GPU family: the name is an advisory pre-rent filter; the on-box PCI-derived die decides. A mismatch gives `box-arch-mismatch`, then destroy and re-rent; a re-rent after a vanished box filters on the shard's die (§1.7). | A unit test maps `Q RTX 4000`, `Q RTX 5000`, `Q RTX 6000`, `Q RTX 8000` and `TITAN RTX` to Turing. An offline coordinator test re-rents on a mismatched die. | no |
| 7 | **S6** Plan inputs: commit the §1.4 class derivation (`plan.py classes`); `scripts/sweep/availability.tsv` for kayfabe's tags with the sha256 of each `.run` and of each CUDA-repo deb that stands in for one, plus its generator, which runs from a trusted network only and refuses to write on a 403 or `.cn` redirect; the per-die table of §1.4, read from the ogkm tags; every exclusion of §1.4; the pinned thin arm list (§1.8); band cells per §1.5; tiers and shards per §1.7; the undeclared cells of lane 11, from the items `pre_fn1_surface_differs` compares (`crates/kf-abi/src/versions.rs:511-550`) as `ranges.tsv` records them. | `plan.py classes` reproduces §1.4's table from the committed `ranges.tsv`. `plan.py --tier full` lists 545.23.08, 575.51.03 and 580.94.02 as `EXCLUDED(unobtainable)` on the host axis, 615.71.09 as `EXCLUDED(out-of-range)`, and, on Blackwell, 535.309.01 to 565.57.01 as `EXCLUDED(driver-predates-die)` on both axes; on Turing, Ampere and Ada no tag predates the die. A Rust test asserts that the plan's undeclared pairs and `pre_fn1_surface_differs` agree on every band pair. A revision whose default arm list differs from the pin is refused. Adding a tag with `regen.sh` changes the plan with no hand edit. | no |
| 8 | **S7** Make the per-box runner sweep-grade (detail below). | `bash -n`, shellcheck, the committed stub test and `matrix_table.py --selftest` all run in CI. The stub test asserts the CUT, NO_RESULT, killed and swap-failed cases and their exit codes. It also asserts the completed failures, each with a nonzero row rc that an UNTESTED-only retry does not re-run: a 25/30 thin row is FAIL; a thin run over 29 arms is FAIL; a full-length 0/4 ladder is FAIL; a `BARE … verdict=FAIL` makes the host's units `UNTESTED(host-cuda-broken)`. | no |
| 9 | **S8** kf3 identity lines: at realize, the host driver version and module flavour as kf-host read them, plus the build stamp; at fn 1, `guest says <v>; MATCH\|RESELECTED\|REFUSED`. | Unit tests on the line formats. The live lines appear in the first H1 box run (task 16). | no |
| 10 | **I1** `scripts/release/build_artifact.sh` and `inputs.lock` as in §2.3, with `kayfabe.mak` and the bundled `lib/`. | Produces the tarball in a clean `ubuntu:22.04` container. `-device help` lists `kf3-gpu` for the shipped configuration and for a `--without-default-devices` build with the same `kayfabe.mak`. The glibc floor is ≤ 2.35. Every NEEDED entry resolves inside `lib/` or to the host-provided set. The extracted tarball lists `kf3-gpu` in clean `ubuntu:22.04` and `ubuntu:24.04` containers with no QEMU or `-dev` packages. `MANIFEST.json` holds the derived, annotated tag lists, `build.needed` and per-file sha256. | no |
| 11 | **I2** Build stamp: `option_env!("KF_BUILD_ID")` in kf-qemu, `<sha>[-dirty]+<ci\|local>`, printed at realize. | A unit test covers the formatter. The sweep refuses an unstamped binary, and a `+local` binary never feeds `SUPPORT.md`. | no |
| 12 | **I3** CI: a per-commit artifact job and a release workflow, both with attestation, adapted from `nvkvm-pv:.github/workflows/release.yml`, with actions pinned by sha. The release also publishes the sources tarball (§2.2). | A test tag and a test commit each produce a tarball, its `.sha256` and an attestation that `gh attestation verify` accepts. No vast involvement. Publishing stays disabled until task 13. | no |
| 13 | **I7** License, owner-gated (§4 Q1): SPDX headers on the overlay files; `LICENSES/` in the tarball with every installed firmware's and bundled library's license; the sources tarball; the owner's ruling recorded in `OWNER_RULINGS.md`. | The ruling is recorded. The license files listed in the manifest match the ruling and cover every bundled library and installed firmware. The sources tarball holds the exact inputs the manifest pins. This must land before any public release. | no |
| 14 | **I4** Artifact mode for provisioning, the per-box runner, the harnesses and `merge_check.sh` (§2.8): the coordinator fetches, checks and pushes; the box checks again; no kf3 is built and `kf3-bins/` is deleted; `QEMU_BIN` and `KF3_BIN` are set, and a harness with `QEMU_BIN` empty refuses; every boot records the running binary's sha256. | On one box, the smoke tier passes with the artifact binary, and every row carries the measured sha256, equal to the manifest's. A binary planted under `kf3-bins/` is never run. A tampered tarball is refused before any boot. | yes |
| 15 | **I6** `kayfabe-run` and `kayfabe-preflight` (§2.5, §2.6), including `--device-args`, which the harnesses' artifact mode then uses (§1.6), and the required `--guest-driver`. | Offline tests with fake `/proc` and `/sys` trees cover every refusal, a missing bundled library and a missing `--guest-driver`. On a box, preflight passes on a READY box and names the closed module and a missing `/dev/kvm`, and the fat guest boots through `kayfabe-run` and passes the ladder. | yes |
| 16 | **H1** Smoke tier on all four families, with the artifact, the launcher lane included. | Each family is covered for (580.159.04, 580.159.04) at one artifact sha256, launcher lane included, and the run exits 0. The final listing shows every box gone. `COST.md` is recorded. On one box, whether a powered-off vast VM stops GPU billing is recorded, before the box-side backstop of §1.11 is relied on. | yes |
| 17 | **I5** Device defaults derived from host facts, or refused with the value that fits; a refusal at realize when guest RAM has no fd. | Unit tests cover the derivation. On a box, a bare `-device kf3-gpu` boots or names the fitting `bar1-size`, and a run without memfd is refused at realize, by name. | yes |
| 18 | **H2** Band tier, on Ampere first, then the other three families. | At least 95 % of each family's non-excluded units have a verdict. Every band E/M/P/D cell is PASS or names a filed defect, and so does every FAIL, INCOMPLETE or FLAKY unit; the run exits 0 or 1. Hosts 590.48.01, 595.84 and 610.57.04 are measured for the first time. `MATRIX.md` is rendered. | yes |
| 19 | **H3** Full tier: two shards per family, one on Blackwell (§1.7). | The H2 floor holds for every family over `plan.json`. `V3_DRIVER_MATRIX.md` §6.0 is regenerated from the ledger, and `SUPPORT.md` is generated for the artifact. | yes |
| 20 | **H4** A non-nested smoke run on an owner-provided physical host, through the manual `--ssh` path (no rent, no destroy). Nothing that changes the host's packages or driver runs: no `provision_full.sh`, no control, no host swap and no `RESTORE_REF`. The swap masks the apt timers, unholds every held package and purges the NVIDIA packages (`provision_host_driver.sh:64`, `:76-78`, `:85-109`), and `RESTORE_REF` would leave 580.159.04, not the host's own driver (`scripts/drivermatrix/sweep.sh:534-537`). The artifact is installed under `/opt` and the instruments are built in the checkout. The run uses the host's installed driver, and only if it is a measured tag; anything else needs a disposable OS install. Owner-gated, §4 Q6. | One non-nested smoke result per release, recorded with `nested=none`, and the host's package and driver state unchanged by the run. | yes |
| 21 | **H5** Optional lanes on the reference pair: the display lane; the app subset once the bundle is built for every SM from `sm_75` to `sm_120`. | Display `DISPLAY_*` verdicts on each family. The app subset runs on a non-Ampere family. | yes |
| 22 | **I8** UVM EFS as one DKMS package per host tag, plus the CI `patch --dry-run` check, after UVM merges (`OWNER_RULINGS.md:133-142`). | CI lists the accepted host tags the patch applies to. One box builds and loads the DKMS module at 580.159.04. | yes |

**Detail of S7 (task 8).** The runner gains:

- the verdict rules of §1.8, applied to parsed counts. `guest_walk.sh` passes on `fast_suite.sh`'s exit
  code, and `cuda_ladder.sh` exits 1 when any `CL_ROW` is not `verdict=PASS` and 2 when it wrote fewer rows
  than planned, so a completed failing row is never an rc=0 row;
- distinct row rc classes: PASS, FAIL, INCOMPLETE and the UNTESTED reasons. In sweep mode `RETRY_FAILED=1`
  gives way to `RETRY_UNTESTED=1`, which re-runs only the UNTESTED class (§1.8);
- the check of the thin arm list's hash against `plan.json` (§1.8);
- a nonzero exit, with distinct codes, when any planned row failed, was cut, was unstaged or failed its
  swap;
- `rows.jsonl` beside its queue log, with the §1.9 fields;
- a firmware check for the box's own die: Turing (and GA100) load `gsp_tu10x.bin`, while GA10x, AD10x and
  GB20x load `gsp_ga10x.bin` (ogkm 595.84 `kernel-open/common/inc/nv-firmware.h:95-107`). Today it checks
  `gsp_ga10x.bin` only (`scripts/drivermatrix/stage_guest_driver.sh:111`, `:115`, `:134`). The check
  cannot tell a guest that predates the die; §1.4's table does that;
- staging against the extracted 6.5 kernel for guests at or below 545: `stage_thin` passes `KREL`, `KBUILD`
  and `KF_GUEST_KROOT` from `stage_guest_kernel.sh` (`V3_DRIVER_MATRIX.md:492-496`). Today it passes none
  (`scripts/drivermatrix/sweep.sh:302-311`), so `stage_guest_driver.sh` builds for the host's 6.8 kernel
  (`stage_guest_driver.sh:36-39`). Only the diagnostic init row of 545.23.08 depends on it.

Around the runner:

- `provision_full.sh` is split into a box-and-tree phase and a driver phase, so the control runs between
  them (§1.2), and gains the artifact mode of task I4.
- `provision_host_driver.sh` gains `.run --check` and the sha256 pin. `stage_guest_driver.sh` pins the
  guest `.run` and the CUDA-repo debs the same way, and takes the coordinator relay on a `.cn` box (§1.11).
- `host_preflight.sh` accepts every family from Turing on (today it accepts only GA10x,
  `scripts/bench/host_preflight.sh:43-52`), with a disk floor per tier (`:69` vs `README.md:43-45`).
- The stale README lines are fixed (`scripts/bench/box/README.md:154-155`).
- The 2026-09-28 stubbed end-to-end test is committed (`V3_DRIVER_MATRIX.md:449-452`).

## 4. Open questions for the owner

- **Q1 — License for distributing binaries.**
  - ⊘ **ANSWERED 2026-10-02 — option (a), as `Apache-2.0 OR GPL-2.0-or-later`** (`LICENSE`;
    `docs/OWNER_RULINGS.md` §G). The question text below is kept as it was asked. ⚠ One more
    blocker remains, the nvproxy-derived `capability.rs` (see §2.9).
  - kf-qemu (Apache-2.0) is linked statically into QEMU (GPLv2). Every crate in the product closure is
    kayfabe's except `libc` 0.2.189, which is `MIT OR Apache-2.0` (its own `Cargo.toml`).
  - Options: (a) dual-license the closure as `Apache-2.0 OR MIT`, or add `GPL-2.0-or-later`, and give the
    three kf3 overlay files `GPL-2.0-or-later` headers; or (b) ship no binaries and offer the source build
    only.
  - Recommendation: (a). This needs the owner's legal reading.
  - Either way, a binary release carries the source obligations of §2.9: the QEMU source, the overlay and
    kayfabe at the tag, and the source of the bundled LGPL library (glib).
- **Q2 — Shipping our own QEMU.**
  - A shipped QEMU 10.2.4 makes kayfabe responsible for its security fixes. Following the 10.2.x stable
    releases means a smoke sweep per bump.
  - Accept that, and the product feature set of §2.3? Recommended: slirp on, TPM on, `qemu-img` included,
    VNC on a unix socket only.
- **Q3 — Spend and cadence.**
  - The estimates of §1.12 are: smoke about $0.40–1.40, band about $5–18, full about $9–30 per sweep.
  - Proposed: smoke on every merge candidate, band on every release, full on each new NVIDIA tag. Up to 8
    boxes at once. What per-run spend cap?
- **Q4 — The band.**
  - Does n−1 count every branch in the measured list (including the feature branches 555, 560 and 565) or
    production branches only?
  - Is a guest newer than its host required or extra?
  - Should out-of-band pairs be refused by name in code, as `V3_DRIVER_MATRIX.md:156-157` asks? No such
    policy exists today.
  - Should the device refuse by name, before fn 1, a guest whose queue element it does not serve? With
    `guest-driver` unset, a guest across the 610 element break fails before fn 1 with no named refusal
    (`V3_DRIVER_MATRIX.md:344-346`), and so would an undeclared 615.71.09 guest; a declared 615.71.09 is
    refused only because the declared string has no encoding (§1.4). Until such a refusal exists, the
    undeclared lane's cross-element cell grades FAIL (§1.8).
  - `kayfabe-run` requires `--guest-driver` (§2.5), so users run the declared configuration the sweep
    measures. Agree, or keep it optional and limit `SUPPORT.md` to same-surface pairs for undeclared use?
- **Q5 — Closed kernel modules.**
  - ⊘ **ANSWERED 2026-10-03 — both flavours are in scope** (`docs/OWNER_RULINGS.md` §J). Closed hosts
    are accepted, never refused, and become a sweep axis on Turing, Ampere and Ada (the closed module
    cannot drive Blackwell). Closed guests in GSP mode get one cell per family in the band tier. For
    the plan: the flavour joins the unit identity (task S4), `plan.py` lists the flavour cells (S6),
    and the host swap installs the requested flavour and checks it on content (S7). The question
    text below is kept as it was asked.
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
- **Q7 — Where unattended sweeps run, and what outlives that machine.**
  - Proposed: from the dev host, which has the vast CLI and ssh. ⊘ *Corrected 2026-10-02: the first draft
    called the dev host durable. It is not (`OWNER_RULINGS.md:164-167`), and a timer there dies with it
    (`scripts/bench/box/README.md:57-58`).*
  - So which backstop? (a) Each box powers itself off at its deadline plus a margin (§1.11), which only
    helps if a powered-off vast VM stops GPU billing; task H1 measures that first, and storage bills
    either way. (b) A second timer on a machine the owner names, which needs the vast API key there.
    (c) Both. Recommended: (a) once measured, plus (b) for full sweeps.
  - Cloud sessions would also need the execd PSK question answered (`STATUS_AND_HANDOFF.md:340`), and a
    coordinator and timer that outlive the session.
- **Q8 — What "supported" means.**
  - The binary accepts every measured host tag that passes the ABI gate (fail-closed by layout). That
    includes 615.71.09, which is out of range, and 545.23.08, which does not build on Linux 6.8, as hosts
    (§2.3 step 9). The manifest annotates both. Should the binary refuse out-of-range hosts by name too?
  - Should the published support statement list only cells the sweep has covered for that artifact, with
    every other in-range accepted tag shown as "accepted, untested"?
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
- **The adversarial review of 2026-10-02, and what its fixes rest on.** Each finding was re-read against
  the cited files before acting; none was rejected. Read for the fixes on 2026-10-02:
  - the per-box runner's rc paths and the thin and ladder scripts' exit codes (§1.8);
  - nvkvm-pv's `driver-predates-gpu` rule, its exit count, its keep-on-error path and its timer
    (§1.3 items 6, 10 and 11; §1.11);
  - the ogkm git tags of all 28 in-range measured tags, for the per-die table (§1.4), and ogkm 595.84's
    `nv-firmware.h`;
  - kf3's realize path and the fn-1 re-selection (§1.4, §2.5), and `ranges.tsv`'s pre-fn-1 items (§1.6);
  - QEMU 10.2.4's Kconfig handling (`kconfig.rst`, `minikconf.py`, `meson.build`, `configure`) and the
    `readelf -d` of a QEMU 10.2.4 build on the dev host (§2.2, §2.3);
  - the committed ladder logs' start and end stamps, for the band estimate (§1.7).
- **Not checked here:**
  - the exact format of `vastai create` output (`scripts/bench/box/README.md:23-24` is relied on);
  - whether a powered-off vast VM stops GPU billing. The backstop of §1.11 relies on it only after task H1
    has measured it;
  - whether the bundled-library tarball starts on a clean host; task I1's container runs record it;
  - any run of the vendored code. Nothing was rented or executed on a box.
