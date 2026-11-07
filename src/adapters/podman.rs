//! `Containers` over the Podman CLI: `podman run --rm` with the spec's
//! mounts, env file and timeout (SLICE-8-7).

use crate::run::{ContainerOutput, ContainerSpec, Containers};

pub struct Podman;

impl Containers for Podman {
    fn run(&self, spec: &ContainerSpec) -> ContainerOutput {
        todo!("SLICE-8-7")
    }
}
