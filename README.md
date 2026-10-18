# Tunewire

A native music player for SoundCloud, YouTube Music, YouTube and Spotify links, built with Tauri v2 + Rust. Audio is decoded and played by **rodio** (symphonia) in-process. There's no Electron, no mpv/ffplay, no API keys, and no OAuth. All search, playlist and stream extraction goes through **yt-dlp**.

```
src/                       vanilla HTML/CSS/JS frontend (no framework, no bundler)
  index.html  styles.css  brands.js (platform logos, simple-icons CC0)
  js/  core · tracks · search · player · queue · library · artist · explore · offline · lyrics
       themes · plugins · foryou · shell · cloud · social · update · settings · shortcuts · boot
       (ordered classic scripts)
  js/lazy/  imports · wrapped · gallery · backup   (loaded on first use)
src-tauri/src/
  lib.rs         app setup, tray, command registration
  state.rs       shared state, playback snapshot, events
  controller.rs  resolve → stream → play, platform fallback, gapless preloading
  commands/      playback · queue · library · discover · offline · customize · system · cloud · extras …
  ytdlp.rs       yt-dlp runner (≤3 processes, retry/backoff, error mapping, bundled copy)
  search.rs      normalization, fuzzy dedup, cross-platform grouping, 5-min cache
  spotify.rs     Spotify metadata             spotify_play.rs  Spotify Premium playback (librespot)
  artist.rs      artist & album pages         lyrics.rs  LRCLIB lyrics
  stream.rs      disk-backed HTTP (ranged, random access) / HLS / local reader
  decoder.rs     symphonia decoder + Opus (libopus)
  audio.rs       rodio engine: gapless, crossfade, loudness normalization, EQ
  media.rs       OS media controls (media keys, Windows overlay, MPRIS)
  stats.rs       play log, stats and Wrapped   foryou.rs  Daily Mixes, Discover Weekly
  cloud.rs       accounts (Discord sign-in, keychain refresh token)
  scrobble.rs    Last.fm / ListenBrainz        backup.rs  backup & restore
  gallery.rs     theme/plugin gallery (checksum-verified installs)
  css_guard.rs   checks theme and plugin CSS for anything that loads from the internet
  report.rs      opt-in error reports (through the `report` Edge Function)
  store.rs       settings + library persistence (atomic, fsync'd, daily snapshots)
  downloads.rs  localfiles.rs  themes.rs  plugins.rs  discord.rs  secrets.rs  error.rs  models.rs
  itests.rs      offline integration tests (fake yt-dlp + local HTTP server)
supabase/        database migrations, row level security tests, Edge Functions (report, share)
gallery/         themes and plugins offered in the in-app gallery (index.json is generated)
docs/            the website (GitHub Pages): landing page and share-link pages
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

**Standalone builds** (`cargo build`, `cargo build --release`, `cargo run`, `cargo tauri build`) embed `src/` at compile time and run without the dev server. Only `cargo tauri dev` loads the frontend from it. If the window says "Could not connect to 127.0.0.1", that binary was built by `cargo tauri dev`; start it through `cargo tauri dev` again or rebuild with `cargo build --release`.

## Releases and auto-update

Tunewire updates itself with `tauri-plugin-updater`. Releases live on GitHub, and every update is verified against the public key built into the app before it's installed. Settings → Tools → Updates has "Check now" and the startup check toggle.

- **Signing key:** `~/.tauri/tunewire.key`, with its password in `~/.tauri/tunewire.key.password`. Never commit these; `.gitignore` excludes them. If the key is lost, existing installs can no longer verify updates, so back it up.
- **CI:** `.github/workflows/release.yml` builds Linux (AppImage, .deb, .rpm), Windows (NSIS) and macOS (universal .dmg/.app) on every `v*` tag, signs them, and publishes a release with `latest.json`.
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

**Playback.** The stream is resolved first: SoundCloud through its own API, everything else through `yt-dlp` (a long-lived yt-dlp process where possible, so a song doesn't pay for Python's startup). Formats are chosen AAC/MP3 first (cheap, precise seeking), then Opus/WebM (decoded with libopus). The URL is checked (http(s) only, never this computer or the local network), then downloaded into a temporary file in ranged chunks that grow from 512 KB to 4 MB, or segment by segment for SoundCloud's HLS. Playback starts after the first chunk. The temporary files are deleted when a song is done and at startup.

Seeking:
- **Progressive streams:** symphonia seeks directly; a position that hasn't downloaded yet is fetched next with a range request.
- **HLS:** the decoder is rebuilt at the right segment (the `#EXTINF` timings are known), which is downloaded next if it isn't there yet, then trimmed to the exact position.
- Audio buffered before the seek is dropped at once, so the old position never plays on.

The decoder runs on its own thread and feeds a bounded channel. The rodio source only polls that channel, so a slow network gives a "Buffering…" indicator and silence, not a frozen audio device. If the output device disappears (headphones unplugged, Bluetooth gone), playback continues on the current default output.

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
- **Scrobbling.** Settings → Scrobbling connects Last.fm (approve in the browser) or ListenBrainz (paste your token). A song is scrobbled after half of it (or 4 minutes) has played; "now playing" is sent when it starts. Listens that can't be sent wait in `scrobbles.json` and are retried. Credentials are kept in the OS keychain. Builds get a Last.fm API account from `TUNEWIRE_LASTFM_KEY`/`TUNEWIRE_LASTFM_SECRET`; users can also enter their own.
- **Backup and restore.** Settings → Backup saves one `.tunewire-backup` file (playlists, Liked Songs, followed artists, history, the listening log, custom themes; never passwords or audio). Restoring saves the current library first. `library.json` also keeps 7 daily snapshots in `backups/`, and a damaged file is repaired from the newest one automatically.
- **Gallery.** Themes → Gallery and Plugins → Gallery install curated items from `gallery/` in this repository. Every file is checked against the SHA-256 in `gallery/index.json`, and a plugin must ask for exactly the permissions the gallery shows. After changing `gallery/`, run `python3 scripts/build-gallery.py`.
- **Undo.** Deleting a playlist, removing songs from one and clearing history happen at once, with an Undo button.

## Online features (accounts, sharing, Listen Together, friends)

Optional, built on Supabase. Audio never touches the server, only song info (titles, artists, links).

- **Accounts are Discord only.** There are no guest accounts; the database refuses writes from anonymous sessions too.
- **Sharing.** Song links carry the song in the link. Playlist, profile and session links open the landing page (or a link-preview server, below). Links that change something (join a collaborative playlist or a session, play a song) ask first.
- **Library sync.** With Settings → Account → "Sync my library", every playlist and Liked Songs has a private online copy (only you can read it), so each device you sign in to has the same library: songs, names, descriptions, covers, folders and pins. It's off until you turn it on. Turning it off on one computer stops syncing there; your other devices keep their copies.
- **How syncing works** (`src/js/sync.js`, `syncops.js`). For each linked playlist, `sync.json` keeps the songs and details as they were at the last sync. What changed here (local vs that) goes out as small operations (`playlist_op`: add, remove, move, order); whether anything changed elsewhere comes from the version number. When both sides changed, the server's copy, which already holds ours, is taken. Since the last-sync state and the library are both on disk, edits made offline still go out after a restart. Every change carries the device's id: a device skips only its own changes, not your other devices'. Deletes are tombstones (`deleted_at`) so other devices learn about them; if many playlists vanish locally at once, nothing is deleted online without asking.
- **Collaborative playlists.** Invite codes are readable only by the owner (through `playlist_invite()`) and can be reset. Edits go out the same way and arrive as numbered change rows (`playlist_changes`); a gap means the playlist is fetched again. Each shared playlist shows its sync status, who's editing, and a Retry when something didn't sync.
- **Listen Together.** A session is a `jams` row only the host can change; members are checked by the server; the host can hand over, remove people and approve suggested songs. Guests chat, suggest songs and vote to skip. Positions line up with the server's clock. A "room" keeps the same link every time.
- **Profiles.** A picture and banner of your own (PNG, JPEG, WebP or animated GIF, up to 8 MB) in the public `profile-media` Storage bucket, each person only in their own folder. A new account starts with its Discord picture and banner; signing in again never changes them.
- **Friends.** Following needs approval. What you play and your Wrapped are only visible to approved followers, and sharing what you play is off until you turn it on. Notifications cover follow requests, accepted follows, playlist activity and friends' sessions.

**Setting up a project**
1. `supabase link`, then `supabase db push` (migrations in `supabase/migrations`).
2. Auth → Providers: enable **Discord** (client ID and secret from discord.com/developers), turn **anonymous sign-ins off**, and allow the redirect URLs `http://127.0.0.1:5590/callback` … `http://127.0.0.1:5594/callback`.
3. Error reports: `supabase secrets set DISCORD_WEBHOOK=<webhook url>` and `supabase functions deploy report`. The webhook never ships in the app.
4. Link previews (optional): Supabase serves function HTML as plain text on `*.supabase.co`, so deploy `supabase/functions/share` on a custom domain (or any Deno host) with `SUPABASE_URL`, `SUPABASE_ANON_KEY` and `SHARE_PUBLIC_URL`, and build the app with `TUNEWIRE_SHARE_URL` set to the same address. Without it, share links go to the landing page.
5. Build with `TUNEWIRE_SUPABASE_URL` and `TUNEWIRE_SUPABASE_ANON_KEY` (CI secrets).

**Tests:** `scripts/test-db.sh` applies every migration to a throwaway Postgres (Docker) and runs the row level security tests in `supabase/tests`. `deno test supabase/functions` tests the Edge Functions. `deno test --allow-read scripts/syncops_test.ts` checks the sync operations against a copy of the server's logic, and `scripts/test-sync.sh` runs library sync end to end: two devices on one account against a local Supabase (needs the Supabase CLI).

## Privacy

- Error reports are **off** until you agree (a question on first start; Settings → Privacy). Reports contain the app version, operating system and the error, with folder names and stream links removed.
- Your library stays on your computer unless you turn on library sync (private copies only you can read). An account is only used for the online features above.

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
| Tab, then ↑ / ↓ | Move into a song list and pick a song (Shift: select several; Home/End) |
| Enter | Play the focused song |
| Shift+F10 or Menu key | Song menu |
| 1–7 | Switch view |
| ? | Shortcut help |

## Data locations

All data lives in the Tauri app data dir for `app.tunewire.player`:
- Linux: `~/.local/share/app.tunewire.player/`
- macOS: `~/Library/Application Support/app.tunewire.player/`
- Windows: `%APPDATA%\app.tunewire.player\`

Contents: `settings.json`, `library.json` (playlists, favorites, followed artists), `history.json`, `downloads.json`, `plays.jsonl` (listening log), `scrobbles.json` (waiting scrobbles), `backups/` (daily snapshots, pre-restore copies), `sync.json` (each synced playlist as of its last sync, and this device's id), `themes/`, `plugins/`. Window size and position are saved by `tauri-plugin-window-state`. Volume, last source, theme, platform order and plugin state are saved in `settings.json`.

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

- **Spotify:** without Premium, playback is a matched upload, and a match can occasionally be wrong. With Premium (Settings → Spotify) Spotify's own audio plays through librespot, an unofficial client; Spotify's terms don't allow third-party clients, so use it knowing that. Search uses unofficial web endpoints that Spotify may change.
- **YouTube Music search:** yt-dlp's flat results carry no artist or duration. They're borrowed from YouTube results when the same video appears there; otherwise the artist is inferred from your query words.
- **Seeking ahead:** seeking past what has downloaded fetches that part first (a range request, or that HLS segment), so it takes one round trip to the server.
- **macOS:** the app isn't signed or notarized by Apple, so the first launch needs right-click → Open.
