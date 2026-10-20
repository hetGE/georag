//! Repository tasks: release bundling, manifest generation, signing helpers.
//! Run with `cargo xtask <task>`. Tasks land with the P0 CI step.

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <bundle|manifest>\n  bundle    package georag, engine/ and config/*.toml.default for this flavour\n  manifest  write manifest.json for a set of bundles";

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "bundle" | "manifest" => {
            eprintln!("xtask {task}: not implemented yet (P0 CI step)");
            ExitCode::from(2)
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
