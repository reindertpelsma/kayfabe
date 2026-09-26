// ★★★★★ THE SINGLE-WRITER RULE, half one: a completion release that never asked the guest's
// own page tables where it lands must be UNSPELLABLE outside `kayfabe-rt`.
//
// `completion_observer.md` §8 measured that this tree has exactly one function that writes a
// completion semaphore and ZERO writers to the page `cuCtxCreate` polls — then stated the
// honest limit in §8.4: the property is "currently accidental, asserted nowhere, and one
// refactor from untrue". This row is the assertion. Every field below is correct and every
// value legal; it is a compile error (E0639) only because `ResolvedRelease` is
// `#[non_exhaustive]`, so the sole way to obtain one is `resolve_releases`.
//
// ⊘ Why it matters that this is a COMPILE error and not a test: the C artifact's M5.38 bug
// was a SECOND WRITER, and a backwards semaphore write is fatal on FIRST occurrence (UVM
// reads any decrease as a 2^32 wrap and `UVM_ASSERT_MSG_RELEASE` is in release builds).
// There is no run in which a second writer shows up as a small regression first.
use kayfabe_arch::ids::GpuVa;
use kayfabe_arch::{CeSemStructure, CpuOperand, CpuPlane, PlaneAddr, Residency};
use kayfabe_rt::cpu_ce::ResolvedRelease;

fn main() {
    // ⊘ Every value here is legal and buildable — `Residency::stable` is the ordinary answer
    // and `CpuOperand` is an ordinary public struct. The ONLY thing wrong with the expression
    // below is that the address never went through the guest's page tables, and that is
    // exactly what the compiler is being made to say.
    let op = CpuOperand {
        residency: Residency::stable(CpuPlane::GuestRam),
        addr: PlaneAddr(0x2059_fff0),
    };
    // The exact release `w226b` watched — guest VA 0x2_0440fff0, payload 1 — minted out of
    // nothing instead of out of the guest's page tables.
    let _forged = ResolvedRelease {
        va: GpuVa(0x2_0440_fff0),
        op,
        payload: 1,
        structure: CeSemStructure::OneWord,
        payload_bytes: 4,
        words: [Some(op), None, None, None],
    };
}
