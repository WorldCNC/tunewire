'use strict';

// ===========================================================================
// Self-update (tauri-plugin-updater; signed releases from GitHub)
// ===========================================================================

const updater = { checking: false, available: null, ready: null, version: null };

async function appVersion() {
  if (!updater.version) {
    try { updater.version = await window.__TAURI__.app.getVersion(); } catch { updater.version = '?'; }
  }
  return updater.version;
}

/**
 * manual=true asks right away and reports "up to date" / errors. Automatic
 * checks download the update quietly, then show a "Restart to update" pill.
 */
async function checkForUpdate(manual = false) {
  if (updater.checking) return;
  if (updater.ready) { if (manual) promptRestart(); return; }
  updater.checking = true;
  try {
    const update = await window.__TAURI__.updater.check();
    updater.available = update || null;
    if (!update) {
      if (manual) toast(`Tunewire ${await appVersion()} is up to date`, 'ok', 2500);
    } else if (manual) {
      promptUpdate(update);
    } else {
      await update.download();
      markReady(update);
    }
  } catch (e) {
    if (manual) toast(`Couldn’t check for updates: ${errMsg(e)}`, 'error');
  } finally {
    updater.checking = false;
  }
}

/** Remember the notes so the new version can show "What's new". */
function rememberNotes(update) {
  try { localStorage.setItem('tw-update-notes', JSON.stringify({ version: update.version, notes: update.body || '' })); } catch { /* optional */ }
}

function markReady(update) {
  updater.ready = update;
  rememberNotes(update);
  const pill = $('#update-pill');
  pill.hidden = false;
  pill.title = `Tunewire ${update.version} is downloaded. Restart to install it.`;
  pill.onclick = promptRestart;
  toast(`Tunewire ${update.version} is ready. Restart when it suits you.`, 'ok', 6000);
}

function promptRestart() {
  const update = updater.ready;
  if (!update) return;
  const notes = (update.body || '').trim();
  const status = h('div', { class: 'meta', style: 'min-height:16px' });
  const restart = h('button', { class: 'btn primary' }, 'Restart now');
  const m = openModal({
    title: `Tunewire ${update.version} is ready`,
    subtitle: 'It’s already downloaded. Restarting takes a few seconds, and your queue picks up where it was.',
    body: h('div', {}, notes ? h('pre', { class: 'release-notes' }, notes) : null, status),
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Later'), restart],
  });
  restart.addEventListener('click', async () => {
    restart.disabled = true;
    status.textContent = 'Installing…';
    try {
      await update.install();
      status.textContent = 'Restarting…';
      await window.__TAURI__.process.relaunch();
    } catch (e) {
      status.textContent = '';
      restart.disabled = false;
      toast(`Update failed: ${errMsg(e)}`, 'error');
    }
  });
}

function promptUpdate(update) {
  const notes = (update.body || '').trim();
  const progress = h('div', { class: 'meta', style: 'min-height:16px' });
  const install = h('button', { class: 'btn primary' }, 'Install and restart');
  const m = openModal({
    title: `Tunewire ${update.version} is available`,
    subtitle: `You have ${updater.version || 'an older version'}.`,
    body: h('div', {}, notes ? h('pre', { class: 'release-notes' }, notes) : null, progress),
    footer: [h('button', { class: 'btn', onclick: () => m.close() }, 'Later'), install],
  });
  install.addEventListener('click', async () => {
    install.disabled = true;
    rememberNotes(update);
    let total = 0, got = 0;
    try {
      await update.downloadAndInstall((ev) => {
        if (ev.event === 'Started') total = ev.data.contentLength || 0;
        if (ev.event === 'Progress') {
          got += ev.data.chunkLength;
          progress.textContent = total ? `Downloading… ${Math.round((got / total) * 100)}%` : `Downloading… ${(got / 1e6).toFixed(1)} MB`;
        }
        if (ev.event === 'Finished') progress.textContent = 'Installing…';
      });
      progress.textContent = 'Restarting…';
      await window.__TAURI__.process.relaunch();
    } catch (e) {
      progress.textContent = '';
      install.disabled = false;
      toast(`Update failed: ${errMsg(e)}`, 'error');
    }
  });
}

/** After an update: show what changed, once. */
async function showWhatsNew() {
  let saved;
  try { saved = JSON.parse(localStorage.getItem('tw-update-notes') || 'null'); } catch { return; }
  if (!saved) return;
  const v = await appVersion();
  if (saved.version !== v) return;
  try { localStorage.removeItem('tw-update-notes'); } catch { /* optional */ }
  const m = openModal({
    title: `What’s new in Tunewire ${v}`,
    body: saved.notes.trim() ? h('pre', { class: 'release-notes' }, saved.notes.trim()) : h('p', {}, 'Tunewire was updated.'),
    footer: [h('button', { class: 'btn primary', onclick: () => m.close() }, 'Nice')],
  });
}

/** Startup check, then hourly, and when you come back to the window after a while. */
let lastUpdateCheck = 0;
function scheduleUpdateChecks() {
  const maybe = (minGap) => {
    if (state.settings?.auto_update === false || Date.now() - lastUpdateCheck < minGap) return;
    lastUpdateCheck = Date.now();
    checkForUpdate(false);
  };
  setTimeout(() => maybe(0), 8000);
  setInterval(() => maybe(55 * 60 * 1000), 60 * 60 * 1000);
  window.addEventListener('focus', () => maybe(60 * 60 * 1000));
}

function updateSettingsRow() {
  const label = h('span', {}, 'Updates');
  appVersion().then((v) => { label.textContent = `Updates · version ${v}`; });
  return h('div', { class: 'setting' },
    h('div', { class: 'label' }, label, h('small', {}, 'With Auto on, new versions download in the background (verified with a signature) and install when you restart.')),
    h('div', { class: 'actions' },
      h('label', { class: 'actions', title: 'Check on startup' }, h('span', { class: 'switch' }, (() => {
        const i = h('input', { type: 'checkbox', checked: state.settings.auto_update !== false });
        i.addEventListener('change', () => updateSettings({ auto_update: i.checked }));
        return i;
      })(), h('span')), h('span', { class: 'meta' }, 'Auto')),
      h('button', { class: 'btn small', onclick: () => checkForUpdate(true) }, updater.ready ? 'Restart to update' : 'Check now')));
}
