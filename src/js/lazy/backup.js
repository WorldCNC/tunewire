'use strict';

// Settings → Backup and Settings → Scrobbling (loaded when Settings opens).

// Backup & restore ----------------------------------------------------------------
async function exportBackup() {
  try {
    const name = await call('backup_file_name');
    const path = await window.__TAURI__.dialog.save({ title: 'Save a backup of your library', defaultPath: name, filters: [{ name: 'Tunewire backup', extensions: ['tunewire-backup'] }] });
    if (!path) return;
    const s = await call('export_backup', { path });
    toast(`Backup saved: ${s.playlists} playlists, ${s.songs} songs, ${s.favorites} liked songs, ${s.plays} plays`, 'ok', 5000);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function pickAndRestore() {
  try {
    const r = await window.__TAURI__.dialog.open({ title: 'Restore a backup', multiple: false, filters: [{ name: 'Tunewire backup', extensions: ['tunewire-backup', 'json'] }] });
    const path = Array.isArray(r) ? r[0] : r;
    if (path) await restoreFrom(path);
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function restoreFrom(path) {
  let s;
  try { s = await call('inspect_backup', { path }); } catch (e) { toast(errMsg(e), 'error'); return; }
  const ok = await confirmModal({
    title: 'Restore this backup?',
    message: `From ${new Date(s.created).toLocaleString()}: ${s.playlists} playlists (${s.songs} songs), ${s.favorites} liked songs, ${s.plays} plays and ${s.themes} themes. This replaces your library, history and settings. Your current library is saved first, so you can go back.`,
    okLabel: 'Restore',
    danger: true,
  });
  if (!ok) return;
  try {
    await call('restore_backup', { path });
    toast('Backup restored', 'ok', 2500);
    // Simplest way to show everything that changed.
    setTimeout(() => location.reload(), 600);
  } catch (e) { toast(errMsg(e), 'error'); }
}

function backupSettingsRows() {
  const list = h('div', { class: 'snapshot-list' }, h('span', { class: 'meta' }, 'Loading…'));
  call('list_snapshots').then((snaps) => {
    if (!snaps.length) { list.replaceChildren(h('span', { class: 'meta' }, 'None yet. A copy is made before every restore.')); return; }
    list.replaceChildren(...snaps.slice(0, 6).map((sn) => h('div', { class: 'snapshot' },
      h('span', { class: 'grow' }, sn.name.startsWith('before-restore') ? 'Before a restore' : sn.name, h('span', { class: 'meta' }, ` · ${new Date(sn.modified).toLocaleString()}`)),
      h('button', { class: 'btn small', onclick: () => restoreFrom(sn.path) }, 'Restore'))));
  }).catch(() => list.replaceChildren());
  return [
    settingRow('Back up your library', 'One file with your playlists, Liked Songs, followed artists, history, listening stats and custom themes. Passwords and audio files are never included. Keep it somewhere safe, or move your library to another computer.',
      h('div', { class: 'actions' }, h('button', { class: 'btn small primary', onclick: exportBackup }, 'Save backup…'), h('button', { class: 'btn small', onclick: pickAndRestore }, 'Restore…'))),
    settingRow('Automatic copies', 'Tunewire also keeps a daily copy of your playlists (last 7 days) and repairs a damaged library file from it on its own.', list),
  ];
}

// Scrobbling ------------------------------------------------------------------------
function scrobbleSettingsRows() {
  const box = h('div', { class: 'lazy-rows' });
  const paint = async () => {
    const st = await call('scrobble_status').catch(() => null);
    const s = state.settings;
    if (!st) { box.replaceChildren(); return; }

    const lfmBtn = h('button', { class: 'btn small primary' }, 'Connect Last.fm');
    lfmBtn.addEventListener('click', async () => {
      lfmBtn.disabled = true;
      lfmBtn.textContent = 'Approve in your browser…';
      try {
        const name = await call('lastfm_connect');
        state.settings = await call('get_settings');
        toast(`Scrobbling to Last.fm as ${name}`, 'ok', 2500);
      } catch (e) { toast(errMsg(e), 'error'); }
      paint();
    });
    const lfm = st.lastfm_connected
      ? h('div', { class: 'actions' }, h('span', { class: 'status-pill ok' }, s.lastfm_user ? `Connected: ${s.lastfm_user}` : 'Connected'),
        h('button', { class: 'btn small', onclick: async () => { await call('scrobble_disconnect', { service: 'lastfm' }); state.settings = await call('get_settings'); paint(); } }, 'Disconnect'))
      : st.lastfm_available ? lfmBtn : h('span', { class: 'meta' }, 'Add your own Last.fm API account below to connect.');

    const token = h('input', { type: 'password', class: 'input w-200', placeholder: 'User token', autocomplete: 'off', spellcheck: 'false', 'aria-label': 'ListenBrainz user token' });
    const lb = st.listenbrainz_connected
      ? h('div', { class: 'actions' }, h('span', { class: 'status-pill ok' }, s.listenbrainz_user ? `Connected: ${s.listenbrainz_user}` : 'Connected'),
        h('button', { class: 'btn small', onclick: async () => { await call('scrobble_disconnect', { service: 'listenbrainz' }); state.settings = await call('get_settings'); paint(); } }, 'Disconnect'))
      : h('div', { class: 'actions' }, token, h('button', { class: 'btn small primary', onclick: async () => {
          if (!token.value.trim()) return;
          try {
            const name = await call('listenbrainz_connect', { token: token.value.trim() });
            state.settings = await call('get_settings');
            toast(`Scrobbling to ListenBrainz as ${name}`, 'ok', 2500);
          } catch (e) { toast(errMsg(e), 'error'); }
          paint();
        } }, 'Connect'));

    const key = h('input', { type: 'text', class: 'input w-150', value: s.lastfm_api_key || '', placeholder: 'API key', spellcheck: 'false', 'aria-label': 'Last.fm API key' });
    const secret = h('input', { type: 'password', class: 'input w-150', placeholder: 'Shared secret', autocomplete: 'off', 'aria-label': 'Last.fm shared secret' });
    const own = h('details', { class: 'own-key' }, h('summary', {}, 'Use your own Last.fm API account'),
      h('p', { class: 'meta' }, 'Create one at last.fm/api/account/create (any name; no callback needed). The secret is kept in your system keychain.'),
      h('div', { class: 'actions' }, key, secret, h('button', { class: 'btn small', onclick: async () => {
        try { await call('set_lastfm_app', { key: key.value, secret: secret.value }); state.settings = await call('get_settings'); toast('Saved', 'ok', 1500); paint(); } catch (e) { toast(errMsg(e), 'error'); }
      } }, 'Save')));

    box.replaceChildren(...[
      settingRow('Last.fm', 'Songs you listen to (half the song, or 4 minutes) are added to your Last.fm profile. Now playing shows there too.', lfm),
      settingRow('ListenBrainz', 'The open alternative. Paste your user token from listenbrainz.org/settings.', lb),
      st.pending ? settingRow('Waiting to send', `${st.pending} listen${st.pending === 1 ? '' : 's'} will be sent when the service can be reached.`, h('span')) : null,
      h('div', { class: 'setting' }, own)].filter(Boolean));
  };
  paint();
  return [box];
}
