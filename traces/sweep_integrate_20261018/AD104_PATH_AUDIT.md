# AD104 / Windows580.88 path inventory against candidate4

STATUS: LIVE, 2026-10-19. CPU evidence and code read; owner host untouched.

The requested expectation of no differences is NOT established. These paths
differ; the current product has not been hardware-validated on the owner GPU.
The guest wire tag is580.65.06; host595.91.07 appears only in the stored oracle.

MEASURED CPU: cudart_gate_ad104_oracle, host_facts_ga106, the per-die/wiring
and consttab tests passed in the integrated release batch. The independent
strict caps comparison against the stored native AD104 page FAILS with100
differing words (ad104_caps_strict_comparison.tsv). The derived-field test and
the exact F5-bisection-page test pass. Those tests do not prove native page
equality, owner-host parity, current Windows boot, or application acceptance.

CODE READ paths that differ:

1. HostAbi gains the public bounded GID control inventory/layout at all compiled
   tags. Automatic host UUID discovery can now carry that query where it was
   previously missing; supplied identity paths retain their existing behavior.
   Independent request and reply-carry tests pin lengths and UUID bytes.
2. HostFacts retains DieId from the same MC_GET_ARCH_INFO answer; query_arch_die
   does not invent a second architecture request. Constant selection now uses
   architecture+implementation rather than treating all dies like GA106.
3. query_gss_replay asks additional fixed, authored host controls, including the
   cudart gate and exact-zero legacy rows. Exact-zero rows are asked once with
   zeros; no background fill is treated as the same unknown-layout request.
   An AD104595 host's measured8162 byte00 wins over GA106's01. The CPU oracle
   pins identical replies to the stored host answers for the guest580 requests.
4. InitTablePolicy uses host-first/per-die/branch/request-scoped fallback. On a
   host miss, AD104 at guest580 does not borrow GA106 cudart/8162 rows. The
   AD1049001/a001 board-specific pairs refuse rather than table a captured
   board value. This is an intentional changed negative path, not parity.
5. rowserve wires GMMU-static and CE fault-buffer rows with labelled authored
   fallbacks; group-P scalars serve only exact measured questions. AD104580
   has no new AD102/TU106 scalar rows. Withheld keys still fall through to the
   existing policy/refusal; their notices are now bounded at64 distinct texts.
6. ZBC defaults come from the exact AD104 host-state rows rather than the old
   universal GA102 capture. Per-die tests pin the published values; a missing
   die starts empty with a refusal, never a neighboring die's defaults.
7. DisplayPlane passes the host die to the caps derivation. The old19-flag
   runtime captured-page probe is removed. AD104580 now gets the authored
   page plus generated family-layer rules and its measured unnamed0x00c word,
   the F5 page, rather than the old captured native page. One hundred word
   offsets differ in the strict comparison; the TSV names every offset/value
   and field/unknown derivation. No unknown native values are copied back.
8. Display effects now use Place: Ada resolves the same context-DMA branch;
   new surface-address branches serve Blackwell. The resolver explicitly
   checks extents before memory access. Malformed declarations can therefore
   refuse earlier; legitimate Ada notifier/semaphore semantics are tested but
   not freshly measured on hardware. Pacing, broker cursor and scrub code are
   preserved; the approved scrub test tuple adds only None, retaining every assertion.
9. WalkKernel uses a fresh GPU event recorded after each launch and queries it
   before collecting. A stale fd cannot authorize completion. The VA worker
   selects its existing pending-event observation interval instead of always
   waiting the idle backstop. This affects AD104 walk completion/scheduling;
   CPU ordering and synchronization-scope tests do not prove performance parity.
10. Host-enabled diagnostics add bounded posted-reply bodies and a process
    metadata quota. These flags are absent from the19 production profile. The
    new CMC pure-In probe is default-off; PR46's native581 allocation row is
    exact AD102, so it does not widen the AD104/580 cell. Harness changes alter
    guest-driver selection, result recovery and process ownership checks.

CODE READ preserved boundaries: F-2 keeps Ada's8-bit upper field; the approved
family gate accepts exactlyc56f, with c36f/c46f refused by exhaustive tests.
The new FSP reset/gate branches apply to Hopper/Blackwell; Ada keeps Falcon.
All candidate4/candidate5 named tests are retained, including the old walk
ratchet name alongside candidate5's explicit store-completion guard.
11. The newly APPROVED heartbeat topic changes AD104 clocks too: the same500ms
    tick reads the validated mapped timer rather than an RM ioctl, subtracts
    this life's fn72 offset after separate millisecond divisions, and skips a
    tick if metadata is contended/missing/invalid. No blocking lock acquisition
    enters this operation. See HEARTBEAT_NONBLOCKING.md and mutation receipts.
    This is another explicit path difference, not an owner-host parity claim.

Groups/latejoin/birth_group and USER relay/lifecycle/doorbell paths beyond the
approved helper are unchanged. Production scrub and pacing stay unchanged.

No current hardware claim follows from this inventory. The matrix keeps all
old passes STALE and current cells UNRUN until the authorized rental suites run.
