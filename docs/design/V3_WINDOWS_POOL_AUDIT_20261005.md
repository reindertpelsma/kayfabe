# Independent review of the Windows pool experiment

**STATUS: RESEARCH, 2026-10-05.** Read-only delta audit of
`c50fad9a..b5e511b02c0288495e88e06c6368b74f75512959` on
`codex/windows-pool-2026-10-05`. This is not a security certification, merge
approval, or a hardware result. No GPU or rental was used by this review.

**Corrections applied at `535e7df9`, 2026-10-05:** exact measured query layouts
replace the one-version gate; the timer class ID comes from the compiled matrix;
policy tests exercise both 24-byte and 40-byte control envelopes across all 29
admitted guest ABIs. The research script and its design now explicitly qualify
the regex source check as textual evidence, not semantic proof. The ABI/RM/chip
suite passed 1,187 tests with zero failures. These changes resolve the two
compatibility corrections and document the audit-method deviation below; they
do not resolve the isolation blocker, pool semantics, or timer mapping.

The isolation fix is on `v3-p1p2` at `31b64802`, not current master
`906a76a4`. It still requires integration and hardware validation. The Windows
experiment also lacks master's USER-channel correction (`76dba5bd`), so its
eventual integration must start from the current base plus the full isolation
fix, rather than assuming that the old Windows base provides those guarantees.

The local, uncommitted replacement of the pool query's 580.65.06-only gate with
an exact measured-layout check was also read. Findings below distinguish the
committed baseline from that proposed correction. Subsequent changes need their
own validation; line references describe the audited baseline.

## Findings and disposition

| Priority | Finding | Scope | Required disposition |
|---|---|---|---|
| Blocker for product integration | Every mirrored address space still receives the framebuffer and guest-RAM identity windows | Existing base, S1-21; not introduced by this delta | Integrate Windows work onto the branch that fixes private Translated spaces before claiming hostile-guest isolation |
| Blocker for enabling pool support by default | Successful QUERY_SIZE returns invented geometry without a pool lifecycle or a validated replacement for pooled preemption | Deliberately incomplete experiment | Keep host opt-in and research status; prove guest userspace behavior and actual host preemption semantics before enabling |
| Correctness prerequisite for timer mapping | Timer allocation success does not create a running PTIMER view | New allocation plus an existing missing mapping | Do not publish TIMER regBases until a real, safe register backing exists |
| Compatibility correction | Committed query uses a one-version equality gate; the timer class ID is a hand-written constant despite an existing generated value | New usage | Use compiler-derived layout/profile data and the generated class constant; preserve unknown-version refusal |
| Audit-method correction | Timer source checker parses C with regular expressions and string splitting | Research tool | Replace with a compiler/parser-based check or clearly mark as a textual spot-check, not semantic proof |
| Protocol incompleteness | Timer parent type and single-instance rule are not enforced | New inert class in existing graph semantics | Document now; enforce before any timer operation can act on backing or schedule an event |
| Reliability follow-up | Every query prints one diagnostic line | New diagnostic surface, existing S1-86 class of issue | Rate-limit or count/summarize before general deployment |

### Existing isolation blocker

`crates/kf-qemu/src/mem.rs:1786` calls `map_window` for both store and guest RAM
in every mirrored space, before any use is classified. This contradicts
`OWNER_RULINGS.md` section Q: an unprivileged passthrough space may contain only
the mappings derived from that guest process's page tables. The existing
`V3_SECURITY_MODEL.md:66` already calls this S1-21 and records additional
unresolved audit items. This review does not re-audit or clear those items.

The narrow Windows delta adds no new such mapping. Nevertheless, saying the
whole experimental branch is secure would be inaccurate. Passing a Windows
boot experiment cannot clear the underlying isolation finding.

### Pool query: bounded bytes, unfinished semantics

The query policy (`crates/kf-rm/src/gfxpool_probe.rs:16`) decodes the configured
guest-driver envelope, rejects serialized parameters, uses checked addition and
a checked slice for the declared parameter extent, then requires exactly 40
parameter bytes. Its encoder (`crates/kf-abi/src/gfxpool.rs:20`) requires
`1 <= maxSlots <= 4096`, emits a newly zeroed 40-byte output, and preserves only
the input slot count. Multiplication uses u64 and the maximum advertised pool
size is 16 MiB. No guest output poison becomes a host address.

The successful reply clones guest bytes and replaces the status and parameter
outputs. It does not read a VMM address, issue a host RM verb, submit GPU work,
change a mapping, or forge a GPU completion. The transport's large-message
assembler separately limits joined messages to 1 MiB and 64 continuations
(`crates/kf-gsp/src/large.rs:38`). The policy is inserted only by the host's
`KF3_GFX_POOL_PROBE=1` environment (`crates/kf-rm/src/lib.rs:456`).

The fixed 4-KiB stride/control size is a **virtual experiment policy**, not an
OGKM-derived physical formula. It has no GPU family or die dependency, but that
does not prove the Windows driver will accept its later consequences. No query
reserves storage or checks framebuffer availability. That is acceptable for a
size query; initialization must later validate actual object ownership, extent,
alignment, lifetime, quotas, and every address it will use. A successful sizing
reply alone does not establish those properties.

The query is currently stateless and runs before object validation. It can
answer a nonexistent client/object with the same deterministic dimensions. This
does not expose a host secret or mutate another namespace. The same placement
would be wrong for initialization, add/remove, binding, or an operation that
references live objects. Those operations need the object graph's current
namespace and incarnation identity, not merely a reused numeric handle.

Keeping this as a diagnostic is justified: it isolates a boot barrier without
passing kernel-only guest control bytes to host RM or importing captured per-die
sizes. Promoting it as implemented pooled preemption would not be justified.
Returning success to later actions without executing their observable effect
would violate owner rulings A.3 and H. Unsupported actions must remain refused.

### Timer: what the new allocation actually authorizes

The new capability entry (`crates/kf-abi/src/capability.rs:902`), decoder shape
(`crates/kf-abi/src/versions.rs:1586`), and classifier
(`crates/kf-chip/src/classes.rs:134`) admit class 4 as `NoDeclaredFacts`/`Other`.
They do not add an engine class, host twin, timer control allowlist, mapping,
alarm, or event completion. Ordinary graph guards still reject undeclared and
reserved namespaces, conflicting live handles, and capacity overflow; live
handles and parked facts are each bounded to 2^18 (`rmgraph.rs:805`, `:1491`,
`:1607`). Existing free/reuse behavior is exercised in the new RPC tests.

OGKM's TimerApi constructor returns NV_OK and its own destructor is empty. Its
resource entry also specifies a Subdevice parent and one instance per parent;
the first two observations do not erase those resource-server constraints or
the behavior of inherited classes. The experiment does not reproduce those
parent/singleton restrictions. With the new class inert, this is a protocol
fidelity gap, not evidence of a host-memory escape. Before acting on a timer,
validate its live origin, parent Subdevice, namespace, and lifecycle.

`tmrapiGetRegBaseOffsetAndSize_IMPL` independently asks for `NV_REG_BASE_TIMER`.
The current omitted register-base row becomes `NV_ERR_NOT_SUPPORTED` in
`gpuGetRegBaseOffset_FWCLIENT`; accepting allocation cannot fix that subsequent
failure. `kf3.c:245` maps live reads only from the host usermode aperture; it
does not currently back the separate PTIMER page. A new timer mapping must use
source/host-derived family facts, trap/drop writes, and audit every register
in the mapped page for read side effects. A read-only mapping alone does not
establish that arbitrary register reads are harmless. Publishing an address
over a static shadow would create a stopped clock and possibly endless waits.

### Driver, family, and die axes

The proposed pool layout guard checks total size plus the offset, width, and
non-array shape of all six fields using the exact measured driver tag. This is
the correct replacement for the one-version branch. It preserves unknown-tag
refusal and does not couple guest and host driver versions. Add policy-level
tests for both pre-575's 24-byte envelope and later 40-byte envelopes; the
committed policy fixture only exercises 580.65.06.

Use `generated::matrix::CLASS_IDS_NV01_TIMER.everywhere_u32()` instead of the
hand-written class value in `submit.rs:2149`; the compiled matrix already has
the constant. This turns future class-ID variation into an explicit build
failure instead of silently retaining 4. Likewise, generated pool command IDs
and consumed field layouts should remain part of regeneration.

Adding a measured 595.91.07 matrix row preserves fail-closed ABI admission; it
does not certify every host control or a usable Windows twin. The committed
Windows-twin metadata has different Linux and Windows changelists and must not
be treated as a proven equivalent firmware/protocol pair.

The 30-tag timer source table is useful corroboration across drivers. Its
checker (`scripts/bench/windows/audit_timer_alloc.py:28`) uses textual patterns,
contrary to the architecture's compiler/parser rule. It cannot reason about
preprocessing, inherited constructors, or alternate conditional definitions.
The manual source reading and the exact reviewed revisions should remain
explicit until a compiler-based check replaces it.

## Evidence and limits

Existing evidence in `traces/windows_pool_20261005/` shows A refused the query,
B accepted it and reached more initialization, and C accepted timer allocation
but still ended with Code 43. These are useful boot experiments; none is a
successful Windows GPU workload or preemption-under-contention result.

The reported 1,185 ABI/RM/chip tests were not rerun by this read-only audit.
Before product integration, apply corrections on the actual integration base,
run the required exact-revision real-GPU gates and Linux regression bar, and
add Windows workload and teardown/re-init evidence. Source-derived invariance
across drivers is stronger than one captured die's numbers, but it must not be
reported as all-family/all-die runtime validation.
