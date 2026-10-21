//! ★ DOORBELL CONTRACT pin (owner, 2026-10-17): the relay's look (`ChanPlane::user_ring_peek_one`,
//! driven by the spin, the completion events and the BACKSTOP timer) must never read the guest's
//! `GP_PUT`. The last doorbell covers every `GP_PUT` write before it; nothing else announces it.
//! A structural test, because the look needs a live plane to run; the decision itself is
//! `kf_chan::user_loop::look_needed` (tested there) and the model's `PUT_POLL` mutant.

#[test]
fn the_look_function_never_reads_gp_put() {
    let src = include_str!("../src/userpump.rs");
    let start = src
        .find("fn user_ring_peek_one")
        .expect("user_ring_peek_one exists");
    let tail = &src[start..];
    // the function ends where the next documented item starts
    let end = tail
        .find("\n    /// Selects the worker's short park interval")
        .expect("the item after user_ring_peek_one moved: update this pin");
    let body = &tail[..end];
    assert!(
        !body.contains("USERD_GP_PUT") && !body.contains("GP_PUT)") && !body.contains(".userd\n"),
        "the look reads GP_PUT: that is a GP_PUT poll, forbidden by the doorbell contract"
    );
    assert!(
        !body.contains("userd.read") && !body.contains("r.userd"),
        "the look must not read guest USERD at all"
    );
    assert!(
        body.contains("look_needed"),
        "the look must decide through kf_chan::user_loop::look_needed"
    );
}
