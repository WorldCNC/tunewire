# Tunewire

A native music player for SoundCloud, YouTube Music, YouTube and Spotify links, built with Tauri v2 + Rust. Audio is decoded and played by **rodio** (symphonia) in-process. There's no Electron, no mpv/ffplay, no API keys, and no OAuth. All search, playlist and stream extraction goes through **yt-dlp**.

```
src/                       vanilla HTML/CSS/JS frontend (no framework, no bundler)
  index.html  styles.css  brands.js (platform logos, simple-icons CC0)
  js/  core · tracks · search · player · queue · library · artist · offline · lyrics
       themes · plugins · settings · shortcuts · boot   (ordered classic scripts)
src-tauri/src/
  lib.rs         app setup, tray, command registration
  state.rs       shared state, playback snapshot, events
  controller.rs  resolve → stream → play, platform fallback, gapless preloading
  commands/      playback · queue · library · discover · offline · customize · system
  ytdlp.rs       yt-dlp runner (≤3 processes, retry/backoff, error mapping, bundled copy)
  spotdl.rs      spotDL runner (Spotify search & playlists)
  search.rs      normalization, fuzzy dedup, cross-platform grouping, 5-min cache
  spotify.rs     Spotify embed-page metadata     artist.rs  artist & album pages
  lyrics.rs      LRCLIB lyrics                   secrets.rs OS keychain
  stream.rs      disk-backed HTTP (ranged, random access) / HLS / local reader
  decoder.rs     symphonia decoder + Opus (libopus)
  audio.rs       rodio engine: gapless, crossfade, loudness normalization
  downloads.rs   offline downloads               localfiles.rs  local music library
  store.rs  themes.rs  plugins.rs  discord.rs  error.rs  models.rs
  itests.rs      offline integration tests (fake yt-dlp + local HTTP server)
```

## Requirements

- **yt-dlp, recent version, on your `PATH`.** It's checked at startup. Site extractors break often, so keep it updated (`yt-dlp -U`, `pipx upgrade yt-dlp`, or your package manager). You can point to a specific binary with `TUNEWIRE_YTDLP=/path/to/yt-dlp`.
- Rust **1.90+**.
- Tauri v2 system dependencies:
  - **Linux:** `webkit2gtk-4.1`, `gtk3`, `libayatana-appindicator` (tray), `alsa-lib`, plus `pkg-config` and a C toolchain.
    Debian/Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev libasound2-dev pkg-config build-essential`
    Arch: `sudo pacman -S webkit2gtk-4.1 gtk3 libayatana-appindicator alsa-lib pkgconf base-devel`
  - **macOS:** Xcode Command Line Tools.
  - **Windows:** MSVC Build Tools + WebView2 (preinstalled on Windows 10/11).

## Build & run

```sh
cd src-tauri
cargo run                    # debug build, starts the app
cargo build --release        # optimized binary: src-tauri/target/release/tunewire(.exe)
cargo test                   # unit + offline integration tests (fake yt-dlp, local HTTP server)
cargo test -- --ignored --nocapture   # live tests: real yt-dlp/spotDL, YouTube Music, SoundCloud, Opus,
                                      # artist pages, lyrics, downloads, keychain
# real-time engine test (gapless + crossfade) on a silent ALSA device:
printf '</usr/share/alsa/alsa.conf>\npcm.!default { type null }\n' > /tmp/null.conf
ALSA_CONFIG_PATH=/tmp/null.conf cargo test -- --ignored engine
```

To get installers (.deb/.AppImage/.rpm, .dmg/.app, .msi/.exe), use the Tauri CLI:

```sh
cargo install tauri-cli --version "^2" --locked
cd src-tauri && cargo tauri build
```

For macOS bundles, first generate an `.icns`: `cargo tauri icon icons/icon.png`.

## Developing without rebuilding

**Live app.** Run this from `src-tauri`:
```sh
cargo tauri dev
```
It starts `scripts/devserver.py` and opens a debug build that loads the frontend from it:
- **Frontend (`src/`):** saving any HTML, CSS or JS file reloads the window within about a second, with no rebuild.
- **Rust (`src-tauri/src/`):** changes trigger an automatic incremental rebuild and restart, about 10–20 s instead of the 2-minute release build.
- **Devtools:** right-click → Inspect element.

Install the CLI once with `cargo install tauri-cli --version "^2" --locked`. The dev server needs Python 3.

**Browser preview (UI only):**
```sh
python3 scripts/devserver.py      # open http://127.0.0.1:1420 in any browser
```
Outside the app there's no Rust backend, so `dev/mock.js` stands in with sample songs, an artist page, synced lyrics, a queue and simulated playback. This is the fastest loop for styling, and it keeps live reload. The mock is injected only by the dev server and is never part of a release build.

**Release builds** embed `src/` at compile time, so only `cargo build --release` / `cargo tauri build` need a full rebuild.

## Releases and auto-update

Tunewire updates itself with `tauri-plugin-updater`. Releases live on GitHub, and every update is verified against the public key built into the app before it's installed. Settings → Tools → Updates has "Check now" and the startup check toggle.

- **Signing key:** `~/.tauri/tunewire.key`, with its password in `~/.tauri/tunewire.key.password`. Never commit these; `.gitignore` excludes them. If the key is lost, existing installs can no longer verify updates, so back it up.
- **CI:** `.github/workflows/release.yml` builds Linux (AppImage, .deb, .rpm) and Windows (NSIS, MSI) on every `v*` tag, signs them, and publishes a release with `latest.json`.
- **Repository secrets:** `TAURI_SIGNING_PRIVATE_KEY` (the key file's contents) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- **Cutting a release:** `scripts/release.sh 0.2.0` bumps the version, commits, tags and pushes; GitHub Actions does the rest.
- **What updates:** auto-update works for the AppImage, the Windows installer and the macOS app. `.deb`/`.rpm` installs update through the package manager.

## How it works

**Search.** Each source maps to one yt-dlp flat listing:

| Source        | yt-dlp target                                          |
|---------------|--------------------------------------------------------|
| SoundCloud    | `scsearchN:QUERY`                                      |
| YouTube       | `ytsearchN:QUERY`                                      |
| YouTube Music | `https://music.youtube.com/search?q=QUERY#songs`       |
| All           | the above in parallel (YouTube can be turned off in Settings) |

Results are normalized:
- case and punctuation are folded and diacritics stripped
- "Artist - Title" uploads are split
- `- Topic` and `VEVO` are removed from channel names
- packaging noise is removed: Official Video/Audio, Lyrics, Visualizer, Remaster(ed), Live, HD/4K, `[label tags]`, and feat./ft. credits

Matching is fuzzy: token Jaccard combined with normalized Levenshtein on titles, artist token overlap, and a ±7 s / 4 % duration check. Variant markers (remix, edit, acoustic, sped up, …) must agree, so "Song" and "Song (Remix)" never merge. Same-platform duplicates are collapsed so each song appears once per platform (a 30 s SoundCloud Go+ preview loses to a full upload). Matches across platforms become one logical track with several versions. Ranking combines query relevance, rank inside each platform, your platform preference, and how many platforms have the song. Raw per-platform results are cached in memory for 5 minutes, so switching between All and a single source, or re-ranking, doesn't spawn yt-dlp again.

**Playback.** `yt-dlp -j -f <audio format> --no-playlist URL` is the `-g -f bestaudio` lookup, but it also returns the format's HTTP headers, codec and duration. Formats are chosen AAC/MP3 first because symphonia has no Opus decoder; YouTube always offers m4a. The URL is validated, then streamed into memory in 512 KB ranged chunks, or segment by segment for SoundCloud's HLS. Playback starts after the first chunk; nothing is written to disk.

Seeking:
- **Progressive streams:** symphonia seeks directly.
- **HLS:** the decoder is rebuilt at the right segment (the `#EXTINF` timings are known), then trimmed to the exact position.

The decoder runs on its own thread and feeds a bounded channel. The rodio source only polls that channel, so a slow network gives a "Buffering…" indicator and silence, not a frozen audio device.

**Version choice and fallback.** Clicking a song that has several versions opens the version picker, with the highest-priority platform preselected. Double-click (or the ▶ button) plays the default right away, and the platform badges play that specific version. If extraction fails, the next platform in your order is tried. If every version fails, Tunewire searches for an equivalent upload as a last resort.

**Spotify.** Spotify audio is DRM-protected, so it always plays through the best YouTube Music or SoundCloud match. Metadata comes from three sources:
- **Search:** Spotify's own web search, the same GraphQL endpoint `open.spotify.com/search` uses, with the anonymous token every public embed page carries. No account, API key or spotDL needed. Results arrive in about 0.7 s once the token is cached. If Spotify changes the search query ID, the current one is re-read from the web player's JavaScript automatically.
- **Pasted track, album and playlist links:** the public embed page, which is instant.
- **spotDL (optional):** a fallback for both, if installed. Its built-in Spotify credentials are shared and often rate-limited; your own app's ID and secret (Settings → Spotify, redirect URI `http://127.0.0.1:9900/`) fix that.

This relies on unofficial Spotify endpoints, so it can break when Spotify changes their site; the fallbacks keep search working when spotDL is installed. Choose the mode in Settings → Spotify.

**Discord Rich Presence.** Settings → Discord. Shows "Listening to <song>" with artwork, artist, a progress bar and an optional "Open on <platform>" button, over Discord's local IPC (`discord-rich-presence` crate). Updates are rate-limited to Discord's limits, and it reconnects automatically if Discord restarts. The activity name comes from the Discord application ID in Settings. Create your own app named "Tunewire" at discord.com/developers and paste its ID. **Vesktop users:** enable Vesktop Settings → arRPC, otherwise there is no Discord socket to connect to. Settings → Discord shows a live connection status.

**Reliability.**
- At most 3 yt-dlp processes run at once, with a 45–60 s timeout each.
- Transient failures are retried with exponential backoff, both for yt-dlp (network, 5xx, 429) and for chunk downloads.
- yt-dlp errors are mapped to typed, user-readable errors: `yt_dlp_missing`, `drm`, `region_locked`, `unavailable`, `network`, `timeout`, `extraction`, `decode`, `audio_device`, `plugin`, …
- A playlist skips dead tracks automatically, but stops after 5 failures in a row.

## More features

- **Gapless playback and crossfade.** The next song starts decoding about 25 s before the current one ends, so songs follow each other with no gap. Optional crossfade of 1–12 s (Settings → Sound).
- **Loudness normalization.** Each song's level is measured from its first seconds and adjusted toward a common target, with a soft limiter against clipping (Settings → Sound).
- **Opus/WebM playback** through libopus, so sources that only offer Opus also play.
- **Disk-backed streaming.** Streams are written to a temporary file instead of RAM. Seeking past what has downloaded makes the downloader jump there with an HTTP range request instead of waiting for everything before it.
- **Synced lyrics** in the Lyrics tab next to the queue, from LRCLIB (free, no key). Click a line to jump there.
- **Artist and album pages.** Click any artist name. The page shows songs from the artist's YouTube Music channel, their SoundCloud tracks, and every release from the official YouTube channel's Releases tab.
- **Offline downloads.** "Download" in a song's menu or on an album or playlist. Downloaded songs play from disk even offline, and are tried first.
- **Local files.** Add music folders; their files are indexed by tags, appear under Local files, and are included in "All" searches.
- **Better YouTube Music results.** Missing artist and duration are borrowed from a parallel YouTube search, and anything still guessed is completed in the background.
- **Spotify secret in the OS keychain** (Secret Service, Windows Credential Manager or macOS Keychain), not in `settings.json`.
- **Bundled yt-dlp (optional).** Settings → Tools → "Install copy" downloads the official yt-dlp build from GitHub, verifies its SHA-256, and uses it. "Update copy" replaces it with the latest release.

## Keyboard shortcuts

| Key | Action |
|---|---|
| Space | Play / pause |
| ← / → | Seek −5 s / +5 s (Shift: 15 s) |
| ↑ / ↓ | Volume |
| M | Mute |
| N / P | Next / previous |
| L | Like current track |
| R | Cycle repeat (off / all / one) |
| Q | Toggle queue panel |
| / or Ctrl+K | Focus search (paste a URL there to play it) |
| 1–7 | Switch view |
| ? | Shortcut help |

## Data locations

All data lives in the Tauri app data dir for `app.tunewire.player`:
- Linux: `~/.local/share/app.tunewire.player/`
- macOS: `~/Library/Application Support/app.tunewire.player/`
- Windows: `%APPDATA%\app.tunewire.player\`

Contents: `settings.json`, `library.json` (playlists, favorites, last 500 history entries), `themes/`, `plugins/`. Window size and position are saved by `tauri-plugin-window-state`. Volume, last source, theme, platform order and plugin state are saved in `settings.json`.

## Themes

A theme is JSON: CSS custom properties plus optional extra CSS.

```json
{
  "name": "My Theme",
  "author": "you",
  "dark": true,
  "vars": { "--bg": "#101014", "--accent": "#ff8a3d", "--font": "\"Inter\", sans-serif" },
  "css": ".player-bar { backdrop-filter: blur(8px); }"
}
```

The Themes view has a live editor with color pickers for every variable, custom CSS, Save, Export and Import.
- **Export** writes `<id>.tunewire-theme.json` to your Downloads folder and copies the JSON to the clipboard.
- **Import** accepts a file or pasted JSON. Try `examples/themes/forest.tunewire-theme.json`.
- Built-in themes are never overwritten; editing one saves a copy.
- Values are validated: no `url()`, no `@import`, nothing that can break out of a declaration.

The variable list is in the `:root` block at the top of `src/styles.css`.

## Plugins

A plugin is a folder in `<data dir>/plugins/<id>/`:

```
manifest.json   { name, version, description, author, main, style?, permissions[], settings[] }
main.js         plugin code
style.css       optional, injected globally (prefix your class names)
```

Enable or disable plugins in the Plugins view; the state persists. **Reload** picks up edits.

**Isolation.** Each plugin runs in its own Web Worker built from a Blob.
- The worker has no DOM, no `window.__TAURI__`, and `self`/`globalThis` are shadowed.
- `fetch`, `XMLHttpRequest`, `WebSocket`, `importScripts`, nested workers and IndexedDB are removed unless the manifest requests `network`.
- Everything goes through a message API that the host checks against the manifest's permissions.
- Plugins can only call an allow-listed set of commands. No backend command can spawn a process or touch arbitrary files, so plugins cannot run system commands.
- Panel HTML is parsed with `DOMParser` and sanitized: no scripts, event handlers, URLs or iframes. Interactivity uses `data-action` attributes.

**Permissions.**

| Permission | Grants |
|---|---|
| `playback` | `get_playback_state`, `play_track`, `play_tracks`, `play_url`, `pause`, `resume`, `toggle_pause`, `stop`, `next_track`, `previous_track`, `seek`, `set_volume`, `set_repeat` |
| `queue` | `get_queue`, `add_to_queue`, `remove_from_queue`, `clear_queue`, `reorder_queue`, `shuffle_queue`, `play_index_cmd` |
| `search` | `search_tracks`, `load_playlist` |
| `library` | `list_playlists`, `create_playlist`, `add_to_playlist`, `list_favorites`, `toggle_favorite`, `list_history` |
| `ui` | panels, player-bar buttons, toasts |
| `storage` | small persistent key/value store |
| `network` | keeps `fetch`/`WebSocket` (shown as a warning in the plugin list) |

**API** (global `tunewire` object inside the worker):

```js
tunewire.on(event, fn)              // 'ready', 'track-started', 'track-ended', 'playback' (≤1/s),
                                    // 'queue-changed', 'search-completed', 'library-changed',
                                    // 'status', 'settings-changed', 'theme-changed', 'action'
tunewire.invoke(cmd, args)          // allow-listed IPC command → Promise
tunewire.playback.state() / toggle() / next() / previous() / seek(sec) / play(track, platform?)
tunewire.ui.addPanel({ id, title, html })   // adds a sidebar entry under "Plugin panels"
tunewire.ui.setPanel(id, html)              // re-render (sanitized)
tunewire.ui.openPanel(id) / removePanel(id)
tunewire.ui.onAction(name, fn)              // clicks/changes on [data-action="name"]; fn({ value, data, form })
tunewire.ui.addButton({ id, label, title }, onClick)  // small button in the player bar
tunewire.ui.toast(message, level)           // 'info' | 'ok' | 'warn' | 'error'
tunewire.settings.get(key) / all() / set(key, value)  // values declared in manifest "settings"
tunewire.storage.get(key) / set(key, value)           // needs "storage"
tunewire.log(...args)
```

Manifest settings (`type`: `text`, `number`, `toggle`, `select` with `options`) are rendered automatically in the Plugins view.

See `src-tauri/plugins/now-playing-notes/` for a complete example: a panel with per-track notes and session stats, a "More by this artist" button that uses `search_tracks` + `add_to_queue`, a player-bar button, and two settings.

## IPC commands

| Area | Commands |
|---|---|
| Health / settings | `health_check`, `get_settings`, `update_settings(patch)` |
| Search / extraction | `search_tracks(query, source, limit)`, `load_playlist(url)`, `extract_stream_url(url)`, `play_url(url)` |
| Playback | `play_track(track, platform?)`, `play_tracks(tracks, start)`, `play_index_cmd(index)`, `pause`, `resume`, `toggle_pause`, `stop`, `next_track`, `previous_track`, `set_volume(volume)`, `seek(position)`, `get_playback_state`, `set_repeat(mode)` |
| Queue | `get_queue`, `add_to_queue(tracks, next?)`, `remove_from_queue(uid)`, `clear_queue(keepCurrent?)`, `reorder_queue(from, to)`, `shuffle_queue`, `set_item_platform(uid, platform)` |
| Library | `list_playlists`, `create_playlist(name, tracks?, sourceUrl?)`, `rename_playlist`, `delete_playlist`, `add_to_playlist`, `remove_from_playlist`, `move_in_playlist`, `list_favorites`, `toggle_favorite`, `list_history`, `clear_history` |
| Themes | `list_themes`, `load_theme(id)`, `get_theme`, `save_theme(theme)`, `delete_theme`, `export_theme(id)`, `import_theme(json)` |
| Plugins | `list_plugins`, `enable_plugin(id)`, `disable_plugin(id)`, `get_plugin_source(id)`, `set_plugin_setting(id, key, value)`, `plugins_dir` |

Events: `playback` (about 4 Hz while playing), `queue-changed`, `track-started`, `track-ended`, `status`, `library-changed`.

## Known limitations

- **Spotify:** playback is always a matched upload, and a match can occasionally be wrong. Search uses unofficial web endpoints that Spotify may change.
- **YouTube Music search:** yt-dlp's flat results carry no artist or duration. They're borrowed from YouTube results when the same video appears there; otherwise the artist is inferred from your query words.
- **Seeking ahead:** seeking past what has downloaded waits for the download to reach that point. Downloads usually finish within seconds.
- **No Opus/WebM decoding:** symphonia lacks Opus, so the format selector avoids it. A source that only offers Opus fails over to the next platform.
- **Media keys:** not wired to OS media controls; use the tray menu or keyboard shortcuts.
