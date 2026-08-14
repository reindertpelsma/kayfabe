// ★★★★★ w323 row 3, and it is the row that makes the guarantee THREAD IDENTITY rather
// than merely provenance.
//
// `kayfabe_core::channel_kind::TrapContract` ruled — correctly, for what it had — that
// *"Rust cannot express 'this call is not on the vCPU thread'"*. That is true of a type
// ALONE. Composed with a per-thread witness and `!Send`, it is false: the private field
// says the token was minted by the constructor, `OffTrap::claim`'s check says the minting
// thread was off-trap, and THIS row says it is still on that thread. Delete `!Send` and a
// worker could mint one and post it to a vCPU, which would launder every host verb on the
// trap path past the gate.
use kayfabe_util::trapwitness::OffTrap;

fn main() {
    // ⚠ THE INSTRUMENT'S OWN TRAP, PAID FOR ONCE (2026-08-14): this row first read
    // `let _ = off;` inside the closure and **COMPILED** — under edition 2021+ closure
    // capture rules `let _ = x` does not capture `x` at all, so the closure stayed `Send`
    // and the row proved nothing while looking exactly like the other three. ⇒ the body
    // must USE the value. `suspect_the_instrument_first`, on a compile-fail row.
    let off = OffTrap::claim("minted on this thread");
    std::thread::spawn(move || {
        off.still_off_trap("a host RM verb on the wrong thread");
    });
}
