'use strict';

// ===========================================================================
// Plugin host
// ===========================================================================
//
// Each plugin runs in its own Web Worker created from a Blob. The worker has
// no DOM and no access to window.__TAURI__; network APIs are removed unless the
// manifest asks for "network". All capabilities go through postMessage and are
// checked here against the manifest's permissions.

/** Commands a plugin may invoke, and the permission each needs. */
const PLUGIN_COMMANDS = {
  get_playback_state: 'playback', play_track: 'playback', play_tracks: 'playback', play_url: 'playback',
  pause: 'playback', resume: 'playback', toggle_pause: 'playback', stop: 'playback',
  next_track: 'playback', previous_track: 'playback', seek: 'playback', set_volume: 'playback', set_repeat: 'playback',
  get_queue: 'queue', add_to_queue: 'queue', remove_from_queue: 'queue', clear_queue: 'queue',
  reorder_queue: 'queue', shuffle_queue: 'queue', play_index_cmd: 'queue',
  search_tracks: 'search', load_playlist: 'search',
  list_playlists: 'library', create_playlist: 'library', add_to_playlist: 'library',
  list_favorites: 'library', toggle_favorite: 'library', list_history: 'library',
};

const PLUGIN_EVENTS = ['ready', 'track-started', 'track-ended', 'playback', 'queue-changed', 'search-completed', 'library-changed', 'status', 'settings-changed', 'theme-changed'];

const ALLOWED_TAGS = new Set(['DIV', 'SPAN', 'P', 'B', 'I', 'U', 'S', 'STRONG', 'EM', 'SMALL', 'MARK', 'UL', 'OL', 'LI', 'H1', 'H2', 'H3', 'H4', 'BR', 'HR',
  'BUTTON', 'INPUT', 'LABEL', 'SELECT', 'OPTION', 'TEXTAREA', 'IMG', 'TABLE', 'THEAD', 'TBODY', 'TR', 'TD', 'TH', 'CODE', 'PRE',
  'SECTION', 'HEADER', 'FOOTER', 'BLOCKQUOTE', 'A', 'PROGRESS', 'DETAILS', 'SUMMARY', 'FORM']);
const ALLOWED_ATTRS = new Set(['class', 'title', 'type', 'value', 'placeholder', 'checked', 'min', 'max', 'step', 'name', 'alt',
  'rows', 'cols', 'selected', 'disabled', 'colspan', 'rowspan', 'open', 'maxlength', 'for', 'id']);

/** Parse untrusted plugin HTML into a safe fragment (no scripts, handlers, URLs). */
function sanitizeHtml(html, pluginId) {
  const doc = new DOMParser().parseFromString(`<body>${String(html ?? '')}</body>`, 'text/html');
  const walk = (node) => {
    for (const el of [...node.children]) {
      if (!ALLOWED_TAGS.has(el.tagName)) { el.remove(); continue; }
      for (const a of [...el.attributes]) {
        const n = a.name.toLowerCase();
        const okSrc = n === 'src' && el.tagName === 'IMG' && /^(https:|data:image\/(png|jpeg|gif|webp);)/i.test(a.value.trim());
        if (!(ALLOWED_ATTRS.has(n) || n.startsWith('data-') || okSrc)) el.removeAttribute(a.name);
      }
      if (el.id) el.id = `plugin-${pluginId}-${el.id}`;
      if (el.hasAttribute('for')) el.setAttribute('for', `plugin-${pluginId}-${el.getAttribute('for')}`);
      if (el.tagName === 'BUTTON') el.type = 'button';
      if (el.tagName === 'INPUT' && /^(file|image|submit|password)$/i.test(el.type)) el.type = 'text';
      walk(el);
    }
  };
  walk(doc.body);
  const frag = document.createDocumentFragment();
  for (const n of [...doc.body.childNodes]) frag.append(document.importNode(n, true));
  return frag;
}

function workerBootstrap(manifest, settings, pluginCode) {
  const perms = JSON.stringify(manifest.permissions || []);
  return `'use strict';
(() => {
  const perms = new Set(${perms});
  if (!perms.has('network')) {
    for (const k of ['fetch', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'WebTransport', 'importScripts', 'Worker', 'SharedWorker', 'BroadcastChannel', 'indexedDB', 'caches', 'Request']) {
      let o = self;
      while (o) {
        try { if (Object.prototype.hasOwnProperty.call(o, k)) Object.defineProperty(o, k, { value: undefined, writable: false, configurable: false }); } catch (_) {}
        o = Object.getPrototypeOf(o);
      }
    }
  }
})();
const tunewire = (() => {
  const post = self.postMessage.bind(self);
  let seq = 0;
  const pending = new Map();
  const handlers = new Map();
  const actions = new Map();
  let settings = ${JSON.stringify(settings || {})};
  const call = (method, args) => new Promise((resolve, reject) => {
    const id = ++seq;
    pending.set(id, { resolve, reject });
    post({ type: 'call', id, method, args });
  });
  const dispatch = (name, payload) => {
    for (const fn of handlers.get(name) || []) {
      try { const r = fn(payload); if (r && r.catch) r.catch((e) => post({ type: 'error', message: String(e && e.stack || e) })); }
      catch (e) { post({ type: 'error', message: String(e && e.stack || e) }); }
    }
  };
  self.onmessage = (e) => {
    const m = e.data || {};
    if (m.type === 'reply') {
      const p = pending.get(m.id);
      if (!p) return;
      pending.delete(m.id);
      m.ok ? p.resolve(m.value) : p.reject(new Error(m.error));
    } else if (m.type === 'event') {
      if (m.name === 'settings-changed') settings = { ...settings, [m.payload.key]: m.payload.value };
      dispatch(m.name, m.payload);
    } else if (m.type === 'action') {
      const fn = actions.get(m.id);
      try { fn && fn(m.payload); } catch (err) { post({ type: 'error', message: String(err && err.stack || err) }); }
      dispatch('action', { id: m.id, ...m.payload });
    }
  };
  const api = {
    manifest: ${JSON.stringify(manifest)},
    on(name, fn) { if (!handlers.has(name)) handlers.set(name, []); handlers.get(name).push(fn); return () => api.off(name, fn); },
    off(name, fn) { handlers.set(name, (handlers.get(name) || []).filter((f) => f !== fn)); },
    invoke: (cmd, args = {}) => call('invoke', { cmd, args }),
    log: (...a) => post({ type: 'log', args: a.map((x) => { try { return typeof x === 'string' ? x : JSON.stringify(x); } catch (_) { return String(x); } }) }),
    ui: {
      addPanel: ({ id, title, html = '' }) => call('ui.addPanel', { id, title, html }),
      setPanel: (id, html) => call('ui.setPanel', { id, html }),
      removePanel: (id) => call('ui.removePanel', { id }),
      openPanel: (id) => call('ui.openPanel', { id }),
      addButton: ({ id, label, title }, onClick) => { if (onClick) actions.set(id, onClick); return call('ui.addButton', { id, label, title }); },
      removeButton: (id) => { actions.delete(id); return call('ui.removeButton', { id }); },
      onAction: (name, fn) => actions.set(name, fn),
      toast: (message, level = 'info') => call('ui.toast', { message: String(message), level }),
    },
    settings: {
      get: (key) => settings[key],
      all: () => ({ ...settings }),
      set: (key, value) => { settings = { ...settings, [key]: value }; return call('settings.set', { key, value }); },
    },
    storage: {
      get: (key) => call('storage.get', { key }),
      set: (key, value) => call('storage.set', { key, value }),
    },
    playback: {
      state: () => api.invoke('get_playback_state'),
      toggle: () => api.invoke('toggle_pause'),
      next: () => api.invoke('next_track'),
      previous: () => api.invoke('previous_track'),
      seek: (position) => api.invoke('seek', { position }),
      play: (track, platform = null) => api.invoke('play_track', { track, platform }),
    },
  };
  return Object.freeze(api);
})();
(function (tunewire, self, globalThis, window) {
${pluginCode}
}).call(undefined, tunewire, undefined, undefined, undefined);
`;
}

class PluginInstance {
  constructor(src) {
    this.id = src.manifest.id;
    this.manifest = src.manifest;
    this.perms = new Set(src.manifest.permissions || []);
    this.settings = src.settings || {};
    this.panels = new Map();   // id -> {title, html}
    this.buttons = new Map();  // id -> element
    this.lastPlaybackEmit = 0;
    const code = workerBootstrap(src.manifest, this.settings, src.js);
    this.url = URL.createObjectURL(new Blob([code], { type: 'text/javascript' }));
    this.worker = new Worker(this.url, { name: `plugin:${this.id}` });
    this.worker.onmessage = (e) => this.onMessage(e.data);
    this.worker.onerror = (e) => { e.preventDefault(); this.fail(e.message || 'script error'); };
    if (src.css) {
      this.style = h('style', { dataset: { plugin: this.id } }, src.css);
      $('#plugin-css').after(this.style);
    }
    setTimeout(() => this.send('event', { name: 'ready', payload: { playback: state.playback, queue: state.queue } }), 0);
  }

  fail(message) {
    this.error = message;
    console.error(`[plugin:${this.id}]`, message);
    toast(`Plugin “${this.manifest.name}”: ${message}`, 'error');
  }

  send(type, data) {
    try { this.worker.postMessage({ type, ...data }); } catch (e) { this.fail(String(e)); }
  }

  emit(name, payload) {
    if (name === 'playback') {
      const now = Date.now();
      if (now - this.lastPlaybackEmit < 1000) return;
      this.lastPlaybackEmit = now;
    }
    this.send('event', { name, payload });
  }

  need(perm) {
    if (!this.perms.has(perm)) throw new Error(`permission "${perm}" not granted in manifest.json`);
  }

  async onMessage(m) {
    if (!m || typeof m !== 'object') return;
    if (m.type === 'log') return console.log(`[plugin:${this.id}]`, ...(m.args || []));
    if (m.type === 'error') return this.fail(m.message);
    if (m.type !== 'call') return;
    try {
      const value = await this.handle(m.method, m.args || {});
      this.send('reply', { id: m.id, ok: true, value });
    } catch (e) {
      this.send('reply', { id: m.id, ok: false, error: errMsg(e) });
    }
  }

  async handle(method, a) {
    switch (method) {
      case 'invoke': {
        const perm = PLUGIN_COMMANDS[a.cmd];
        if (!perm) throw new Error(`command "${a.cmd}" is not available to plugins`);
        this.need(perm);
        return call(a.cmd, a.args || {});
      }
      case 'ui.addPanel': {
        this.need('ui');
        const id = String(a.id || 'main').replace(/[^\w-]/g, '').slice(0, 40);
        this.panels.set(id, { title: String(a.title || this.manifest.name).slice(0, 60), html: a.html || '' });
        plugins.renderNav();
        return id;
      }
      case 'ui.setPanel': {
        this.need('ui');
        const p = this.panels.get(a.id);
        if (!p) throw new Error(`no panel "${a.id}"`);
        p.html = a.html || '';
        if (plugins.activePanel?.plugin === this && plugins.activePanel.id === a.id) plugins.renderPanel();
        return true;
      }
      case 'ui.removePanel':
        this.need('ui');
        this.panels.delete(a.id);
        plugins.renderNav();
        return true;
      case 'ui.openPanel':
        this.need('ui');
        if (!this.panels.has(a.id)) throw new Error(`no panel "${a.id}"`);
        plugins.openPanel(this, a.id);
        return true;
      case 'ui.addButton': {
        this.need('ui');
        const id = String(a.id).slice(0, 40);
        this.buttons.get(id)?.remove();
        const btn = h('button', { class: 'icon-btn', title: String(a.title || a.label || '').slice(0, 80) }, String(a.label || '•').slice(0, 3));
        btn.addEventListener('click', () => this.send('action', { id, payload: {} }));
        this.buttons.set(id, btn);
        $('#plugin-player-slot').append(btn);
        return true;
      }
      case 'ui.removeButton':
        this.need('ui');
        this.buttons.get(a.id)?.remove();
        this.buttons.delete(a.id);
        return true;
      case 'ui.toast':
        this.need('ui');
        toast(`${this.manifest.name}: ${String(a.message).slice(0, 300)}`, ['info', 'ok', 'warn', 'error'].includes(a.level) ? a.level : 'info');
        return true;
      case 'settings.set': {
        await call('set_plugin_setting', { id: this.id, key: String(a.key), value: a.value ?? null });
        this.settings[a.key] = a.value;
        return true;
      }
      case 'storage.get':
        this.need('storage');
        return state.settings?.plugin_settings?.[this.id]?.[`__storage:${a.key}`] ?? null;
      case 'storage.set': {
        this.need('storage');
        const key = `__storage:${String(a.key).slice(0, 60)}`;
        await call('set_plugin_setting', { id: this.id, key, value: a.value ?? null });
        state.settings.plugin_settings ??= {};
        (state.settings.plugin_settings[this.id] ??= {})[key] = a.value;
        return true;
      }
      default:
        throw new Error(`unknown API "${method}"`);
    }
  }

  stop() {
    this.worker.terminate();
    URL.revokeObjectURL(this.url);
    this.style?.remove();
    for (const b of this.buttons.values()) b.remove();
    this.buttons.clear();
    this.panels.clear();
  }
}

const plugins = {
  running: new Map(),
  activePanel: null,

  async start(id) {
    this.stop(id);
    try {
      const src = await call('get_plugin_source', { id });
      this.running.set(id, new PluginInstance(src));
    } catch (e) {
      toast(`Plugin “${id}” failed to load: ${errMsg(e)}`, 'error');
    }
  },

  stop(id) {
    const p = this.running.get(id);
    if (!p) return;
    p.stop();
    this.running.delete(id);
    if (this.activePanel?.plugin === p) { this.activePanel = null; if (state.view === 'plugin-panel') showView('plugins'); }
    this.renderNav();
  },

  emit(name, payload) {
    if (!PLUGIN_EVENTS.includes(name)) return;
    for (const p of this.running.values()) p.emit(name, payload);
  },

  renderNav() {
    const nav = $('#plugin-nav');
    nav.replaceChildren();
    for (const p of this.running.values()) {
      for (const [id, panel] of p.panels) {
        const active = this.activePanel?.plugin === p && this.activePanel.id === id && state.view === 'plugin-panel';
        nav.append(h('button', { class: `nav-item${active ? ' active' : ''}`, title: panel.title, html: `${icon('puzzle')}<span>${esc(panel.title)}</span>`, onclick: () => this.openPanel(p, id) }));
      }
    }
  },

  openPanel(plugin, id) {
    this.activePanel = { plugin, id };
    showView('plugin-panel');
    this.renderNav();
    this.renderPanel();
  },

  renderPanel() {
    const ap = this.activePanel;
    const panel = ap?.plugin.panels.get(ap.id);
    if (!panel) return;
    $('#plugin-panel-title').textContent = panel.title;
    const body = $('#plugin-panel-body');
    body.replaceChildren(sanitizeHtml(panel.html, ap.plugin.id));
  },
};

/** Forward [data-action] clicks/changes from plugin panels to the plugin. */
function panelAction(e) {
  const el = e.target.closest('[data-action]');
  const ap = plugins.activePanel;
  if (!el || !ap) return;
  if (e.type === 'click' && /^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName) && el.type !== 'checkbox') return;
  const form = {};
  for (const f of $$('[name]', $('#plugin-panel-body'))) {
    form[f.name] = f.type === 'checkbox' ? f.checked : f.value;
  }
  const data = { ...el.dataset };
  delete data.action;
  ap.plugin.send('action', { id: el.dataset.action, payload: { panel: ap.id, value: el.type === 'checkbox' ? el.checked : el.value, data, form } });
}
$('#plugin-panel-body').addEventListener('click', panelAction);
$('#plugin-panel-body').addEventListener('change', panelAction);

async function renderPlugins() {
  try {
    state.plugins = await call('list_plugins');
    $('#plugins-dir').innerHTML = `Plugins folder: <code>${esc(await call('plugins_dir'))}</code>. Each plugin is a folder with <code>manifest.json</code> and <code>main.js</code>, and runs in a sandboxed worker without access to system commands.`;
  } catch (e) { toast(errMsg(e), 'error'); return; }
  const list = $('#plugin-list');
  if (!state.plugins.length) {
    list.replaceChildren(h('div', { class: 'empty' }, h('p', {}, 'No plugins installed.')));
    return;
  }
  list.replaceChildren(...state.plugins.map((p) => {
    const m = p.manifest;
    const toggle = h('input', { type: 'checkbox', checked: p.enabled, disabled: !m });
    toggle.addEventListener('change', async () => {
      try {
        if (toggle.checked) {
          await call('enable_plugin', { id: p.id });
          state.settings = await call('get_settings');
          await plugins.start(p.id);
          toast(`Enabled ${m.name}`, 'ok', 1800);
        } else {
          await call('disable_plugin', { id: p.id });
          plugins.stop(p.id);
          toast(`Disabled ${m.name}`, 'info', 1800);
        }
      } catch (e) { toast(errMsg(e), 'error'); toggle.checked = !toggle.checked; }
      renderPlugins();
    });
    const card = h('div', { class: 'card plugin-card' },
      h('span', { class: 'p-ico', html: icon('puzzle') }),
      h('div', {},
        h('h3', {}, m ? m.name : p.id, m?.version ? h('span', { class: 'faint fw-400 fs-12' }, `v${m.version}`) : null),
        m ? h('p', {}, m.description || '') : null,
        m ? h('div', { class: 'actions mt-8' },
          m.author ? h('span', { class: 'faint fs-12' }, `by ${m.author}`) : null,
          ...(m.permissions || []).map((perm) => h('span', { class: `perm${perm === 'network' ? ' risky' : ''}`, title: perm === 'network' ? 'Can make network requests' : '' }, perm))) : null),
      h('label', { class: 'switch', title: p.enabled ? 'Disable' : 'Enable' }, toggle, h('span')),
      p.error ? h('div', { class: 'plugin-error' }, p.error) : null,
      plugins.running.get(p.id)?.error ? h('div', { class: 'plugin-error' }, `Runtime error: ${plugins.running.get(p.id).error}`) : null);
    if (p.enabled && m?.settings?.length) {
      const saved = state.settings?.plugin_settings?.[p.id] || {};
      const box = h('div', { class: 'plugin-settings' });
      for (const spec of m.settings) {
        const val = saved[spec.key] ?? spec.default;
        let input;
        if (spec.type === 'toggle') {
          input = h('input', { type: 'checkbox', checked: !!val });
        } else if (spec.type === 'select') {
          input = h('select', { class: 'input' }, ...(spec.options || []).map((o) => h('option', { value: o, selected: o === val }, o)));
        } else {
          input = h('input', { class: 'input', type: spec.type === 'number' ? 'number' : 'text', value: val ?? '' });
        }
        input.addEventListener('change', async () => {
          const value = spec.type === 'toggle' ? input.checked : spec.type === 'number' ? Number(input.value) : input.value;
          try {
            await call('set_plugin_setting', { id: p.id, key: spec.key, value });
            ((state.settings.plugin_settings ??= {})[p.id] ??= {})[spec.key] = value;
            plugins.running.get(p.id)?.send('event', { name: 'settings-changed', payload: { key: spec.key, value } });
          } catch (e) { toast(errMsg(e), 'error'); }
        });
        box.append(h('div', { class: 'setting py-6' }, h('div', { class: 'label' }, spec.label), spec.type === 'toggle' ? h('label', { class: 'switch' }, input, h('span')) : input));
      }
      card.append(box);
    }
    return card;
  }));
}
$('#plugins-reload').addEventListener('click', async () => {
  for (const id of [...plugins.running.keys()]) plugins.stop(id);
  state.settings = await call('get_settings');
  for (const id of state.settings.enabled_plugins) await plugins.start(id);
  renderPlugins();
  toast('Plugins reloaded', 'ok', 1500);
});
