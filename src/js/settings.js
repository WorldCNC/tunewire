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
    ...[['auto', 'Automatic'], ['spotdl', 'spotDL only'], ['embed', 'Embed page only'], ['off', 'Off']]
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
  const clientId = h('input', { type: 'text', value: s.discord_client_id || '', placeholder: 'Application ID', style: 'width:190px', spellcheck: 'false' });
  clientId.addEventListener('change', () => updateSettings({ discord_client_id: clientId.value.trim() }));
  const recheck = (fn) => h('button', { class: 'btn small', onclick: async () => { await fn(); renderSettings(); } }, 'Check again');

  body.replaceChildren(
    group('Playback', null,
      settingRow('Source priority', 'When a song exists on several platforms, the first available one plays. If it fails, the next one is tried.', order),
      settingRow('Results per source', null, limit),
      settingToggle('Search YouTube in “All”', 'Adds regular YouTube uploads (live versions, covers) to combined searches.', 'include_youtube_in_all'),
      settingToggle('Close to tray', 'Closing the window keeps music playing. Quit from the tray icon.', 'close_to_tray')),
    group('Spotify', 'spotify',
      settingRow('Metadata source', 'Spotify audio is DRM-protected, so songs play from a matching YouTube Music or SoundCloud upload. spotDL adds search and is the fallback for playlist import; the embed page loads pasted links instantly.', backend),
      settingRow('spotDL', spotdl ? `Version ${spotdl}` : 'Not found. Install with <code>pipx install spotdl</code> or <code>uv tool install spotdl</code>.',
        h('div', { class: 'actions' }, h('span', { class: `status-pill ${spotdl ? 'ok' : 'bad'}` }, spotdl ? 'Installed' : 'Missing'), recheck(refreshTools))),
      settingToggle('Search Spotify in “All”', 'Waits up to 15 seconds for Spotify, then shows the other results without it.', 'include_spotify_in_all'),
      settingRow('Spotify app credentials', 'Optional. spotDL’s built-in credentials are shared and often rate-limited, which can make a search take a minute. A free app from <code>developer.spotify.com/dashboard</code> avoids that (redirect URI: <code>http://127.0.0.1:9900/</code>). The secret is stored in your system keychain, not in a file.', h('div', { class: 'actions' }, credId, credSecret))),
    group('Sound', null,
      settingToggle('Volume normalization', 'Evens out loudness between songs and platforms. Measured from the first seconds of each song, with a soft limiter against clipping.', 'normalize'),
      settingRow('Crossfade', 'Overlap the end of a song with the start of the next. At 0, songs still play back to back without a gap.', crossfadeControl())),
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
