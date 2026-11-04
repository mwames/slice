#!/usr/bin/env bash
# Builds the Rust agent image for this repository's Cargo.lock.
#
# Dependencies are vendored here, on the host, with your own git credentials;
# the image build only copies them in. The tag carries a hash of Cargo.lock,
# so a changed lockfile gets a new image instead of a stale one.
set -euo pipefail

claude_version=2.1.291

repo=$(git rev-parse --show-toplevel)
tag="slice-rust:$(shasum -a 256 "$repo/Cargo.lock" | cut -c1-12)"

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT

# Not --quiet: that also drops the source-replacement config printed on stdout.
(cd "$ctx" && cargo vendor --locked --manifest-path "$repo/Cargo.toml" vendor 2>/dev/null) \
  | sed 's#^directory = "vendor"#directory = "/opt/vendor"#' > "$ctx/cargo-config.toml"
cp "$repo/Cargo.toml" "$repo/Cargo.lock" "$repo/agents/rust/Containerfile" "$ctx/"

podman build --build-arg CLAUDE_VERSION="$claude_version" --tag "$tag" "$ctx"
echo "$tag"
