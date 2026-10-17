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
  home: '<path d="M3 11 12 4l9 7"/><path d="M5 10v10h5v-6h4v6h5V10"/>',
  pulse: '<path d="M3 12h4l3-8 4 16 3-8h4"/>',
  gift: '<rect x="3" y="8" width="18" height="5" rx="1"/><path d="M5 13v8h14v-8M12 8v13M12 8S10 3 7.5 4 9 8 12 8zm0 0s2-5 4.5-4S15 8 12 8z"/>',
  chart: '<path d="M4 20V10M10 20V4M16 20v-7M22 20H2"/>',
  expand: '<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
  radio: '<circle cx="12" cy="12" r="2"/><path d="M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.6 5.6a9 9 0 0 0 0 12.8M18.4 5.6a9 9 0 0 1 0 12.8"/>',
  translate: '<path d="M4 5h9M8.5 3v2M6 5c1 4 3.5 7 7 8.5M11 5c-1 4-3.5 7-7 8.5M13 21l4-9 4 9M14.5 18h5"/>',
  sliders: '<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>',
  discord: '',
  chevLeft: '<path d="m14.5 6-6 6 6 6"/>',
  chevRight: '<path d="m9.5 6 6 6-6 6"/>',
  browse: '<rect x="3.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.5"/><path d="M17 13.5v7M13.5 17h7"/>',
  bell: '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>',
  winMin: '<path d="M5 12h14"/>',
  winMax: '<rect x="5" y="5" width="14" height="14" rx="1"/>',
  winRestore: '<rect x="5" y="8" width="11" height="11" rx="1"/><path d="M8 8V5h11v11h-3"/>',
  library: '<path d="M4 4v16M9 4v16"/><path d="m14 4.5 5.5 15"/>',
  npView: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M14 4v16"/><circle cx="8.5" cy="12" r="2.2"/>',
  speaker: '<rect x="6" y="2.5" width="12" height="19" rx="2"/><circle cx="12" cy="15" r="3.2"/><path d="M12 7h.01"/>',
  moon: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
  pin: '<path d="M9 3h6l-1 6 4 4v2H6v-2l4-4z"/><path d="M12 15v6"/>',
  smartShuffle: '<path d="M16 3h5v5M4 20 21 3M21 16v5h-5M15 15l6 6"/><path d="M6 2.5l1 2.5 2.5 1L7 7 6 9.5 5 7 2.5 6 5 5z" fill="currentColor" stroke="none"/>',
  sort: '<path d="M4 7h16M7 12h10M10 17h4"/>',
  folderPlus: '<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2.5h8.5A1.5 1.5 0 0 1 21 9v9.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z"/><path d="M12 11v6M9 14h6"/>',
  importIcon: '<path d="M12 3v12M7 10l5 5 5-5"/><path d="M4 15v4a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4"/>',
  friends: '<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><circle cx="17" cy="9" r="2.8"/><path d="M15.5 14.2A5 5 0 0 1 21.5 19"/>',
  share: '<path d="M12 3v13M7 8l5-5 5 5"/><path d="M5 13v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6"/>',
  verified: '<path d="M12 2.5 14.6 5l3.5-.3.8 3.4 3 1.9-1.4 3.2 1.4 3.2-3 1.9-.8 3.4-3.5-.3L12 21.5 9.4 19l-3.5.3-.8-3.4-3-1.9L3.5 12 2.1 8.8l3-1.9.8-3.4 3.5.3z" fill="currentColor" stroke="none"/><path d="m8.5 12 2.5 2.5 4.5-5" stroke="#fff"/>',
};

function icon(name) {
  return `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] || ''}</svg>`;
}

function hydrateIcons(root = document) {
  for (const b of $$('button[title]:not([aria-label])', root)) if (!b.textContent.trim()) b.setAttribute('aria-label', b.title);
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

/**
 * The same picture at about `px` pixels. Covers are shown small almost
 * everywhere; loading (and keeping decoded) 500–1200 px versions for a
 * 48 px row was most of the app's memory.
 */
function imgUrl(url, px = 96) {
  if (!url || typeof url !== 'string' || url.startsWith('data:')) return url;
  if (/googleusercontent\.com|ggpht\.com/.test(url)) {
    if (/=w\d+-h\d+/.test(url)) return url.replace(/=w\d+-h\d+[^/?#]*/, `=w${px}-h${px}-l90-rj`);
    if (/=s\d+/.test(url)) return url.replace(/=s\d+[^/?#]*/, `=s${px}`);
    return url;
  }
  if (/i\.ytimg\.com\/vi\//.test(url)) return url.replace(/\/(maxresdefault|sddefault|hqdefault|mqdefault|default)\.jpg/, px <= 320 ? '/mqdefault.jpg' : '/hqdefault.jpg');
  if (/sndcdn\.com/.test(url)) return url.replace(/-(t\d+x\d+|large|original|crop|t500x500)\./, px <= 100 ? '-large.' : px <= 300 ? '-t300x300.' : '-t500x500.');
  if (/i\.scdn\.co\/image\/ab67616d0000(b273|1e02|4851)/.test(url)) return url.replace(/ab67616d0000(b273|1e02|4851)/, px <= 80 ? 'ab67616d00004851' : px <= 300 ? 'ab67616d00001e02' : 'ab67616d0000b273');
  return url;
}

/** A wide banner at about `w` pixels wide (keeps its shape). */
function bannerUrl(url, w) {
  const m = url && url.match(/=w(\d+)-h(\d+)/);
  if (!m) return url;
  const h2 = Math.round((Number(m[2]) / Number(m[1])) * w);
  return url.replace(/=w\d+-h\d+/, `=w${w}-h${h2}`);
}

/** Tiny element builder: h('div', {class: 'x', onclick}, child, 'text'). */
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  if (tag === 'img' && attrs?.src) {
    // Right-size covers; 'data-px' asks for a bigger one (headers, tiles).
    // Default decoding: an image the browser already has shows in the same
    // frame (async decoding left rebuilt lists blank for a frame: a flicker).
    attrs = { ...attrs, src: imgUrl(attrs.src, Number(attrs['data-px']) || 96) };
  }
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
  // Icon-only buttons: screen readers read the tooltip as the name.
  if (tag === 'button' && el.title && !el.hasAttribute('aria-label') && !el.textContent.trim()) el.setAttribute('aria-label', el.title);
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

/**
 * Cover art that survives re-renders: lists that are rebuilt (Your Library,
 * the queue) get the same element back, already decoded, so covers don't
 * blink each time. Keyed by what the art shows; the oldest are let go.
 */
const keptArt = new Map();
function keepArt(key, make) {
  let el = keptArt.get(key);
  if (el) keptArt.delete(key);
  else el = make();
  keptArt.set(key, el);
  if (keptArt.size > 800) keptArt.delete(keptArt.keys().next().value);
  return el;
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

/**
 * Load a rarely used part of the app (js/lazy/<name>.js) on first use.
 * Its functions replace the small stubs that called this.
 */
const lazyLoads = new Map();
function lazyModule(name) {
  if (!lazyLoads.has(name)) {
    lazyLoads.set(name, new Promise((resolve, reject) => {
      const s = document.createElement('script');
      s.src = `js/lazy/${name}.js`;
      s.onload = () => resolve();
      s.onerror = () => { lazyLoads.delete(name); reject({ kind: 'error', message: `Couldn’t load ${name}` }); };
      document.body.append(s);
    }));
  }
  return lazyLoads.get(name);
}
// Stubs: the real functions arrive with their module.
function openImport() { return lazyModule('imports').then(() => openImport()); }
function openSpotifyImport() { return lazyModule('imports').then(() => openSpotifyImport()); }
function pickSpotify(...a) { return lazyModule('imports').then(() => pickSpotify(...a)); }
function openGallery(kind) { return lazyModule('gallery').then(() => openGallery(kind)); }
function backupSettingsRows() { const box = h('div', { class: 'lazy-rows' }); lazyModule('backup').then(() => box.replaceWith(...backupSettingsRows())).catch(() => {}); return [box]; }
function scrobbleSettingsRows() { const box = h('div', { class: 'lazy-rows' }); lazyModule('backup').then(() => box.replaceWith(...scrobbleSettingsRows())).catch(() => {}); return [box]; }

/** Hooks run after a command succeeds (online sync of playlist edits). */
const commandHooks = [];
async function call(cmd, args) {
  try {
    // Playlist edits: remember the songs before (removals are by index).
    const before = /_playlist$|_in_playlist$|_from_playlist$/.test(cmd) && args?.id ? state.playlists.find((p) => p.id === args.id)?.tracks.slice() : null;
    const result = await invoke(cmd, args);
    for (const f of commandHooks) { try { f(cmd, args, result, before); } catch (e) { console.error(e); } }
    return result;
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
  view: 'home',
  panel: 'queue',
  libFilter: null,
  libSort: 'recents',
  libQuery: '',
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

/**
 * A short message. `action: {label, onClick}` adds a button (Undo, Retry…);
 * returns { close } so callers can dismiss it early.
 */
function toast(message, level = 'info', ms, { action } = {}) {
  const ico = { info: 'info', ok: 'check', warn: 'alert', error: 'alert' }[level] || 'info';
  const el = h('div', { class: `toast ${level}`, role: level === 'error' ? 'alert' : 'status' },
    h('span', { class: 't-ico', html: icon(ico) }),
    h('div', { class: 'msg' }, message),
    action ? h('button', { class: 'toast-action', onclick: () => { close(); action.onClick(); } }, action.label) : null,
    h('button', { title: 'Dismiss', 'aria-label': 'Dismiss', html: icon('x'), onclick: () => close() }));
  let closed = false;
  const close = () => { if (closed) return; closed = true; el.classList.add('out'); setTimeout(() => el.remove(), 200); };
  $('#toasts').append(el);
  const toasts = $$('.toast', $('#toasts'));
  if (toasts.length > 4) toasts[0].remove();
  // Toasts with a button stay a little longer (time to reach it).
  setTimeout(close, ms ?? (action ? 7000 : level === 'error' ? 8000 : level === 'warn' ? 6000 : 3000));
  return { close };
}

/**
 * Undo for a destructive action: the change is shown at once; `undo` runs
 * if the button is pressed. Returns the toast.
 */
function undoToast(message, undo) {
  return toast(message, 'ok', 7000, { action: { label: 'Undo', onClick: async () => {
    try { await undo(); toast('Undone', 'info', 1500); } catch (e) { toast(errMsg(e), 'error'); }
  } } });
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
let lastMenuAnchor = null;
function openMenu(anchor, items, { onClose, at } = {}) {
  closeMenu();
  // A hidden anchor (a control moved into the "more" menu): open where the
  // previous menu was instead of the window's corner.
  if (!at && anchor && !anchor.getClientRects().length && lastMenuAnchor?.getClientRects().length) anchor = lastMenuAnchor;
  if (anchor?.getClientRects().length) lastMenuAnchor = anchor;
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
  // Context menus open at the pointer; button menus under the button.
  const r = at ? { left: at.clientX, right: at.clientX + menu.offsetWidth, top: at.clientY, bottom: at.clientY } : anchor.getBoundingClientRect();
  const mw = menu.offsetWidth, mh = menu.offsetHeight;
  let left = Math.min(at ? r.left : r.right - mw, window.innerWidth - mw - 8);
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

// Pages are routes: drill-down pages (artist, album, playlist…) pass a
// `reopen` function so Back / Forward can show them again.
const navStack = [];
const fwdStack = [];
let curRoute = { view: 'home', reopen: null };

function showView(name, { push = true, reopen = null } = {}) {
  if (push && (curRoute.view !== name || reopen)) {
    navStack.push(curRoute);
    fwdStack.length = 0;
    if (navStack.length > 50) navStack.shift();
  }
  curRoute = { view: name, reopen };
  state.view = name;
  // A page opened again from navigation animates its rows again.
  const viewEl = $(`#view-${name}`);
  viewEl?.querySelectorAll(':scope > div').forEach((d) => { d._shownId = null; });
  for (const v of $$('.view')) v.classList.toggle('active', v.id === `view-${name}`);
  for (const n of $$('.nav-item[data-view]')) n.classList.toggle('active', n.dataset.view === name);
  if (name !== 'plugin-panel') for (const n of $$('#plugin-nav .nav-item')) n.classList.remove('active');
  $('#nav-home')?.classList.toggle('active', name === 'home');
  clearSelection();
  renderLibrary();
  updateNavButtons();
  if (name === 'history') refreshHistory();
  if (name === 'themes') renderThemes();
  if (name === 'plugins') renderPlugins();
  if (name === 'settings') renderSettings();
  if (name === 'downloads') renderDownloads();
  if (name === 'local') renderLocal();
  if (name === 'charts') renderCharts();
  if (name === 'releases') renderReleases();
  if (name === 'home') renderHome();
  if (name === 'stats') renderStats();
  if (name === 'favorites') renderFavorites();
  if (name === 'search' && !$('#search-input').value.trim()) { showSearchBody(false); renderHero(); }
}

function restoreRoute(r) {
  if (r.reopen) r.reopen();
  else showView(r.view, { push: false });
  curRoute = r;
  updateNavButtons();
}
function goBack() {
  const prev = navStack.pop();
  if (!prev) return;
  fwdStack.push(curRoute);
  restoreRoute(prev);
}
function goForward() {
  const next = fwdStack.pop();
  if (!next) return;
  navStack.push(curRoute);
  restoreRoute(next);
}
function updateNavButtons() {
  const b = $('#nav-back'), f = $('#nav-fwd');
  if (b) b.disabled = !navStack.length;
  if (f) f.disabled = !fwdStack.length;
}

/** Kept for pages that still show an inline Back button. */
function backButton() {
  return h('button', { class: 'btn small back-btn', html: `${icon('back')}Back`, onclick: goBack });
}

$('.sidebar').addEventListener('click', (e) => {
  const b = e.target.closest('.nav-item[data-view]');
  if (b) showView(b.dataset.view);
});
document.addEventListener('mouseup', (e) => {
  // Mouse back/forward buttons.
  if (e.button === 3) { e.preventDefault(); goBack(); }
  if (e.button === 4) { e.preventDefault(); goForward(); }
});

// Surface unexpected frontend errors instead of failing silently.
// …and report them (Settings → Privacy can turn this off; the backend scrubs and rate-limits).
const QUIET_KINDS = new Set(['network', 'timeout', 'cancelled', 'unavailable', 'region_locked', 'drm', 'empty_results', 'not_found', 'invalid', 'yt_dlp_missing']);
function reportError(kind, message, details = '') {
  try { invoke('report_error', { kind, message: String(message).slice(0, 2000), details: `${details}\nview: ${state.view}`.slice(0, 3000) }).catch(() => {}); } catch { /* never throw from here */ }
}
window.addEventListener('error', (e) => {
  toast(`Something went wrong: ${e.message}`, 'error');
  reportError('frontend', e.message, `${e.filename?.split('/').pop() || ''}:${e.lineno}:${e.colno}\n${e.error?.stack || ''}`);
});
window.addEventListener('unhandledrejection', (e) => {
  toast(`Something went wrong: ${errMsg(e.reason)}`, 'error');
  if (!QUIET_KINDS.has(e.reason?.kind)) reportError('frontend', errMsg(e.reason), e.reason?.stack || JSON.stringify(e.reason || '').slice(0, 1500));
});

function panelOpen() {
  const app = $('#app');
  return window.innerWidth <= 1200 ? app.classList.contains('queue-open') : !app.classList.contains('queue-hidden');
}
function setPanelOpen(open) {
  const app = $('#app');
  if (window.innerWidth <= 1200) app.classList.toggle('queue-open', open);
  else app.classList.toggle('queue-hidden', !open);
  try { localStorage.setItem('tw-panel-open', open ? '1' : '0'); } catch { /* optional */ }
  syncPanelButtons();
}
/** Toggle the right panel on a tab: opens it, switches tab, or closes it. */
function togglePanel(name) {
  if (panelOpen() && state.panel === name) setPanelOpen(false);
  else { setPanelOpen(true); showPanel(name); }
}
function toggleQueue() { togglePanel('queue'); }
function syncPanelButtons() {
  const open = panelOpen();
  $('#btn-np')?.classList.toggle('on', open && state.panel === 'now');
  $('#btn-lyrics')?.classList.toggle('on', open && state.panel === 'lyrics');
  $('#toggle-queue')?.classList.toggle('on', open && state.panel === 'queue');
}
$('#toggle-queue').addEventListener('click', toggleQueue);
