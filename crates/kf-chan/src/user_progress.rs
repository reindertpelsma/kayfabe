//! Ordered plan/cell receipts, not a VM adapter or GPU-work executor.
//!
//! Intermediate HOST-consumption receipts retire private command cells only.
//! Final guest-source progress requires every cell and software action in order.
//! No clock, reference value, error or cancellation supplies a receipt.

use crate::user_batch::{Action, MAX_OPS, WORDS};
use crate::user_consumption::{ConsumedSource, Consumption, Publication};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Commands,
    Software,
}

/// Host-authored action identity. Fields cannot be supplied by guest bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    native: u64,
    plan: u64,
    ordinal: u32,
    kind: Kind,
}

/// Only Commands::observe_get creates a native HOST-consumption receipt.
#[derive(Clone, Copy, Debug)]
struct Receipt(Ticket);

/// A started action; a backpressured command must retain this exact ticket.
#[cfg(test)]
struct Step<'a> {
    /// Allocation/plan/action identity, never an output guest cursor.
    pub ticket: Ticket,
    /// Immutable bounded words or one ordered software action.
    pub action: &'a Action,
}

/// Named refusal; none advances source progress or frees a command cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Zero/wrong/reused native allocation or host plan identity.
    Lifetime,
    /// Invalid action ordering, duplicate publication, or a replayed receipt.
    Order,
    /// Malformed or over-budget host plan.
    Plan,
    /// A software action must never be submitted to hardware.
    Kind,
    /// Native command ledger refused; its state remains authoritative.
    Native(crate::user_consumption::Refusal),
}

/// One immutable source snapshot, held through all of its actions.
/// Caller allocates a strictly increasing nonzero plan identity within a native
/// allocation; native/source generations must never be derived from guest handles.
struct Ordered {
    native: u64,
    plan: u64,
    source: ConsumedSource,
    actions: Vec<Action>,
    issued: usize,
    consumed: usize,
}

impl Ordered {
    /// Source GP/USERD backing, epoch and index bounds are an external admission
    /// gate. This model does not resolve mappings or grant a lifetime pin.
    pub fn new(
        native: u64,
        plan: u64,
        source: ConsumedSource,
        actions: Vec<Action>,
    ) -> Result<Self, Refusal> {
        if native == 0 || plan == 0 || source.generation == 0 {
            return Err(Refusal::Lifetime);
        }
        if actions.len() > MAX_OPS
            || actions.iter().any(|a| {
                matches!(a,
            Action::Commands(w) if w.is_empty() || w.len() > WORDS || w.len() % 2 != 0)
            })
        {
            return Err(Refusal::Plan);
        }
        Ok(Self {
            native,
            plan,
            source,
            actions,
            issued: 0,
            consumed: 0,
        })
    }

    /// Start in order. Commands can pipeline; software requires all previous
    /// HOST receipts and blocks later actions until its actual successful result.
    /// After this call, a failed/backpressured native publish must retain/retry
    /// the exact ticket; an uncertain I/O result stops/quiesces the channel.
    fn ready_ticket(&self) -> Option<Ticket> {
        if self.issued > self.consumed
            && matches!(
                self.actions.get(self.issued - 1),
                Some(Action::Software(_) | Action::OwnInvalidate)
            )
        {
            return None;
        }
        let kind = match self.actions.get(self.issued)? {
            Action::Commands(_) => Kind::Commands,
            Action::Software(_) | Action::OwnInvalidate if self.consumed == self.issued => {
                Kind::Software
            }
            Action::Software(_) | Action::OwnInvalidate => return None,
        };
        Some(Ticket {
            native: self.native,
            plan: self.plan,
            ordinal: self.issued as u32,
            kind,
        })
    }

    #[cfg(test)]
    fn issue_next(&mut self) -> Option<Step<'_>> {
        let ticket = self.ready_ticket()?;
        self.issued += 1;
        Some(Step {
            ticket,
            action: &self.actions[ticket.ordinal as usize],
        })
    }

    fn finish(&mut self, ticket: Ticket, kind: Kind) -> Result<(), Refusal> {
        if ticket.native != self.native || ticket.plan != self.plan {
            return Err(Refusal::Lifetime);
        }
        if ticket.kind != kind {
            return Err(Refusal::Kind);
        }
        if ticket.ordinal as usize != self.consumed || self.consumed >= self.issued {
            return Err(Refusal::Order);
        }
        let actual = match self.actions.get(self.consumed) {
            Some(Action::Commands(_)) => Kind::Commands,
            Some(Action::Software(_) | Action::OwnInvalidate) => Kind::Software,
            None => return Err(Refusal::Order),
        };
        if actual != kind {
            return Err(Refusal::Kind);
        }
        self.consumed += 1;
        Ok(())
    }

    /// Verified consumption of the private cell; no engine completion claimed.
    pub fn command_consumed(&mut self, receipt: Receipt) -> Result<(), Refusal> {
        self.finish(receipt.0, Kind::Commands)
    }

    /// Call ONLY after the channel-owned software action actually succeeded.
    /// An error, pending action, elapsed time or canceled request is not success.
    pub fn software_done(&mut self, ticket: Ticket) -> Result<(), Refusal> {
        self.finish(ticket, Kind::Software)
    }

    /// The sole guest-source progress result: all actions in this snapshot have
    /// been HOST-consumed/served. Still not permission to unmap engine operands.
    /// Adapter must revalidate THIS source generation before writing guest USERD.
    pub fn source_progress(&self) -> Option<ConsumedSource> {
        (self.consumed == self.actions.len()).then_some(self.source)
    }
}

/// Native private command-cell ledger. Its inner source fields encode HOST
/// (plan,action) cookies ONLY; they are never interpreted as guest GP_GET.
struct Commands {
    native: u64,
    inner: Consumption,
    last: Option<(u64, u32)>,
}

/// Worker work already reserved in stream order. Write Commands completely
/// before PUT; uncertain I/O stops/quiesces the channel, with no cancellation.
pub enum Work<'a> {
    /// Authored complete packets and their reserved native descriptors.
    Commands {
        /// Reserved three-descriptor bundle; no cancellation after possible I/O.
        publication: Publication,
        /// Complete host-authored packets for this bounded private cell.
        words: &'a [u32],
    },
    /// Reconcile only this channel's own space after preceding HOST receipts.
    /// No later work issues until the VA manager's actual successful result.
    OwnInvalidate {
        /// Same ordered software receipt discipline, never a guest API handle.
        ticket: Ticket,
    },
    /// No later work can issue until this channel-owned action really succeeds.
    Software {
        /// Opaque action identity, valid only for the current native plan.
        ticket: Ticket,
        /// Channel-owned method to serve before any later work.
        call: crate::tmode::SwCall,
    },
}

/// One live native allocation, one active source plan, permanently bound ledger.
/// Guest handles supply neither allocation identity nor host plan sequence.
pub struct Channel {
    native: u64,
    next_plan: u64,
    commands: Commands,
    active: Option<Ordered>,
}

impl Channel {
    /// NEW native channel with private cursors zero; never reset a live channel.
    pub fn new(entries: u32, native: u64) -> Result<Self, Refusal> {
        Ok(Self {
            native,
            next_plan: 0,
            commands: Commands::new(entries, native)?,
            active: None,
        })
    }

    /// Admit one complete bounded immutable parser/planner snapshot. Source pins
    /// and VA epoch are external admission gates. Aggregate GP fragments while
    /// parser.has_pending(): a copied header is not a retired source packet.
    /// Empty plans require the explicit no-op entry point instead.
    pub fn start(
        &mut self,
        source: ConsumedSource,
        actions: Vec<Action>,
        parser: &crate::user_stream::State,
    ) -> Result<(), Refusal> {
        if actions.is_empty() || parser.has_pending() {
            return Err(Refusal::Plan);
        }
        self.admit(source, actions)
    }

    /// Only for a fully inspected no-op snapshot, e.g. canonical zero GP NOPs.
    /// This cannot complete a header-only fragment or other unfinished packet.
    /// No GPU work/action is behind this CPU parsing result; source/epoch pins
    /// must still be validated and the result explicitly taken before replacement.
    pub fn start_noop(
        &mut self,
        source: ConsumedSource,
        parser: &crate::user_stream::State,
    ) -> Result<(), Refusal> {
        if parser.has_pending() {
            return Err(Refusal::Plan);
        }
        self.admit(source, Vec::new())
    }

    fn admit(&mut self, source: ConsumedSource, actions: Vec<Action>) -> Result<(), Refusal> {
        if self.active.is_some() {
            return Err(Refusal::Order);
        }
        let plan = self.next_plan.checked_add(1).ok_or(Refusal::Lifetime)?;
        let active = Ordered::new(self.native, plan, source, actions)?;
        self.next_plan = plan;
        self.active = Some(active);
        Ok(())
    }

    /// Bounded next action. Full leaves the command unissued for exact retry.
    /// Software waits for all preceding HOST receipts and blocks later actions.
    pub fn next(&mut self) -> Result<Option<Work<'_>>, Refusal> {
        let Some(active) = self.active.as_mut() else {
            return Ok(None);
        };
        let Some(ticket) = active.ready_ticket() else {
            return Ok(None);
        };
        let publication = if ticket.kind == Kind::Commands {
            if self.commands.inner.available_gp() < 3 {
                return Ok(None);
            }
            // All refusals precede native I/O and leave both ledgers unchanged.
            Some(self.commands.publish(ticket)?)
        } else {
            None
        };
        active.issued += 1;
        Ok(Some(
            match (&active.actions[ticket.ordinal as usize], publication) {
                (Action::Commands(words), Some(publication)) => {
                    Work::Commands { publication, words }
                }
                (Action::OwnInvalidate, None) => Work::OwnInvalidate { ticket },
                (Action::Software(call), None) => Work::Software {
                    ticket,
                    call: *call,
                },
                _ => unreachable!("private ready_ticket kind agrees with immutable action"),
            },
        ))
    }

    /// Fresh coherent ordered hardware GET from THIS private native USERD only.
    /// A cursor/refusal from any other allocation never grants source progress.
    pub fn observe_get(&mut self, native: u64, get: u32) -> Result<(), Refusal> {
        for receipt in self.commands.observe_get(native, get)? {
            self.active
                .as_mut()
                .ok_or(Refusal::Lifetime)?
                .command_consumed(receipt)?;
        }
        Ok(())
    }

    /// ONLY a real successful result of the emitted channel-owned software act.
    pub fn software_done(&mut self, ticket: Ticket) -> Result<(), Refusal> {
        self.active
            .as_mut()
            .ok_or(Refusal::Order)?
            .software_done(ticket)
    }

    /// Inspect completion without releasing single-active-plan admission.
    pub fn source_progress(&self) -> Option<ConsumedSource> {
        self.active.as_ref().and_then(Ordered::source_progress)
    }

    /// Once, after revalidating the pinned source generation for guest USERD.
    /// This is HOST consumption, never permission to unmap engine operands.
    pub fn take_source_progress(&mut self, generation: u64) -> Result<ConsumedSource, Refusal> {
        let source = self.source_progress().ok_or(Refusal::Order)?;
        if source.generation != generation {
            return Err(Refusal::Lifetime);
        }
        self.active = None;
        Ok(source)
    }
}

#[cfg(test)]
#[path = "user_progress_model.rs"]
mod model;

impl Commands {
    /// Bind this ledger permanently to one live native allocation/USERD.
    pub fn new(entries: u32, native: u64) -> Result<Self, Refusal> {
        Ok(Self {
            native,
            inner: Consumption::new(entries, native).map_err(Refusal::Native)?,
            last: None,
        })
    }

    /// No cancellation after reservation or possibly-visible I/O. Full/error
    /// grants no new publication and does not advance this cookie sequence.
    pub fn publish(&mut self, ticket: Ticket) -> Result<Publication, Refusal> {
        if ticket.native != self.native {
            return Err(Refusal::Lifetime);
        }
        if ticket.kind != Kind::Commands {
            return Err(Refusal::Kind);
        }
        if let Some((plan, ordinal)) = self.last {
            if ticket.plan < plan || (ticket.plan == plan && ticket.ordinal <= ordinal) {
                return Err(Refusal::Order);
            }
            if ticket.plan > plan && self.inner.pending_sources() != 0 {
                return Err(Refusal::Order);
            }
        }
        let publication = self
            .inner
            .publish(ConsumedSource {
                generation: ticket.plan,
                guest_get: ticket.ordinal,
            })
            .map_err(Refusal::Native)?;
        self.last = Some((ticket.plan, ticket.ordinal));
        Ok(publication)
    }

    /// Fresh ordered coherent hardware GET from THIS private native USERD only.
    /// Caller checks native lifetime; stale/guest/modulo-reset samples are invalid.
    pub fn observe_get(&mut self, native: u64, get: u32) -> Result<Vec<Receipt>, Refusal> {
        // Reserve this bounded output BEFORE observing. The inner ledger also
        // allocates before mutation; no fallible conversion follows credit release.
        let mut out = Vec::new();
        out.try_reserve_exact(self.inner.pending_sources())
            .map_err(|_| Refusal::Native(crate::user_consumption::Refusal::Allocation))?;
        let consumed = self
            .inner
            .observe_get(native, get)
            .map_err(Refusal::Native)?;
        for cookie in consumed {
            out.push(Receipt(Ticket {
                native: self.native,
                plan: cookie.generation,
                ordinal: cookie.guest_get,
                kind: Kind::Commands,
            }));
        }
        Ok(out)
    }
}
