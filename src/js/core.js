/* Tunewire frontend — vanilla JS, talks to the Rust backend only via Tauri IPC. */
'use strict';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ===========================================================================
// Utilities
// ===========================================================================

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

const ICONS = {
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  list: '<path d="M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01"/>',
  heart: '<path d="M12 20s-7-4.5-9.3-9A5.2 5.2 0 0 1 12 5.6 5.2 5.2 0 0 1 21.3 11C19 15.5 12 20 12 20z"/>',
  heartFill: '<path fill="currentColor" d="M12 20s-7-4.5-9.3-9A5.2 5.2 0 0 1 12 5.6 5.2 5.2 0 0 1 21.3 11C19 15.5 12 20 12 20z"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  palette: '<path d="M12 3a9 9 0 1 0 0 18c1.1 0 2-.9 2-2 0-.5-.2-1-.5-1.3-.3-.4-.5-.8-.5-1.3 0-1.1.9-2 2-2h2.3A4.7 4.7 0 0 0 22 9.7C22 5.9 17.5 3 12 3z"/><circle cx="7.5" cy="11" r="1"/><circle cx="10" cy="7" r="1"/><circle cx="15" cy="7" r="1"/>',
  puzzle: '<path d="M10 4a2 2 0 1 1 4 0v2h4v4h-2a2 2 0 1 0 0 4h2v4h-4v-2a2 2 0 1 0-4 0v2H6v-4H4a2 2 0 1 1 0-4h2V6h4z"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  queue: '<path d="M3 6h12M3 12h12M3 18h7"/><path d="M16 14.5v6l5-3z" fill="currentColor"/>',
  play: '<path d="M7 4.5v15l13-7.5z" fill="currentColor" stroke="none"/>',
  pause: '<rect x="6" y="4.5" width="4" height="15" rx="1" fill="currentColor" stroke="none"/><rect x="14" y="4.5" width="4" height="15" rx="1" fill="currentColor" stroke="none"/>',
  prev: '<path d="M19 5v14L9 12z" fill="currentColor"/><path d="M6 5v14"/>',
  next: '<path d="M5 5v14l10-7z" fill="currentColor"/><path d="M18 5v14"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor" stroke="none"/>',
  repeat: '<path d="m17 2 4 4-4 4"/><path d="M3 11V9a3 3 0 0 1 3-3h15"/><path d="m7 22-4-4 4-4"/><path d="M21 13v2a3 3 0 0 1-3 3H3"/>',
  repeatOne: '<path d="m17 2 4 4-4 4"/><path d="M3 11V9a3 3 0 0 1 3-3h15"/><path d="m7 22-4-4 4-4"/><path d="M21 13v2a3 3 0 0 1-3 3H3"/><path d="M11 10.5h1.2V15"/>',
  shuffle: '<path d="M16 3h5v5"/><path d="M4 20 21 3"/><path d="M21 16v5h-5"/><path d="m15 15 6 6"/><path d="m4 4 5 5"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
  save: '<path d="M5 3h11l3 3v15H5z"/><path d="M8 3v5h7V3M8 21v-7h8v7"/>',
  volume: '<path d="M4 9v6h4l5 4V5L8 9z" fill="currentColor"/><path d="M16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12"/>',
  volumeLow: '<path d="M4 9v6h4l5 4V5L8 9z" fill="currentColor"/><path d="M16.5 8.5a5 5 0 0 1 0 7"/>',
  mute: '<path d="M4 9v6h4l5 4V5L8 9z" fill="currentColor"/><path d="m17 9 5 5M22 9l-5 5"/>',
  keyboard: '<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>',
  upload: '<path d="M12 16V4M7 9l5-5 5 5M4 20h16"/>',
  download: '<path d="M12 4v12M7 11l5 5 5-5M4 20h16"/>',
  clipboard: '<rect x="8" y="3" width="8" height="4" rx="1"/><path d="M8 5H5v16h14V5h-3"/>',
  refresh: '<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/>',
  dots: '<circle cx="5" cy="12" r="1.4" fill="currentColor"/><circle cx="12" cy="12" r="1.4" fill="currentColor"/><circle cx="19" cy="12" r="1.4" fill="currentColor"/>',
  grip: '<circle cx="9" cy="6" r="1.2" fill="currentColor"/><circle cx="15" cy="6" r="1.2" fill="currentColor"/><circle cx="9" cy="12" r="1.2" fill="currentColor"/><circle cx="15" cy="12" r="1.2" fill="currentColor"/><circle cx="9" cy="18" r="1.2" fill="currentColor"/><circle cx="15" cy="18" r="1.2" fill="currentColor"/>',
  up: '<path d="m6 15 6-6 6 6"/>',
  down: '<path d="m6 9 6 6 6-6"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  music: '<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>',
  layers: '<path d="m12 3 9 5-9 5-9-5z"/><path d="m3 13 9 5 9-5"/>',
  playNext: '<path d="M4 6h10M4 12h10M4 18h6"/><path d="M16 14v6l5-3z" fill="currentColor"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/>',
  link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
  check: '<path d="m5 12 5 5 9-10"/>',
  folder: '<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2.5h8.5A1.5 1.5 0 0 1 21 9v9.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z"/>',
  mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>',
  disc: '<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="2.5"/>',
  back: '<path d="m15 18-6-6 6-6"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
  saved: '<path d="M12 3v12M7 10l5 5 5-5"/><path d="M5 21h14"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
  alert: '<path d="M12 3 2 20h20z"/><path d="M12 10v4M12 17h.01"/>',
  bolt: '<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>',
  sparkle: '<path d="M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2.5 2.5M15.5 15.5 18 18M6 18l2.5-2.5M15.5 8.5 18 6"/>',
  chart: '<path d="M4 20V10M10 20V4M16 20v-7M22 20H2"/>',
  expand: '<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
  radio: '<circle cx="12" cy="12" r="2"/><path d="M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.6 5.6a9 9 0 0 0 0 12.8M18.4 5.6a9 9 0 0 1 0 12.8"/>',
  translate: '<path d="M4 5h9M8.5 3v2M6 5c1 4 3.5 7 7 8.5M11 5c-1 4-3.5 7-7 8.5M13 21l4-9 4 9M14.5 18h5"/>',
  sliders: '<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>',
  discord: '',
};

function icon(name) {
  return `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] || ''}</svg>`;
}

function hydrateIcons(root = document) {
  for (const el of $$('i[data-icon]', root)) {
    el.outerHTML = el.className ? `<span class="${esc(el.className)}">${icon(el.dataset.icon)}</span>` : icon(el.dataset.icon);
  }
  for (const el of $$('[data-brand]', root)) {
    el.classList.add(el.dataset.brand);
    el.innerHTML = plogo(el.dataset.brand);
  }
}

function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

/** Tiny element builder: h('div', {class: 'x', onclick}, child, 'text'). */
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (k === 'html') el.innerHTML = v;
    else if (k === 'dataset') Object.assign(el.dataset, v);
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}

function fmtTime(sec) {
  if (sec == null || !isFinite(sec) || sec < 0) return '–:––';
  sec = Math.floor(sec);
  const hrs = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = String(sec % 60).padStart(2, '0');
  return hrs ? `${hrs}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

function timeAgo(ms) {
  const d = (Date.now() - ms) / 1000;
  if (d < 60) return 'just now';
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  if (d < 86400 * 7) return `${Math.floor(d / 86400)} d ago`;
  return new Date(ms).toLocaleDateString();
}

function errMsg(e) {
  if (e == null) return 'Unknown error';
  if (typeof e === 'string') return e;
  if (e.message) return e.message;
  if (e.kind) return e.kind.replace(/_/g, ' ');
  return String(e);
}

const PLATFORMS = {
  ytmusic: { label: 'YouTube Music', short: 'YT Music' },
  soundcloud: { label: 'SoundCloud', short: 'SoundCloud' },
  youtube: { label: 'YouTube', short: 'YouTube' },
  spotify: { label: 'Spotify', short: 'Spotify' },
  local: { label: 'Local files', short: 'Local' },
};

/** Inline SVG of a platform's logo. */
function plogo(platform) {
  if (platform === 'local') return icon('folder').replace('class="i"', 'class="plogo"');
  const b = window.BRANDS?.[platform];
  return b ? `<svg class="plogo" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="${b.path}"/></svg>` : icon('music');
}

/** Pill with logo + name. */
function badge(platform, { button = false, title, preview = false, short = false } = {}) {
  const p = PLATFORMS[platform] || { label: platform, short: platform };
  const el = h(button ? 'button' : 'span', {
    class: `pchip ${platform}${preview ? ' preview' : ''}`,
    title: title || (preview ? `${p.label} (30-second preview only)` : p.label),
    html: `${plogo(platform)}<span>${esc(short ? p.short : p.label)}</span>`,
  });
  return el;
}

/** Round icon-only platform button. */
function pdot(platform, { title, preview = false, onClick } = {}) {
  const el = h('button', { class: `pdot ${platform}${preview ? ' preview' : ''}`, title: title || PLATFORMS[platform]?.label, html: plogo(platform) });
  if (onClick) el.addEventListener('click', (e) => { e.stopPropagation(); onClick(e); });
  return el;
}

function miniLogo(platform, { button = false, title } = {}) {
  return h(button ? 'button' : 'span', { class: `mini-logo ${platform}`, title: title || PLATFORMS[platform]?.label, html: plogo(platform) });
}

function thumbEl(src, cls = 'thumb') {
  const t = h('div', { class: cls });
  if (src) t.append(h('img', { src, loading: 'lazy', alt: '', referrerpolicy: 'no-referrer', onerror: (e) => { e.target.remove(); t.innerHTML = icon('music'); } }));
  else t.innerHTML = icon('music');
  return t;
}

const EQ = '<span class="eq"><span></span><span></span><span></span></span>';

function isUrl(s) {
  return /^(https?:\/\/|spotify:)/i.test(s.trim());
}

function debounce(fn, ms) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}

function throttle(fn, ms) {
  let last = 0, timer = null, lastArgs;
  return (...a) => {
    lastArgs = a;
    const now = Date.now();
    if (now - last >= ms) { last = now; fn(...a); }
    else if (!timer) timer = setTimeout(() => { timer = null; last = Date.now(); fn(...lastArgs); }, ms - (now - last));
  };
}

async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    throw (typeof e === 'object' && e) ? e : { kind: 'error', message: String(e) };
  }
}

// ===========================================================================
// State
// ===========================================================================

const state = {
  settings: null,
  playback: null,
  queue: { items: [], current: null },
  playlists: [],
  favorites: [],
  favKeys: new Set(),
  history: [],
  results: [],
  view: 'search',
  source: 'all',
  selectedPlaylist: null, // id of local playlist, or 'remote'
  remote: null,           // RemotePlaylist currently shown
  themes: [],
  editingTheme: null,
  plugins: [],
  tools: null,
  downloads: [],
  downloadKeys: new Set(),
  downloading: new Map(),
  draggingProgress: false,
  loadingKey: null,
};

// ===========================================================================
// Toasts, modals, menus
// ===========================================================================

function toast(message, level = 'info', ms) {
  const ico = { info: 'info', ok: 'check', warn: 'alert', error: 'alert' }[level] || 'info';
  const el = h('div', { class: `toast ${level}` },
    h('span', { class: 't-ico', html: icon(ico) }),
    h('div', { class: 'msg' }, message),
    h('button', { title: 'Dismiss', html: icon('x'), onclick: () => close() }));
  const close = () => { el.classList.add('out'); setTimeout(() => el.remove(), 200); };
  $('#toasts').append(el);
  const toasts = $$('.toast', $('#toasts'));
  if (toasts.length > 4) toasts[0].remove();
  setTimeout(close, ms ?? (level === 'error' ? 8000 : level === 'warn' ? 6000 : 3000));
}

let modalStack = [];
function openModal({ title, subtitle, body, footer, wide = false, onClose }) {
  const backdrop = h('div', { class: 'modal-backdrop' });
  const modal = h('div', { class: 'modal', role: 'dialog', 'aria-modal': 'true', style: wide ? 'width:min(720px,100%)' : null },
    h('header', {}, h('h2', {}, title), subtitle ? h('p', { class: 'muted' }, subtitle) : null),
    h('div', { class: 'body' }, body),
    footer ? h('footer', {}, footer) : null);
  backdrop.append(modal);
  const close = () => {
    backdrop.remove();
    modalStack = modalStack.filter((m) => m.close !== close);
    onClose?.();
  };
  backdrop.addEventListener('mousedown', (e) => { if (e.target === backdrop) close(); });
  $('#modal-root').append(backdrop);
  modalStack.push({ close, el: backdrop });
  setTimeout(() => (modal.querySelector('[autofocus]') || modal.querySelector('input, button.primary'))?.focus(), 30);
  return { close, modal };
}

function promptModal({ title, label, value = '', okLabel = 'OK', placeholder = '' }) {
  return new Promise((resolve) => {
    let done = false;
    const input = h('input', { class: 'input', value, placeholder, autofocus: true });
    const ok = () => { done = true; m.close(); resolve(input.value.trim() || null); };
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') ok(); });
    const m = openModal({
      title,
      body: h('div', {}, label ? h('p', { class: 'muted' }, label) : null, input),
      footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), h('button', { class: 'btn primary', onclick: ok }, okLabel)],
      onClose: () => { if (!done) resolve(null); },
    });
    setTimeout(() => input.select(), 40);
  });
}

function confirmModal({ title, message, okLabel = 'OK', danger = false }) {
  return new Promise((resolve) => {
    let done = false;
    const m = openModal({
      title,
      body: h('p', { class: 'muted' }, message),
      footer: [
        h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'),
        h('button', { class: `btn ${danger ? 'danger' : 'primary'}`, onclick: () => { done = true; m.close(); resolve(true); } }, okLabel),
      ],
      onClose: () => { if (!done) resolve(false); },
    });
  });
}

let openMenuEl = null;
function closeMenu() {
  if (openMenuEl) {
    openMenuEl.onClose?.();
    openMenuEl.remove();
    openMenuEl = null;
  }
}

/** items: [{label, icon, onClick} | {label: 'x', heading: true} | 'sep'] */
function openMenu(anchor, items, { onClose } = {}) {
  closeMenu();
  const menu = h('div', { class: 'menu', role: 'menu' });
  for (const it of items) {
    if (it === 'sep') { menu.append(h('hr')); continue; }
    if (it.heading) { menu.append(h('div', { class: 'menu-label' }, it.label)); continue; }
    menu.append(h('button', {
      role: 'menuitem',
      html: `${it.icon ? icon(it.icon) : ''}<span>${esc(it.label)}</span>`,
      onclick: (e) => { e.stopPropagation(); closeMenu(); it.onClick?.(); },
    }));
  }
  document.body.append(menu);
  const r = anchor.getBoundingClientRect();
  const mw = menu.offsetWidth, mh = menu.offsetHeight;
  let left = Math.min(r.right - mw, window.innerWidth - mw - 8);
  left = Math.max(8, left);
  let top = r.bottom + 4;
  if (top + mh > window.innerHeight - 8) top = Math.max(8, r.top - mh - 4);
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;
  menu.onClose = onClose;
  openMenuEl = menu;
}
document.addEventListener('mousedown', (e) => { if (openMenuEl && !openMenuEl.contains(e.target)) closeMenu(); });
window.addEventListener('blur', closeMenu);
window.addEventListener('resize', closeMenu);

// ===========================================================================
// Navigation
// ===========================================================================

const navStack = [];
function showView(name, { push = true } = {}) {
  if (push && state.view && state.view !== name) navStack.push(state.view);
  if (navStack.length > 30) navStack.shift();
  state.view = name;
  for (const v of $$('.view')) v.classList.toggle('active', v.id === `view-${name}`);
  for (const n of $$('.nav-item[data-view]')) n.classList.toggle('active', n.dataset.view === name);
  if (name !== 'plugin-panel') for (const n of $$('#plugin-nav .nav-item')) n.classList.remove('active');
  renderPlaylistList();
  if (name === 'history') refreshHistory();
  if (name === 'themes') renderThemes();
  if (name === 'plugins') renderPlugins();
  if (name === 'settings') renderSettings();
  if (name === 'downloads') renderDownloads();
  if (name === 'local') renderLocal();
  if (name === 'charts') renderCharts();
  if (name === 'releases') renderReleases();
}

function goBack() {
  const prev = navStack.pop();
  if (prev) showView(prev, { push: false });
}

/** "← Back" button for drill-down pages (artist, album). */
function backButton() {
  return h('button', { class: 'btn small back-btn', html: `${icon('back')}Back`, onclick: goBack });
}

$('.sidebar').addEventListener('click', (e) => {
  const b = e.target.closest('.nav-item[data-view]');
  if (b) showView(b.dataset.view);
});

// Surface unexpected frontend errors instead of failing silently.
window.addEventListener('error', (e) => toast(`Something went wrong: ${e.message}`, 'error'));
window.addEventListener('unhandledrejection', (e) => toast(`Something went wrong: ${errMsg(e.reason)}`, 'error'));

function toggleQueue() {
  const app = $('#app');
  if (window.innerWidth <= 1180) app.classList.toggle('queue-open');
  else app.classList.toggle('queue-hidden');
}
$('#toggle-queue').addEventListener('click', toggleQueue);
