//! `Containers` over the Podman CLI: `podman run --rm` with the spec's
//! mounts, env file and timeout (SLICE-8-7).

use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::run::{ContainerExit, ContainerOutput, ContainerSpec, Containers};

pub struct Podman;

/// Podman's own exit status when it couldn't run the container.
const PODMAN_FAILED: i32 = 125;
/// What podman exits with when `--timeout` stops the container: 255 (seen
/// with podman 5 on macOS), or 128 plus SIGKILL or SIGTERM.
const TIMEOUT_STATUSES: [i32; 3] = [137, 143, 255];

impl Containers for Podman {
    fn run(&self, spec: &ContainerSpec) -> ContainerOutput {
        let mut command = Command::new("podman");
        command
            .args(["run", "--rm", "--timeout"])
            .arg(spec.timeout_secs.to_string())
            .arg("--env-file")
            .arg(&spec.env_file);
        for m in &spec.mounts {
            let ro = if m.read_only { ":ro" } else { "" };
            command
                .arg("-v")
                .arg(format!("{}:{}{ro}", m.host.display(), m.container));
        }
        command.arg(&spec.image).args(&spec.command);

        let started = Instant::now();
        let out = match command.output() {
            Ok(out) => out,
            Err(e) => {
                return ContainerOutput {
                    exit: ContainerExit::FailedToStart(format!("run podman: {e}")),
                    stdout: String::new(),
                    stderr: String::new(),
                };
            }
        };
        let elapsed = started.elapsed();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let code = out
            .status
            .code()
            .unwrap_or_else(|| 128 + out.status.signal().unwrap_or(0));
        let exit = if code == PODMAN_FAILED {
            ContainerExit::FailedToStart(format!("podman could not run the container: {}", stderr.trim()))
        } else if TIMEOUT_STATUSES.contains(&code) && elapsed >= Duration::from_secs(spec.timeout_secs) {
            ContainerExit::TimedOut
        } else {
            ContainerExit::Exited(code)
        };
        ContainerOutput { exit, stdout, stderr }
    }
}
