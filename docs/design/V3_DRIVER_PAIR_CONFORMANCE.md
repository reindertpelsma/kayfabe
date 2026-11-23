# V3 DRIVER-PAIR CONFORMANCE — does every guest driver work against every host driver?

> **STATUS: DESIGN-ONLY, 2026-11-20** (the first slice, the host axis and the guest ABI-table verdict, is code:
> `crates/kf-abi/tests/driver_pair_conformance.rs`, table `traces/driver_matrix/pair_table.tsv`; everything else below is
> design). Owner question, 2026-11-20: the guest picks its own driver (automatic by default, a CLI pin for security);
> the guest must never choose the HOST ABI; can we know, statically, that every guest driver against every host driver
> works for every message NVIDIA forwards, or that each small feature difference is resolved correctly, and can we
> report the failures?

## 1. The short answer

- The pair is **not a product of two unknowns**. The guest version only selects how kayfabe *decodes what the guest
  sends and encodes what the faked GSP answers* (`kf_abi::versions::table_for(guest)`); the host version only selects how
  kayfabe *encodes the verbs it authors for the host* (`kf_abi::hostabi::HostAbi::for_host(host)`). Both are keyed on an
  exact measured tag and meet only in kayfabe's canonical ("bench", 580.159.04) form. So the matrix is two per-axis tables
  plus a join, not N x M separate experiments. The per-axis tables are computable in about a second.
- What a static test **can** prove: the struct layout, size and field presence at every measured tag, what is zero-defaulted
  or dropped, which requests are refused and why, that a refusal is a refusal. What only hardware can prove: that a carried
  value *means* the same, that the host driver accepts the request, that a GSS-legacy id (no ogkm name) has the size the host
  firmware wants, anything the compiler cannot see.
- First slice (code): host axis, 78 controls x 30 measured host tags, plus the guest ABI-table verdict for 30 tags. Result,
  in one line per host: only the bench host (580.159.04) is `OK` for all 78; hosts below 580 refuse 21-24 controls by name;
  every other 580.x host has **20 `UNKNOWN`** cells (section 6).

## 2. What is forwarded (counted from the tree, 2026-11-20, `origin/next` `7fb1c98a0`)

The word "forwarded" is wrong for v3 and the numbers show why: the guest's RM talks to a *faked* GSP and kayfabe
**authors** the host calls (`ARCHITECTURE.md`, "Control plane: authored, never forwarded"). Three different sets:

| set | what | count | source |
|---|---|---|---|
| **A. served/probed by the faked GSP** (what the guest can ask and kayfabe knows by name) | control ids / control names / alloc-parameter structs / classes | **421** / **282** / **35** / **25** | `tools/api_boundaries/served.json` (`derive_served.py`). ⊘ A *reference* census: ids that appear as literals in `crates/kf-*` (168 code-only), in the Windows list (92 Windows-only), or both (110 + 45 + ...). It is not a statement that each is answered. |
| **A'. GSP RPC functions dispatched** | `NV_VGPU_MSG_FUNCTION_*` constants / distinct ones used by kf-rm, kf-gsp, kf-core | 19 / 17 | `crates/kf-abi/src/generated/rpc.rs`, grep |
| **B. host controls kayfabe sends the host** | `HOST_CONTROLS` | **78** = 58 measured structs (2 of them renamed by the SDK) + 16 fixed-size opaque GSS-legacy + 4 branch-sized GSS-legacy | `crates/kf-abi/src/hostabi.rs` `HOST_CONTROLS`, and the table (78 distinct control labels) |
| **C. GSS-legacy requests asked at realize** | rows in `gssreplay::ROWS` (authored request, host reply kept) | **29** `Row {` entries | `crates/kf-abi/src/gssreplay.rs` |
| **D. host verbs other than controls** | `pub fn` of `kf-host` (alloc, map, unmap, export/import fd, doorbell, ...) | 48 | `crates/kf-host/src/lib.rs` (the audit in section 3 counts the ioctl call sites) |

Only B, C and D reach the host driver. A is what the guest side must be *decoded* against, and is where the guest axis
(per guest tag) matters.

## 3. Where guest bytes could influence a host command today (adversarial reading)

Method: a read-only, grep-first reading of every site where a guest-supplied value (a length, offset, handle, version, selector,
class id, flag word, GPA) could reach a command sent to the host driver (`kf-host`, `kf-linux-raw`, `kf-chan`, `kf-mem`, `kf-rm`
hostquery and `gssreplay`, `kf-core::HostOps`), done by a separate reviewer (an Opus-tier agent, per the models-by-risk rule) and
spot-checked by me (finding 1 re-read in `chan.rs`; the rest are the reviewer's readings, marked so). Counts: 18 NVIDIA host ioctl call
sites in production `kf-host` (+2 nvidia-drm ioctls in `kf-linux-raw/src/drm.rs`), 10 `HostOps` methods (`kf-core/src/plane.rs:114`).

### 3.1 The questions asked, answered

| question | answer | where |
|---|---|---|
| Can the guest choose the HOST ABI / layout? | **No.** The host ABI is built from the host driver's own version string (`NV_ESC_CHECK_VERSION_STR` on the host `nvidiactl`), exact membership in the measured set, no nearest match, one construction site, no setter. An unreadable, unparsable or unmeasured host version, or a host where a passed-through struct differs from the bench's, is refused by name at open. | `kf-host/src/lib.rs:335-344` (`read_version`), `:224-250` (`host_abi_gate`), `:255-268` (`PASSED_THROUGH`), `hostabi.rs:189` (`for_host`) |
| Can the guest choose a host command id or `paramsSize`? | **No.** Every `raw_control` caller outside `kf-host`/`kf-harness` uses a constant command or one fixed at realize; the size comes from the host row, never from the guest (`control_plan`). `raw_control_opaque` has no caller outside the harness. The GSS-legacy bit-15 rule (`capability.rs:616`) only admits a command to the bridge; an admitted control with no model is `GspRuleControlUnserviced`, and `gssreplay::answer` serves bytes recorded at realize after an exact `params.len() == row.size` check. | `lib.rs:278-295`, `kf-rm/src/rmrpc/mod.rs:1654`, `gssreplay.rs:373,436`; tests `lib.rs:2561 the_guest_request_selects_the_control_never_the_size`, `:2719 adjacent_unmeasured_hosts_are_refused_by_name`, `:2671 every_matrix_host_gets_its_golden_outcome_for_the_five_controls`, `kf-rm/tests/gss_legacy_answer.rs`, `gss_exact_zero_rows.rs` |
| What does the guest driver version do? | It selects the guest-side layout table, which comes from the device property `guest-driver=` (default: the host's version), not from the guest. fn 1 (`rpc_set_guest_system_info`) is only *checked* against it: a mismatched or undecodable string gets `NV_ERR_NOT_SUPPORTED` and the guest never boots; the string is never parsed into a host choice. | `kf-rm/src/guestsysinfo.rs:97,120-138` |
| One guest-selectable switch | fn 1's Windows identity re-latches the Windows profile on every fn 1 (`windows-support` defaults to on): it changes the mirror-space `vaBase` (a constant) and admits a cross-client DISABLE_CHANNELS re-enable and an async preempt, both acting only on this VM's own twins, event pointer always 0. Bounded by construction, but guest-selected. | `kf-rm/src/winprofile.rs:36,94`, `chanlink.rs:273,1986-2020`, `channel.rs:2076-2104` |

Arithmetic on guest lengths and offsets: every path read uses `checked_*`, `get(..)` or `saturating_*` (`chanlink.rs:1560`,
`rmrpc/mod.rs:1622`, `gssreplay.rs:373`, `kf-mem/src/ledger.rs:432-438`); no unchecked slice indexed by a guest value was found.
Guest handle to host handle: maps are keyed `(client, handle)`, cross-client references are refused by name (ZCULL
`chanlink.rs:1667`, DISABLE_CHANNELS `:2013`, DEBUGGER `chan.rs:5676`), host handles are minted by kayfabe, never taken from the guest.

### 3.2 Proven bounded (sample; the reviewer's full table has 25 sites)

Doorbell value (`kf-trap/src/tokenindex.rs:56`, test `:437`), VA mapping and unmapping from the walk diff
(`kf-mem/src/ledger.rs:80-117,432`, `apply.rs:298,914-1092`, `channel.rs:371`; tests `channel.rs:3373 hostile_rows_are_refused_before_any_host_call`,
`lib.rs:2324 the_kernel_mapping_bit_is_cleared_whatever_the_caller_sets`), parameter-length-to-`u32` conversions (`lib.rs:1005-1281`),
the pre-compile plan (`kf-host/tests/precomp_read.rs:158 oversized_plan_never_issues_an_ioctl`). Bounded only by the host RM (kayfabe
validates the shape, not the value): `CTXSW_PREEMPTION` `flags/gfxp/cilp` (`chanlink.rs:1600-1640`), `SET_TIMESLICE` `us` (`:1712`: policy
question, a guest can lengthen the timeslice of its own groups), exception mask, context-DMA notifier offset, PRAMIN view offset.

### 3.3 FINDINGS: not provably bounded (reported loudly; none is a malformed or guest-chosen host command)

All four are about *how many* objects or *where*, not about a guest-chosen command, ABI or size. Nothing below was run; each is
a reading, and no regression test exists yet.

1. **GT200_DEBUGGER allocations are uncapped** (`kf-qemu/src/chan.rs:5668-5727`, re-read by me). Checks: `app_client == client` and the
   object must be this client's GR twin object. No busy-handle check, no per-VM cap; `m.insert((client, handle), ..)` silently replaces
   an entry and the host `0x83de` object it replaced is freed only at session end. `ACT_QUEUE_CAP` (4096, `chan.rs:137`) bounds
   in-flight deferred acts, not the total. A hostile guest kernel loops fn ALLOC 0x83de over its own 3D object: one host RM allocation
   per send, host memory grows for the VM's lifetime. Unverified: whether the rmgraph object seat refuses a duplicate guest handle before
   chanlink runs (distinct handles are unbounded either way). *Proposed test:* `chan::tests::debugger_alloc_is_capped_and_a_busy_handle_is_refused`
   over a recording host: the (cap+1)th alloc and a repeated `(client, handle)` are refused before any host call, `dbg.len() <= cap`.
2. **Engine objects per twin have no committed cap** (`chan.rs:2788-2855` `admit_engine_object`). Pending objects are capped at 64
   (`PENDING_ENGINE_OBJECTS`) and busy handles refused (`:2797-2803`), but committed `v.objects` is not bounded: one host alloc per fresh
   handle. *Test:* `engine_objects_per_twin_are_capped`.
3. **Host-handle counter wraps without a check** (`kf-host/src/lib.rs:1149-1155` `mint`, low severity). `wrapping_add(1)` from
   `0xCAFE0001`; after about 889 million mints it wraps through 0 to `0xCAFE0000`, kayfabe's own client handle. RM rejects a *live* handle,
   but a stale handle kayfabe still holds (a deferred act's) could then name a newer object. *Test:*
   `mint_refuses_at_exhaustion_and_never_reissues_a_remembered_handle` (set `next` near `u32::MAX`; assert an error, or that no value in
   `parents` is ever returned).
4. **ZCULL_BIND takes a guest 64-bit VA with no overlap check against kayfabe's own placements** (`kf-rm/src/chanlink.rs:1685-1747` to
   `kf-host/src/channel.rs:1948`, low severity). Checked: size 24, `w(0) == h.client`, `mode <= ZCULL_MODE_MAX`. Not checked: overlap with
   the twin's reserved set (`MapTarget::reserved`, `kf-mem/src/ledger.rs:307`), e.g. the read-only USERD page
   (`kf-chan/src/user_storage.rs:371`). Whether a context-switch write then obeys the read-only PTE is unmeasured. *Test:*
   `zcull_bind_refuses_a_va_inside_a_vmm_placement`.

Also flagged, not a host-command finding: the PRAMIN re-point runs `NV_ESC_RM_MAP_MEMORY` from the trap
(`kf-mem/src/cpuwin.rs:944` to `kf-qemu/src/mem.rs:1181/1202`, `arm_store_in_trap`): an ioctl on the vCPU path. The reviewer did not
check it against the "nothing blocks on a vCPU" rule; it needs a look by whoever owns that rule.

### 3.4 Observation that bears on the pair table

`HOST_CONTROLS` is an inventory, **not an allowlist**. For a control with no row, `control_carry` returns pass-through at the bench host and
at every host inside `[580.65.06, 581)` (`hostabi.rs:1232`), and `UnlistedControl` only outside it. So a control that code starts sending
without adding a row is sent unchecked to all nine measured 580.x hosts (the bench and eight others). The table therefore says nothing about such a control;
the guard is a static test: *every command constant passed to `raw_control` in `crates/kf-*` is a `HOST_CONTROLS` row* (the reviewer
did not enumerate this set by hand, it is the cheap next test, and it makes the "78" a closed number).

### 3.5 Not finished by the reviewer

Whether rmgraph refuses duplicate guest allocation handles before chanlink; the bound inside the PRAMIN slot walker; every
`apply_register` arm in `kf-qemu/src/device.rs`; the internals of `export_object_to_fd` / `import_object_from_fd` (`lib.rs:1807/1837`,
the second has no caller outside `kf-host`); the kf-harness probe bodies (operator tools).

## 4. Which ogkm tags exist and what is usable as an oracle

- Upstream `NVIDIA/open-gpu-kernel-modules` has **227 tags** (checked with `git ls-remote --tags`, 2026-11-20; 515.43.04
  ... 620.73.04). **30 are measured** (`traces/driver_matrix/ranges.tsv`, `tools/drivermatrix/tags.txt`), 197 are not.
  Measured 580.x tags are 9 of the 27 upstream 580.x tags; a host on any other 580.x release (580.142, 580.167.08, ...) is
  `HostAbiError::Unmeasured`, refused by name before any verb (`hostabi.rs` `for_host`). Newer than any measured tag:
  615.78.08, 620.70.06, 620.73.04.
- Draft PR #351 adds 15 tags (30 -> 45: 535.54.03, 535.86.05, 550.40.53, 545.23.06, 545.29.02, 550.163.01, 535.261.03,
  570.211.01, 570.195.03, 570.172.08, 580.159.03, 580.196.07, 595.58.03, 595.71.05, 595.112.07); it is stale against
  `next` (1849 files differ, mostly deletions), so it needs a re-measure, not a merge: `regen.sh <tags>` is about 2 minutes per
  tag. 595.91.07 is already on `next` through #522.
- **Offline oracle = the committed measurement.** `ranges.tsv` (7,614 rows: 338 `sizeof`, 3,644 `field`, 3,630 `value`)
  is what `dm.py` compiled out of ogkm at each tag (`gcc -g` with the tag's own Makefile flags, layouts read from DWARF). The
  slice reads only that, so it needs no tree, no network and no compiler.
- **Source trees** (to re-derive or to read code): `third_party/ogkm` (gitlink `57130a27`, 610.x), `third_party/ogkm-580`
  (`b81d58ee`, 580.159.04) are submodules that are not initialised by default, and `ogkm-595` (`722ae845`, tag 595.84) is
  pinned only in `scripts/ci/fetch_oracles.sh`. Those three are anchored by **commit id**.
- ⚠ **Anchor gap (finding, not a bug in the table).** The other 27 measured tags are fetched by `dm.py fetch` with
  `git clone --branch <tag>`: anchored by tag *name*, and `ranges.tsv` does not record the commit each tag resolved to.
  A moved tag would change a measurement with no diff to show it. Per the standalone trust-anchor rule (only an
  official-origin checksum or a GitHub attestation counts; a pin we typed does not) the proposed fix is to record the
  resolved commit id of each tag in `tags.txt` at measurement time and have `dm.py fetch` fail on a mismatch: git verifies the
  object ids against the official origin on fetch, so the commit id is an official-origin anchor in the same way as the three
  pins above. Needs network, no GPU. Not done in this slice.
- **Not oracle-able from ogkm:** GSS-legacy control ids (bit 15 set: no ogkm header names them; the GSP firmware checks the size,
  ogkm only forwards its status). `OWNER_RULINGS` section AH tier order applies: tier 1 (ogkm, any version, generated or Windows code)
  is exhausted for them, tier 2 (an unprivileged host ioctl) is what `hostabi.rs` already cites per row, one exact tag at a time.

## 5. What a static conversion test proves, and what it cannot

| fact | static (this table) | needs hardware |
|---|---|---|
| a struct's `sizeof` and every consumed field's offset at a tag | yes, from the compiled ogkm layout | no |
| a field exists or not at a tag; defaulted (zero) or dropped | yes: `DEFAULTED(+f)` / `ABSENT-ON-HOST -f` | whether zero is the *right* default for the consumer |
| a request is expressible at a host (no data lost) | yes, zero request always; the all-ones probe gives the worst case | the real authored request never hitting the worst case |
| arrays that shrink or grow between tags | yes: `REFUSED-IF-FULL` names the smaller capacity | the real list length |
| the control id is the SDK's for that name at every tag | yes (`every_host_control_id_is_the_measured_one`, existing) | no |
| refusal reasons are named and never silent | yes: `REFUSED(NoHeader / Layout ...)`, pinned against the production carry | no |
| a GSS-legacy size at a host tag | **no**: `UNKNOWN` unless an exact tag was probed | an unprivileged probe on that tag (tier 2) |
| the host driver accepts what we send; the guest accepts what we answer | no | yes (gates, fast suite, ladders) |
| a guest field derived from a host fact has the same meaning across the pair | no | yes |

⊘ The layouts cover the fields kayfabe **consumes** (the spec), not every field of a struct: "field presence" is the
consumed set, and the table says nothing about fields kayfabe never reads.

## 6. The matrix, its size, and the first slice

Shape: `guest tag (30) x host tag (30) x control`. Because the axes are independent and meet only in the canonical form,
the verdict of a pair is the worse of its two per-axis verdicts for the controls that join them, so the table stores
the axes (30 guest + 30 x 78 host cells = 2,370 cells, stored run-length coded: **398 lines, 48 KB**) and the product is
derived (30 x 30 = 900 pairs; with the 421 served ids about 379,000 cells). Runtime of the slice: the whole test file is
**about 1 second** after the 17 s first build of `kf-abi`.

Verdict vocabulary: `OK`, `OK(moved ..)`, `DEFAULTED(+f)`, `CONDITIONAL(ABSENT-ON-HOST -f; REFUSED-IF-FULL a)`,
`REFUSED(reason)`, `UNKNOWN(reason)`, defined in the test's module comment.

Measured by the slice (host axis; counts of the 78 controls per host; `summary` rows of the table):

| host tags | OK | DEFAULTED | CONDITIONAL | UNKNOWN | REFUSED |
|---|---|---|---|---|---|
| 535.309.01, 545.23.08 | 45, 46 | 0 | 9, 8 | 0 | 24 |
| 550.40.07 ... 575.64.05 (13 tags) | 48-52 | 0 | 5-9 | 0 | 21 |
| 580.65.06, 580.82.07 | 57 | 0 | 1 | **20** | 0 |
| 580.94.02 ... 580.178.04 except 580.159.04 (6 tags) | 58 | 0 | 0 | **20** | 0 |
| **580.159.04 (bench)** | 78 | 0 | 0 | 0 | 0 |
| 590.48.01 | 57 | 0 | 2 | 0 | 19 |
| 595.84 | 61 | 0 | 3 | 0 | 14 |
| 595.91.07 | 75 | 0 | 3 | 0 | 0 |
| 610.43.02, 610.57.04 | 54 | 1 | 4 | 0 | 19 |
| 615.71.09 | 52 | 2 | 5 | 0 | 19 |

Reading it:

1. **Every host outside 580.x except 595.91.07 refuses GSS-legacy rows.** Hosts below 580: 20 `NoHeader` refusals (no layout
   measured for that host) plus 1 `Layout` refusal (4 at 535.309.01 and 545.23.08: a struct that does not exist at that tag);
   590.48.01, 610.43.02, 610.57.04, 615.71.09: 19 `NoHeader`; 595.84: 14. They are the design working: a host that cannot be asked
   is not asked, by name. They are also the exact list of "this pair does not work" for the controls concerned.
2. **The 20 `UNKNOWN` cells at every non-bench 580.x host are the loudest result.** Inside `[580.65.06, 581)` the 16 opaque
   and 4 branch-sized GSS-legacy rows pass through *at the 580.159.04 size*. That is a contract measured on one host tag; nine
   580.x tags are matrix tags and nothing but the bench's probe stands behind the other eight. No ogkm tag can settle it (tier 1
   exhausted); a tier-2 probe per tag can. Proposed: run the existing unprivileged `kf-ctl-probe KF_PROBE_GSS_ROWS=1` once per 580.x
   tag the project claims to support and move each tag's row to `versions` (`MeasuredOpaque`), which turns `UNKNOWN` into `OK`
   with a trace path. This is the cheapest high-value hardware step.
3. **595.84 refuses 14 rows that 595.91.07 passes**: the `MeasuredOpaque` rows list only 595.91.07 (the 595 branch profile
   `GSP_BRANCH_595`, which names both tags, covers only the `BranchSized` rows). A tier-2 probe at 595.84 would close it. Fail-closed:
   a coverage gap, not a safety issue.
4. **`ABSENT-ON-HOST` is a silent default on the consuming side.** At <= 570 hosts, `NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER`
   has no `physicalCpcId`, `ugpuId`, `virtualTpcId` (575: no `physicalCpcId`, `virtualTpcId`): the carried reply reads 0 for them,
   by design (`from_host` keeps absent fields zero). Whether a zero is acceptable to whatever consumes that reply is a semantic
   question the table cannot answer; it is the list of places to look. Same for `GPFIFO_SCHEDULE.bSkipEnable` (<= 570),
   `BUS_GET_C2C_INFO.bLinkInHS` (<= 570) and the `DEFAULTED` fields at 610+ (`BUS_GET_C2C_INFO.bEncryptionEnabled`,
   `GR_GET_GLOBAL_SM_ORDER.virtualDpcId`, and and, not a control row but the same carry, `NV_CHANNEL_ALLOC_PARAMS.hHandleVASpace` at 610 (pinned by `a_field_only_the_host_has_is_defaulted`).
5. **Guest axis:** 29 of 30 tags get an ABI table; **615.71.09 is refused** by name (`GSP_MSG_QUEUE_ELEMENT` has a third
   shape, the encryption union, that no encoder speaks; `V3_DRIVER_MATRIX.md` section 6). "OK" there means the table builds
   and the layouts the encoders were written for are present, **not** that every RPC reply encodes correctly at that tag.

Reports. `pair_table.tsv` is committed and the test `the_committed_pair_table_is_what_the_code_says_now` fails when it is
stale, so a changed ABI table is a red CI diff in which each changed line is a pair that got better or worse (it runs in
`cargo test -p kf-abi`, already in CI; `KF_REGEN_PAIR_TABLE=1 cargo test -p kf-abi --test driver_pair_conformance`
regenerates it). The file lives beside `ranges.tsv` under `traces/driver_matrix/` rather than `docs/` because the test reads it
at compile time and `.dockerignore` excludes `/docs`.

## 7. Next steps, in order

1. (Done, this change.) Host axis and guest ABI-table verdict as above.
2. Guest axis per control: for each guest tag and each id of set A, the decoded layout size (`control_params`), the
   capability verdict (`capabilities_for`), and the transcode of the served reply to that tag's layout (`rpc_set_guest_system_info_v`,
   GSP static info). Needs the guest-control-to-host-control join (which host verbs feed which guest answer) written down once
   (`kf-rm` `hostquery`); that table is the only hand-maintained part, and a test can check it is total over set B.
3. The `UNKNOWN` closure of point 2 above (hardware, unprivileged, no guest).
4. Commit-pin the 27 tag-name-anchored tags (section 4), re-measure the 15 tags of #351 on top of `next`.
5. Tags newer than 615.71.09 (615.78.08, 620.70.06, 620.73.04) before they reach users: `regen.sh` and read the diff.
6. A guest-side adversarial fuzz of the decoders is separate work (section 3 lists the regression tests proposed).
