#!/usr/bin/env bash
# Builds the Node agent image for a repository's package.json and
# package-lock.json.
#
# usage: agents/build-node-image.sh [repo]   (default: the current repository)
#
# The tag hashes everything the image is built from, so a changed lockfile,
# Containerfile or Claude Code version gets a new image instead of a stale one.
set -euo pipefail

claude_version=2.1.291

here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "${1:-.}" rev-parse --show-toplevel)
name=$(basename "$repo" | tr '[:upper:]' '[:lower:]')
hash=$( (cat "$repo/package.json" "$repo/package-lock.json" "$here/node/Containerfile"; echo "$claude_version") \
  | shasum -a 256 | cut -c1-12)
tag="$name-node:$hash"

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT

cp "$repo/package.json" "$repo/package-lock.json" "$here/node/Containerfile" "$ctx/"

podman build --build-arg CLAUDE_VERSION="$claude_version" --tag "$tag" "$ctx" >&2
echo "$tag"
