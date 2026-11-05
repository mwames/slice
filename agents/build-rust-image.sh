#!/usr/bin/env bash
# Builds the Rust agent image for this repository's Cargo.lock.
#
# Dependencies are vendored here, on the host, with your own git credentials;
# the image build only copies them in. The tag hashes everything the image is
# built from, so a changed lockfile, Containerfile or Claude Code version gets
# a new image instead of a stale one.
set -euo pipefail

claude_version=2.1.291

repo=$(git rev-parse --show-toplevel)
hash=$( (cat "$repo/Cargo.lock" "$repo/agents/rust/Containerfile"; echo "$claude_version") \
  | shasum -a 256 | cut -c1-12)
tag="slice-rust:$hash"

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT

# Not --quiet: that also drops the source-replacement config printed on stdout.
(cd "$ctx" && cargo vendor --locked --manifest-path "$repo/Cargo.toml" vendor 2>/dev/null) \
  | sed 's#^directory = "vendor"#directory = "/opt/vendor"#' > "$ctx/cargo-config.toml"
cp "$repo/Cargo.toml" "$repo/Cargo.lock" "$repo/agents/rust/Containerfile" "$ctx/"

podman build --build-arg CLAUDE_VERSION="$claude_version" --tag "$tag" "$ctx"
echo "$tag"
