#!/bin/sh
# Bump the version everywhere, commit, tag and push → GitHub builds the release.
# Usage: scripts/release.sh 0.2.0
set -e
V="$1"
[ -n "$V" ] || { echo "usage: $0 <version, e.g. 0.2.0>"; exit 1; }
cd "$(dirname "$0")/.."
sed -i.bak "s/^version = \".*\"/version = \"$V\"/" src-tauri/Cargo.toml && rm src-tauri/Cargo.toml.bak
python3 - "$V" <<'PY'
import json, sys
p = "src-tauri/tauri.conf.json"
c = json.load(open(p)); c["version"] = sys.argv[1]
json.dump(c, open(p, "w"), indent=2); open(p, "a").write("\n")
PY
(cd src-tauri && cargo update -p tunewire --quiet)
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json
git commit -m "Release v$V"
git tag "v$V"
git push && git push origin "v$V"
echo "Tag v$V pushed. Watch the build under the repository's Actions tab."
