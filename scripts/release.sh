#!/usr/bin/env bash
# Publish a new version: ./scripts/release.sh 1.0.5
# Bumps the version, checks the changelog has a section for it, runs the
# tests, commits, tags and pushes. GitHub Actions builds, signs and publishes
# (Windows + Linux, ~6-8 min); installed apps pick it up within the hour.
set -euo pipefail
cd "$(dirname "$0")/.."
v="${1:?usage: scripts/release.sh <version>}"
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 1.2.3"; exit 1; }
git diff --quiet && git diff --cached --quiet || { echo "commit or stash your changes first"; exit 1; }
grep -q "^## v$v\$" CHANGELOG.md || { echo "add a '## v$v' section to CHANGELOG.md first (it becomes the release notes)"; exit 1; }
sed -i "0,/\"version\": \"[^\"]*\"/s//\"version\": \"$v\"/" src-tauri/tauri.conf.json
sed -i "0,/^version = \"[^\"]*\"/s//version = \"$v\"/" src-tauri/Cargo.toml
(cd src-tauri && cargo update -p tunewire --offline -q && cargo test -q)
git commit -qam "Release v$v"
git tag "v$v"
git push -q && git push -q origin "v$v"
echo "Released v$v: https://github.com/WorldCNC/tunewire/actions"
