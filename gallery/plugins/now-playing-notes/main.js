// Example Tunewire plugin.
//
// Runs inside a sandboxed Web Worker: no DOM, no Tauri bridge, no network
// (this manifest does not request "network"). Everything goes through the
// `tunewire` object; see README.md → "Plugin API".

let current = null; // { item, version } of the playing track
const session = { started: 0, platforms: {} };

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

async function render() {
  const t = current?.item?.track;
  const note = t ? (await tunewire.storage.get(`note:${t.key}`)) || '' : '';
  const platforms = Object.entries(session.platforms)
    .map(([p, n]) => `<li>${esc(p)}: <b>${n}</b></li>`)
    .join('');
  await tunewire.ui.setPanel('main', `
    <section class="npn">
      <h3>${t ? esc(t.title) : 'Nothing playing'}</h3>
      <p class="npn-sub">${t ? esc(t.artist || 'Unknown artist') + (current.version ? ' · via ' + esc(current.version.platform) : '') : 'Start a track to take notes on it.'}</p>
      ${t ? `
        <textarea name="note" rows="5" placeholder="Your notes about this track…">${esc(note)}</textarea>
        <div class="npn-row">
          <button data-action="save-note">Save note</button>
          <button data-action="more-by-artist">More by this artist</button>
        </div>` : ''}
      <h4>This session</h4>
      <ul>
        <li>Tracks started: <b>${session.started}</b></li>
        ${platforms}
      </ul>
    </section>`);
}

tunewire.on('ready', async ({ playback }) => {
  await tunewire.ui.addPanel({ id: 'main', title: 'Now Playing Notes' });
  await tunewire.ui.addButton({ id: 'info', label: '✎', title: 'Open Now Playing Notes' }, () => tunewire.ui.openPanel('main'));
  if (playback?.current) current = { item: playback.current, version: playback.version };
  await render();
});

tunewire.on('track-started', async ({ item, version }) => {
  current = { item, version };
  session.started += 1;
  session.platforms[version.platform] = (session.platforms[version.platform] || 0) + 1;
  if (tunewire.settings.get('toast_on_change')) {
    tunewire.ui.toast(`Now playing: ${item.track.title} — ${item.track.artist || 'Unknown'}`);
  }
  await render();
});

tunewire.ui.onAction('save-note', async ({ form }) => {
  const t = current?.item?.track;
  if (!t) return;
  await tunewire.storage.set(`note:${t.key}`, form.note || '');
  tunewire.ui.toast('Note saved', 'ok');
});

tunewire.ui.onAction('more-by-artist', async () => {
  const t = current?.item?.track;
  if (!t?.artist) return;
  const n = Number(tunewire.settings.get('similar_count')) || 3;
  try {
    const res = await tunewire.invoke('search_tracks', { query: t.artist, source: 'all', limit: 10 });
    const picks = res.tracks.filter((x) => x.key !== t.key).slice(0, n);
    if (!picks.length) return tunewire.ui.toast('Nothing found', 'warn');
    await tunewire.invoke('add_to_queue', { tracks: picks, next: false });
    tunewire.ui.toast(`Queued ${picks.length} more by ${t.artist}`, 'ok');
  } catch (e) {
    tunewire.ui.toast(`Search failed: ${e.message}`, 'error');
  }
});
