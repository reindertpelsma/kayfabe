// ★★★★★ w323: THE ASYMMETRY, AS A COMPILE ERROR.
//
// Owner ruling 2026-08-14: a late MAP costs a GPU fault (fail-safe, and this tree measured
// containment — a bystander ran 2 675 519 verified iterations through one). A late REVOKE
// leaves a host-GPU translation live into guest pages the guest has already released and
// Linux has reused (fail-DANGEROUS). ⇒ deferring a revocation is not a latency choice, it
// is a leak window whose duration IS the deferral.
//
// So "put the unmap on the deferred queue too" must not be a code review that has to happen
// every time. It is this.
use kayfabe_device::pubqueue::{PublicationQueue, Revocation};

fn main() {
    let q = PublicationQueue::new();
    let _ = q.offer(Revocation::of_a_host_mapping());
}
