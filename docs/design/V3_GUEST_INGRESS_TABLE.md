# V3 guest-ingress table: the control/class allowlist, generated from ogkm (task I7)

**STATUS: LIVE, 2026-11-20 (branch `claude/no-gvisor-20261120`; code, generator and GPU-free tests; NOT yet run
on hardware: §7 is the validation list).** Replaces the hand-ported allowlist that `crates/kf-abi/src/capability.rs`
carried until this date. Owner decisions of 2026-11-20 (relayed by the coordinator): remove gVisor from the
QEMU-linked tree; a new driver branch needs only ogkm and the generator; tables and ids come from ogkm, and where
ogkm lacks information kayfabe applies its own judgement, in its own words, with its own evidence, listed for
review; the new permissions must never be weaker than the old (a test, not a generator input); keep and improve
the attribution to gVisor's nvproxy as the design reference.

## 1. What the gate is

`kf_abi::capability::CapabilityTable` answers, per guest driver tag, whether a guest may name an RM control word
or an allocation class at all. Default deny. An id with a row is admitted or **refused by name** with a reason
(`Refusal`); an id with no row is refused as `NotOnAllowlist`. Admission is necessary, never sufficient: every
admitted id still needs a consumer after the gate (`DriverAbiTable::control_params` / `alloc_params`, the channel
plane's constants in `kf_rm::chanlink`, the display link), and an id without one is refused there. All refusals
reach the guest as the same envelope status, `NV_ERR_NOT_SUPPORTED`.

The rows are `crates/kf-abi/src/ingress_generated.rs`, one per id with one run per span of equal verdicts over
the 30 driver-matrix tags (`traces/driver_matrix/ranges.tsv`, the same tags `table_for` accepts). There is no
nearest-tag fallback. Each control row also says whether kayfabe ever forwards the id to a host
(`forwarded: false` for the ids in `tools/api_boundaries/answered_never_forwarded.txt`).

## 2. The policy (kayfabe's)

| item | admitted at tag T iff |
|---|---|
| control | kayfabe serves it, T exports it, and every exporting class's flags at T include `NON_PRIVILEGED` and none of `PRIVILEGED`, `INTERNAL`, `RM_TEST_ONLY_CODE` (flags decoded by NAME with T's own `control.h`: the bit values moved in R555) |
| class | kayfabe serves it and T's `resource_list.h` gives it `RS_FLAGS_ALLOC_NON_PRIVILEGED` and neither `_PRIVILEGED` nor `_KERNEL_PRIVILEGED`; or it is a client root (internal class `RmClientResource`: `serverAllocClient` has no resource privilege check, `ogkm-580: libraries/resserv/src/rs_server.c:728-766`) |
| either | a row of `tools/api_boundaries/ingress_rules.tsv` overrides the derived verdict for its tags |

"Served" is what kayfabe's own code names (`tools/api_boundaries/derive_served.py` → `served.json`: control ids
and names, class ids and names, plus the Windows census and the matrix lists). **The gate is excluded
from that scan** (`EXCLUDE` in `derive_served.py`): before I7, 71 control ids and 122 control names were "served"
only because the gate itself named them, so the old `served.json` was circular. A control with no visibility flag
is kernel-only in RM (`ogkm-580: kernel/rmapi/control.c:702`) and is refused (`KernelOnly`). The GSS-legacy bit
(bit 15) earns no pass: an unserviced GSS-legacy control is refused at the gate.

## 3. kayfabe's own rows (`ingress_rules.tsv`)

⊘ **Correction, 2026-11-24 (security review of PR #545, F1).** The channel link (`kf_rm::chanlink`, ahead of the
object seat in the served chain) carried six controls to host-twin acts without asking the gate: the GSS NVENC
session pair `0x20808163`/`0x20808164` (no row), `DISABLE_USERMODE_CHANNELS` `0x20801117` and
`DMA_SET_DEFAULT_VASPACE` `0x00801812` (refused `KernelOnly`), and `STOP_CHANNEL` `0xa06f0112` and
`FIFO_SET_CHANNEL_PROPERTIES` `0x0080170f` (refused by `AnsweredInChain` rows, which said "nothing needs it
admitted"; the link did). Every act was authored, bounded and on the VM's own objects, so nothing reached further
than intended, but "the gate decides every guest control that reaches a host act" was false. Now the link asks the
gate about every control before any arm, and the table says what is served: four `Mode2Rpc` allow rows (the
NVENC pair, `DISABLE_USERMODE_CHANNELS`, `DMA_SET_DEFAULT_VASPACE`: each reaches kayfabe from the guest KERNEL RM
over GSP RPC), and the two `AnsweredInChain` refuse rows are deleted (ogkm exports both `NON_PRIVILEGED`, so the
derived verdict admits them). The counts below are therefore 53 allow rows (46 `Mode2Rpc`) and 119 refuse rows
(29 `AnsweredInChain`). Guest-visible behaviour is unchanged: these ids were served before and are served now.

- **49 allow rows**: 42 `Mode2Rpc` (they reach kayfabe only because in Mode 2 the guest's KERNEL RM talks to
  kayfabe as its GSP: the page-directory carriers, PROMOTE/EVICT_CTX, the CUDA-limit pair, MMU fault info, the
  display objects, `GP100_UVM_SW`, the deferred-API class and controls from 580.65.06, the two video classes
  from 570.86.15, ...) and 7 `Empirical` (caps queries measured on a real host ioctl stream, `NV01_EVENT`,
  `AMPERE_B`). Their content is the pre-I7 rows of those two origins, carried over unchanged; their comments were
  rewritten in the TSV's own words.
- **121 refuse rows**, kayfabe's judgements where ogkm's flags alone would admit a served id. The categories are
  `Judgement` in `capability.rs`: `RegisterAccess` (3), `Profiler` (1), `SmDebugger` (3), `CallerMemory` (2),
  `NoBoardBus` (1), `HostScope` (40: power, ECC, PCIe/NVLink counters, BIOS, scheduling, caches, fabric),
  `DisplayEngine` (27), `AnsweredInChain` (31: answered by kayfabe's served chain before the gate),
  `EngineNotModelled` (8), `NotValidatedAtTag` (5: the deferred-API / kernel-callback pair below 580.65.06).
  Each row carries its reason and evidence; they exist so the new gate is never wider than the old one (§5) and
  are listed here for the owner's after-the-fact review.

A rule row must spell the id the way some ogkm tag names it; the generator refuses a misnamed row.

## 4. The generator and a new driver branch

`tools/api_boundaries/ingress_policy.py`:

```
ingress_policy.py extract --clone <ogkm blobless clone> --out <work>/facts --digest   # every tag of ranges.tsv
ingress_policy.py extract --tree 620.1.2=<ogkm tree> --out <work>/facts --digest     # one new tag
ingress_policy.py emit      # writes ingress_generated.rs and ingress_report.tsv
ingress_policy.py check     # exit 1 if either is stale
```

`extract` reads only the exported-method tables (`g_*_nvoc.c`), `control.h`, `resource_list.h`, `nvos.h` and the
SDK `ctrl/` and `class/` headers, and keeps the decisive facts in `tools/api_boundaries/ingress_facts.tsv` (one row
per run of equal facts over the tags; about 2,300 rows). A new tag is: add it to the driver matrix, `extract --tree` it,
`emit`. No hand step; `test_a_new_tag_needs_only_its_tree` runs exactly that on a synthetic tag with renumbered
flag bits, a new class and a renumbered control. Header of the output: `@generated from ogkm 535.309.01..615.71.09
(30 tags) + served.json by tools/api_boundaries/ingress_policy.py`, `SPDX-License-Identifier: Apache-2.0 OR
GPL-2.0-or-later`; NVIDIA's MIT notice stays in `THIRD_PARTY_NOTICES.md` §1.

Counts at this commit: 293 control rows and 123 class rows. At 580.159.04 the gate admits 74 controls (55 by ogkm
flags, 14 `Mode2Rpc`, 5 `Empirical`) and 98 classes; the pre-I7 gate listed 170 controls and 119 classes there
and also passed every bit-15 control and every `0x2081xxxx` control by rule. `ingress_report.tsv` lists every
refusal by name and the 98 served control ids that no ogkm tag exports (`unknown-to-ogkm`: GSP-firmware-only
GSS-legacy ids and Windows-census ids); those are refused by the default, with one exception, the `Empirical` row
`0x0080028b` (no ogkm name; kept allowed as before, flagged for the owner).

## 5. Parity with the pre-I7 gate (tests)

⊘ **Correction, 2026-11-24 (security review of PR #545, F1).** Item 2 below says that a newly refused id with no
consumer is answered "the same `NV_ERR_NOT_SUPPORTED` either way". That was false for the NVENC session pair: the
old gate admitted it (bit 15), the new table had no row, and the channel link served it before the gate, so the
guest was served, not refused. The test's universe did not contain the pair and its consumer list named only the
link's own constants. Now (a) the universe and the consumer list include every control the link claims
(`kf_rm::chanlink::CLAIMED_CONTROLS` and the two version-dependent ids), (b)
`every_control_the_channel_link_claims_is_admitted_at_every_tag`, `the_channel_link_asks_the_gate_before_any_arm`
and `the_claimed_list_is_exactly_the_match_arms` pin the link to the gate, and (c) the exceptions file lists seven
ids: the three below and the four ids of §3's correction that the old gate also refused while the link served
them.

The pre-I7 verdicts are kept ONLY as a test oracle: `crates/kf-rm/tests/fixtures/pre_i7_gate_verdicts.tsv`
(id, verdict, tag span; no name, comment, ordering or structure of that table). `crates/kf-rm/tests/ingress_parity.rs`:

1. **Never wider** (owner, hard): an id admitted now where the old gate refused it fails, unless the reviewed
   `tools/api_boundaries/ingress_parity_exceptions.tsv` lists it. It lists three, each NON_PRIVILEGED in ogkm with
   the same name and flags at the neighbouring tags where the old gate admitted it: `0x00801909`
   PERF_CUDA_LIMIT_SET_CONTROL and `0x20802068` PERF_GET_CURRENT_PSTATE at 550.40.07 (the old gate filed that tag
   under its 545 block), `0x20801357` FB_GET_STATUS at 570.86.15..570.148.08.
2. **Newly refused only where nothing consumes it**: the old gate admitted 120 ids by row, and 80 more
   (bit 15 set, or class `0x2081`) by rule, that the new one refuses (NV0000 client controls the guest's own RM answers, profiler/fabric/IMEX/P2P/
   NVJPG/confidential-compute classes, ...). The test proves that none of them has a consumer after the gate at
   any tag kayfabe can assemble a driver table for (no control params shape, no `kf_rm::chanlink` constant, no `alloc_shape`), so the guest sees the same
   `NV_ERR_NOT_SUPPORTED` either way; only the refusal census tag changes (`ControlNotPermitted` instead of
   `UnknownControl` / `UnmappedAllocClass` / the removed `GspRuleControlUnserviced`).
3. **Lane evidence stays admitted**: `crates/kf-rm/tests/fixtures/ingress_must_stay_allowed.tsv`, 68 ids (51
   controls answered `NV_OK` and 17 classes allocated with status 0 in the committed lane traces, with the file
   that shows it) must stay admitted wherever the old gate admitted them. Left out, with the reason in the file:
   `0x20808159`/`0x20808162` (answered by `kf_rm::inittables` before the gate), `0x20810108` (already refused in
   v3), `0xcb33` (status 0 only in pre-v3 logs; no alloc shape in v3). The committed traces hold only part of
   the lanes' logs, so this list is a lower bound (§7 closes it).

Mutants (`scripts/ci/ingress_policy_mutants.py`, 11, each killed by at least two named tests): privileged /
internal / kernel-only control admitted, control policy flipped, class check skipped, privileged class admitted,
absent-at-tag control admitted, unknown control given a row, gate admits unlisted ids, gate admits refused rows,
gate skips the class check.

## 6. Licence (task I7, owner position)

⊘ *Correction 2026-11-23 (security audit stage 2, S2-04b in `docs/audits/2026-11-23-v3-stage2.md`): the scope
stated in the next paragraph left out code the kf3 binary embeds. The gate now also scans `cuda/display` (the
display PTX kf-cuda embeds) and `firmware/kf-gop` (the GOP driver kf-gop-image's build script builds and embeds,
with its own `Cargo.lock`), follows build-dependency edges, and fails on a third-party crate that its
`EXTERNAL_PINNED` set does not name (today `libc`).*

The QEMU-linked tree no longer contains gVisor-derived code, data, comments or citations;
`scripts/ci/qemu_license_closure.py` walks kf-qemu's cargo closure (19 workspace crates, one external crate,
`libc`, `MIT OR Apache-2.0`) plus `qemu/hw/misc/kf3` and `cuda/walk`, and fails on a file named in `LICENSE`
exception 2, an SPDX tag with no GPL-2.0-compatible option, a gVisor/nvproxy mention, or a third-party crate
licence without a GPL-compatible option. Exception 2 now covers only `crates/kayfabe-abi/src/capability.rs`
(frozen; linked only into the raw test client `kayfabe-rm-ladder`, which is not the QEMU device and is not
distributed) and the archive. The owner's position: tables and ids come from NVIDIA's MIT release, a safety
judgement is not copyrightable, risk low; a counsel check before the first public binary is recommended, not
blocking. Attribution: `THIRD_PARTY_NOTICES.md` §4 and `docs/howto/credits.md` credit nvproxy as the design
reference for a default-deny allowlist at the guest ingress.

## 7. What only hardware can prove (for the coordinator)

At this branch's revision, against the same lanes at `next`:

1. `v3_gates.sh` 9/9, `fast_suite.sh` 30/30 (raw and thin), `cuda_ladder.sh`, the 66/66 app matrix, the Windows
   gates (580.88 guest, tag 580.65.06: the deferred-API class/controls and `NV01_EVENT_KERNEL_CALLBACK` must stay
   admitted), the merge bar.
2. In every qemu log, the refusal census by id (the `kf-rm: RPC-REFUSED` lines and the census at exit); any id
   refused here and not on `next` is a regression. Expected: identical sets except
   that ids formerly tagged `UnknownControl` / `UnmappedAllocClass` / `GspRuleControlUnserviced` may now be tagged
   `ControlNotPermitted` / `AllocClassNotPermitted` (same guest status).
3. One guest at a non-bench tag on each side of the flag renumbering (a 550.x or 545.x guest, and a 595.x guest)
   boots and runs the CUDA rung: their gates are different rows of the same table.
