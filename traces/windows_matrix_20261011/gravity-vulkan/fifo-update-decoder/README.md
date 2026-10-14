**STATUS: RESEARCH, 2026-10-12 04:37UTC.**

Typed FIFO_UPDATE_CHANNEL_INFO bundle, derived from the existing generated public
ogkm layout at the guest version; no captured offsets or values. All six scalar
widths and checked field ends are validated within the complete measured struct.
Layout absence is local to this optional bundle and does not disable other5080
kinds. Short/missing-layout payloads decode Unknown without any out-of-span read.
The current T-space classifier explicitly refuses the typed variant; no native
migration verb, cursor reset, mapping or relay migration is enabled. The existing
diagnostic match now uses the typed variant instead of an opaque Unserved name.

MEASURED: failing-first test compile failed for absent FifoUpdate/variant/layout
field, then full cargo test -p kf-abi -p kf-rm -p kf-qemu -p kf-chan passed.
Synthetic scalar round trips cover Windows580.88 and generated range endpoints
535.309.1/615.71.9, every short member prefix and missing optional layout.
Independent read-only review found no source blocker. No hardware claim for this
new decoder checkpoint; preceding cec428397 Linux9+10/30 PASS is separate.
Runtime still must validate same-client/channel identity, USERD ownership,
alignment/spans/capacity, source pins/epoch and real ordered migration semantics.
