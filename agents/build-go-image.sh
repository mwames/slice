#!/usr/bin/env bash
# Builds the Go agent image for a repository's go.mod and go.sum.
#
# usage: agents/build-go-image.sh [repo]   (default: the current repository)
#
# Modules are downloaded here, on the host, with your own credentials; the
# image build only copies them in. The tag hashes everything the image is
# built from, so a changed go.sum, Containerfile or Claude Code version gets a
# new image instead of a stale one.
set -euo pipefail

claude_version=2.1.291

here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "${1:-.}" rev-parse --show-toplevel)
name=$(basename "$repo" | tr '[:upper:]' '[:lower:]')
hash=$( (cat "$repo/go.mod" "$repo/go.sum" "$here/go/Containerfile"; echo "$claude_version") \
  | shasum -a 256 | cut -c1-12)
tag="$name-go:$hash"

ctx=$(mktemp -d)
# Module cache files are read-only; make them removable before cleaning up.
trap 'chmod -R u+w "$ctx" 2>/dev/null; rm -rf "$ctx"' EXIT

# GOWORK=off: a local go.work could point a module at a checkout on the host,
# and the module would then be missing from the image.
(cd "$repo" && GOWORK=off GOMODCACHE="$ctx/modcache" GOFLAGS=-modcacherw go mod download)
cp "$here/go/Containerfile" "$ctx/"

podman build --build-arg CLAUDE_VERSION="$claude_version" --tag "$tag" "$ctx" >&2
echo "$tag"
