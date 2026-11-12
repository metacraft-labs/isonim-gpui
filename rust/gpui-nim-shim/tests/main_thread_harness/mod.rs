//! A minimal test runner for the integration-test targets declared
//! `harness = false` in `Cargo.toml` because their cases must run on the
//! process's MAIN thread.
//!
//! libtest runs every `#[test]` on a worker thread. On macOS gpui builds
//! its platform (`gpui_platform::current_platform` -> `MacPlatform::new`)
//! only on the main thread and panics anywhere else
//! (`gpui-pre-macos-0.3.5/src/platform.rs:210`), so a case that needs the
//! platform -- or the shim entry point that obtains it -- cannot be a
//! `#[test]`. Such a target is its own `main` and hands its cases to
//! [`run`].
//!
//! [`run`] runs the cases one after another on the calling (main)
//! thread, each under `catch_unwind` so one failure does not hide the
//! others. It prints libtest's per-case lines and its `test result:`
//! summary line, which `ci/run-suite.sh` counts, and exits 101 -- the
//! code libtest uses -- when any case failed, so the failure fails
//! `cargo test`. The default panic hook still prints each failing
//! assertion and its location. Running the cases in sequence also gives
//! them what `#[serial]` gave them as `#[test]`s.

/// One case: its name, as libtest would print it, and its body.
pub type Case = (&'static str, fn());

/// Run `cases` in order on the calling thread; exit 101 if any failed.
pub fn run(cases: &[Case]) {
    println!();
    println!("running {} tests", cases.len());
    let mut failed: Vec<&str> = Vec::new();
    for (name, case) in cases {
        let outcome = std::panic::catch_unwind(*case);
        println!("test {name} ... {}", if outcome.is_ok() { "ok" } else { "FAILED" });
        if outcome.is_err() {
            failed.push(name);
        }
    }
    let passed = cases.len() - failed.len();
    println!();
    if !failed.is_empty() {
        println!("failures:");
        for name in &failed {
            println!("    {name}");
        }
        println!();
    }
    println!(
        "test result: {}. {passed} passed; {} failed; 0 ignored; 0 measured; 0 filtered out",
        if failed.is_empty() { "ok" } else { "FAILED" },
        failed.len()
    );
    if !failed.is_empty() {
        std::process::exit(101);
    }
}
