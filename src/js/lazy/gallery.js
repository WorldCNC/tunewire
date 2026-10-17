'use strict';

// Theme & plugin gallery (loaded on first use). The list and every file come
// from the Tunewire repository; the backend checks each file's checksum and
// that a plugin asks for exactly the permissions shown here.

const PERMISSION_TEXT = {
  playback: 'Control playback',
  queue: 'Change the queue',
  search: 'Search for songs',
  library: 'Read and change your playlists and Liked Songs',
  ui: 'Add panels and buttons',
  storage: 'Keep its own small notes',
  network: 'Connect to the internet',
};

async function openGallery(kind = 'themes') {
  const body = h('div', { class: 'gallery' }, h('div', { class: 'gallery-grid' }, ...Array.from({ length: 6 }, () => h('div', { class: 'gallery-card skeleton' }, h('div', { class: 'sk sk-block' }), h('div', { class: 'sk w-60' }), h('div', { class: 'sk w-40' })))));
  const tabs = h('div', { class: 'chips', role: 'tablist' });
  const m = openModal({ title: 'Gallery', subtitle: 'Themes and plugins picked by the Tunewire project. Everything is checked before it’s installed.', body: h('div', {}, tabs, body), wide: true, footer: [h('button', { class: 'btn primary', onclick: () => m.close() }, 'Done')] });
  let g;
  try { g = await call('gallery_index'); } catch (e) { body.replaceChildren(h('p', { class: 'meta' }, `Couldn’t load the gallery: ${errMsg(e)}`)); return; }
  const installedThemes = new Set((await call('list_themes').catch(() => [])).map((t) => t.name));
  const installedPlugins = new Set((await call('list_plugins').catch(() => [])).map((p) => p.id));
  const show = (k) => {
    kind = k;
    tabs.replaceChildren(...[['themes', `Themes (${g.themes.length})`], ['plugins', `Plugins (${g.plugins.length})`]].map(([id, label]) =>
      h('button', { role: 'tab', 'aria-selected': String(id === kind), class: id === kind ? 'active' : '', onclick: () => show(id) }, label)));
    const items = kind === 'themes' ? g.themes : g.plugins;
    body.replaceChildren(items.length ? h('div', { class: 'gallery-grid' }, ...items.map((it) => (kind === 'themes' ? themeCard(it, installedThemes) : pluginCard(it, installedPlugins)))) : h('p', { class: 'meta' }, 'Nothing here yet.'));
  };
  show(kind);
}

function themeCard(t, installed) {
  const [bg, accent, text] = [...(t.preview || []), '#111', '#4d8dff', '#eee'];
  const btn = h('button', { class: 'btn small primary' }, installed.has(t.name) ? 'Installed' : 'Install');
  btn.disabled = installed.has(t.name);
  btn.addEventListener('click', async () => {
    btn.disabled = true;
    btn.textContent = 'Installing…';
    try {
      const meta = await call('gallery_install', { kind: 'theme', id: t.id });
      installed.add(t.name);
      btn.textContent = 'Installed';
      toast(`“${t.name}” installed`, 'ok', 2500, { action: { label: 'Use it', onClick: async () => { applyTheme(await call('load_theme', { id: meta.id })); state.settings.theme = meta.id; renderThemes?.(); } } });
      state.themes = await call('list_themes').catch(() => state.themes);
    } catch (e) { btn.disabled = false; btn.textContent = 'Install'; toast(errMsg(e), 'error'); }
  });
  const swatch = h('div', { class: 'theme-swatch' });
  swatch.style.setProperty('--sw-bg', bg);
  swatch.style.setProperty('--sw-accent', accent);
  swatch.style.setProperty('--sw-text', text);
  swatch.append(h('span', { class: 'sw-bar' }), h('span', { class: 'sw-dot' }), h('span', { class: 'sw-line' }), h('span', { class: 'sw-line short' }));
  return h('div', { class: 'gallery-card' }, swatch,
    h('b', {}, t.name), h('div', { class: 'meta' }, t.author ? `by ${t.author}` : ''), t.description ? h('p', { class: 'small' }, t.description) : null,
    h('div', { class: 'actions' }, btn));
}

function pluginCard(p, installed) {
  const btn = h('button', { class: 'btn small primary' }, installed.has(p.id) ? 'Update' : 'Install');
  btn.addEventListener('click', async () => {
    const ok = await confirmModal({
      title: `Install “${p.name}”?`,
      message: `${p.name} can: ${(p.permissions.length ? p.permissions.map((x) => PERMISSION_TEXT[x] || x).join(' · ') : 'nothing beyond showing up in the Plugins list')}. Plugins run in a sandbox and can’t run programs or read your files.${p.permissions.includes('network') ? ' This one can connect to the internet.' : ''}`,
      okLabel: 'Install',
    });
    if (!ok) return;
    btn.disabled = true;
    btn.textContent = 'Installing…';
    try {
      await call('gallery_install', { kind: 'plugin', id: p.id });
      installed.add(p.id);
      btn.textContent = 'Installed';
      toast(`“${p.name}” installed. Turn it on in Plugins.`, 'ok', 3000, { action: { label: 'Open Plugins', onClick: () => showView('plugins') } });
      if (state.view === 'plugins') renderPlugins();
    } catch (e) { btn.disabled = false; btn.textContent = 'Install'; toast(errMsg(e), 'error'); }
  });
  return h('div', { class: 'gallery-card' },
    h('div', { class: 'plugin-ico', html: icon('puzzle') }),
    h('b', {}, p.name, p.version ? h('span', { class: 'meta' }, ` v${p.version}`) : null), h('div', { class: 'meta' }, p.author ? `by ${p.author}` : ''),
    p.description ? h('p', { class: 'small' }, p.description) : null,
    h('div', { class: 'perm-list' }, ...p.permissions.map((x) => h('span', { class: `perm${x === 'network' ? ' warn' : ''}` }, PERMISSION_TEXT[x] || x))),
    h('div', { class: 'actions' }, btn));
}
