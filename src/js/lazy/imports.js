'use strict';

// ===========================================================================
// Importing: your Spotify library, playlist links (Spotify, YouTube Music,
// YouTube, SoundCloud, Apple Music) and CSV files from export tools.
// ===========================================================================

const importUi = { progress: null };
listen('import-progress', (e) => importUi.progress?.(e.payload));

function openImport() {
  const status = h('div', { class: 'import-status meta' });
  const linkInput = h('input', { class: 'input', placeholder: 'Paste a Spotify, Apple Music, YouTube or SoundCloud link', spellcheck: 'false' });
  const linkBtn = h('button', { class: 'btn primary' }, 'Import');
  const file = h('input', { type: 'file', accept: '.csv,text/csv,.tsv,.txt', hidden: true });
  const m = openModal({
    title: 'Import playlists',
    subtitle: 'Bring your music from other apps. Songs are matched to playable recordings automatically.',
    wide: true,
    body: h('div', { class: 'import-box' },
      h('button', { class: 'import-option', onclick: () => { m.close(); openSpotifyImport(); } },
        h('span', { class: 'src-ico spotify', html: plogo('spotify') }),
        h('div', {}, h('b', {}, 'Your Spotify library'), h('span', {}, 'Liked Songs and all your playlists, private ones too. You log in once in your browser.')),
        h('span', { html: icon('chevRight') })),
      h('button', { class: 'import-option', onclick: () => { m.close(); importSpotifyProfile(); } },
        h('span', { class: 'src-ico', html: icon('user') }),
        h('div', {}, h('b', {}, 'Someone’s public Spotify playlists'), h('span', {}, 'Paste a Spotify profile link.')),
        h('span', { html: icon('chevRight') })),
      h('div', { class: 'import-label' }, 'A playlist or album link'),
      h('div', { class: 'import-link' }, linkInput, linkBtn),
      h('div', { class: 'import-label' }, 'A CSV file'),
      h('button', { class: 'import-option', onclick: () => file.click() },
        h('span', { class: 'src-ico', html: icon('upload') }),
        h('div', {}, h('b', {}, 'Choose a CSV file'), h('span', {}, 'From Exportify, TuneMyMusic, Soundiiz or any spreadsheet with title and artist columns.')),
        h('span', { html: icon('chevRight') })),
      file, status),
  });
  const busy = (on, text = '') => {
    linkBtn.disabled = on;
    status.innerHTML = on ? `<span class="spinner"></span> ${esc(text)}` : esc(text);
  };
  importUi.progress = (p) => { if (status.isConnected) status.innerHTML = `<span class="spinner"></span> Matching songs · ${p.done} of ${p.total}`; };
  const importLink = async () => {
    const url = linkInput.value.trim();
    if (!url) return linkInput.focus();
    busy(true, 'Reading the playlist…');
    try {
      let id, missing = 0;
      if (/music\.apple\.com\//.test(url)) {
        const r = await call('import_apple_music', { url });
        id = r.playlist.id;
        missing = r.missing;
      } else {
        const pl = await call('load_playlist', { url });
        const created = await call('create_playlist', { name: pl.title || 'Imported playlist', tracks: pl.tracks, sourceUrl: url });
        id = created.id;
      }
      m.close();
      toast(missing ? `Imported. ${missing} songs couldn’t be found.` : 'Playlist imported', 'ok', 4000);
      await refreshPlaylists({ silent: true });
      openPlaylist(id);
    } catch (e) {
      busy(false, errMsg(e));
    }
  };
  linkBtn.addEventListener('click', importLink);
  linkInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') importLink(); });
  file.addEventListener('change', async () => {
    const f = file.files[0];
    if (!f) return;
    let rows;
    try { rows = csvRows(await f.text()); } catch (e) { busy(false, e.message); return; }
    if (!rows.length) { busy(false, 'No songs found in that file. It needs a title (or track name) column.'); return; }
    const name = f.name.replace(/\.(csv|tsv|txt)$/i, '').replace(/[_-]+/g, ' ').trim() || 'Imported playlist';
    busy(true, `Found ${rows.length} songs. Matching…`);
    try {
      const r = await call('import_rows', { name, rows });
      m.close();
      toast(r.missing ? `Imported “${name}”. ${r.missing} songs couldn’t be found.` : `Imported “${name}”`, 'ok', 4000);
      await refreshPlaylists({ silent: true });
      openPlaylist(r.playlist.id);
    } catch (e) { busy(false, errMsg(e)); }
  });
  setTimeout(() => linkInput.focus(), 40);
}

// CSV -------------------------------------------------------------------------------

function parseCsv(text) {
  const first = text.split(/\r?\n/, 1)[0] || '';
  const delim = [',', ';', '\t'].map((d) => [d, first.split(d).length]).sort((a, b) => b[1] - a[1])[0][0];
  const rows = [];
  let row = [], field = '', quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') { field += '"'; i++; }
      else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"') quoted = true;
    else if (c === delim) { row.push(field); field = ''; }
    else if (c === '\n' || c === '\r') {
      if (c === '\r' && text[i + 1] === '\n') i++;
      row.push(field); field = '';
      if (row.some((x) => x.trim())) rows.push(row);
      row = [];
    } else field += c;
  }
  row.push(field);
  if (row.some((x) => x.trim())) rows.push(row);
  return rows;
}

/** Map an export's columns to {title, artist, album, duration, spotify_id}. */
function csvRows(text) {
  const table = parseCsv(text.replace(/^﻿/, ''));
  if (table.length < 2) return [];
  const head = table[0].map((x) => x.trim().toLowerCase());
  const find = (...names) => head.findIndex((x) => names.includes(x));
  const iTitle = find('track name', 'title', 'name', 'song', 'track', 'song name', 'track title');
  const iArtist = find('artist name(s)', 'artist name', 'artist', 'artists', 'artist(s)', 'artist names');
  const iAlbum = find('album name', 'album', 'release', 'album title');
  const iMs = find('duration (ms)', 'duration_ms', 'track duration (ms)');
  const iDur = find('duration', 'time', 'length');
  const iUri = find('track uri', 'spotify uri', 'uri', 'spotify id', 'track id', 'spotify - id', 'spotify track id');
  if (iTitle < 0) throw new Error('This file has no title / track name column.');
  const out = [];
  for (const r of table.slice(1)) {
    const title = (r[iTitle] || '').trim();
    if (!title) continue;
    let duration = null;
    if (iMs >= 0 && Number(r[iMs])) duration = Number(r[iMs]) / 1000;
    else if (iDur >= 0 && r[iDur]) {
      const d = r[iDur].trim();
      duration = /:/.test(d) ? d.split(':').reduce((a, b) => a * 60 + Number(b), 0) : Number(d) > 10000 ? Number(d) / 1000 : Number(d) || null;
    }
    const uri = iUri >= 0 ? (r[iUri] || '') : '';
    const spotifyId = uri.match(/(?:track[:/])?([A-Za-z0-9]{22})(?:\b|$)/)?.[1] || null;
    out.push({ title, artist: iArtist >= 0 ? (r[iArtist] || '').replace(/\s*;\s*/g, ', ').trim() : '', album: iAlbum >= 0 ? (r[iAlbum] || '').trim() || null : null, duration, spotify_id: spotifyId });
  }
  return out;
}

// Spotify library ----------------------------------------------------------------------

// What the Spotify sign-in and listing are doing (shown while waiting).
listen('spotify-login-step', (e) => { const el = document.querySelector('.sp-step'); if (el) el.textContent = String(e.payload); });

async function openSpotifyImport() {
  const body = h('div', { class: 'import-box' });
  const footer = h('div', { class: 'actions' });
  let waiting = false;
  // Closing the dialog while the browser sign-in is open stops the wait.
  const m = openModal({ title: 'Import your Spotify library', wide: true, body, footer: [footer], onClose: () => { if (waiting) call('cancel_sign_in').catch(() => {}); } });
  const loginBtn = h('button', { class: 'btn primary' }, 'Log in with Spotify');
  body.replaceChildren(
    h('p', {}, 'Your Liked Songs and playlists come over to Tunewire. Spotify’s login page opens in your browser; Tunewire only asks to read your library.'),
    h('p', { class: 'muted small' }, 'This works with free accounts too. Songs play from their matching recordings on YouTube Music.'));
  footer.replaceChildren(h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), loginBtn);
  loginBtn.addEventListener('click', async () => {
    loginBtn.disabled = true;
    body.replaceChildren(h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), h('span', { class: 'sp-step' }, 'Waiting for you to log in in your browser…')),
      h('p', { class: 'muted small' }, 'Didn’t open? Check your browser for a new tab. After you approve, come back here.'));
    footer.replaceChildren(h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'));
    waiting = true;
    let lib;
    try { lib = await call('spotify_library_login'); waiting = false; } catch (e) {
      waiting = false;
      if (e?.kind === 'cancelled') return;
      footer.replaceChildren(h('button', { class: 'btn', onclick: () => m.close() }, 'Close'), loginBtn);
      body.replaceChildren(h('p', { class: 'muted' }, errMsg(e)));
      loginBtn.disabled = false;
      loginBtn.textContent = 'Try again';
      return;
    }
    if (!body.isConnected) return;
    pickSpotify(lib, body, footer, m);
  });
}

function pickSpotify(lib, body, footer, m, opts = {}) {
  if (!lib.playlists.length && !lib.liked) {
    body.replaceChildren(
      h('p', {}, opts.heading ? 'No public playlists were found for this profile.' : h('span', {}, 'Logged in as ', h('b', {}, lib.user), ', but Spotify didn’t list any playlists or Liked Songs for this account.')),
      h('p', { class: 'muted small' }, 'You can still bring playlists over one at a time: paste a Spotify playlist link into the search box, or use Import a link or file.'));
    footer.replaceChildren(h('button', { class: 'btn primary', onclick: () => m.close() }, 'OK'));
    return;
  }
  const picked = new Set(lib.playlists.map((p) => p.id));
  let liked = lib.liked > 0;
  const likedCb = h('input', { type: 'checkbox', checked: liked, disabled: !lib.liked, onchange: (e) => { liked = e.target.checked; paint(); } });
  const all = h('input', { type: 'checkbox', checked: true, onchange: (e) => { lib.playlists.forEach((p) => (e.target.checked ? picked.add(p.id) : picked.delete(p.id))); $$('.sp-pl', body).forEach((c) => { c.checked = e.target.checked; }); paint(); } });
  const go = h('button', { class: 'btn primary' });
  const paint = () => { go.textContent = `Import ${picked.size + (liked ? 1 : 0)}`; go.disabled = !picked.size && !liked; };
  body.replaceChildren(...[
    opts.heading ? h('p', {}, opts.heading) : h('p', {}, `Logged in as `, h('b', {}, lib.user), '. Choose what to bring over:'),
    lib.liked ? h('label', { class: 'import-row' }, likedCb, h('div', { class: 'lib-art liked size-36', html: icon('heartFill') }), h('span', { class: 'name' }, 'Liked Songs'), h('span', { class: 'meta' }, `${lib.liked} songs → your Liked Songs`)) : null,
    lib.playlists.length ? h('label', { class: 'import-row border-b'}, all, h('span', { class: 'name fw-700' }, `All playlists (${lib.playlists.length})`)) : null,
    h('div', { class: 'import-list' }, ...lib.playlists.map((p) => h('label', { class: 'import-row' },
      h('input', { type: 'checkbox', class: 'sp-pl', checked: true, onchange: (e) => { e.target.checked ? picked.add(p.id) : picked.delete(p.id); paint(); } }),
      thumbEl(p.thumbnail, 'thumb'), h('span', { class: 'name' }, p.name), h('span', { class: 'meta' }, `${p.total} songs${p.owner && p.owner !== lib.user ? ` · by ${p.owner}` : ''}`))))].filter(Boolean));
  footer.replaceChildren(h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), go);
  paint();
  go.addEventListener('click', async () => {
    go.disabled = true;
    const chosen = lib.playlists.filter((p) => picked.has(p.id)).map((p) => [p.id, p.name]);
    const line = h('div', { class: 'loading-line' }, h('span', { class: 'spinner' }), 'Starting…');
    const bar = h('div', { class: 'match-bar maxw-none' }, h('i'));
    body.replaceChildren(line, bar);
    importUi.progress = (p) => {
      if (!line.isConnected) return;
      line.lastChild.textContent = p.label === 'Done' ? 'Finishing…' : `Importing ${p.label} (${Math.min(p.done + 1, p.total)} of ${p.total})`;
      bar.firstChild.style.width = `${(p.done / Math.max(1, p.total)) * 100}%`;
    };
    try {
      const r = await call('spotify_library_import', { liked, playlists: chosen });
      m.close();
      const parts = [];
      if (r.liked) parts.push(`${r.liked} liked songs`);
      if (r.playlists) parts.push(`${r.playlists} playlist${r.playlists === 1 ? '' : 's'}`);
      toast(`Imported ${parts.join(' and ') || 'nothing new'}.${r.missing ? ` ${r.missing} playlist${r.missing === 1 ? '' : 's'} couldn’t be read (Spotify refused it).` : ''} Songs are being matched for instant playback in the background.`, r.missing ? 'warn' : 'ok', 7000);
      await refreshPlaylists({ silent: true });
      if (r.first_id) openPlaylist(r.first_id);
      else if (r.liked) showView('favorites');
    } catch (e) {
      body.replaceChildren(h('p', { class: 'muted' }, errMsg(e)));
      go.disabled = false;
    }
  });
}
