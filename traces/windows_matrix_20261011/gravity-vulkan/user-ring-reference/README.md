# Native USERD reference discriminator

**STATUS: RESEARCH, 2026-10-12.** Native reference cases complete; parser/receipt models unconnected. No VM relay or app claim.

Hypotheses/falsifiers precede hardware in
`docs/design/V3_WINDOWS_USER_RING_RELAY_REVIEW_20261012.md`, USERD reference section.
PublicC56F only: read-only ring, unmapped USERD, USER channel, virtual4096-byte CE
copy then WFI_ALL/SET_REFERENCE. Identical immediate/held commands; only the
acquire fixture's initial value/publication time differs. The50ms hold is an
explicit bench input; no product gate/timeout release. No writable private GPU
completion mapping. Capture source/SHA/PID/time/journal/cleanup with run_reference.py.

GPU-free probe4 tests and full kf-abi suite PASS; logs alongside. These additions
are bench-only plus unused public-header ABI constants. Exact70aa Linux regression
covers the unchanged VM runtime; this revision has no Windows hardware result.

## Measured native result, 2026-10-12 00:39UTC

Exact9b65e3c9e52a70b596e6dd1135673ed494bd6554, binarySHA256
fe29f5b4bc9876e8b93df0fb396f3021d0b197ab524b2daee57b2e0319656db1.
HostRTX4070/595.91.07, chosen channelC56F, uid/gid65534, USER births verified.

- immediate reference: PASS, copy4096 bytes matches, CE semaphore1/reference1,
  GP_GET1/PUT1, elapsed2205us, zeroXid, clean teardown.
- held reference: PASS,48 polls before explicit acquire signal, reference0 and
  CE semaphore0 throughout sampled hold, GP_GET fetched while held. After signal,
  reference1/correct copy/CE semaphore1, elapsed52847us, zeroXid, clean teardown.

This supports the limited native FIFO/CE reference mechanism, not every-engine
completion or native migration. The polled4096-byte copy does not independently
prove arbitrary engine visibility at every possible instant. No writable private
completion GPU mapping or personal VM/image used. Original metadata/logs/journal/
collector output in result/ and native-session.log. Post00:40:41UTC GPUidle,
lockfree/DMA-FQ/stop-file absent,129119285248bytesfree. No queued job or rental.

GPU-free follow-up: reference receipt window refuses out-of-flight/stale values,
handles32-bit wrap within bounded outstanding work, never usesGET as completion.
Parser takes an owned bounded snapshot and transactionally checks framing and
channel-owned bindings; software calls and guest SET_REFERENCE become ordered IR,
never native hardware receipts. Seven parser tests plus two new receipt tests;
full kf-chan70lib/all integrations PASS. Failing-first and full green logs retained.
Legacy/control/subroutine/cross-entry packet support, mapping/backing lifetime,
actual cursor migration and native adapter remain open. No VM runtime call exists.

Direct-GP descriptor follow-up: inspect the whole C56F descriptor before copying
commands. Conditional fetch, subroutine, reserved/older privilege flags and
nonzero control entries are refused. The canonical zero NOP is explicit; SYNC_WAIT
is retained for a future adapter. Trusted byte budget and complete40-bit VA span
are checked first. Nine parser/descriptor cases and full kf-chan/kf-abi suites
PASS (stream-direct-* logs). This remains unconnected and does not pin mappings,
implement cross-entry packets or claim native cursor transfer.
