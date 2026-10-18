/* Fake Tauri backend for previewing the UI in a normal browser.
 * Injected only by scripts/devserver.py, and only active when the page is NOT
 * running inside the Tauri app. Never bundled into release builds. */
(function () {
  'use strict';
  if (window.__TAURI__ || window.__TAURI_INTERNALS__) return;

  const DEMO = !!new URLSearchParams(location.search).get('demo');
  const hues = [[265, 320], [190, 240], [20, 340], [150, 200], [280, 210], [40, 10], [330, 280]];
  const art = (i) => {
    const [a, b] = hues[i % hues.length];
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 120"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${a},85%,60%)"/><stop offset="1" stop-color="hsl(${b},80%,35%)"/></linearGradient><radialGradient id="r" cx=".3" cy=".25" r=".7"><stop offset="0" stop-color="#fff" stop-opacity=".55"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient></defs><rect width="120" height="120" fill="url(#g)"/><circle cx="${40 + (i * 23) % 50}" cy="${50 + (i * 17) % 40}" r="${26 + (i % 3) * 8}" fill="url(#r)"/><circle cx="90" cy="92" r="${10 + i % 4 * 4}" fill="#fff" fill-opacity=".18"/></svg>`;
    return `data:image/svg+xml,${encodeURIComponent(svg)}`;
  };
  const N = (real, demo) => (DEMO ? demo : real);
  const yt = (id) => (DEMO ? art([...id].reduce((n, c) => n + c.charCodeAt(0), 0)) : `https://i.ytimg.com/vi/${id}/mqdefault.jpg`);
  const V = (platform, title, artist, duration, id, extra = {}) => ({
    platform, url: platform === 'local' ? id : `https://example.com/${platform}/${id}`, id, title, artist, duration,
    thumbnail: id.length === 11 ? yt(id) : null, preview: false, ...extra,
  });
  const T = (title, artist, duration, versions) => ({
    key: `${artist}::${title}`.toLowerCase(), title, artist, duration,
    thumbnail: versions.find((v) => v.thumbnail)?.thumbnail || null, versions, score: 1,
  });
  const tracks = [
    T('One More Time', 'Daft Punk', 321, [V('ytmusic', 'One More Time', 'Daft Punk', 320, 'fa5IWHDbftI'), V('soundcloud', 'One More Time', 'Daft Punk', 322, 'sc1'), V('youtube', 'One More Time (Official Video)', 'Daft Punk', 322, 'FGBhQbmPwH8'), V('spotify', 'One More Time', 'Daft Punk', 320, 'sp1')]),
    T('One More Time (Radio Edit)', 'Daft Punk', 236, [V('ytmusic', 'One More Time (Radio Edit)', 'Daft Punk', 236, 'wU26xVT_vBU')]),
    T('Harder, Better, Faster, Stronger', 'Daft Punk', 224, [V('ytmusic', 'Harder, Better, Faster, Stronger', 'Daft Punk', 224, 'GDpmVUEjagg'), V('soundcloud', 'Harder Better Faster Stronger', 'Daft Punk', 224, 'sc2')]),
    T('Get Lucky', 'Daft Punk, Pharrell Williams', 369, [V('youtube', 'Get Lucky', 'Daft Punk', 369, '5NV6Rdv1a3I'), V('spotify', 'Get Lucky', 'Daft Punk', 369, 'sp2')]),
    T('Digital Love', 'Daft Punk', 301, [V('ytmusic', 'Digital Love', 'Daft Punk', 301, 'Qm9EpPeVrn8'), V('soundcloud', 'Digital Love', 'Daft Punk', 301, 'sc4')]),
    T('Instant Crush', 'Daft Punk, Julian Casablancas', 337, [V('ytmusic', 'Instant Crush', 'Daft Punk', 337, 'a5uQMwRMHcs')]),
    T('Veridis Quo', 'Daft Punk', 345, [V('soundcloud', 'Veridis Quo', 'Daft Punk', 345, 'sc5', { preview: true })]),
  ];

  // ?demo=1: made-up songs with generated cover art (screenshots for the
  // landing page: no real artists or album covers).
  if (DEMO) {
    const names = [['Neon Harbor', 'Velvet Static'], ['Glass Lanterns', 'Mira Vale'], ['Afterglow Avenue', 'The Low Tides'], ['Paper Satellites', 'Kai Rowan'], ['Midnight Arcade', 'Velvet Static'], ['Slow Orbit', 'Juno Park'], ['Golden Hour Radio', 'Mira Vale']];
    tracks.forEach((t, i) => {
      const [title, artist] = names[i % names.length];
      Object.assign(t, { title, artist, key: `${artist}::${title}`.toLowerCase(), thumbnail: art(i) });
      for (const v of t.versions) Object.assign(v, { title, artist, thumbnail: art(i) });
    });
  }

  let settings = {
    volume: 0.7, platform_order: ['local', 'ytmusic', 'soundcloud', 'youtube', 'spotify'], last_source: 'all', search_limit: 12,
    theme: 'midnight', enabled_plugins: [], plugin_settings: {}, close_to_tray: false, include_youtube_in_all: true,
    spotify_backend: 'auto', include_spotify_in_all: true, spotify_client_id: '', spotify_client_secret: '',
    normalize: true, crossfade: 0, lyrics: true, local_folders: ['/home/you/Music'], downloads_dir: '', ytdlp_source: 'system',
    auto_update: false, discord_rpc: false, discord_client_id: '', discord_buttons: true, repeat: 'off', example_plugin_installed: true,
    autoplay: true, eq_enabled: false, eq_gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], output_device: '', cover_accent: true, charts_country: 'ZZ', search_history: ['daft punk', 'tame impala'], background: 'art', background_strength: 45, background_image: '',
  };
  const mix = (id, kind, title, subtitle, ts) => ({ id, kind, title, subtitle, covers: ts.map((t) => t.thumbnail).filter(Boolean).slice(0, 4), tracks: ts });
  const forYou = (stale) => ({
    daily: stale ? [] : [mix('daily-1', 'daily', 'Daily Mix 1', N('Daft Punk, Justice, Cassius and more', 'Velvet Static, Mira Vale and more'), tracks.slice(0, 6)), mix('daily-2', 'daily', 'Daily Mix 2', N('Tame Impala, Mild High Club and more', 'Juno Park, The Low Tides and more'), tracks.slice(2, 7))],
    discover: stale ? null : mix('discover', 'discover', 'Discover Weekly', 'New music picked for you · refreshed every Monday', tracks.slice(1, 6)),
    local: [mix('repeat', 'repeat', 'On Repeat', 'Your most played songs of the last 30 days', tracks.slice(0, 5))],
    stale, empty: false,
  });
  let uid = 0;
  const queue = { items: tracks.slice(0, 5).map((t) => ({ uid: ++uid, track: t, platform: null })), current: 0 };
  let mockSpotifyUser = null;
  const followed = [];
  const playback = { status: 'paused', position: 42, duration: 321, buffered: 0.6, stalled: false, volume: 0.7, repeat: 'off', index: 0, current: queue.items[0], version: tracks[0].versions[0], message: null, preloaded: null };
  const playlists = [{ id: 'p1', name: 'Late night drive', tracks: tracks.slice(2), created: 0, updated: 0 }, { id: 'p2', name: 'Gym', tracks: tracks.slice(0, 3), created: 0, updated: 0 }];
  // ?big=3000 adds a playlist that long (scrolling benchmarks).
  const big = Number(new URLSearchParams(location.search).get('big')) || 0;
  if (big) {
    const ids = ['fa5IWHDbftI', 'wU26xVT_vBU', 'GDpmVUEjagg', '5NV6Rdv1a3I', 'Qm9EpPeVrn8', 'a5uQMwRMHcs', 'FGBhQbmPwH8'];
    playlists.push({ id: 'pbig', name: `Big library (${big})`, created: 0, updated: 0, added: {}, tracks: Array.from({ length: big }, (_, i) => T(`Song number ${i + 1}`, `Artist ${i % 97}`, 120 + (i % 200), [V('ytmusic', `Song number ${i + 1}`, `Artist ${i % 97}`, 120 + (i % 200), ids[i % ids.length], { album: `Album ${i % 41}` }), V('soundcloud', `Song ${i + 1}`, `Artist ${i % 97}`, 121, `sc${i}`)])) });
  }
  let favorites = tracks.slice(0, 2);
  let plSeq = 0;
  const listeners = {};
  const emit = (name, payload) => (listeners[name] || []).forEach((cb) => cb({ payload }));
  const snap = () => ({ items: queue.items, current: queue.current });
  const startItem = (i) => {
    queue.current = i;
    Object.assign(playback, { status: 'playing', position: 0, current: queue.items[i], version: queue.items[i].track.versions[0], duration: queue.items[i].track.duration, index: i });
    emit('queue-changed', snap());
    emit('playback', { ...playback });
    emit('track-started', { item: playback.current, version: playback.version });
  };

  // Simulated playback clock.
  setInterval(() => {
    if (playback.status !== 'playing') return;
    playback.position += 0.25;
    if (playback.duration && playback.position >= playback.duration) {
      if (queue.current + 1 < queue.items.length) startItem(queue.current + 1);
      else { playback.status = 'idle'; emit('playback', { ...playback }); }
      return;
    }
    emit('playback-tick', { status: playback.status, position: playback.position, duration: playback.duration, buffered: Math.min(1, playback.position / playback.duration + 0.3), stalled: false });
  }, 250);

  const R = {
    get_settings: () => settings,
    update_settings: ({ patch }) => (settings = { ...settings, ...patch }),
    health_check: () => ({ ok: true, version: 'mock', message: 'Mock backend (browser preview)', path: 'yt-dlp' }),
    tools_status: () => ({ spotdl: null, spotify_secret_saved: false, bundled_ytdlp: false, local_tracks: 1 }),
    spotify_account: () => mockSpotifyUser,
    spotify_login: () => new Promise((r) => setTimeout(() => r(mockSpotifyUser = 'devuser'), 800)),
    spotify_logout: () => { mockSpotifyUser = null; },
    list_output_devices: () => [{ id: 'alsa:default', name: 'Built-in Audio', default: true }, { id: 'alsa:hdmi', name: 'HDMI Output', default: false }],
    search_suggestions: ({ query }) => ['', ' remix', ' live', ' lyrics', ' acoustic'].map((x) => query + x),
    search_preview: async ({ query }) => {
      await new Promise((r) => setTimeout(r, 120));
      const q = query.toLowerCase();
      const songs = tracks.filter((t) => `${t.title} ${t.artist}`.toLowerCase().includes(q.split(' ')[0])).slice(0, 3);
      return {
        queries: ['', ' remix', ' live', ' lyrics'].map((x) => query + x),
        items: [
          { kind: 'artist', title: songs[0]?.artist || 'Daft Punk', subtitle: '72.5M monthly audience', browse_id: 'UCmock', video_id: null, thumbnail: yt('fa5IWHDbftI'), track: null },
          ...songs.map((t) => ({ kind: 'song', title: t.title, subtitle: `Song • ${t.artist} • 76M plays`, browse_id: null, video_id: t.versions[0].id, thumbnail: t.thumbnail, track: t })),
          { kind: 'album', title: 'Discovery', subtitle: `Album • ${songs[0]?.artist || 'Daft Punk'} • 2001`, browse_id: 'MPREmock', video_id: null, thumbnail: yt('fa5IWHDbftI'), track: null },
        ],
      };
    },
    search_history: () => settings.search_history,
    clear_search_history: ({ query }) => { settings.search_history = query ? settings.search_history.filter((q) => q !== query) : []; },
    chart_countries: () => [{ code: 'ZZ', name: 'Global' }, { code: 'US', name: 'United States' }, { code: 'IL', name: 'Israel' }],
    get_charts: ({ country }) => ({ country, playlists: [
      { title: `Top 100 Music Videos ${country}`, url: 'https://music.youtube.com/playlist?list=PLmock1', thumbnail: yt('fa5IWHDbftI'), group: 'Video charts' },
      { title: `Trending 20 ${country}`, url: 'https://music.youtube.com/playlist?list=PLmock2', thumbnail: yt('NMRhx71bGo4'), group: 'Video charts' }],
      artists: ['Daft Punk', 'Tame Impala', 'Hiatus Kaiyote', 'Bonobo'].map((name, i) => ({ name, url: 'https://www.youtube.com/channel/UCmock' + i, thumbnail: null, rank: i + 1 })) }),
    list_followed: () => followed,
    follow_artist: ({ name, url, thumbnail }) => {
      const i = followed.findIndex((a) => a.name.toLowerCase() === name.toLowerCase());
      if (i >= 0) { followed.splice(i, 1); return false; }
      followed.unshift({ name, url, thumbnail, seen: [], followed_at: Date.now() });
      return true;
    },
    new_releases: () => followed.flatMap((a, k) => [0, 1, 2].map((i) => ({ artist: a.name, title: `${a.name} — Release ${i + 1}`, url: `https://music.youtube.com/playlist?list=OLAKmock${k}${i}`, thumbnail: yt('fa5IWHDbftI'), new: i === 0 }))),
    mark_releases_seen: () => {},
    translate_lyrics: ({ lines }) => ({ lines: lines.map((l) => (l ? `[en] ${l}` : l)), source_lang: 'fr' }),
    cover_color: () => '#3d7bd9',
    spotify_user_playlists: () => [{ name: 'Mock mix', url: 'https://open.spotify.com/playlist/mock1', tracks: 3, thumbnail: null, owner: 'dev' }],
    sync_playlist: ({ id }) => playlists.find((p) => p.id === id)?.tracks.length || 0,
    prefetch_track: () => true,
    recommend_for_playlist: ({ tracks: seeds, nonce }) => tracks.filter((t) => !seeds.some((s) => s.key === t.key)).map((t, i) => ({ track: t, because: seeds[(i + (nonce || 0)) % seeds.length].title, seeds: 1 + (i % 2) })),
    background_image_data: () => 'data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACwAAAAAAQABAAACAkQBADs=',
    start_radio: ({ track }) => { queue.items = [{ uid: ++uid, track, platform: null }, ...tracks.filter((t) => t.key !== track.key).map((t) => ({ uid: ++uid, track: t, platform: null }))]; startItem(0); },
    discord_status: () => (settings.discord_rpc ? 'connected' : 'off'),
    list_themes: () => [{ id: 'midnight', name: 'Midnight', author: 'Tunewire', dark: true, builtin: true, accent: '#e5a54b', bg: '#121212' }, { id: 'daylight', name: 'Daylight', author: 'Tunewire', dark: false, builtin: true, accent: '#b86e0c', bg: '#fafafa' }, { id: 'synthwave', name: 'Synthwave', author: 'Tunewire', dark: true, builtin: true, accent: '#ff6ec7', bg: '#16101f' }],
    load_theme: async ({ id }) => { settings.theme = id; return (await fetch(`/__themes/${id}.json`)).json(); },
    get_theme: async ({ id }) => (await fetch(`/__themes/${id}.json`)).json(),
    get_queue: snap,
    get_playback_state: () => ({ ...playback }),
    search_tracks: async ({ query }) => { await new Promise((r) => setTimeout(r, 400)); return { tracks: tracks.filter((t) => `${t.title} ${t.artist}`.toLowerCase().includes((query.split(' ')[0] || '').toLowerCase())).concat([]).slice(0) || tracks, warnings: [], cached: false }; },
    play_track: ({ track, platform }) => { queue.items.splice(queue.current + 1, 0, { uid: ++uid, track, platform }); startItem(queue.current + 1); },
    play_tracks: ({ tracks: ts, start }) => { queue.items = ts.map((t) => ({ uid: ++uid, track: t, platform: null })); startItem(start || 0); },
    play_index_cmd: ({ index }) => startItem(index),
    toggle_pause: () => { playback.status = playback.status === 'playing' ? 'paused' : 'playing'; emit('playback', { ...playback }); },
    pause: () => { playback.status = 'paused'; emit('playback', { ...playback }); },
    resume: () => { playback.status = 'playing'; emit('playback', { ...playback }); },
    stop: () => { playback.status = 'idle'; emit('playback', { ...playback }); },
    next_track: () => queue.current + 1 < queue.items.length && startItem(queue.current + 1),
    previous_track: () => queue.current > 0 && startItem(queue.current - 1),
    seek: ({ position }) => { playback.position = position; },
    set_volume: ({ volume }) => { playback.volume = volume; settings.volume = volume; return volume; },
    set_repeat: ({ mode }) => { playback.repeat = settings.repeat = mode; emit('playback', { ...playback }); return mode; },
    add_to_queue: ({ tracks: ts, next }) => { const items = ts.map((t) => ({ uid: ++uid, track: t, platform: null })); queue.items.splice(next ? queue.current + 1 : queue.items.length, 0, ...items); emit('queue-changed', snap()); return snap(); },
    remove_from_queue: ({ uid: u }) => { queue.items = queue.items.filter((i) => i.uid !== u); emit('queue-changed', snap()); return snap(); },
    reorder_queue: ({ from, to }) => { const [it] = queue.items.splice(from, 1); queue.items.splice(to, 0, it); emit('queue-changed', snap()); return snap(); },
    clear_queue: () => { queue.items = [queue.items[queue.current]]; queue.current = 0; emit('queue-changed', snap()); return snap(); },
    shuffle_queue: () => snap(),
    set_item_platform: () => null,
    list_playlists: () => playlists,
    create_playlist: ({ name, tracks: ts }) => { const p = { id: `p${Date.now()}${++plSeq}`, name, tracks: ts || [], created: 0, updated: 0, added: {} }; playlists.push(p); emit('library-changed', 'playlists'); return p; },
    rename_playlist: ({ id, name }) => { playlists.find((p) => p.id === id).name = name; emit('library-changed', 'playlists'); },
    delete_playlist: ({ id }) => { playlists.splice(playlists.findIndex((p) => p.id === id), 1); emit('library-changed', 'playlists'); },
    add_to_playlist: ({ id, tracks: ts }) => { const p = playlists.find((x) => x.id === id); const fresh = ts.filter((t) => !p.tracks.some((x) => x.key === t.key)); p.tracks.push(...fresh); emit('library-changed', 'playlists'); return fresh.length; },
    remove_from_playlist: ({ id, index, key }) => { const p = playlists.find((x) => x.id === id); const i = key ? p.tracks.findIndex((t) => t.key === key) : index; p.tracks.splice(i, 1); emit('library-changed', 'playlists'); },
    move_in_playlist: () => emit('library-changed', 'playlists'),
    list_favorites: () => favorites,
    toggle_favorite: ({ track }) => { const has = favorites.some((t) => t.key === track.key); favorites = has ? favorites.filter((t) => t.key !== track.key) : [track, ...favorites]; emit('library-changed', 'favorites'); return !has; },
    list_history: () => tracks.slice(0, 4).map((t, i) => ({ track: t, platform: t.versions[0].platform, played_at: Date.now() - i * 3600e3 })),
    clear_history: () => null,
    list_downloads: () => [{ key: tracks[0].key, track: tracks[0], version: tracks[0].versions[0], path: '/x.m4a', size: 5186561, added: 0 }],
    download_tracks: ({ tracks: ts }) => ts.length,
    delete_download: () => null,
    downloads_dir: () => '/home/you/.local/share/app.tunewire.player/downloads',
    list_local: () => [T('Local Demo', 'You', 200, [V('local', 'Local Demo', 'You', 200, '/home/you/Music/demo.mp3', { album: 'Demo Album' })])],
    add_local_folder: () => 1, remove_local_folder: () => 1, rescan_local: () => 1,
    artist_page: async () => { await new Promise((r) => setTimeout(r, 300)); return { name: 'Daft Punk', thumbnail: yt('fa5IWHDbftI'), url: 'x', songs: tracks.slice(0, 6), soundcloud: tracks.slice(2, 4) }; },
    artist_albums: () => [['Discovery', 'Qm9EpPeVrn8'], ['Random Access Memories', '5NV6Rdv1a3I'], ['Homework', 'GDpmVUEjagg'], ['Human After All', 'a5uQMwRMHcs']].map(([title, id]) => ({ title, url: `https://example.com/album/${id}`, thumbnail: yt(id), tracks: 0 })),
    album_cover: () => null,
    album_page: () => ({ title: 'Discovery', url: 'x', platform: 'ytmusic', tracks: tracks.slice(0, 5) }),
    complete_metadata: () => [],
    get_lyrics: () => ({ synced: '[00:00.00] One more time\n[00:08.00] We\'re gonna celebrate\n[00:15.00] Oh yeah, all right\n[00:22.00] Don\'t stop the dancing\n[00:30.00] One more time', plain: null, instrumental: false, source: 'LRCLIB' }),
    list_plugins: () => [], plugins_dir: () => '/home/you/.local/share/app.tunewire.player/plugins',
    play_url: () => ({ title: 'Link', url: 'x', platform: 'youtube', tracks: tracks.slice(0, 1) }),
    load_playlist: () => ({ title: 'Remote playlist', url: 'x', platform: 'soundcloud', tracks: tracks.slice(1, 5) }),
    set_spotify_secret: () => null, ytdlp_install: () => ({ ok: true, version: 'mock' }),
    for_you: () => forYou(true),
    refresh_for_you: async () => { await new Promise((r) => setTimeout(r, 400)); return forYou(false); },
    listening_stats: ({ range }) => {
      const top = tracks.slice(0, 6);
      const series = range === 'year' ? ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] : Array.from({ length: 28 }, (_, i) => `Oct ${i + 1}`);
      return {
        range, from: 0, to: Date.now(), minutes: 1834.5, plays: 412, skips: 37, tracks: 128, artists: 46, new_artists: 19, days_active: 25, streak: 6, longest_streak: 12,
        top_tracks: top.map((t, i) => ({ track: t, plays: 40 - i * 5, minutes: (40 - i * 5) * 4 })),
        top_artists: [[N('Daft Punk', 'Velvet Static'), 'fa5IWHDbftI'], [N('Tame Impala', 'Mira Vale'), 'wU26xVT_vBU'], [N('Steve Lacy', 'Juno Park'), 'GDpmVUEjagg'], [N('Djo', 'Kai Rowan'), '5NV6Rdv1a3I']].map(([name, id], i) => ({ name, plays: 90 - i * 15, minutes: 400 - i * 60, thumbnail: yt(id), top_track: tracks[i] })),
        hours: Array.from({ length: 24 }, (_, h) => Math.max(0, 60 - Math.abs(h - 21) * 7)),
        weekdays: [120, 180, 90, 200, 260, 310, 150],
        series: series.map((label, i) => ({ label, minutes: 20 + ((i * 37) % 90) })),
        platforms: [{ label: 'YouTube Music', minutes: 1200 }, { label: 'SoundCloud', minutes: 400 }, { label: 'Spotify', minutes: 234.5 }],
        busiest_day: { label: 'Oct 11 2026', minutes: 214 }, estimated: false, years: [2026, 2025], first_play: Date.now() - 400 * 864e5,
      };
    },
    save_share_image: ({ name }) => `/home/you/Pictures/${name}.png`,
    ytm_search_extras: async () => {
      await new Promise((r) => setTimeout(r, 120));
      const card = (kind, title, subtitle, id, extra = {}) => ({ kind, title, subtitle, browse_id: id, video_id: null, thumbnail: yt('fa5IWHDbftI'), track: null, ...extra });
      return {
        top: card('artist', 'Daft Punk', 'Artist • 30M monthly audience', 'UCdaft'),
        artists: [card('artist', 'Daft Punk', 'Artist', 'UCdaft'), card('artist', 'Justice', 'Artist', 'UCjustice')],
        albums: ['Discovery', 'Homework', 'Random Access Memories'].map((t, i) => card('album', t, `Album • Daft Punk • ${2001 - i}`, `MPREb_${i}`)),
        playlists: [card('playlist', 'This Is Daft Punk', 'Playlist • YouTube Music', 'VLPLdaft')],
      };
    },
    ytm_artist: async ({ name }) => ({
      name: name || 'Daft Punk', browse_id: 'UCdaft', banner: yt('fa5IWHDbftI'), thumbnail: yt('fa5IWHDbftI'), audience: '30M monthly audience', subscribers: '9M', description: N('French electronic duo.', 'Dream-pop duo from a city that never sleeps.'),
      top_songs: tracks.slice(0, 6), all_songs: 'VLPLall',
      albums: ['Discovery', 'Homework'].map((t, i) => ({ kind: 'album', title: t, subtitle: `Album • ${2001 - i}`, browse_id: `MPREb_${i}`, thumbnail: yt('Qm9EpPeVrn8'), track: null })),
      singles: [{ kind: 'single', title: 'Get Lucky', subtitle: 'Single • 2013', browse_id: 'MPREb_s', thumbnail: yt('5NV6Rdv1a3I'), track: null }],
      related: [{ kind: 'artist', title: 'Justice', subtitle: '5M monthly audience', browse_id: 'UCjustice', thumbnail: null, track: null }],
      shelves: [],
    }),
    ytm_collection: async ({ id }) => ({ title: id === 'VLPLall' ? 'All songs' : 'Discovery', kind: id?.startsWith('VL') ? 'Playlist' : 'Album', artist: 'Daft Punk', artist_browse: 'UCdaft', year: '2001', cover: yt('Qm9EpPeVrn8'), description: '', browse_id: id || 'MPREb_0', tracks: tracks.slice(0, 5) }),
    ytm_find_album: () => 'MPREb_0',
    ytm_moods: () => [{ title: 'Moods & moments', items: [{ title: 'Chill', params: 'p1', color: '#7fb3ff' }, { title: 'Workout', params: 'p2', color: '#ff7f50' }] }, { title: 'Genres', items: [{ title: 'Pop', params: 'p3', color: '#ff5fa2' }] }],
    ytm_mood_page: () => [{ title: 'Chill pop', cards: [{ kind: 'playlist', title: 'Chill Hits', subtitle: 'Playlist', browse_id: 'VLchill', thumbnail: yt('Qm9EpPeVrn8'), track: null }] }],
    artist_soundcloud: () => [],
    spotify_library_login: async () => { await new Promise((r) => setTimeout(r, 200)); return { user: 'devuser', liked: 3, playlists: [{ id: 'sp1', name: 'Road trip', owner: 'devuser', total: 2, thumbnail: null }, { id: 'sp2', name: 'Focus', owner: 'Spotify', total: 3, thumbnail: null }] }; },
    spotify_library_import: async ({ liked, playlists: pls }) => {
      let first = null;
      for (const [id, name] of pls) { const p = { id: `p${id}`, name, tracks: tracks.slice(0, 2), created: Date.now(), updated: Date.now(), added: {} }; playlists.push(p); first = first || p.id; emit('import-progress', { done: 1, total: pls.length, label: name }); }
      if (liked) { favorites = [...tracks.slice(4, 7), ...favorites]; emit('library-changed', 'favorites'); }
      emit('library-changed', 'playlists');
      return { playlists: pls.length, songs: pls.length * 2, liked: liked ? 3 : 0, missing: 0, first_id: first };
    },
    // Online: a test harness can point these at a local Supabase (window.__SB).
    cloud_config: () => (window.__SB ? { url: window.__SB.url, anon_key: window.__SB.anon } : null),
    cloud_session: () => window.__SBsession || null,
    cloud_sign_in: () => { throw { kind: 'unavailable', message: 'not in the preview' }; },
    cloud_sign_out: () => { window.__SBsession = null; },
    set_playlist_cloud: ({ id, cloudId, role }) => { const p = playlists.find((x) => x.id === id); p.cloud_id = cloudId; p.cloud_role = role; emit('library-changed', 'playlists'); },
    replace_playlist_tracks: ({ id, tracks: ts, expect }) => {
      const p = playlists.find((x) => x.id === id);
      if (expect && expect.join('\n') !== p.tracks.map((t) => t.key).join('\n')) throw { kind: 'conflict', message: 'the songs changed' };
      p.tracks = ts; emit('library-changed', 'playlists');
    },
    replace_favorites: ({ tracks: ts, expect }) => {
      if (expect && expect.join('\n') !== favorites.map((t) => t.key).join('\n')) throw { kind: 'conflict', message: 'the songs changed' };
      favorites = ts; emit('library-changed', 'favorites');
    },
    // Kept per tab (a test can give two tabs different "devices").
    load_sync_state: () => sessionStorage.getItem('tw-sync') || '',
    save_sync_state: ({ data }) => { sessionStorage.setItem('tw-sync', data); },
    spotify_profile_playlists: () => ({ name: 'Friend', playlists: [{ id: 'fp1', name: 'Friend mix', owner: 'Friend', total: 2, thumbnail: null }] }),
    report_error: () => null, reports_available: () => false, cover_wash: () => (DEMO ? null : 'data:image/png;base64,iVBORw0KGgo='),
    import_apple_music: () => { const p = { id: `pam${Date.now()}`, name: 'Apple Mix', tracks: tracks.slice(1, 4), created: Date.now(), updated: Date.now(), added: {} }; playlists.push(p); emit('library-changed', 'playlists'); return { playlist: p, missing: 1 }; },
    import_rows: ({ name, rows }) => { const p = { id: `pcsv${Date.now()}`, name, tracks: tracks.slice(0, Math.min(rows.length, 5)), created: Date.now(), updated: Date.now(), added: {} }; playlists.push(p); emit('library-changed', 'playlists'); return { playlist: p, missing: 0 }; },
    apply_matches: ({ tracks: ts }) => ts,
    remove_many_from_playlist: ({ id, indices, keys }) => { const p = playlists.find((x) => x.id === id); const n = p.tracks.length; p.tracks = keys ? p.tracks.filter((t) => !keys.includes(t.key)) : p.tracks.filter((_, i) => !indices.includes(i)); emit('library-changed', 'playlists'); return n - p.tracks.length; },
    move_many_in_playlist: ({ id, indices, to, keys }) => { const p = playlists.find((x) => x.id === id); const idx = keys ? p.tracks.map((t, i) => (keys.includes(t.key) ? i : -1)).filter((i) => i >= 0) : indices; const mv = idx.map((i) => p.tracks[i]); p.tracks = p.tracks.filter((_, i) => !idx.includes(i)); p.tracks.splice(to, 0, ...mv); emit('library-changed', 'playlists'); },
    update_playlist_meta: ({ id, patch }) => { const p = playlists.find((x) => x.id === id); if (patch.name) p.name = patch.name; if (patch.description != null) p.description = patch.description; if (patch.pinned != null) p.pinned = patch.pinned; if (patch.folder != null) p.folder = patch.folder || null; if (patch.cover != null) p.cover = patch.cover || null; emit('library-changed', 'playlists'); },
    set_sleep_timer: ({ minutes, endOfTrack }) => { playback.sleep_at = endOfTrack ? 0 : minutes ? Date.now() + minutes * 60000 : null; emit('playback', playback); return playback.sleep_at; },
    set_shuffle: ({ on }) => { settings.shuffle = on; playback.shuffle = on; emit('playback', playback); return queue; },
    // Undo, backup, scrobbling, gallery.
    restore_playlist: ({ playlist, index }) => { playlists.splice(index ?? playlists.length, 0, playlist); emit('library-changed', 'playlists'); },
    restore_history: () => emit('library-changed', 'history'),
    backup_file_name: () => 'Tunewire-2026-10-17.tunewire-backup',
    export_backup: ({ path }) => ({ path, created: Date.now(), playlists: playlists.length, songs: playlists.reduce((n, p) => n + p.tracks.length, 0), favorites: favorites.length, plays: 412, themes: 0 }),
    inspect_backup: ({ path }) => ({ path, created: Date.now() - 864e5, playlists: 4, songs: 120, favorites: 30, plays: 900, themes: 1 }),
    restore_backup: () => null,
    list_snapshots: () => [{ path: '/b/library-20261016.json', name: 'library-20261016.json', modified: Date.now() - 864e5 }],
    scrobble_status: () => ({ lastfm_available: true, lastfm_connected: false, listenbrainz_connected: false, pending: 0 }),
    lastfm_connect: async () => { await new Promise((r) => setTimeout(r, 600)); settings.lastfm_user = 'dev'; settings.scrobble_lastfm = true; return 'dev'; },
    listenbrainz_connect: ({ token }) => { if (token.length < 8) throw { kind: 'invalid', message: 'that ListenBrainz token isn’t valid' }; settings.listenbrainz_user = 'dev'; return 'dev'; },
    scrobble_disconnect: () => null, set_lastfm_app: () => null,
    gallery_index: async () => (await fetch('/__gallery/index.json')).json(),
    gallery_install: async ({ kind, id }) => { await new Promise((r) => setTimeout(r, 300)); return kind === 'theme' ? { id, name: id } : { name: id, permissions: [] }; },
  };

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        const f = R[cmd];
        if (!f) { console.warn('[mock] unhandled command', cmd, args); return null; }
        return structuredClone(await f(args || {}));
      },
    },
    event: { listen: async (name, cb) => { (listeners[name] ||= []).push(cb); return () => {}; } },
    app: { getVersion: async () => 'dev' },
    updater: { check: async () => null },
    process: { relaunch: async () => location.reload() },
    dialog: { open: async () => '/home/you/Music', save: async ({ defaultPath }) => `/home/you/${defaultPath}` },
  };
  console.info('%cTunewire mock backend active (browser preview)', 'color:#e5a54b');
})();
