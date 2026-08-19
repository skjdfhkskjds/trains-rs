use std::env;
use std::path::Path;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace = Path::new(manifest_dir)
        .parent()
        .expect("xtask must be inside the workspace");

    let status = Command::new(workspace.join("scripts/build.sh"))
        .current_dir(workspace)
        .status()
        .expect("failed to start scripts/build.sh");

    if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
