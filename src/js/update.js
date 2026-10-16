'use strict';

// ===========================================================================
// Self-update (tauri-plugin-updater; signed releases from GitHub)
// ===========================================================================

const updater = { checking: false, available: null, version: null };

async function appVersion() {
  if (!updater.version) {
    try { updater.version = await window.__TAURI__.app.getVersion(); } catch { updater.version = '?'; }
  }
  return updater.version;
}

/** manual=true shows "up to date" / errors; automatic checks stay quiet. */
async function checkForUpdate(manual = false) {
  if (updater.checking) return;
  updater.checking = true;
  try {
    const update = await window.__TAURI__.updater.check();
    updater.available = update || null;
    if (update) promptUpdate(update);
    else if (manual) toast(`Tunewire ${await appVersion()} is up to date`, 'ok', 2500);
  } catch (e) {
    if (manual) toast(`Couldn’t check for updates: ${errMsg(e)}`, 'error');
  } finally {
    updater.checking = false;
  }
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

function updateSettingsRow() {
  const label = h('span', {}, 'Updates');
  appVersion().then((v) => { label.textContent = `Updates · version ${v}`; });
  return h('div', { class: 'setting' },
    h('div', { class: 'label' }, label, h('small', {}, 'New versions are downloaded from the project’s GitHub releases and verified with a signature before installing.')),
    h('div', { class: 'actions' },
      h('label', { class: 'actions', title: 'Check on startup' }, h('span', { class: 'switch' }, (() => {
        const i = h('input', { type: 'checkbox', checked: state.settings.auto_update !== false });
        i.addEventListener('change', () => updateSettings({ auto_update: i.checked }));
        return i;
      })(), h('span')), h('span', { class: 'meta' }, 'Auto')),
      h('button', { class: 'btn small', onclick: () => checkForUpdate(true) }, 'Check now')));
}
