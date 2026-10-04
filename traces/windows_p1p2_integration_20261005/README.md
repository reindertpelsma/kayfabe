# Windows, current master, and P1/P2 integration

**STATUS: RESEARCH, 2026-10-05.** Local integration and GPU-free evidence. No
hardware job, VM, rental, or master promotion was performed by this work.
The product revision is **`e71e4a8b`**, branch
`codex/p1p2-integration-2026-10-05`, **KF3 ABI 20**. The three experimental
switches `KF3_TSPACE`, `KF3_GFX_POOL_PROBE`, and `KF3_TIMER_MAP` remain off by
default. This is not a secure-default or Windows-success claim.

## Source and conflict resolution

| Revision | Content |
|---|---|
| `906a76a4` | Current master, including candidate-2 product `9d82f259`, P0 USER birth checks, scratch-window bounds, display broker/max-fps and display-SW support |
| `31b64802` | Complete P1/P2 implementation/review tip; needed for private Translated spaces, ring isolation, and the associated bounds/refusals |
| `a5a350a8` | Master + P1/P2 merge; preserves both sets of counters, adds the optional twin to a display test fixture, fixes CI's zero-unsafe count without hiding read errors |
| `c1d4e415` | Windows experiment, including exact compiled pool-query layout guards, generated timer class ID, signed GOP and 595.91.07 measurements |
| `6e52aebd` | Windows union, ABI 19; keeps all master broker/display arguments and adds the signed-GOP path after them |
| `831f6bd7` | Independently developed native read-only timer mapping |
| `e71e4a8b` | Timer integrated as ABI 20; new FFI output alignment checks and reviewed QEMU unsafe ratchet 59 → 61 |
| `8e29da95` | Runner-only `--private-translated-space` switch; clears inherited values and records each independent experiment flag |
| `0880e340` | Carries native timer evidence and its source audit from `45a694ae`; no additional product change |

This is a merge of the full P1/P2 branch, not a partial S1-21 patch. The master
USER-channel birth guard remains in place. Windows policy-chain insertion
retains the current display-SW configuration. Both C and Rust carry all realize
arguments in the same order; the wire-mirror tests compile their declarations.
Matrix resolution retains master-only `NV9072_ALLOCATION_PARAMETERS` cells and
the newer Windows driver's measured cells.

Three ordinary integration defects were fixed before the passing results:

- The display-SW `PtChan` fixture needed `twin: None` after P1/P2 added that field.
- The max-fps configuration fixture needed `gop_efi: None` after the Windows merge.
- Master added hotplug layouts absent from the older 580.65.06 display fixture.
  Running the current `tools/derive_display_layouts.sh` against actual OGKM tag
  `580.65.06` (`8032cb60785ff04a49698fbe1fac8781bd37b0d6`) added exactly one
  size, four field offsets and one command constant. The strict cross-fixture
  equality test was retained and passes; no values were copied from a capture.

## Local checks

All Cargo runs used `flock /tmp/kayfabe-cargo.lock`, `CARGO_BUILD_JOBS=2`, and a
distinct target at `/tmp/kayfabe-p1p2-integration-target`. Logs include warnings,
not just the pass lines. These suites overlap and their counts must not be
summed as independent coverage.

| Source | Command scope | Result | Evidence |
|---|---|---|---|
| `a5a350a8` | `cargo test -p kf-abi -p kf-chip -p kf-host -p kf-mem -p kf-chan -p kf-qemu -p kf-harness` | 919 passed, 0 failed, 0 ignored; 61 suites | `p1p2-tests.log` |
| `6e52aebd` | Above plus `-p kf-rm -p kf-gsp -p kf-disp -p kf-oprom -p kf-gop-image` | 1,681 passed, 0 failed, 0 ignored; 124 suites | `windows-union-tests.log` |
| `e71e4a8b` | `cargo test -p kf-abi -p kf-host -p kf-linux-raw -p kf-rm -p kf-trap -p kf-qemu -p kf-harness` | 1,516 passed, 0 failed, 0 ignored; 93 suites, including raw-layer negative compilation tests | `timer-integration-tests.log` |
| `e71e4a8b` | `python3 -m unittest discover -s scripts/ci -p 'test_*.py'` | 35 passed | `python-tests.log` |
| `e71e4a8b` | Actual unsafe-containment script extracted from `.github/workflows/ci.yml` | Pass, including `kf-chan:0` and `kf-qemu:61` | `unsafe-gate.log` |
| `e71e4a8b` | Matrix compared with compiler output at all 30 exact tags | 129,480 cells, 0 differences | `matrix-compare.log`, `compare_matrix.py` |
| `e71e4a8b` | `cargo fmt --all --check`, `git diff --check` | Pass | Exit status checked locally |
| `8e29da95` | Python compile plus mocked runner executions (all flags inherited; explicit off/all/private cases) | Inherited flags removed, explicit flags independent, `command.json` agrees; no actual VM | Runner-only local check |

To repeat the matrix check from the repository root:

```sh
python3 traces/windows_p1p2_integration_20261005/compare_matrix.py FULL_COMPILER_SWEEP
```

The full compiler sweep used here was `/tmp/kf-windows-dm-20261005/sweep`; it is
regeneratable by `tools/drivermatrix/dm.py`. The check supplements it with the
committed, separately compiled timer structure outputs under
`traces/windows_timer_20261005/matrix`. Missing tags or conflicting overlapping
measurements fail. It compares every committed range cell, including absent
fields; it is not a physical-GPU coverage claim.

The only configured local QEMU source was **11.1.1**. A C syntax check against
that source stopped at its moved `hw/qdev-properties.h` header
(`local-qemu11-inapplicable.log`). This is not a supported-QEMU compile result.
The actual QEMU **10.2.4** C/archive build and serial GPU runs are owned by the
parent task and need their own exact-revision evidence. No code was adapted to
the unrelated newer QEMU version to make this check pass.

## Independent timer review

The timer path authors fixed host verbs using its own client/subdevice and
newly minted timer object. Guest bytes, handles and VMM pointers do not reach
those calls. The host ABI layer supplies exact-tag control translation; timer
layout equality gates host/guest compatibility. The timer BAR0 base comes
from the live host query, without a captured die row. A missing layout,
unsupported map, host page larger than 4 KiB, or overlap with a special BAR
region refuses realization when explicitly requested.

Reads use RM's read-only register view, `PROT_READ`, an uncached requirement
consistent with BAR0, and QEMU's read-only ROM backing. A distinct disposition
prevents confusion with the usermode page. Every guest store overlapping the
timer page returns before queue/shadow/host work; it cannot update an alarm.
The mapping is created at realization, never on a vCPU. No CPU timer simulator
or forged GPU completion was added. The whole-page read permission and lack
of destructive read registers are documented against source in
`docs/design/V3_WINDOWS_TIMER_MAPPING.md`; native unprivileged evidence belongs
to source `831f6bd7`, not this integrated build.

Native ownership drops the mmap, releases its RM view and frees its object.
QEMU still uses the existing process-lifetime leaked-device model, so its
RAMBlock cannot outlive the mapping. Complete hot-unplug reclamation remains
an existing limitation. The new FFI validates null/alignment before writing
caller-owned output slots; the two new unsafe allowances are itemized beside
the CI ratchet. No new guest-to-host forwarding capability was found in this
delta. This limited review does not clear the general security audit backlog.

The earlier timer graph parent/singleton omission remains protocol
incompleteness: the new host backing is one host-authored read-only device
resource, not a host object created by each guest allocation. No timer control,
alarm or completion API was admitted by this mapping. Such additions would
need object ownership/lifetime validation before gaining behavior.

## Remaining security and hardware bar

**S1-21 remains open on the default path.** `KF3_TSPACE=1` selects the new
private Translated address space; it has not passed this integration's
hardware bar. The pool query still advertises explicitly invented virtual
geometry as a diagnostic, with no pool lifecycle. All three experiments remain
opt-in. Native timer success and a successful Windows boot would not prove
hostile-guest isolation, full preemption semantics, or portability across dies.

Required next work is specified in `docs/design/V3_P1P2_TSPACE.md` §8:

1. Build QEMU 10.2.4 and the Rust archive at the exact integrated revision; run
   bare-metal gates first, then `v3_gates.sh` 9/9, build/raw-client/fast-guest,
   and the 30/30 guest suite serially. Record euid, CapEff and USER birth bits.
2. Implement and run the still-unwritten T-WINDOW-USER positive/negative
   control and T-RING-TRANSLATED guest-kernel probe. Check physical-mode CE on
   USER and Translated twins, unchanged private canaries, and channel-local
   refusal/fault behavior. A stock-driver happy path cannot substitute.
3. Record T-space construction before births, source/ring bounds, injected
   negative-control counters, and per-family census. Run A/B application,
   CUDA, graphics, LLM and performance lanes, including an unprivileged guest
   application, unload/re-init and resource return to baseline.
4. Keep new Windows comparisons separate from narrow A/B/C/D evidence. The
   runner may use `--private-translated-space --pool-probe --timer-map`, with
   `--revision` naming the exact compiled binary. No success assertion or
   default change precedes the required evidence.

All product code and these logs are on the public branch. Neither this review
nor its tests depend on persistent access to the borrowed PC or a Vast disk.
