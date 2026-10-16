'use strict';

// ===========================================================================
// Settings
// ===========================================================================

async function updateSettings(patch) {
  try {
    state.settings = await call('update_settings', { patch });
  } catch (e) { toast(errMsg(e), 'error'); }
}

function settingToggle(label, help, key) {
  const input = h('input', { type: 'checkbox', checked: !!state.settings[key] });
  input.addEventListener('change', async () => {
    await updateSettings({ [key]: input.checked });
    if (key === 'discord_rpc') toast(input.checked ? 'Discord Rich Presence on' : 'Discord Rich Presence off', 'ok', 1800);
    if (key === 'show_sources') document.body.classList.toggle('show-sources', input.checked);
    if (key === 'cover_accent') {
      if (input.checked) updateCoverAccent(state.playback?.version?.thumbnail || state.playback?.current?.track?.thumbnail);
      else clearCoverAccent();
    }
  });
  return h('div', { class: 'setting' }, h('div', { class: 'label' }, label, h('small', { html: help })), h('label', { class: 'switch' }, input, h('span')));
}

function settingRow(label, help, control) {
  return h('div', { class: 'setting' }, h('div', { class: 'label' }, label, help ? h('small', { html: help }) : null), control);
}

function group(title, brand, ...rows) {
  return h('div', { class: `group ${brand || ''}` }, h('h2', { html: `${brand ? plogo(brand) : ''}<span>${esc(title)}</span>` }), ...rows);
}

let discordPoll = null;
function discordStatusRow() {
  const pill = h('span', { class: 'status-pill' }, '…');
  const help = h('small', {});
  const update = async () => {
    if (!pill.isConnected) { clearInterval(discordPoll); discordPoll = null; return; }
    const s = await call('discord_status').catch(() => 'error: unavailable');
    const ok = s === 'connected';
    pill.className = `status-pill ${ok ? 'ok' : s === 'off' ? '' : 'bad'}`;
    pill.textContent = ok ? 'Connected' : s === 'off' ? 'Off' : s.startsWith('waiting') ? 'Not connected' : 'Error';
    help.innerHTML = ok ? 'Your profile shows the current song while something is playing.'
      : s === 'off' ? 'Turn on Rich Presence below.'
      : s.includes('socket') ? 'No Discord client socket found. Start Discord. If you use <b>Vesktop</b>, open its Settings → Vesktop Settings and enable <b>arRPC</b> (Rich Presence); Vesktop only accepts other apps when that is on.'
      : esc(s.replace(/^(waiting|error): /, ''));
  };
  update();
  clearInterval(discordPoll);
  discordPoll = setInterval(update, 2000);
  return h('div', { class: 'setting' }, h('div', { class: 'label' }, 'Status', help), pill);
}

function renderSettings() {
  const s = state.settings;
  const body = $('#settings-body');

  const order = h('div', { class: 'order-list' });
  const move = async (i, d) => {
    const arr = [...s.platform_order];
    [arr[i], arr[i + d]] = [arr[i + d], arr[i]];
    await updateSettings({ platform_order: arr });
    renderSettings();
  };
  order.replaceChildren(...s.platform_order.map((p, i) => h('div', { class: 'order-item' },
    h('span', { class: 'n' }, String(i + 1)), miniLogo(p), h('span', { class: 'name' }, PLATFORMS[p].label),
    h('button', { class: 'icon-btn small', html: icon('up'), disabled: i === 0, title: 'Move up', onclick: () => move(i, -1) }),
    h('button', { class: 'icon-btn small', html: icon('down'), disabled: i === s.platform_order.length - 1, title: 'Move down', onclick: () => move(i, 1) }))));
  const limit = h('input', { type: 'number', min: 3, max: 40, value: s.search_limit, style: 'width:70px' });
  limit.addEventListener('change', () => updateSettings({ search_limit: Number(limit.value) }));

  const spotdl = state.tools?.spotdl;
  const backend = h('select', {},
    ...[['auto', 'Automatic'], ['embed', 'Spotify web only'], ['spotdl', 'spotDL for links'], ['off', 'Off']]
      .map(([v, l]) => h('option', { value: v, selected: s.spotify_backend === v }, l)));
  backend.addEventListener('change', () => updateSettings({ spotify_backend: backend.value }));
  const credId = h('input', { type: 'text', value: s.spotify_client_id || '', placeholder: 'Client ID', style: 'width:150px', spellcheck: 'false' });
  const secretSaved = !!state.tools?.spotify_secret_saved;
  const credSecret = h('input', { type: 'password', value: '', placeholder: secretSaved ? 'Saved in keychain' : 'Client secret', style: 'width:150px', spellcheck: 'false', autocomplete: 'off' });
  credId.addEventListener('change', () => updateSettings({ spotify_client_id: credId.value.trim() }).then(() => toast('Client ID saved', 'ok', 1500)));
  credSecret.addEventListener('change', async () => {
    try {
      await call('set_spotify_secret', { secret: credSecret.value.trim() });
      credSecret.value = '';
      await refreshTools();
      toast('Secret saved in your system keychain', 'ok', 2000);
      renderSettings();
    } catch (e) { toast(errMsg(e), 'error'); }
  });
  const user = state.spotifyUser;
  const account = h('div', { class: 'actions' },
    h('span', { class: `status-pill ${user ? 'ok' : ''}` }, user ? `Logged in: ${user}` : 'Not logged in'),
    user
      ? h('button', { class: 'btn small', onclick: async () => { await call('spotify_logout'); state.spotifyUser = null; toast('Logged out of Spotify', 'info', 1500); renderSettings(); } }, 'Log out')
      : h('button', { class: 'btn small primary', onclick: async (e) => {
          e.target.disabled = true;
          e.target.textContent = 'Waiting for browser…';
          try {
            state.spotifyUser = await call('spotify_login');
            toast(`Logged in to Spotify as ${state.spotifyUser}`, 'ok', 2500);
          } catch (err) { toast(errMsg(err), 'error'); }
          renderSettings();
        } }, 'Log in with Spotify'));
  const clientId = h('input', { type: 'text', value: s.discord_client_id || '', placeholder: 'Application ID', style: 'width:190px', spellcheck: 'false' });
  clientId.addEventListener('change', () => updateSettings({ discord_client_id: clientId.value.trim() }));
  const recheck = (fn) => h('button', { class: 'btn small', onclick: async () => { await fn(); renderSettings(); } }, 'Check again');

  body.replaceChildren(
    group('Playback', null,
      settingRow('Source priority', 'When a song exists on several platforms, the first available one plays. If it fails, the next one is tried.', order),
      settingRow('Results per source', null, limit),
      settingToggle('Search YouTube in “All”', 'Adds regular YouTube uploads (live versions, covers) to combined searches.', 'include_youtube_in_all'),
      settingToggle('Autoplay', 'When the queue runs out, keep playing similar songs (YouTube Music radio).', 'autoplay'),
      settingToggle('Close to tray', 'Closing the window keeps music playing. Quit from the tray icon.', 'close_to_tray')),
    group('Spotify', 'spotify',
      settingRow('Spotify account', 'Log in to play Spotify’s own audio (Ogg Vorbis up to 320 kbps) when you pick Spotify as the source. Needs <b>Spotify Premium</b>; your login opens in the browser and only a reusable token is stored on this computer. Without it, Spotify songs play from the same recording on YouTube Music.', account),
      settingRow('Metadata source', 'Search uses Spotify’s own web search (no account needed); pasted links load from the public embed page. spotDL is only a fallback.', backend),
      settingRow('spotDL (optional fallback)', spotdl ? `Version ${spotdl}` : 'Not installed. Optional: <code>pipx install spotdl</code> or <code>uv tool install spotdl</code>.',
        h('div', { class: 'actions' }, h('span', { class: `status-pill ${spotdl ? 'ok' : 'bad'}` }, spotdl ? 'Installed' : 'Missing'), recheck(refreshTools))),
      settingToggle('Search Spotify in “All”', 'Adds Spotify results to combined searches (about half a second).', 'include_spotify_in_all'),
      settingRow('Spotify app credentials (spotDL)', 'Only used by the spotDL fallback. Its built-in credentials are shared and often rate-limited, which can make it take a minute. A free app from <code>developer.spotify.com/dashboard</code> avoids that (redirect URI: <code>http://127.0.0.1:9900/</code>). The secret is stored in your system keychain, not in a file.', h('div', { class: 'actions' }, credId, credSecret))),
    group('Sound', null,
      settingToggle('Volume normalization', 'Evens out loudness between songs and platforms. Measured from the first seconds of each song, with a soft limiter against clipping.', 'normalize'),
      settingRow('Crossfade', 'Overlap the end of a song with the start of the next. At 0, songs still play back to back without a gap.', crossfadeControl()),
      settingRow('Output device', 'Where the sound goes. Switching continues the current song on the new device.', outputDeviceControl()),
      eqSetting()),
    group('Appearance', null,
      settingRow('Background', 'What shows faintly behind the panels.', backgroundControl()),
      settingRow('Background strength', null, strengthControl()),
      settingToggle('Accent from album art', 'Tints buttons and highlights with a color from the playing song’s cover.', 'cover_accent'),
      settingToggle('Show source icons on songs', 'Always show the YouTube Music / SoundCloud / YouTube / Spotify icons on song rows (otherwise only when you hover a song).', 'show_sources')),
    group('Lyrics', null,
      settingToggle('Show lyrics', 'Fetches synced lyrics from LRCLIB (lrclib.net) for the Lyrics tab next to the queue.', 'lyrics')),
    group('Downloads & local files', null,
      settingRow('Download folder', esc(s.downloads_dir || 'Default (app data folder)'), h('div', { class: 'actions' },
        h('button', { class: 'btn small', onclick: async () => { const d = await pickFolder('Choose download folder'); if (d) { await updateSettings({ downloads_dir: d }); renderSettings(); } } }, 'Change'),
        s.downloads_dir ? h('button', { class: 'btn small', onclick: async () => { await updateSettings({ downloads_dir: '' }); renderSettings(); } }, 'Reset') : null)),
      settingRow('Music folders', (s.local_folders || []).length ? s.local_folders.map(esc).join('<br>') : 'None. Local files are searched together with online sources.', h('button', { class: 'btn small', onclick: () => showView('local') }, 'Manage'))),
    group('Discord', 'discord',
      discordStatusRow(),
      settingToggle('Rich Presence', 'Shows the current song, artist, cover and progress on your Discord profile while the desktop client is running.', 'discord_rpc'),
      settingToggle('Link button', 'Adds an “Open on YouTube Music” (or SoundCloud, …) button to the status.', 'discord_buttons'),
      settingRow('Application ID', 'Discord shows this application’s name as the activity. To show “Tunewire”, create an application with that name at <code>discord.com/developers</code> and paste its ID.', clientId)),
    group('Tools', null,
      updateSettingsRow(),
      settingRow('yt-dlp', `${esc($('#health').title || '')}<br><code>${esc(state.ytdlpPath || '')}</code>`, recheck(checkHealth)),
      settingRow('yt-dlp copy', 'Use the yt-dlp installed on your system, or let Tunewire keep its own up-to-date copy (official build from GitHub, checksum-verified).', ytdlpControl()),
      settingRow('Keyboard shortcuts', null, h('button', { class: 'btn small', onclick: showShortcuts }, 'Show'))));
}

async function refreshTools() {
  state.tools = await call('tools_status').catch(() => ({ spotdl: null }));
  state.spotifyUser = await call('spotify_account').catch(() => null);
  const el = $('#health-spotdl');
  const mode = state.settings?.spotify_backend;
  el.className = `health ${state.tools.spotdl ? 'ok' : mode === 'off' || mode === 'embed' ? 'off' : 'bad'}`;
  $('.health-text', el).textContent = state.tools.spotdl ? `spotDL ${state.tools.spotdl}` : 'spotDL not installed';
  el.title = state.tools.spotdl ? 'Spotify search & playlists available' : 'Install spotDL for Spotify search (pipx install spotdl)';
  if (state.view === 'search' && $('#search-body').hidden) renderHero();
}

async function checkHealth(target) {
  const el = $('#health');
  el.className = 'health';
  $('.health-text', el).textContent = 'Checking yt-dlp…';
  const res = await call('health_check').catch((e) => ({ ok: false, message: errMsg(e) }));
  el.classList.add(res.ok ? 'ok' : 'bad');
  $('.health-text', el).textContent = res.ok ? `yt-dlp ${res.version}` : 'yt-dlp missing';
  el.title = res.message;
  state.ytdlpPath = res.path;
  if (target) target.textContent = res.message;
  return res;
}

function backgroundControl() {
  const s = state.settings;
  const sel = h('select', {},
    ...[['art', 'Playing song’s cover'], ['tint', 'Color from the cover'], ['image', 'My picture'], ['plain', 'Plain']]
      .map(([v, l]) => h('option', { value: v, selected: (s.background || 'art') === v }, l)));
  sel.addEventListener('change', async () => {
    if (sel.value === 'image' && !s.background_image) { await pickBackgroundImage(); return; }
    await updateSettings({ background: sel.value });
    updateAmbient();
    renderSettings();
  });
  const name = s.background_image ? s.background_image.split(/[\\/]/).pop() : '';
  return h('div', { class: 'actions' }, sel,
    (s.background || 'art') === 'image' ? h('button', { class: 'btn small', title: s.background_image || '', onclick: pickBackgroundImage }, name ? `Change (${name})` : 'Choose…') : null);
}

function strengthControl() {
  const v = state.settings.background_strength ?? 45;
  const label = h('span', { class: 'meta', style: 'width:38px;text-align:right' }, `${v}%`);
  const range = h('input', { type: 'range', min: 0, max: 100, step: 5, value: v, style: 'width:160px' });
  range.style.setProperty('--fill', `${v}%`);
  range.addEventListener('input', () => {
    label.textContent = `${range.value}%`;
    range.style.setProperty('--fill', `${range.value}%`);
    document.documentElement.style.setProperty('--amb-strength', (range.value / 100).toFixed(2));
  });
  range.addEventListener('change', () => updateSettings({ background_strength: Number(range.value) }));
  return h('div', { class: 'actions' }, range, label);
}

function outputDeviceControl() {
  const sel = h('select', { style: 'max-width:240px' }, h('option', { value: '' }, 'System default'));
  call('list_output_devices').then((devs) => {
    for (const d of devs) sel.append(h('option', { value: d.id, selected: d.id === state.settings.output_device }, d.default ? `${d.name} (default)` : d.name));
    if (state.settings.output_device && !devs.some((d) => d.id === state.settings.output_device)) {
      sel.append(h('option', { value: state.settings.output_device, selected: true }, 'Saved device (not connected)'));
    }
  }).catch(() => {});
  sel.addEventListener('change', () => updateSettings({ output_device: sel.value }).then(() => toast('Output device changed', 'ok', 1500)));
  return sel;
}

const EQ_BANDS = ['31', '62', '125', '250', '500', '1k', '2k', '4k', '8k', '16k'];
const EQ_PRESETS = {
  Flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  'Bass boost': [6, 5, 4, 2, 0, 0, 0, 0, 0, 0],
  'Treble boost': [0, 0, 0, 0, 0, 1, 2, 4, 5, 6],
  Vocal: [-2, -2, -1, 1, 3, 4, 3, 1, 0, -1],
  Loudness: [5, 4, 2, 0, -1, 0, 1, 2, 4, 5],
  Electronic: [4, 3, 1, 0, -1, 1, 0, 1, 3, 4],
  Rock: [4, 3, 1, -1, -2, -1, 1, 2, 3, 4],
  Acoustic: [3, 2, 1, 1, 2, 2, 2, 2, 1, 1],
  'Bass cut': [-6, -5, -3, -1, 0, 0, 0, 0, 0, 0],
};

function eqSetting() {
  const s = state.settings;
  const gains = [...(s.eq_gains || EQ_PRESETS.Flat)];
  const presetName = () => Object.keys(EQ_PRESETS).find((k) => EQ_PRESETS[k].every((g, i) => g === gains[i])) || 'Custom';
  const preset = h('select', {});
  const fillPresets = () => {
    const cur = presetName();
    preset.replaceChildren(...Object.keys(EQ_PRESETS).map((k) => h('option', { value: k, selected: k === cur }, k)),
      ...(cur === 'Custom' ? [h('option', { value: 'Custom', selected: true }, 'Custom')] : []));
  };
  const save = throttle(() => updateSettings({ eq_gains: gains }), 150);
  const sliders = gains.map((g, i) => {
    const val = h('span', { class: 'eq-val' }, `${g > 0 ? '+' : ''}${g}`);
    const r = h('input', { type: 'range', min: -12, max: 12, step: 1, value: g, class: 'eq-slider', 'aria-label': `${EQ_BANDS[i]} Hz` });
    r.addEventListener('input', () => {
      gains[i] = Number(r.value);
      val.textContent = `${gains[i] > 0 ? '+' : ''}${gains[i]}`;
      fillPresets();
      save();
    });
    r.addEventListener('dblclick', () => { r.value = 0; r.dispatchEvent(new Event('input')); });
    return h('div', { class: 'eq-band' }, val, r, h('span', { class: 'eq-hz' }, EQ_BANDS[i]));
  });
  preset.addEventListener('change', () => {
    const p = EQ_PRESETS[preset.value];
    if (!p) return;
    p.forEach((g, i) => { gains[i] = g; const r = sliders[i].querySelector('input'); r.value = g; sliders[i].querySelector('.eq-val').textContent = `${g > 0 ? '+' : ''}${g}`; });
    updateSettings({ eq_gains: gains });
  });
  fillPresets();
  const enabled = h('input', { type: 'checkbox', checked: !!s.eq_enabled });
  const panel = h('div', { class: `eq-panel${s.eq_enabled ? '' : ' off'}` }, ...sliders);
  enabled.addEventListener('change', () => { panel.classList.toggle('off', !enabled.checked); updateSettings({ eq_enabled: enabled.checked }); });
  return h('div', { class: 'setting eq-setting' },
    h('div', { class: 'label' }, 'Equalizer', h('small', {}, 'Ten bands from 31 Hz to 16 kHz. Double-click a slider to reset it.')),
    h('div', { class: 'actions' }, preset, h('label', { class: 'switch' }, enabled, h('span'))),
    panel);
}

function crossfadeControl() {
  const s = state.settings;
  const label = h('span', { class: 'meta', style: 'width:34px;text-align:right' }, `${s.crossfade || 0} s`);
  const range = h('input', { type: 'range', min: 0, max: 12, step: 1, value: s.crossfade || 0, style: 'width:140px' });
  range.style.setProperty('--fill', `${((s.crossfade || 0) / 12) * 100}%`);
  range.addEventListener('input', () => { label.textContent = `${range.value} s`; range.style.setProperty('--fill', `${(range.value / 12) * 100}%`); });
  range.addEventListener('change', () => updateSettings({ crossfade: Number(range.value) }));
  return h('div', { class: 'actions' }, range, label);
}

function ytdlpControl() {
  const s = state.settings;
  const sel = h('select', {}, h('option', { value: 'system', selected: s.ytdlp_source === 'system' }, 'System'), h('option', { value: 'bundled', selected: s.ytdlp_source === 'bundled', disabled: !state.tools?.bundled_ytdlp }, 'Tunewire’s copy'));
  sel.addEventListener('change', async () => { await updateSettings({ ytdlp_source: sel.value }); await checkHealth(); renderSettings(); });
  const btn = h('button', { class: 'btn small' }, state.tools?.bundled_ytdlp ? 'Update copy' : 'Install copy');
  btn.addEventListener('click', async () => {
    btn.disabled = true;
    btn.textContent = 'Downloading…';
    try {
      const hres = await call('ytdlp_install');
      toast(`yt-dlp ${hres.version || ''} installed`, 'ok');
      await refreshTools();
      state.settings = await call('get_settings');
      await checkHealth();
      renderSettings();
    } catch (e) {
      toast(errMsg(e), 'error');
      btn.disabled = false;
      btn.textContent = 'Retry';
    }
  });
  return h('div', { class: 'actions' }, sel, btn);
}
