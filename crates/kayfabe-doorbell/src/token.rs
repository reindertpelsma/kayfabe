//! The token word — one `u64` per token, and the only thing any CAS touches.
//!
//! `THE_DESIGN.md` §5.1: *"one `u64` per token: route, state, `applied_seq` stamp, host token —
//! every CAS. No neighbour can fail it."* ⇒ The whole point of packing into one word is that a
//! token's CAS can never be failed by a NEIGHBOUR's activity. A struct-of-fields, or a word
//! shared between two tokens, reintroduces exactly the false contention this removes.
//!
//! ⊘ **The token is MASKED, not validated** (§5.1). Masking is what the hardware does with
//! undecoded bits, and it removes an error path — a validation branch here would be a refusal
//! the hardware itself does not make.

use core::sync::atomic::{AtomicU64, Ordering};

/// The token's state. §5.2 names four, **plus a retired state** the same section then shows is
/// mandatory: *"A token needs a retired state, or channel recycling is a use-after-free."*
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    /// Nothing queued, nobody acting.
    Idle = 0,
    /// The guest rang; no worker has claimed it.
    Rung = 1,
    /// A worker owns it. ★ §5.2: *"A bit that says work exists is not a bit that says you may
    /// touch the hardware."* Without this exclusion two workers walk one channel against one
    /// cursor and `[c0,p1)` executes twice.
    Busy = 2,
    /// Owned, and rung again while owned. The put-back case.
    BusyRung = 3,
    /// Retired. §5.2: free goes `IDLE|RUNG → DEAD` and **waits out `BUSY`** before dropping the
    /// twin; allocation goes `DEAD → IDLE` with the new route. ⊘ Without it: the guest frees a
    /// channel while its token is `BUSY`, recycles the channel id, and the new channel's first
    /// ring lands on a word still carrying the OLD route — while the old worker is still reading
    /// what it believes is a pushbuffer.
    Dead = 4,
}

impl State {
    #[inline]
    fn from_bits(b: u64) -> State {
        match b & STATE_MASK {
            0 => State::Idle,
            1 => State::Rung,
            2 => State::Busy,
            3 => State::BusyRung,
            _ => State::Dead,
        }
    }
}

/// How a token's work is served. §7's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Route {
    /// We do not know this token. ⊘ The DEFAULT, deliberately: an unknown token must not be
    /// guessed into a plane.
    Unknown = 0,
    /// The guest's ring reaches hardware unparsed.
    Passthrough = 1,
    /// We translate addresses and submit.
    Translated = 2,
    /// We implement it; there is no GPU counterpart.
    Emulated = 3,
}

impl Route {
    #[inline]
    fn from_bits(b: u64) -> Route {
        match b & ROUTE_MASK {
            1 => Route::Passthrough,
            2 => Route::Translated,
            3 => Route::Emulated,
            _ => Route::Unknown,
        }
    }
}

// ---- the layout -----------------------------------------------------------------------------
//
// ⚠ §5.1 sizes the token from **what the register can express, not what is legal**: 12 bits of
// channel + 7 of runlist on Ampere, plus a doorbell-type bit on Blackwell ⇒ 19–21 bits. We carry
// 21 and mask.
const STATE_BITS: u32 = 3;
const ROUTE_BITS: u32 = 2;
pub const HOST_TOKEN_BITS: u32 = 21;
const SEQ_BITS: u32 = 64 - STATE_BITS - ROUTE_BITS - HOST_TOKEN_BITS; // 38

const STATE_SHIFT: u32 = 0;
const ROUTE_SHIFT: u32 = STATE_SHIFT + STATE_BITS;
const HOST_SHIFT: u32 = ROUTE_SHIFT + ROUTE_BITS;
const SEQ_SHIFT: u32 = HOST_SHIFT + HOST_TOKEN_BITS;

const STATE_MASK: u64 = (1 << STATE_BITS) - 1;
const ROUTE_MASK: u64 = (1 << ROUTE_BITS) - 1;
pub const HOST_TOKEN_MASK: u64 = (1 << HOST_TOKEN_BITS) - 1;
const SEQ_MASK: u64 = (1 << SEQ_BITS) - 1;

/// The decoded token word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub state: State,
    pub route: Route,
    pub host_token: u32,
    /// §5.2: *"stamp the ring position, THEN `fetch_or(RUNG)` — order matters."*
    pub applied_seq: u64,
}

impl Token {
    #[inline]
    pub fn decode(w: u64) -> Token {
        Token {
            state: State::from_bits(w >> STATE_SHIFT),
            route: Route::from_bits(w >> ROUTE_SHIFT),
            host_token: ((w >> HOST_SHIFT) & HOST_TOKEN_MASK) as u32,
            applied_seq: (w >> SEQ_SHIFT) & SEQ_MASK,
        }
    }
    #[inline]
    pub fn encode(self) -> u64 {
        // ⊘ MASK, never assert. §5.1: masking is what the hardware does with undecoded bits.
        ((self.state as u64 & STATE_MASK) << STATE_SHIFT)
            | ((self.route as u64 & ROUTE_MASK) << ROUTE_SHIFT)
            | ((self.host_token as u64 & HOST_TOKEN_MASK) << HOST_SHIFT)
            | ((self.applied_seq & SEQ_MASK) << SEQ_SHIFT)
    }
}

/// One token's word.
#[derive(Debug, Default)]
#[repr(transparent)]
pub struct TokenWord(AtomicU64);

/// What a claim attempt found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// We own it; act.
    Won(Token),
    /// Someone else owns it, or it is not rung, or it is retired. ⊘ §5.2: *"fail ⇒ not ours"* —
    /// and that is a normal outcome, never an error.
    NotOurs,
}

/// What releasing a claim asks the caller to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// Clean exit; nothing more to do.
    Idled,
    /// It was rung while we held it — act again without republishing. §5.2's `BUSY_RUNG → BUSY`.
    ActAgain,
    /// ⊘ The bounded loop expired. §5.2: *"After K rounds the worker republishes and moves on.
    /// ★ That is a timeslice, and it is what hardware does for the same reason."*
    /// The caller MUST republish (bit then summary) — see [`crate::bitmap`].
    RepublishAndMoveOn,
}

impl TokenWord {
    pub const fn new() -> TokenWord {
        TokenWord(AtomicU64::new(0))
    }

    #[inline]
    pub fn load(&self) -> Token {
        Token::decode(self.0.load(Ordering::Acquire))
    }

    /// Install a route at allocation. §5.2: allocation is `DEAD → IDLE` **with the new route**.
    /// Returns false if the token was not retired — allocating over a live token is the
    /// use-after-free this state exists to prevent.
    pub fn allocate(&self, route: Route, host_token: u32) -> bool {
        let cur = self.0.load(Ordering::Acquire);
        let t = Token::decode(cur);
        if t.state != State::Dead && t.state != State::Idle {
            return false;
        }
        let next = Token { state: State::Idle, route, host_token, applied_seq: 0 }.encode();
        self.0.compare_exchange(cur, next, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// ★ THE vCPU PATH. §5.2: *"stamp the ring position, THEN `fetch_or(RUNG)`"*.
    ///
    /// Returns `true` when this ring produced a **transition into RUNG** — i.e. the caller must
    /// publish a bitmap bit and bump the wake word. ⊘ Returning `false` is the case §5.2 warns
    /// about: *"a further ring from the guest returns early because `RUNG` is already set, so it
    /// produces no bit and no wake"* — which is CORRECT here (a bit is already published) and is
    /// only a bug if a put-back failed to republish.
    ///
    /// ⚠ Never blocks, never allocates, takes no lock: §48 and §41.
    pub fn ring(&self, applied_seq: u64) -> bool {
        let mut cur = self.0.load(Ordering::Acquire);
        loop {
            let t = Token::decode(cur);
            // ⊘ A retired token absorbs the ring. The guest may ring a channel it just freed;
            // that is not an error and must not resurrect the token.
            if t.state == State::Dead {
                return false;
            }
            let next_state = match t.state {
                State::Idle => State::Rung,
                State::Rung => State::Rung,
                State::Busy => State::BusyRung,
                State::BusyRung => State::BusyRung,
                State::Dead => unreachable!(),
            };
            // ⚠ The stamp is written in the SAME word as the state, so "stamp then set RUNG"
            // is one CAS and cannot be observed half-done. That is stronger than the two-step
            // §5.2 describes and satisfies it by construction.
            let next = Token { state: next_state, applied_seq, ..t }.encode();
            match self.0.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
                // Publish iff we moved IDLE → RUNG. Every other transition already has a bit
                // outstanding or is owned by a worker who will re-check.
                Ok(_) => return t.state == State::Idle,
                Err(seen) => cur = seen,
            }
        }
    }

    /// Worker: `CAS RUNG → BUSY`. §5.2.
    pub fn claim(&self) -> Claim {
        let cur = self.0.load(Ordering::Acquire);
        let t = Token::decode(cur);
        if t.state != State::Rung {
            // ⚠ §5.1: *"The bitmap is a hint; the word is the truth. A bit set over an IDLE word
            // costs one look."* This is that look, and it is the whole cost.
            return Claim::NotOurs;
        }
        let next = Token { state: State::Busy, ..t }.encode();
        match self.0.compare_exchange(cur, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => Claim::Won(t),
            Err(_) => Claim::NotOurs,
        }
    }

    /// Worker: finished a round of acting. `rounds_done` is how many times we have already acted
    /// on this claim; `k` is the bound from §5.2.
    ///
    /// ⊘ **The bound is not tuning — it is the inner privilege boundary (§4).** *"A process
    /// ringing its own channel in a tight loop keeps a worker in act → CAS fails → act forever;
    /// with enough channels it pins every worker and the guest kernel's own scrub and UVM
    /// channels starve."*
    pub fn release(&self, rounds_done: u32, k: u32) -> Release {
        let mut cur = self.0.load(Ordering::Acquire);
        loop {
            let t = Token::decode(cur);
            let (next_state, out) = match t.state {
                State::Busy => (State::Idle, Release::Idled),
                State::BusyRung if rounds_done < k => (State::Busy, Release::ActAgain),
                // Timeslice expired: hand it back as RUNG and make the caller republish.
                State::BusyRung => (State::Rung, Release::RepublishAndMoveOn),
                // ⊘ Freed under us. Leave it retired; the free path is waiting on exactly this.
                State::Dead => return Release::Idled,
                s => panic!("release() on a token in state {s:?} — not owned by this worker"),
            };
            let next = Token { state: next_state, ..t }.encode();
            match self.0.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return out,
                Err(seen) => cur = seen,
            }
        }
    }

    /// Worker: claimed it, but cannot act yet. §5.2: *"cannot act yet ⇒ `CAS BUSY → RUNG`, then
    /// re-publish"*.
    ///
    /// ⚠ §5.2 also says *"found but unactionable counts as NO WORK for the purpose of sleeping;
    /// otherwise a worker spins on a token whose enabling register write also needs a worker."*
    /// The caller must not count this as progress.
    pub fn put_back(&self) -> Release {
        let mut cur = self.0.load(Ordering::Acquire);
        loop {
            let t = Token::decode(cur);
            if t.state == State::Dead {
                return Release::Idled;
            }
            let next = Token { state: State::Rung, ..t }.encode();
            match self.0.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Release::RepublishAndMoveOn,
                Err(seen) => cur = seen,
            }
        }
    }

    /// Free path, first half. §5.2: `IDLE|RUNG → DEAD`, and **waits out `BUSY`**.
    /// Returns `false` while a worker still owns it — the caller must retry, and must NOT drop
    /// the twin until this returns `true`.
    pub fn retire(&self) -> bool {
        let cur = self.0.load(Ordering::Acquire);
        let t = Token::decode(cur);
        match t.state {
            State::Busy | State::BusyRung => false,
            State::Dead => true,
            _ => {
                let next = Token { state: State::Dead, route: Route::Unknown, host_token: 0, applied_seq: 0 }.encode();
                self.0.compare_exchange(cur, next, Ordering::AcqRel, Ordering::Acquire).is_ok()
            }
        }
    }
}
