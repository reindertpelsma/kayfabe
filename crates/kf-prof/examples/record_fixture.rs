//! Explicit diagnostic fixture, never part of a production binary.
use std::{io::Write, path::PathBuf, time::Duration};
fn main() {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("fixture output directory"),
    );
    let s = kf_prof::runtime::Session::create_diagnostic(&path).expect("diagnostic session");
    let _binding = s.bind_current(1).expect("bind");
    let _span = kf_prof::site(
        kf_prof::tables::Site::BrokerReceive,
        kf_prof::tables::State::Receive,
        42,
    );
    let _owed = kf_prof::ledger_open(kf_prof::tables::Kind::HeldReply, 42);
    std::thread::sleep(Duration::from_millis(1100));
    println!("READY");
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(30));
    s.stop().unwrap();
}
