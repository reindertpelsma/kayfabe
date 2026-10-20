STATUS: PARTIAL / VALIDATION STOPPED, 2026-10-20.

CODE READ update: both coordinator choices below are APPROVED: explicit
`guest-os=windows|linux`, default Linux, and a separately reviewed sweep-derived
caps replacement. The old guest-OS decision stop is superseded. No property
implementation is accepted yet. C6 stopped on its mutation-wrapper rejection;
see [the current report](../../traces/beta_candidate_20261020/README.md).
Complete every-reader inventory, KEEP-default audit and final migration table
remain outstanding. No flags have been removed from the published product.

# Beta flag policy: guest-OS selection decision

STATUS: IMPLEMENTATION AUTHORIZED, 2026-10-20. CODE READ: the coordinator, with
the owner's delegated decision for tonight, approved explicit
`guest-os=windows|linux`, default `linux`, as the single replacement of the
nineteen Windows environment selectors. Preserve Linux behaviour; keep
non-derived Windows behaviour behind the property; do not hardwire the captured
probe set. Continue tests-first GPU-free Stages 2–4; hardware belongs to the
night reproduction lane and remains deferred to Stage 5. This supersedes the
stop below. The original decision evidence remains the construction rationale.

STATUS: RESEARCH / STOPPED BEFORE IMPLEMENTATION, 2026-10-19. CODE READ at
candidate-5 `f8f9d1a5f`. BETA CANDIDATE; no supported external-user install path
is offered. This is a partial policy and decision record, **not** the completed
Stage 2 inventory, an implemented configuration API, or a no-flag boot claim.

## Stop and falsifier

INFERRED question registered before reading the flag paths: can guest facts
already available in the device select the Windows behaviour while preserving
the Linux default? FALSIFIER registered before that read: any setting must be
chosen before a reliable guest-OS fact is available. The Stage 2 item 2 stop
applies when that choice cannot be derived with confidence.

CODE READ result: the falsifier fires for the current construction path.
Windows identity exists later; it is not currently a complete, early, per-device
configuration authority. The missing fact is not evidence that Windows is
intrinsically undetectable, and it is not an invitation to infer OS from a PID,
USERD layout or channel kind. No hardwiring or property change was made.

## Evidence for the decision

| CODE READ source at candidate-5 | Finding | Implication |
|---|---|---|
| `crates/kf-qemu/src/device.rs:367`, `:924-940`; `crates/kf-qemu/src/display.rs:2146-2210` | `Device::realize` calls `DisplayPlane::build` and selects SDR processing and the caps page from environment before guest RPC contact | A flag-to-fn1 substitution is insufficient for these construction-time settings. No current fn1 callback rebuilds the display plane. |
| `crates/kf-rm/src/lib.rs:331-379`; `crates/kf-abi/src/guestsysinfo.rs:200-252` | Fn1 has a strict reported-version/Windows-twin decoder; matching a Linux ABI version returns `twin=None`, and a known Windows twin can select the same Linux ABI key | There is a declared Windows identity, but version-key equality does not carry a complete OS/profile decision or rebuild the display. `guest-driver=` is an ABI selection, not an OS selector. |
| `crates/kf-gsp/src/sysinfo.rs:12-18`, `:44-90` | Fn72 precedes fn1. `bGspNocatEnabled` is logged at the provisional layout; the code explicitly says it is never decided on, and layout/length disagreement is reported rather than resolved | The earlier hint does not establish safe automatic selection for every supported version or construction-time setting. A proof must account for provisional-layout ambiguity. |
| `crates/kf-rm/src/chanlink.rs:914-943` | `ChannelPolicy::new` snapshots `KF3_WIN_USER_CHANNELS_PASSTHROUGH`; Windows identity starts false | The existing later identity check does not automatically enable the user-work route when no flag is set. Both the feature selection and channel-admission facts matter. |
| `crates/kf-qemu/src/chan.rs:1733-1754` | Kernel-GR CE/work selectors default off. The source explains that enabling them changes a kernel channel created by Linux | Globally enabling the Windows profile violates the instruction to retain Linux default behaviour. |
| `crates/kf-qemu/src/device.rs:968-975`, `:999-1005`; `crates/kf-abi/src/guest_os.rs:250-264` | The live object/memory recipe supplies `GuestOs::Linux`. The separate `GuestOs::Windows.client_kind_rule()` returns `None` | Adding an OS property cannot safely be implemented by globally swapping this pre-existing enum. Behaviour selection and existing ABI/privilege interpretation must be reviewed separately. |
| `qemu/hw/misc/kf3/kf3.c:1936-1985` | QOM exposes `guest-driver` and display choices, but no `guest-os` property | The recommended property below is proposed, not a command that works on this branch. |
| `crates/kf-qemu/src/display.rs:146-157`, `:2189-2208`; [T-space policy](V3_TSPACE_HARDWIRED.md) | The P19 caps selector is explicitly a captured-page probe that may not ship as written | Choosing the OS source does not resolve the independent requirement to replace captured/experimental behaviour. Keep the binding provenance rule; do not promote the probe merely by deleting its environment read. |

INFERRED: confidence is insufficient for automatic selection of the complete P19
profile. This is a concrete construction/identity gap, not a measured failed boot.
No GPU, guest or host observation has been made by C6.

## Options for the owner/coordinator

| Option | Contract | Risk / proof required | Recommendation |
|---|---|---|---|
| INFERRED A: one explicit device property, proposed `guest-os=windows\|linux`, default `linux` | Linux follows existing no-env settings; Windows selects an audited per-device behaviour profile before display/policy construction. The future launcher maps `--os windows` to this property and sets zero `KF3_*` environment flags | Requires plumbing through QOM/FFI, construction and the current process-global `OnceLock` readers. Preserve current `GuestOs` ABI interpretation until separately proven; reject invalid names before effects. Retain hostile-guest admission/permissions even when a guest declares Windows | **Recommend A.** It is the conservative fallback allowed by Stage 2 item 2. Await the requested written choice before implementation. |
| INFERRED B: automatic profile from fn1 or validated fn72, with explicit override | No OS property needed for a recognised stock guest | Must prove early identity/layout correctness, all pre-fn1 observable state, safe publication before consumption, unknown/conflicting identity behaviour, life/reset transitions and multi-device separation. No vCPU wait or timer/poll workaround. The current code does not provide that proof | Defer until proof exists; do not silently ship an `auto` heuristic. |
| CODE READ / INFERRED C: enable P19 globally | Both OSes receive Windows settings | Changes Linux kernel-GR and other defaults; contradicts Stage 2 item 2 and the existing Linux privilege/mapping restrictions | Reject under the current instruction. |

CODE READ: the owner already authorized replacing redundant behaviour flags and
permits a single explicit Windows property. This stop asks for the exact profile
selection decision mandated by the lane, not generic permission to edit files.
The existing captured-page prohibition is binding and is not put to a new vote.
The deferred sweep's derived-page replacement needs scoped review and hardware
acceptance; full sweep inclusion is a separate ledger decision.

## Partial inventory: historical P19 migration inputs

CODE READ: this list is the exact nineteen `=1` names in the sweep source
`codex/sweep-integrate-20261018` at `b6accbca2`,
`traces/sweep_integrate_20261018/MATRIX.md` (the named historical P19 profile).
It is not inferred from the older, different generic Windows launcher. The
current launcher `scripts/bench/windows/windows_broker.sh:284` still mixes
measurement names and obsolete `KF3_TIMER_MAP`; it is not the P19 authority.

CODE READ: every selector below is OFF when unset in candidate-5. All nineteen
historical requests are ON. These are **selector defaults**, not a claim that
every path is effective: driver/cell/route gates still apply; the host-owned
runlist policy precedes the older runlist probe. No single effective-config
struct or table-driven P19 equivalence test was implemented in this stopped
work. All proposed migration verdicts await the OS decision and path review.

| Name (CODE READ) | Class in this partial audit | Reader | No-env selector / historical P19 | Migration status |
|---|---|---|---|---|
| `KF3_WIN_USER_CHANNELS_PASSTHROUGH` | Windows behaviour | `kf-rm/src/chanlink.rs`; `kf-host/src/channel.rs` | OFF / ON | Pending per-device Windows profile; preserve Linux native route. |
| `KF3_WIN_TWIN_DEFAPI_OBJECT` | Windows behaviour | `kf-qemu/src/chan.rs::twin_defapi_object` | OFF / ON | Pending profile and existing user-work/identity admission. |
| `KF3_KERNEL_GR_CE` | OS-sensitive behaviour | `kf-qemu/src/chan.rs::kernel_gr_ce` | OFF / ON | Pending; never turn on globally for Linux. |
| `KF3_KERNEL_GR_WORK` | OS-sensitive behaviour | `kf-qemu/src/chan.rs::kernel_gr_work` | OFF / ON | Pending; paired with kernel GR/CE, allowlists and USER assertion retained. |
| `KF3_TRANSLATED_CE_RELAY` | Completion behaviour | `kf-qemu/src/chan.rs::translated_ce_relay` | OFF / ON | Pending; idle-only ablation is not load evidence. |
| `KF3_DEFERRED_API` | Translated software-method behaviour | `kf-qemu/src/chan.rs::deferred_api_trigger` | OFF / ON | Pending; retain owner U's admission and ordering contract. |
| `KF3_PREEMPT_BIND_PROBE` | Behaviour despite probe name | `kf-rm/src/preempt_bind_probe.rs` | OFF / ON | Review stub contract before promotion; no measurement exemption. |
| `KF3_DISPLAY_IMP_ENABLE` | Display behaviour | `kf-rm/src/display.rs` | OFF / ON | Pending profile and supported display cell. |
| `KF3_DISPLAY_CTRL_PROBE` | Behaviour despite probe name | `kf-rm/src/display_ctrl_probe.rs` | OFF / ON | Review authored/public reply contract; not blanket hardwire permission. |
| `KF3_ZCULL_BIND_PROBE` | Behaviour despite probe name | `kf-rm/src/preempt_bind_probe.rs` | OFF / ON | Review current cross-client stub contract. |
| `KF3_SW_RUNLIST_HOST_OWNED` | Scheduling behaviour | `kf-rm/src/sw_runlist_host.rs`; `kf-qemu/src/chan.rs` | OFF / ON | Pending; retain source warning and owner W decision record. |
| `KF3_ASYNC_PREEMPT` | Completion/recovery behaviour | `kf-rm/src/chanlink.rs::async_preempt_enabled` | OFF / ON | Pending; idle-only ablation is not recovery/load evidence. |
| `KF3_GFX_POOL_PROBE` | Behaviour despite probe name | `kf-rm/src/lib.rs`; `gfxpool_probe.rs` | OFF / ON | Review fabricated sizing/lifecycle contract before promotion. |
| `KF3_SW_RUNLIST_PROBE` | Behaviour despite probe name | `kf-rm/src/lib.rs`; `sw_runlist_probe.rs` | OFF / ON | Check actual shadowing/redundancy against host-owned runlist first. |
| `KF3_MEMORY_LIST_PROBE` | Registration behaviour | `kf-rm/src/lib.rs`; `rmrpc/policy.rs` | OFF / ON | Pending; retain bounds and ownership validation. |
| `KF3_DISPLAY_SDR_COLOR` | Display behaviour | `kf-qemu/src/display.rs::DisplayPlane::build` | OFF / ON | Requires a construction-time Windows choice or proven safe reconfiguration. |
| `KF3_DISPLAY_CAPS_PROBE` | Captured-page diagnostic behaviour, prohibited as shipped product | `kf-qemu/src/display.rs::caps_probe_on` | OFF / ON | Do not hardwire captured page; select/review derived replacement separately. |
| `KF3_DISPLAY_LUT_MIRROR` | Display behaviour | `kf-qemu/src/display.rs::color_experiments` | OFF / ON | Pending, coupled to SDR processing. |
| `KF3_DISPLAY_ILUT_OFFSET_256` | Display behaviour | `kf-qemu/src/display.rs::color_experiments` | OFF / ON | Pending, coupled to SDR processing. |

CODE READ evidence limits: [flag inventory](V3_FLAG_INVENTORY.md) and
[ablation record](../../traces/windows_flag_ablation_20261010/README.md) distinguish
measured boot requirements from guesses. The latter uses `89721ca4`, six runs,
idle desktop plus Edge for 180 seconds. It is not candidate-5 load/aging
certification. In particular, dropping ASYNC_PREEMPT or TRANSLATED_CE_RELAY
being clean there does not justify deleting those completion paths.

## Defaults and rules that must survive the resume

CODE READ: `KF3_TSG_MIRROR` is absent from the candidate-5 product; no mirror is
imported. `KF3_TIMER_MAP` has no reader in the audited product source roots:
validated native timer mapping is already automatic. Existing T-space and
kernel-video hardwiring is documented in [V3_TSPACE_HARDWIRED.md](V3_TSPACE_HARDWIRED.md).

CODE READ: production lifecycle is `default`; PUT cache and lazy observe default
ON, while the optional lifecycle measurement mode is inactive when unset.
Preserve these production behaviours and their owner-retained controls. An
unset measurement control must not be confused with disabling its underlying
production optimisation. `KF3_DIAG_STEP_METERS` and `KF3_DIAG_EVENT_RING` are
optional/off and never boot prerequisites. Owner AB protects measurement,
diagnostic and negative-control names; no rename was requested.

MEASURED: C6 changed zero flag readers/defaults/launcher exports. Scoped P19
names before/after are 19/19; hardwired 0; deleted 0. The total number of
`KF3_*` reads across all required roots and launchers, their complete
classifications/KEEP-default audit and effective config comparison are
**NOT DONE**, rather than assigned misleading lexical-grep totals.

CODE READ: remaining work after a written choice is the complete inventory,
config design, tests-only EXPECTED RED commit, hardwiring fix, ≥2 killing tests
per mutant, full per-crate count retention, launcher migration and all Stages
3–4 install/preflight/build checks. No continuation or hardware slot is implied
by this decision document.
