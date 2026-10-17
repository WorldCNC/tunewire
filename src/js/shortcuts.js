'use strict';

// ===========================================================================
// Keyboard shortcuts
// ===========================================================================

const SHORTCUTS = [
  ['Space', 'Play / pause'],
  ['← / →', 'Seek −5 s / +5 s (Shift: 15 s)'],
  ['↑ / ↓', 'Volume up / down'],
  ['M', 'Mute'],
  ['N / P', 'Next / previous track'],
  ['L', 'Like the current track'],
  ['R', 'Cycle repeat mode'],
  ['S', 'Shuffle on / off'],
  ['Alt+← / Alt+→', 'Back / forward'],
  ['Tab', 'Move into a song list, then ↑ / ↓ to pick a song'],
  ['Enter', 'Play the focused song'],
  ['Shift+↑ / Shift+↓', 'Select several songs'],
  ['Home / End', 'First / last song in the list'],
  ['Shift+F10', 'Song menu (or the Menu key)'],
  ['Ctrl+A', 'Select all songs in a list'],
  ['Q', 'Toggle queue panel'],
  ['/ or Ctrl+K', 'Focus search'],
  ['1 – 7', 'Switch view'],
  ['Esc', 'Close dialog / menu'],
  ['?', 'This help'],
];

function showShortcuts() {
  openModal({
    title: 'Keyboard shortcuts',
    body: h('div', { class: 'shortcuts' }, ...SHORTCUTS.flatMap(([k, d]) => [h('div', {}, ...k.split(' ').map((p) => (p === '/' || p === 'or' || p === '–' ? ` ${p} ` : h('kbd', {}, p)))), h('div', { class: 'muted' }, d)])),
  });
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    if (openMenuEl) return closeMenu();
    if (modalStack.length) return modalStack[modalStack.length - 1].close();
    if (fsp.el) return closeFullscreen();
    if (selectedRows().length) return clearSelection();
    if (document.activeElement?.blur) document.activeElement.blur();
    return;
  }
  const t = e.target;
  const typing = t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName) && !(t.type === 'range' || t.type === 'checkbox');
  if (typing || modalStack.length) return;
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); $('#search-input').focus(); $('#search-input').select(); return; }
  if (e.altKey && e.key === 'ArrowLeft') { e.preventDefault(); goBack(); return; }
  if (e.altKey && e.key === 'ArrowRight') { e.preventDefault(); goForward(); return; }
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  const views = ['home', 'search', 'favorites', 'history', 'stats', 'charts', 'settings'];
  switch (e.key) {
    case ' ': e.preventDefault(); call('toggle_pause'); break;
    case 'ArrowLeft': e.preventDefault(); seekBy(e.shiftKey ? -15 : -5); break;
    case 'ArrowRight': e.preventDefault(); seekBy(e.shiftKey ? 15 : 5); break;
    case 'ArrowUp': e.preventDefault(); setVolume(Number(volumeEl.value) / 100 + 0.05); break;
    case 'ArrowDown': e.preventDefault(); setVolume(Number(volumeEl.value) / 100 - 0.05); break;
    case 'm': case 'M': $('#btn-mute').click(); break;
    case 'n': case 'N': call('next_track'); break;
    case 'p': case 'P': call('previous_track'); break;
    case 'l': case 'L': $('#np-like').click(); break;
    case 'r': case 'R': $('#btn-repeat').click(); break;
    case 's': case 'S': $('#btn-shuffle').click(); break;
    case 'q': case 'Q': toggleQueue(); break;
    case '/': e.preventDefault(); $('#search-input').focus(); $('#search-input').select(); break;
    case '?': showShortcuts(); break;
    default:
      if (/^[1-7]$/.test(e.key)) showView(views[Number(e.key) - 1]);
  }
});
