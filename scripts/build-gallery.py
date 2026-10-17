#!/usr/bin/env python3
"""Writes gallery/index.json from the files in gallery/ (checksums included).

Run after adding or changing a gallery theme or plugin:
    python3 scripts/build-gallery.py
"""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent.parent / "gallery"
DESCRIPTIONS = {
    "forest": "Deep greens and soft mint, easy on the eyes at night.",
    "aurora": "Night-sky navy with a glowing aurora green.",
    "paper": "A warm light theme, like a printed page.",
    "ember": "Dark coal with a warm orange glow.",
}


def entry(path: Path) -> dict:
    return {"path": path.relative_to(root).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


themes = []
for f in sorted((root / "themes").glob("*.json")):
    t = json.loads(f.read_text())
    v = t["vars"]
    themes.append({
        "id": f.stem, "name": t["name"], "author": t.get("author", ""),
        "description": DESCRIPTIONS.get(f.stem, ""),
        "preview": [v.get("--bg", "#111"), v.get("--accent", "#4d8dff"), v.get("--text", "#eee")],
        "file": entry(f),
    })

plugins = []
for d in sorted(p for p in (root / "plugins").iterdir() if p.is_dir()):
    m = json.loads((d / "manifest.json").read_text())
    plugins.append({
        "id": d.name, "name": m["name"], "author": m.get("author", ""), "description": m.get("description", ""),
        "version": m.get("version", ""), "permissions": m.get("permissions", []),
        "files": [entry(f) for f in sorted(d.iterdir()) if f.is_file()],
    })

(root / "index.json").write_text(json.dumps({"themes": themes, "plugins": plugins}, indent=2, ensure_ascii=False) + "\n")
print(f"gallery: {len(themes)} themes, {len(plugins)} plugins")
