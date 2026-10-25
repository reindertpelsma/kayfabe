use kayfabe_linux_raw::census::{phase, record_sequence, snapshot};
use kayfabe_linux_raw::{CharDevice, DevDir, ioctl};

/// `/dev/null` answers every ioctl with `ENOTTY`, which is what makes it the right
/// fixture: the census must count a **refused** ioctl exactly like a served one, because
/// *"how many times did we enter the driver"* is the question it answers.
///
/// One test per integration binary: unrelated ioctl writers must run in other processes.
#[test]
fn the_census_counts_at_the_syscall_logs_in_order_and_survives_the_log_being_off() {
    let dir = DevDir::open(c"/dev").expect("/dev exists on a Linux host");
    let d = CharDevice::openat(&dir, c"null").expect("/dev/null exists on a Linux host");
    let mut arg = [0u8; 8];

    // Exercise two distinct nonzero prior positions in separate binaries.
    let prime = ioctl::readwrite(0x46, 198, 8).expect("8 bytes encodes");
    for _ in 0..PRIOR_IOCTLS {
        assert!(d.ioctl(prime, &mut arg, &mut []).is_err());
    }
    assert_eq!(snapshot().total, PRIOR_IOCTLS);

    // (a) with the log ON: the record is written, and it carries every field.
    record_sequence(true);
    phase("census-test");
    let before = snapshot().total;
    let req = ioctl::readwrite(0x46, 200, 8).expect("8 bytes encodes");
    assert!(
        d.ioctl(req, &mut arg, &mut []).is_err(),
        "/dev/null ENOTTYs"
    );
    let after = snapshot();
    assert!(after.total > before, "the total moved");
    let mine: Vec<_> = after
        .log
        .iter()
        .filter(|r| r.phase == "census-test" && r.nr == 200)
        .collect();
    assert_eq!(mine.len(), 1, "exactly one record, and it is in the log");
    assert_eq!(mine[0].magic, 0x46, "the driver magic is carried");
    assert_eq!(mine[0].size, 8, "the copy length is carried");
    assert_ne!(mine[0].errno, 0, "a REFUSAL is recorded AS a refusal");
    assert_eq!(
        mine[0].seq,
        before + 1,
        "the sequence number is the process-wide position"
    );

    // (b) with the log OFF: the total still moves. That is what makes a phase shortfall
    // readable as "ioctls outside any phase" rather than as "the log was off".
    record_sequence(false);
    let before_off = snapshot().total;
    let req = ioctl::readwrite(0x46, 201, 8).expect("8 bytes encodes");
    let _ = d.ioctl(req, &mut arg, &mut []);
    let off = snapshot();
    assert_eq!(off.total, before_off + 1, "counted with the log off");
    assert!(
        !off.log.iter().any(|r| r.nr == 201),
        "and NOT logged — the two switches are independent"
    );
    phase("");
}
