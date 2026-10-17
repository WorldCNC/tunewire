'use strict';

// ===========================================================================
// Themes
// ===========================================================================

let appliedVars = [];
function applyTheme(t) {
  const root = document.documentElement;
  for (const k of appliedVars) root.style.removeProperty(k);
  for (const [k, v] of Object.entries(t.vars || {})) root.style.setProperty(k, v);
  appliedVars = Object.keys(t.vars || {});
  root.dataset.mode = t.dark === false ? 'light' : 'dark';
  $('#theme-css').textContent = t.css || '';
  // Same picture as before: nothing to fade (re-fading it made the background blink).
  if (typeof updateAmbient === 'function' && state.settings) updateAmbient();
}

async function activateTheme(id) {
  try {
    const t = await call('load_theme', { id });
    applyTheme(t);
    state.settings.theme = t.id;
    state.editingTheme = structuredClone(t);
    renderThemes();
    plugins.emit('theme-changed', { id: t.id, name: t.name });
  } catch (e) { toast(errMsg(e), 'error'); }
}

async function renderThemes() {
  try { state.themes = await call('list_themes'); } catch (e) { toast(errMsg(e), 'error'); return; }
  const grid = $('#theme-grid');
  grid.replaceChildren(...state.themes.map((t) => {
    const card = h('button', { class: `theme-card${state.settings?.theme === t.id ? ' active' : ''}`, onclick: () => activateTheme(t.id) },
      h('div', { class: 'theme-swatch', style: `background:${t.bg || '#111'}` },
        h('div', { class: 'mock' }, h('i', { style: `background:${t.accent};width:40%` }), h('i', { style: `background:${t.dark ? '#fff' : '#000'};width:70%` }), h('i', { style: `background:${t.dark ? '#fff' : '#000'};width:55%` })),
        h('span', { style: `background:${t.accent || '#888'}` })),
      h('div', { class: 'theme-info' },
        h('div', {}, h('div', { class: 'name' }, t.name), h('small', {}, `${t.builtin ? 'Built-in' : t.author || 'Custom'} · ${t.dark ? 'dark' : 'light'}`)),
        state.settings?.theme === t.id ? h('span', { html: icon('check'), class: 'accent' }) : null));
    return card;
  }));
  if (!state.editingTheme || state.editingTheme.id !== state.settings.theme) {
    try { state.editingTheme = await call('get_theme', { id: state.settings.theme }); } catch { /* ignore */ }
  }
  renderThemeEditor();
}

const isColor = (v) => /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(v.trim());
const expandHex = (v) => (v.length === 4 ? '#' + [...v.slice(1)].map((c) => c + c).join('') : v);

function renderThemeEditor() {
  const t = state.editingTheme;
  const box = $('#theme-editor');
  if (!t) { box.replaceChildren(); return; }
  const preview = () => applyTheme(t);
  const nameIn = h('input', { class: 'input', value: t.builtin ? `${t.name} (custom)` : t.name, placeholder: 'Theme name' });
  const authorIn = h('input', { class: 'input', value: t.author || '', placeholder: 'Author' });
  const darkIn = h('input', { type: 'checkbox', checked: t.dark !== false });
  darkIn.addEventListener('change', () => { t.dark = darkIn.checked; preview(); });

  const grid = h('div', { class: 'editor-grid' });
  for (const [k, v] of Object.entries(t.vars)) {
    const text = h('input', { type: 'text', class: 'input', value: v, spellcheck: 'false' });
    const row = h('div', { class: 'var-row' }, h('label', { title: k }, k));
    if (isColor(v)) {
      const color = h('input', { type: 'color', value: expandHex(v) });
      color.addEventListener('input', () => { text.value = color.value; t.vars[k] = color.value; preview(); });
      text.addEventListener('input', () => { if (isColor(text.value)) color.value = expandHex(text.value); });
      row.append(color);
    }
    text.addEventListener('input', debounce(() => { t.vars[k] = text.value; preview(); }, 150));
    row.append(text);
    grid.append(row);
  }
  const css = h('textarea', { class: 'input', rows: 5, spellcheck: 'false', placeholder: '/* Optional extra CSS, e.g. */\n.player-bar { backdrop-filter: blur(8px); }' }, t.css || '');
  css.addEventListener('input', debounce(() => { t.css = css.value; preview(); }, 250));

  const save = async () => {
    const theme = { ...t, name: nameIn.value.trim() || 'Custom theme', author: authorIn.value.trim(), id: t.builtin ? '' : t.id, css: css.value };
    try {
      const meta = await call('save_theme', { theme });
      toast(`Saved theme “${meta.name}”`, 'ok');
      await activateTheme(meta.id);
    } catch (e) { toast(errMsg(e), 'error'); }
  };
  const exportTheme = async () => {
    try {
      const res = await call('export_theme', { id: t.id });
      try { await navigator.clipboard.writeText(res.json); } catch { /* clipboard may be unavailable */ }
      toast(res.path ? `Exported to ${res.path} (also copied to clipboard)` : 'Theme JSON copied to clipboard', 'ok', 6000);
    } catch (e) { toast(errMsg(e), 'error'); }
  };
  const del = async () => {
    if (!(await confirmModal({ title: 'Delete theme?', message: `“${t.name}” will be deleted.`, okLabel: 'Delete', danger: true }))) return;
    try {
      await call('delete_theme', { id: t.id });
      await activateTheme('midnight');
    } catch (e) { toast(errMsg(e), 'error'); }
  };

  box.replaceChildren(
    h('div', { class: 'view-head small' }, h('h2', {}, `Edit ${t.name}`),
      h('div', { class: 'actions' },
        h('button', { class: 'btn', html: `${icon('refresh')}Reset`, onclick: async () => { state.editingTheme = await call('get_theme', { id: t.id }); applyTheme(state.editingTheme); renderThemeEditor(); } }),
        h('button', { class: 'btn', html: `${icon('download')}Export`, onclick: exportTheme }),
        !t.builtin ? h('button', { class: 'btn danger-ghost', html: `${icon('trash')}Delete`, onclick: del }) : null,
        h('button', { class: 'btn primary', html: `${icon('save')}${t.builtin ? 'Save as new theme' : 'Save'}`, onclick: save }))),
    h('p', { class: 'muted mb-10 fs-13'}, 'Changes preview immediately. Saving a built-in theme creates a copy.'),
    h('div', { class: 'editor-head' }, nameIn, authorIn, h('label', { class: 'actions' }, h('span', { class: 'switch' }, darkIn, h('span')), 'Dark')),
    grid,
    h('h3', { class: 'fs-13 my-6' }, 'Custom CSS'),
    css);
}

async function importThemeJson(json) {
  try {
    const meta = await call('import_theme', { json });
    toast(`Imported “${meta.name}”`, 'ok');
    await activateTheme(meta.id);
  } catch (e) { toast(errMsg(e), 'error'); }
}
$('#theme-import-file').addEventListener('change', async (e) => {
  const f = e.target.files[0];
  e.target.value = '';
  if (f) importThemeJson(await f.text());
});
$('#theme-import-paste').addEventListener('click', () => {
  const ta = h('textarea', { class: 'input', rows: 12, placeholder: '{ "name": "…", "vars": { "--bg": "#000" } }', autofocus: true });
  const m = openModal({
    title: 'Import theme', subtitle: 'Paste a theme JSON shared by someone else.', body: ta, wide: true,
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Cancel'), h('button', { class: 'btn primary', onclick: () => { m.close(); importThemeJson(ta.value); } }, 'Import')],
  });
});
