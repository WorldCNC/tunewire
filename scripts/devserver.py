#!/usr/bin/env python3
"""Dev server for the frontend.

    python3 scripts/devserver.py            # http://127.0.0.1:1420

- Serves src/ with live reload: the page reloads itself ~1 s after any file
  in src/ changes (no rebuild needed).
- Inside `cargo tauri dev` the app window loads this server (see devUrl in
  tauri.conf.json), so the real backend is used.
- Opened in a normal browser there is no Tauri backend, so dev/mock.js fakes
  one with sample data — handy for UI work with browser devtools.
"""
import http.server
import json
import os
import socketserver
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src"
DEV = ROOT / "dev"
THEMES = ROOT / "src-tauri" / "themes"
PORT = int(os.environ.get("TUNEWIRE_DEV_PORT", "1420"))
# Benchmarks serve a different tree (an older checkout): TUNEWIRE_DEV_ROOT.
if os.environ.get("TUNEWIRE_DEV_ROOT"):
    ROOT = Path(os.environ["TUNEWIRE_DEV_ROOT"]).resolve()
    SRC, DEV, THEMES = ROOT / "src", ROOT / "dev", ROOT / "src-tauri" / "themes"

INJECT = """<script>
/* Tunewire dev: live reload + mock backend when opened in a plain browser. */
(function () {
  let last = null;
  setInterval(async () => {
    try {
      const m = await (await fetch('/__mtime', { cache: 'no-store' })).text();
      if (last && m !== last) location.reload();
      last = m;
    } catch (_) {}
  }, 700);
})();
</script>
<script src="/__dev/mock.js"></script>
"""


def latest_mtime() -> str:
    newest = 0.0
    for p in SRC.rglob("*"):
        if p.is_file():
            newest = max(newest, p.stat().st_mtime)
    return str(newest)


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(SRC), **kw)

    def log_message(self, fmt, *args):  # quiet
        pass

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def _send(self, body: bytes, ctype: str):
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/__mtime":
            return self._send(latest_mtime().encode(), "text/plain")
        if path.startswith("/__dev/"):
            f = DEV / path[len("/__dev/"):]
            if f.is_file() and DEV in f.resolve().parents:
                return self._send(f.read_bytes(), "text/javascript")
        if path.startswith("/__gallery/"):
            g = ROOT / "gallery"
            f = (g / path[len("/__gallery/"):]).resolve()
            if f.is_file() and g.resolve() in f.parents:
                return self._send(f.read_bytes(), "application/json")
        if path.startswith("/__themes/"):
            f = THEMES / Path(path).name
            if f.is_file():
                return self._send(f.read_bytes(), "application/json")
        if path in ("/", "/index.html"):
            html = (SRC / "index.html").read_text(encoding="utf-8")
            # Before any app script so the mock can define window.__TAURI__.
            html = html.replace('<script src="brands.js"></script>', INJECT + '<script src="brands.js"></script>', 1)
            return self._send(html.encode(), "text/html; charset=utf-8")
        return super().do_GET()


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


if __name__ == "__main__":
    with Server(("127.0.0.1", PORT), Handler) as httpd:
        print(f"Tunewire dev server: http://127.0.0.1:{PORT}  (serving {SRC})", flush=True)
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            sys.exit(0)
