# V3 licence scoping: what blocks a distributable QEMU build, and how to unblock it

**STATUS: DESIGN-ONLY, 2026-10-31.** Research and options only; no code, no licence text and no
`LICENSE` change is made by this document. Written by an AI agent. It is not legal advice and not a
clean-room statement (`V3_PROVENANCE.md` section 3 already says kayfabe is not a clean-room project).
Everything marked NEEDS-COUNSEL below is a question for a lawyer, not a conclusion. Source tree:
`next` at `7fdbf7314`. The AI legal review of 2026-10-23 (private, not committed) reached the same
blocker; this document adds the file inventory, the replacement sources and the option costs.

Labels: MEASURED = a count or a check I ran on 2026-10-31 with the command shown in section 8.
INFERRED = my reading of code or licence text, with the reasoning. UNSURE = I could not decide.

## 0. Result in ten lines

1. The blocker is one file in the product: `crates/kf-abi/src/capability.rs` (3607 lines). It holds
   about 292 allowlist rows, of which 243 are tagged as taken from gVisor's `nvproxy`. It is linked
   into the QEMU binary through `kf-qemu` -> `libkf_qemu.a`. (Section 2.)
2. The rows are NVIDIA identifiers and names (facts that ogkm publishes under MIT). What is gVisor's
   is the selection (which ~290 of ~760 unprivileged controls and ~146 unprivileged classes a guest
   may name) and the version-boundary layout. ogkm cannot supply a selection. (Section 3.)
3. The old table is a ready oracle: a 2438-line dump of every decision at every boundary is already
   committed, with a test that compares the full text. Any replacement can be diffed row by row.
4. Recommended: option (a), re-derive from ogkm plus a kayfabe-authored policy file plus kayfabe's own
   consumer set, with the old table as the oracle. Estimate 7 to 9 working days of agent and review
   time and one hardware day, then counsel review. (Section 5.)
5. Keep today's rule (no public QEMU binary) until (a) lands. That is option (c) and it is free, but it
   costs adoption. The no-binary rule also has a second, independent reason (security audit, README).
6. A "linking exception" in our own licence cannot fix this; only the gVisor authors could grant
   permission for their code, and QEMU's terms are not ours to change. (Section 4.2.)
7. A GPLv3 route is closed by QEMU itself: its tree contains GPL-2.0-only files. (Section 4.1.)
8. The frozen `kayfabe-abi` twin and the `archive/` files carry the same Apache-2.0-only content but are
   not linked into QEMU; they can stay as a narrowed, documented exception. (Section 2.4.)
9. The honest provenance statement is "generated from ogkm by tool X and policy file Y, and diffed
   against the earlier table", never "clean room". (Section 6.)
10. Main uncertainty: whether counsel accepts a re-derivation done by people and agents who have read
    gVisor. The mitigation is to make the selection rule new, not to re-express the old selection.

## 1. The problem chain

- `LICENSE` grants `Apache-2.0 OR GPL-2.0-or-later`, with exceptions. Exception 2 lists files that
  derive from gVisor's `nvproxy` (Apache-2.0) and "are available under the Apache License 2.0 only until
  they are re-derived independently".
- `qemu/hw/misc/kf3/meson.build` adds the device with `system_ss.add(when: 'CONFIG_KF3', ...)` and
  links `libkf_qemu.a` with a static link group into the one QEMU system binary.
- QEMU's own `LICENSE` says the emulator as a whole is GPL version 2 (read in `/opt/qemu-src/LICENSE`
  on 2026-10-31; that tree is 9.2.0, the pin in `V3_SWEEP_AND_INSTALL.md` is 10.2.4).
- The FSF position is that Apache-2.0 is incompatible with GPL-2.0-only (patent-termination and
  indemnity terms count as further restrictions). So one binary cannot satisfy both texts for that file.
- Result in `docs/howto/credits.md` and the README: a QEMU binary built from this tree is not yet
  distributable. Building and running from source is unaffected; the Apache-2.0 option of the dual grant
  stays valid for everyone who does not combine the file with GPL-2.0-only code.

## 2. Inventory (MEASURED 2026-10-31, commands in section 8)

### 2.1 Headline totals

| scope | files naming gVisor or nvproxy | notes |
|---|---|---|
| v3 product, `crates/kf-*` | 17 | 4 hold derived content, 13 only cite file and line |
| frozen pre-v3 tree, `crates/kayfabe-*` | 14 | 1 holds derived rows (its own `capability.rs`); 13 cite |
| tooling outside crates | 3 | `scripts/claim_ledger.py`, `scripts/ci/claims-debt.json`, `tools/drivermatrix/tags.txt` (comments and ledger text only) |
| `archive/` | 73 (code and docs) | 11 files are the LICENSE and CREDITS "derived" set, 4418 lines |
| `docs/`, `site/`, top-level docs | 46 | prose, research tables and citations; not code |
| `traces/` | 15 | evidence text that cites gVisor; not code |
| `qemu/`, `cuda/`, `broker/`, `firmware/`, `ci/`, `.github/` | 0 | none |

Authored SPDX Apache-2.0-only markers (not dual) exist only in `archive/`: four integration test sources
under `archive/nvkvm/tests/integration/` (none of them mentions gVisor). None of the `kf-abi` and
`kf-rm` files discussed below carries an SPDX line, except the generated MIT files; `LICENSE` is their
grant. Other crates (`kf-broker`, for one) do carry the dual line (grep of `SPDX-License-Identifier`
over `crates/kf-*`, 2026-10-31).

### 2.2 The v3 files (the ones a QEMU binary or the v3 tests can reach)

"In QEMU" means the crate is in the dependency closure of `kf-qemu` (its `Cargo.toml` lists `kf-abi` and
`kf-rm`; no `kf-*` crate depends on a `kayfabe-*` crate, checked with a grep of `^kayfabe-` in
`crates/kf-*/Cargo.toml`, 2026-10-31).

| path | lines | what it is, in plain words | in QEMU | derived share |
|---|---|---|---|---|
| `crates/kf-abi/src/capability.rs` | 3607 | the guest-facing allowlist: which control ids and which allocation classes a guest at a given driver version may name, plus the explicit-denial list and two rule-based passes | yes | see 2.3 |
| `crates/kf-abi/src/versions.rs` | 2523 | per-driver-version ABI tables; the `CAPS_ROWS` region (about lines 395 to 500) picks which capability table applies from which version and keeps boundary notes | yes | the boundary choice and notes follow nvproxy's version chain; rest is generated-layout logic |
| `crates/kf-abi/src/transcribed.rs` | 555 | hand-written layouts that the generator does not yet produce; `Nvos46ParametersPre580` (56-byte struct before driver 580.65.06) names three sources, one of them nvproxy | yes | one struct (about 60 lines); the file itself says ogkm 575.51.02 added to the generator input replaces it |
| `crates/kf-abi/tests/fixtures/capability_550_610_before_535.txt` | 2438 | dump of every control and class decision at the eight earlier boundaries, with origin tag; the regression oracle | no (test data) | all of it is a rendering of the derived policy |
| `crates/kf-abi/tests/oracle_layout.rs` | 1314 | proves generated layouts against ogkm, nvproxy and the C artifact; 24 lines cite nvproxy by file and line | no | offsets and sizes (NVIDIA ABI facts), cited |
| `crates/kf-abi/src/{lib,view,wire,gsslegacy,cudartinit,gspstaticinfo}.rs` | 420 / 1044 / 621 / 369 / 503 / 1722 | wire views, errors, init tables; 1 to 8 lines each mention nvproxy as a pointer or as a negative result ("nvproxy does not know this id") | yes | none (citations; view.rs mentions that the verb split follows nvproxy's, an idea not text) |
| `crates/kf-abi/gen/src/main.rs`, `generated/classes.rs` | 1580 / 1457 | the ogkm generator and its output; one doc string each cites a negative nvproxy finding | generated file yes | none |
| `crates/kf-rm/src/rmrpc/mod.rs` | 2020 | guest GSP RPC ingress: calls the capability gate; 4 comment lines cite nvproxy | yes | none |
| `crates/kf-rm/src/inittables.rs`, `tests/rmrpc_bridge.rs`, `tests/support/rpcwire.rs` | 3301 / 7268 / 1258 | init tables and tests; 1, 2 and 1 lines mention nvproxy | inittables yes, tests no | none |

Not listed by `LICENSE` but carrying derived content, by my reading: the `CAPS_ROWS` region of
`versions.rs`, the fixture, and the NVOS46 struct in `transcribed.rs`. LICENSE names only
`kf-abi/src/capability.rs`, its `kayfabe-abi` twin, two archive headers, one archive Go test and the
CREDITS list. INFERRED: the first three are small or test-only, but an honest exception list should
name them or they should be re-derived in the same change. NEEDS-COUNSEL.

### 2.3 What is inside `capability.rs`

Line split by region (MEASURED 2026-10-31 with `awk` over the line ranges in section 8):

| lines | share | content | derived? |
|---|---|---|---|
| 1 to 155 | 4% | module documentation: design argument, boundary table, many `file:line` pointers into gVisor | pointers only; the boundary table (eight rows) follows nvproxy's version chain |
| 156 to 674 | 14% | types and logic: `Origin`, `ControlEntry`, `ClassEntry`, `DeniedBecause`, `CapabilityTable`, lookup by binary search, the two rule passes | the shape (inherit/replace model, deny-before-rule order) is kayfabe's own; the two rule constants (`RM_GSS_LEGACY_MASK`, `NV2081_BINAPI_CLASS`) are facts |
| 675 to 2107 | 40% | data: 14 allow statics, 2 deny statics, 10 boundary tables | yes: this is the derived part |
| 2120 to 3607 | 41% | unit tests | tests assert named rows, so they follow the data |

Row counts (MEASURED 2026-10-31 by a throw-away parser over the data region, described in
section 8): 292 allowlist rows in the 14 allow statics, tagged `Nvproxy` 243, `Mode2Rpc` 41, `Empirical`
7, one row my parser could not tag. Of the 243: 154 controls and 89 allocation classes. The deny
statics hold 11 control and 5 class rows, of which one class row is tagged `Nvproxy`. The `Mode2Rpc`
and `Empirical` rows (48) are kayfabe's own additions (rows observed on a real stream, or rows that only
exist because the guest talks GSP RPC) and are not a problem.

The frozen twin `crates/kayfabe-abi/src/capability.rs` (3222 lines) has the same 248 `Origin::Nvproxy`
tokens (including documentation), 9 `Empirical` and 17 `Mode2Rpc` (the v3 file has 43 `Mode2Rpc`).

### 2.4 Which binaries link what

- `kf-qemu` (`crate-type = ["rlib", "staticlib"]`) depends on `kf-abi`, `kf-rm`, `kf-chan`, `kf-host`,
  `kf-chip`, `kf-gsp` and others. `libkf_qemu.a` is linked into `qemu-system-*` by the meson fragment
  above. This is the only artifact the licence blocker concerns.
- Other `kf-abi` dependents: `kf-census` (a tool binary) and `kf-harness` (20 binaries under
  `src/bin/`). They are Apache-2.0 OR GPL-2.0-or-later with the one file; as separate programs they have
  no GPL-2.0-only partner, so they can be distributed under Apache-2.0.
- `crates/kayfabe-abi/src/capability.rs` is linked by `kayfabe-rm-ladder` (the frozen raw client the
  bare-metal suite uses) and its closure; not by QEMU. `AGENTS.md` requires that closure to keep building.
- `archive/nvkvm/src/qemu/*` is the old C QEMU device. No script, crate, tool or workflow outside
  `archive/` references `archive/nvkvm/src` (grep on 2026-10-31, hits only in comments of the two
  capability files and in test docs). It is not part of the kf3 build.
- `.github/workflows/release-build.yml` builds the whole workspace and attests the binaries; it does not
  build QEMU and publishes nothing by itself.

### 2.5 How the table is used (INFERRED from the call sites listed by the grep in section 8, 2026-10-31)

- Consumers in non-test code: `rmrpc::translate_alloc` and `translate_control` (generic gate, by any
  id), about ten explicit checks in `kf-rm/src/chanlink.rs` (one per control or class the channel plane
  serves, for example the TSG timeslice control and GR preemption mode), one in `rmrpc/policy.rs`, one in
  `kf-rm/src/display.rs`, one in `kf-rm/src/sw_runlist_probe.rs` (a test) and one in `kf-abi/versions.rs`
  (the deferred-API class).
- Cost: a binary search over sorted static slices, per guest RM alloc or control RPC, on the register
  drainer side. The vCPU trap path only enqueues BAR0 writes and never calls it. It runs at boot (the
  guest driver's init RPCs) and at run time (each guest RM alloc or control, for example CUDA allocations),
  not per doorbell. The data sits in read-only memory. There is no performance reason to keep the current
  shape or to change it.
- INFERRED: of the roughly 290 rows, only the ids that kf-rm names explicitly (about ten controls and
  the classes with an `AllocParams` decoder) change behaviour when removed; the rest are a default-deny
  statement that the module's own documentation calls "defence in depth over a decision the driver makes
  again". UNSURE: whether the fake GSP's generic answer path would then serve an id the table no longer
  lists; that is what the shadow census in section 5 measures.

## 3. What each part is for, and what could replace it without reading gVisor

Sources I checked on 2026-10-31 (read-only, no gVisor text opened):

- ogkm 580.65.06 (`/tmp/open-gpu-kernel-modules-580.65.06`): the generated `g_*_nvoc.c` export tables list
  every control with its id, `flags` word (bit `0x8` is `RMCTRL_FLAGS_NON_PRIVILEGED`, bit `0x4` is
  `PRIVILEGED`, defined in `src/nvidia/inc/kernel/rmapi/control.h`) and parameter size. V3_BUILD.md
  already uses these flags for the host-query side.
- ogkm 595.84 (`/workspace/ogkm-tars/...`): `src/nvidia/src/kernel/rmapi/resource_list.h` lists every
  class with `RS_FLAGS_ALLOC_NON_PRIVILEGED`. The 580.65.06 copy at `/tmp` is trimmed and lacks that file;
  a real generator must read it at each pinned tag (the submodules `third_party/ogkm*` have it).
- `tools/drivermatrix/` (`dm.py`, `sdk.spec`, `tags.txt` with 29 tags): compiler-based extraction of
  integer macros and struct layouts per ogkm tag. It is the right shape for ids and names per version.
- The old tables, the fixture and `kf-census` (decodes guest-to-GSP streams) are kayfabe's own data.

### 3.1 Cross-check I ran: ogkm flags against the old table

MEASURED 2026-10-31 with the script sketched in section 8, on the data region of `capability.rs`:

| quantity | value |
|---|---|
| controls the table lists with origin `Nvproxy` | 154 |
| of those, found in the 580.65.06 export tables with the non-privileged flag | 147 |
| of those, not found in the export tables | 7 |
| all controls in the 580.65.06 export tables | 1339 |
| of those, flagged non-privileged | 757 |
| classes the table lists with origin `Nvproxy` | 89 |
| of those, flagged alloc-non-privileged in ogkm 595.84 | 86 (the other three are the root client classes) |
| all classes in 595.84 `resource_list.h` | 209 |
| of those, flagged alloc-non-privileged | 146 |

What this shows: ogkm's flags agree with the old table (every row but 7 controls and the 3 root classes
is non-privileged), and they are a superset. A flag-only derivation would admit 757 controls and 146
classes, about four to five times the present ~168 controls and ~115 classes. So ogkm gives existence,
version range, names and the privilege floor, but not the selection. The selection must come from a new,
kayfabe-authored rule.

The 7 controls absent from the export tables: two are expected (one the table itself says was removed at
555.42.02, one removed at 575.51.02), and five are the legacy-shim controls (`BUILD_VERSION`,
`MSENC_GET_CAPS`, `BIOS_GET_INFO`, `FB_GET_INFO`, `BUS_GET_INFO`), which the 580.65.06 sources mention
only in `src/nvidia/interface/deprecated/rmapi_deprecated_control.c`, `embedded_param_copy.c` and the
display and push code. So a generator also needs the deprecated-control path as a second source. The
legacy-GSS bit rule is also used by ogkm in `rmapi_deprecated_control.c` and
`rmapi_gss_legacy_control.c`; I did not find the `#define` of the mask in the trimmed copy.

### 3.2 Per file: generate, measure, or hand-write

| item | replacement source | generator shape | hand-written part |
|---|---|---|---|
| allowlist ids and names, per version | ogkm headers via `tools/drivermatrix/dm.py` (`sdk.spec` extended with the control prefixes already used), 29 tags | extend the tag sweep to emit, per tag, the set of control ids with name, flags, param size; and class ids with name and alloc flags | none for ids and names |
| version boundaries (eight today) | tag-to-tag diff of that set: a boundary exists where the permitted set changes | computed, not listed; the diff names exactly which rows enter or leave | the policy decides which differences matter |
| the selection ("which of the 757") | a kayfabe-authored policy file with category rules in ogkm terms: object class family (system, device, subdevice, memory, channel), flag bits (non-privileged, route-to-physical, no-GPUs-lock), the denylist categories (registers, debugger, profiling, fabric, power, internal) | policy file is input data in the repo (a few dozen lines); generator applies it | the policy file; owner reviews it (security) |
| rows the guest actually needs | kayfabe's own consumers (the ids named at the call sites in 2.5) plus a census of guest traffic from `kf-census` over the 30 raw, 30 fast, app and Windows lanes | census tool already exists; a committed multi-lane id list does not | choosing the lanes |
| `Mode2Rpc` and `Empirical` rows (48) | already kayfabe's | keep as a hand file | yes, as today |
| deny list with reasons (16 rows) | kayfabe's own reasoning and ogkm flags | keep as a hand file | yes |
| two rule constants | ogkm (`rmapi_deprecated_control.c` and `cl2081` class header, define located at the pinned tag) | read at the tag | none |
| `Nvos46ParametersPre580` | ogkm 575.51.02 `nvos.h`, already in the 29 tags | add the tag's `NVOS46` to the generator input as the file's own comment says | delete the hand struct |
| `oracle_layout.rs` nvproxy section | compiler-measured offsets from `dm.py` (`offsetof` per tag) | already the matrix approach | re-point 24 citations |
| the fixture | regenerated by the existing `dump_capability_policy` example when the new table lands, after the diff has been triaged | n/a | the triage |

The point of "derive, never capture" is met by (ids, names, versions, flags) coming from ogkm and the
selection coming from a policy file that says why, not from a list that says what.

## 4. Distribution analysis

### 4.1 Why the current binary cannot go out under one licence

- Under GPL-2.0 the whole combined work must be offered under GPL-2.0. The Apache-2.0-only file cannot be
  offered under it (FSF reading; NEEDS-COUNSEL for the user's jurisdiction).
- Choose GPL-3.0 for the whole binary instead? Apache-2.0 is compatible with GPL-3.0. But QEMU's text says
  the emulator is GPL-2 as a whole, and QEMU's tree carries files marked `GPL-2.0-only`. MEASURED
  2026-10-31 (`grep -rIl 'SPDX-License-Identifier: GPL-2.0-only'` over `/opt/qemu-src`, 9.2.0): 47
  files with that tag, among them `util/crc-ccitt.c`, `hw/gpio/pcf8574.c`, `hw/net/igb_regs.h`,
  `hw/xen/xen_pt.h`. QEMU's `LICENSE` also names `hw/vfio/`, `linux-user/` and `bsd-user/` as v2-only.
  UNSURE: which of them the kf3 build configuration compiles in (I did not run the build, disk is tight);
  the first one lives under `util/`, which is always built. INFERRED: a GPL-3.0 conveyance of the
  general QEMU build is therefore not available; counsel should decide whether a reduced configuration
  would be.
- The kayfabe side is fine in a GPL-2.0 combination: the dual grant lets the receiver choose
  GPL-2.0-or-later; generated files are MIT; `broker/` files are `GPL-2.0 OR Apache-2.0` and take the GPL
  option; `libc` is `MIT OR Apache-2.0` and takes MIT. The only exception is the one file.
- Distribution also owes the corresponding source (QEMU tarball, kf3 overlay, kayfabe at the tag), the
  `LICENSES/` directory for firmware and bundled libraries (glib is LGPL), as `V3_SWEEP_AND_INSTALL.md`
  section 2.9 already lists. A dependency licence audit of the Rust standard library and its vendored
  parts for the static link has not been done (small, listed in section 7).

### 4.2 Options

Option (a): re-derive from ogkm with generators (the recommendation).

- What: build the generator and the policy file of section 3, emit a generated table in place of the
  data region, keep the types and logic, port the tests. The old table and the fixture are the oracle.
- Effort (an estimate, not a measurement): generator about 500 to 700 lines of Python, 2 days; policy
  file and its review by the owner, 1 day; table swap and port of about 1500 lines of tests, 2 days;
  diff triage against the 2438-line fixture, 1 to 2 days; documentation and LICENSE rewrite, 0.5 day;
  one hardware day for the gates. About 7 to 9 working days, in three draft PRs, plus counsel time.
- Test strategy (the "nothing is lost" rule):
  1. Freeze the oracle: the committed fixture and `dump_capability_policy` output at `7fdbf7314`.
  2. Emit the new table into a side file; diff per boundary and per row; every removed row needs a
     reason ("no consumer, not hit in any lane, owner accepts") and every added row a reason ("ogkm flags
     it non-privileged and the policy admits its family").
  3. A shadow census: for one run of each lane (v3 gates, raw 30, fast 30, CUDA ladder, app matrix,
     Windows), record which control and class ids reach the gate and in which state. Rows never hit and
     never named by code are the drop candidates. "Never hit in our lanes" is not "never needed", so
     the policy file, not the census, decides admission.
  4. Hardware: `scripts/bench/v3_gates.sh` 9/9, then the build and the 30/30 fast suite on the new
     revision, per `AGENTS.md`. Bare metal first.
- Risks: (i) a looser policy widens what a hostile guest may name (a real change, owner decision);
  (ii) a tighter one breaks a guest or an app that only the old list admitted; (iii) the result is not
  clean-room; (iv) boundaries now follow ogkm diffs and may not match gVisor's, so some old boundaries
  split or merge, which is correct but noisy in the diff.

Option (a'): the smallest variant of (a), consumer-derived. The table is exactly the ids kayfabe's own
code names plus the deny list plus the two rules, with ids and names generated from ogkm. It makes the
provenance trivial (own code is the source) and is the cheapest (about 4 to 5 days), but it is the
largest behaviour change and depends on the shadow census to show that nothing else is served through
the table. Pick it only if the census says the generic gate matters little.

Option (b): keep Apache-2.0 for those files and distribute under a combination counsel approves.

- b1, GPL-3.0 conveyance: closed unless a reduced QEMU build with no v2-only file compiles (4.1).
- b2, "linking exception" in our licence: cannot work. An exception we write covers only code we own.
  The gVisor authors would have to grant the permission, and Apache-2.0 offers no way for us to do it;
  QEMU's licence is not ours to change.
- b3, ask the gVisor authors for a GPLv2 permission for those files: possible in principle, I rate it
  unlikely to be answered and it cannot be planned around. UNSURE.
- b4, separate process: put the policy (or the whole kayfabe core) in a separate program that talks to
  QEMU over a socket (the credits page already lists Caraxes, which uses vfio-user with another VMM). A
  separate program is not a derivative by the FSF's usual reading, but kayfabe's design is one process,
  with BAR0 write trapping lockless inside the VMM (`ARCHITECTURE.md`), so this is a redesign with a
  per-write cost. Not recommended for a licence reason alone.
- b5, data split: ship the allowlist as a data file read at start-up (Apache-2.0, next to the
  GPL binary) rather than compiled in. A data file read by a program is normally not part of the
  program, so the binary has no Apache-only code in it. Cost 1 to 2 days (loader, fail-closed when the
  file is missing, hash pinned in the build). It leaves the Apache-only content in the repo and in the
  distribution, so the repository stays "dual with an exception", the opposite of the stated goal, and
  the aggregation reading is a counsel question. A good fallback if (a) cannot finish, not a goal.

Option (c): no binaries; ship a reproducible recipe plus source (today's state).

- Cost to adoption: every user must build QEMU 10.2.4 with the two-hunk overlay, the Rust toolchain and
  the host libraries (`scripts/bench/build_kf3.sh`), which the README already tells them. There is no
  release asset and no container image, and some users and distributions will not build a patched QEMU.
  It carries no legal risk beyond what the repository already states.
- It is the right posture until the security audit is done anyway (README: "no public binary ships" for
  security reasons as well). So (c) and (a) are not alternatives in time; (c) holds until (a) and the
  audit are both done.

### 4.3 Recommendation

Do (a), starting with the shadow census and the policy file because those decide whether (a) or (a') is
the right size. Keep (c) until counsel has seen the result. Do not spend effort on b1 to b4. Keep b5 as a
named fallback only. Meanwhile make the LICENSE exception list honest (section 7, items 1 to 3), which is
free.

## 5. Plan, with falsifiers

Each step states the result that would stop or redirect it.

1. Shadow census of the gate (read-only logging; a small code change, outside this document). Falsifier:
   if lanes hit a large set of ids that no code names (for example more than 40), the table is a real
   ingress gate and (a') is wrong; do (a) with a census-informed policy.
2. Policy file draft by the owner's rule: families allowed, categories denied (registers, debugger,
   profiling, fabric, power, internal). Falsifier: if the owner wants the surface to stay exactly as it
   is today, no ogkm rule will reproduce a curated list, and the only honest result is to re-express the
   same list from the census; counsel must then say whether that is independent enough.
3. Generator and diff report. Falsifier: if the per-boundary diff shows more than a few dozen rows
   changing for reasons the owner cannot accept, stop and shrink the policy, not the oracle.
4. Hardware gates on the new revision. Falsifier: any 30/30 or app regression traced to a refused id.
5. Counsel review of the provenance record (section 6) before the first binary.

## 6. Provenance: what can honestly be said

A statement that survives scrutiny: "The allowlist rows are generated by `<tool>` from NVIDIA's
open-gpu-kernel-modules at the tags in `tags.txt` (MIT), filtered by `<policy file>` written for kayfabe,
and compared against the table of revision `7fdbf7314`, whose differences are listed in `<report>`." It
must not say clean room, and must not say "independently developed" without the history: people and
agents on this project read gVisor's `nvproxy` (this repository pins it as `third_party/gvisor`, and
`archive/nvkvm/CREDITS` describes the earlier derivation). The git history still holds the old file under
Apache-2.0, which is compliant as long as its notices stay.

What a counsel review would need:

- the old and new tables, the generator, the policy file and the diff report, each with its commit;
- the dates and the identities (people, agents) that read gVisor, so the reviewer can judge how much of
  the policy file could be recollection (this document can supply what the repo records; it cannot supply
  what individuals remember);
- the position on selection and arrangement: whether a set of ids selected by a new rule, but overlapping
  heavily with the old selection, is a copy. Facts and function-driven lists get little protection in
  many systems; "little" is not "none" (NEEDS-COUNSEL);
- whether to keep a note to Google in the notices (Apache-2.0 attribution for the history).

## 7. Lists

### NEEDS-COUNSEL (short)

1. Is the FSF reading (Apache-2.0 not combinable with GPL-2.0-only) the one to plan on in the owner's
   jurisdiction, and is there any accepted route besides re-derivation?
2. Is a new-rule re-derivation by people and agents who read the source independent enough to drop the
   exception, and what record do you want?
3. Does an Apache-2.0 data file read at run time (b5) keep the GPL binary clean?
4. May the repository say "GPL-2.0-or-later" on files whose only partner content is MIT and still sit in
   one QEMU binary with GPL-2.0-only files? (Expected yes; confirm.)
5. Are the three small items (CAPS_ROWS region, the fixture, the NVOS46 struct) derivative at all?
6. Database-right exposure for a table of ids in jurisdictions that have one. Low, but ask.

### NEEDS-OWNER

1. Approve option (a) and the shadow census; choose (a) or (a') after the census.
2. Own the policy file: which ogkm families and categories a guest may name. This is a security decision.
3. Whether the frozen `kayfabe-abi` copy and the 11 archive files stay (as a documented, unlinked
   exception) or leave the tree (history keeps them).
4. Fix now, no counsel needed: (1) name the three extra spots in `LICENSE` exception 2 or re-derive them
   in the same PR; (2) `archive/nvkvm/src/abi/nvgpu.h` and `uvm.h` carry an in-file
   `GPL-2.0 OR Apache-2.0` line while `LICENSE` says Apache-2.0 only, and four archive test sources carry
   Apache-2.0-only SPDX lines; reconcile the text; (3) record in `THIRD_PARTY_NOTICES.md` which QEMU
   tag a binary would be built from.
5. Whether to run a dependency licence audit (Rust standard library parts, `libc`) before any binary.
6. Whether (c) stays the public posture after (a) lands, until the security audit.

## 8. How these counts were made (2026-10-31, `next` at `7fdbf7314`)

```sh
grep -rIil 'gvisor\|nvproxy' crates scripts tools qemu cuda firmware broker ci .github Cargo.toml   # 34 files
grep -rIil 'gvisor\|nvproxy' archive | wc -l            # 73
grep -rIil 'gvisor\|nvproxy' docs site README.md ARCHITECTURE.md LICENSE THIRD_PARTY_NOTICES.md AGENTS.md CLAUDE.md | wc -l   # 46
grep -rIil 'gvisor\|nvproxy' traces | wc -l             # 15
grep -rn 'capabilities()' crates/kf-*/src crates/kf-*/tests   # call sites, comment lines removed
awk 'NR>=675 && NR<=2107' crates/kf-abi/src/capability.rs   # data region; tests start at the cfg(test) line (2120)
```

The row parser reads `ControlEntry`, `ClassEntry` and `Origin::` tokens above the `cfg(test)` line of
`capability.rs`. The ogkm cross-check parses `flags`, `accessRight` and `methodId` triples from
`src/nvidia/generated/g_*_nvoc.c` (580.65.06) and the `RS_ENTRY` blocks of `resource_list.h` (595.84),
and matches by id (controls) or by NVIDIA name (classes). Both scripts are throw-away and not committed;
the generator in option (a) replaces them. The QEMU licence scan is `grep -rIl` for
`SPDX-License-Identifier: GPL-2.0-only` over `/opt/qemu-src` (9.2.0). Not run: any build, any
hardware, any Vast box.
