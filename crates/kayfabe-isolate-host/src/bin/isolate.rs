//! The isolate process.
//!
//! Everything it does lives in `kayfabe_isolate_host::child`; this file is the entry point
//! and nothing else, so the serve loop stays testable as a library function rather than as
//! a `main`.
//!
//! It is deliberately hostile to being run by hand: it takes no defaults, and it expects
//! descriptors on numbers only its parent can have placed there.

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match kayfabe_isolate_host::child::ChildArgs::parse(args) {
        Ok(args) => std::process::ExitCode::from(
            u8::try_from(kayfabe_isolate_host::child::serve(&args)).unwrap_or(1),
        ),
        Err(why) => {
            eprintln!("kayfabe-isolate: {why}");
            std::process::ExitCode::from(64)
        }
    }
}
