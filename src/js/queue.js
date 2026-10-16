'use strict';

// ===========================================================================
// Queue panel
// ===========================================================================

function itemPlatform(item, isCurrent) {
  if (isCurrent && state.playback?.version) return state.playback.version.platform;
  if (item.platform) return item.platform;
  return sortedVersions(item.track)[0]?.platform;
}

function renderQueue() {
  const { items, current } = state.queue;
  $('#queue-count').textContent = items.length ? `· ${items.length}` : '';
  const list = $('#queue-list');
  if (!items.length) {
    list.replaceChildren(h('div', { class: 'empty', style: 'padding:50px 16px' },
      h('p', {}, 'Queue is empty'), h('p', { class: 'muted small' }, 'Add songs from search or a playlist.')));
    return;
  }
  const frag = document.createDocumentFragment();
  const row = (item, i) => {
    const t = item.track;
    const isCur = i === current;
    const plat = itemPlatform(item, isCur);
    const logo = plat ? miniLogo(plat, { button: true, title: `Playing from ${PLATFORMS[plat]?.label}. Click to change.` }) : null;
    logo?.addEventListener('click', (e) => { e.stopPropagation(); openVersions(t, { queueUid: item.uid, current: plat }); });
    const menu = (e) => {
      e.preventDefault();
      openMenu(e.currentTarget, [
        { label: 'Play', icon: 'play', onClick: () => call('play_index_cmd', { index: i }) },
        { label: 'Choose source…', icon: 'layers', onClick: () => openVersions(t, { queueUid: item.uid, current: plat }) },
        { label: 'Add to playlist…', icon: 'list', onClick: () => addToPlaylistMenu(e.target, [t]) },
        { label: state.favKeys.has(t.key) ? 'Unlike' : 'Like', icon: 'heart', onClick: () => toggleFavorite(t) },
        'sep',
        { label: 'Remove from queue', icon: 'x', onClick: () => call('remove_from_queue', { uid: item.uid }) },
      ]);
    };
    if (isCur) {
      const el = h('div', { class: 'q-now', dataset: { index: i, uid: item.uid } }, thumbEl(t.thumbnail),
        h('div', { class: 't-main' }, h('div', { class: 't-title', title: t.title }, t.title),
          h('div', { class: 't-sub' }, h('span', { html: EQ }), logo, h('span', { style: 'overflow:hidden;text-overflow:ellipsis' }, t.artist || ''))));
      el.addEventListener('contextmenu', menu);
      return el;
    }
    const el = h('div', { class: `q-item${current != null && i < current ? ' past' : ''}`, draggable: 'true', dataset: { index: i, uid: item.uid } },
      h('div', { class: 'q-handle', html: icon('grip'), title: 'Drag to reorder' }),
      thumbEl(t.thumbnail),
      h('div', { class: 't-main' },
        h('div', { class: 't-title', title: t.title }, t.title),
        h('div', { class: 't-sub' }, logo, h('span', { style: 'overflow:hidden;text-overflow:ellipsis' }, t.artist || ''))),
      h('div', { class: 'q-actions' },
        h('button', { class: 'icon-btn', title: 'Move up', html: icon('up'), disabled: i === 0, onclick: (e) => { e.stopPropagation(); moveQueue(i, i - 1); } }),
        h('button', { class: 'icon-btn', title: 'Move down', html: icon('down'), disabled: i === items.length - 1, onclick: (e) => { e.stopPropagation(); moveQueue(i, i + 1); } }),
        h('button', { class: 'icon-btn', title: 'Remove', html: icon('x'), onclick: (e) => { e.stopPropagation(); call('remove_from_queue', { uid: item.uid }).catch((er) => toast(errMsg(er), 'error')); } })));
    el.addEventListener('click', () => call('play_index_cmd', { index: i }).catch((e) => toast(errMsg(e), 'error')));
    el.addEventListener('contextmenu', menu);
    return el;
  };
  if (current != null && items[current]) {
    frag.append(h('div', { class: 'q-section' }, 'Now playing'), row(items[current], current));
  }
  const upcoming = items.map((it, i) => [it, i]).filter(([, i]) => current == null || i > current);
  if (upcoming.length) frag.append(h('div', { class: 'q-section' }, 'Next up'), ...upcoming.map(([it, i]) => row(it, i)));
  const past = current != null ? items.slice(0, current).map((it, i) => [it, i]) : [];
  if (past.length) frag.append(h('div', { class: 'q-section' }, 'Played'), ...past.map(([it, i]) => row(it, i)));
  list.replaceChildren(frag);
  updateQueueIndicators();
}

function updateQueueIndicators() {
  const eq = $('#queue-list .eq');
  if (eq) eq.classList.toggle('paused', state.playback?.status !== 'playing');
}

async function moveQueue(from, to) {
  if (to < 0 || to >= state.queue.items.length || from === to) return;
  try { await call('reorder_queue', { from, to }); } catch (e) { toast(errMsg(e), 'error'); }
}

// Drag & drop reordering
let queueDrag = null;
const queueList = $('#queue-list');
queueList.addEventListener('dragstart', (e) => {
  const el = e.target.closest('.q-item');
  if (!el) return;
  queueDrag = Number(el.dataset.index);
  el.classList.add('dragging');
  e.dataTransfer.effectAllowed = 'move';
  e.dataTransfer.setData('text/plain', String(queueDrag));
});
queueList.addEventListener('dragover', (e) => {
  if (queueDrag == null) return;
  e.preventDefault();
  const el = e.target.closest('.q-item');
  $$('.q-item', queueList).forEach((x) => x.classList.remove('drag-over-top', 'drag-over-bottom'));
  if (!el) return;
  const r = el.getBoundingClientRect();
  el.classList.add(e.clientY < r.top + r.height / 2 ? 'drag-over-top' : 'drag-over-bottom');
});
queueList.addEventListener('drop', (e) => {
  e.preventDefault();
  const el = e.target.closest('.q-item');
  const from = queueDrag;
  if (el && from != null) {
    const r = el.getBoundingClientRect();
    let to = Number(el.dataset.index) + (e.clientY < r.top + r.height / 2 ? 0 : 1);
    if (to > from) to -= 1;
    moveQueue(from, Math.min(to, state.queue.items.length - 1));
  }
});
queueList.addEventListener('dragend', () => {
  queueDrag = null;
  $$('.q-item', queueList).forEach((x) => x.classList.remove('drag-over-top', 'drag-over-bottom', 'dragging'));
});

$('#queue-shuffle').addEventListener('click', () => call('shuffle_queue').then(() => toast('Shuffled upcoming tracks', 'info', 1500)));
$('#queue-clear').addEventListener('click', () => call('clear_queue', { keepCurrent: true }));
$('#queue-save').addEventListener('click', async () => {
  const tracks = state.queue.items.map((i) => i.track);
  if (!tracks.length) return toast('Queue is empty', 'info');
  const name = await promptModal({ title: 'Save queue as playlist', value: `Queue ${new Date().toLocaleDateString()}`, okLabel: 'Save' });
  if (!name) return;
  call('create_playlist', { name, tracks }).then(() => toast(`Saved ${name}`, 'ok')).catch((e) => toast(errMsg(e), 'error'));
});
