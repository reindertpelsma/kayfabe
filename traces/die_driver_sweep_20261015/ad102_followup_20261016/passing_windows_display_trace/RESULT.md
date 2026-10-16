# Passing Ada connected-display reference versus Blackwell abort

**STATUS: ANSWERED, 2026-10-16 17:09 UTC. Metadata narrows the phase; no causal reply-byte claim.**

MEASURED: owned AD102 instance56267214, host595.91.07, Windows580.88 /
32.0.15.8088, private7d1be8882. The diagnostic process102707 uses the19production
flags plus bounded RPC metadata and posted-body diagnostics. Code0, DWM/Explorer,
CUDA cup2/3/8, hardware D3D11+12 and zero TDR since boot pass at16:58:33UTC.
The separate production19flag hold passes421seconds with zero TDR; that process
has both diagnostics off. Actual flags and receipts are in ../windows580/.

All3166policy metadata commands before the diagnostic stop were observed with
zero metadata quota notices. The exported API phase is explicitly ordinals0..999;
full metadata remains on the rental. The posted-body logger reached its512record
cap:78public display/control bodies were exported,434opaque/allocation bodies
were omitted before export. There is no complete-body-stream claim. Source-log
SHA/size and all selected-control counts are in summary.json.

| Event | Passing Ada policy ordinal | Blackwell complete385-command sequence | Earlier382-command trace |
|---|---:|---:|---:|
|Last initial display-window allocation|281, classC67A|276, classCA7A|274|
|Immediate next command|282: controlC3700201, policy envelope0|277: SET_RMFREE_FLAGS, then Free|275: SET_RMFREE_FLAGS, then Free|
|First control73010C|550; more calls later|not issued|not issued|

The two extra Other4 records in the complete Blackwell reconstruction shift its
display ordinals by2; its explicit unload adds the third extra record at the end.
Do not mix the older274/275 convention with276/277. Ada's last A58 is280; its
last window allocation281 likewise has no following A58. Blackwell's last A58
is275, followed by its last window allocation276 and teardown277.

**Falsifier result, mixed:** the passing Ada driver does not issue native
headless GB206's four73010C queries immediately before C3700201. Their absence
at that exact point is therefore insufficient to identify the failure. Ada does
issue C3700201 immediately after its initial window allocations and continues;
Blackwell frees the display objects there. The proposed falsifier that passing
Ada also omits C3700201 did not fire. This keeps display reply/state as the first
ranked investigation, without proving a particular capability word or reply byte.

| Control/object | Passing Ada captured interval |
|---|---|
|20800A89,20800AB2,20800AB4,CBCB|not issued in all3166captured commands; zero metadata truncation|
|20802801|once, ordinal149, policy envelope0|
|20802806|once, ordinal160, policy envelope0|
|C3700201|once, ordinal282, policy envelope0|
|73010C|26times, first550, policy envelopes0|

The actual posted subset contains21A58 and nine73010C replies, all with
envelope0. Their public body bytes are retained in posted-public-display-bodies.txt.
Policy results precede held-action completion and are not silently promoted to
actual posted statuses: effective_result remains unknown in the metadata TSV.
The generic aligner's native_* columns mean **passing Ada reference** here;
both sides are kayfabe guests.255API keys match across Ada's1000-command phase
and all385Blackwell commands. Matching, repeated-key correspondence and semantic
class correspondence remain inference; this is not native VFIO wire validation
or reply-body equality. Pretty metadata has no wire-reply pairing to validate.

Next ranked run: rentable GB206, same private binary, bounded actual posted
display/A58 bodies and display=off with an explicit CA70 allocation census;
then one public-header capability group under B2's stop rule. Native per-die
body capture is useful only after a virtual difference worth checking exists.
No full native-page injection was made or authorized. No CBCB ack or cross-ABI
CCU scalar was invented. Blackwell Windows remains red; no family is complete.
