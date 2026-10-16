'use strict';

// ===========================================================================
// Player bar
// ===========================================================================

const progressEl = $('#progress');

function renderPlayback() {
  const p = state.playback;
  if (!p) return;
  const item = p.current;
  const track = item?.track;
  $('#np-title').textContent = track ? track.title : 'Nothing playing';
  $('#np-title').title = track?.title || '';
  $('#np-artist').textContent = track ? (track.artist || 'Unknown artist') : '';
  $('#np-artist').disabled = !track?.artist;

  const cover = $('#np-cover');
  const thumb = p.version?.thumbnail || track?.thumbnail;
  if (cover.dataset.src !== (thumb || '')) {
    cover.dataset.src = thumb || '';
    $('#player-bar').style.setProperty('--np-art', thumb ? `url("${thumb.replace(/"/g, '%22')}")` : 'none');
    cover.innerHTML = thumb ? '' : icon('music');
    if (thumb) cover.append(h('img', { src: thumb, alt: '', onerror: (e) => { e.target.remove(); cover.innerHTML = icon('music'); } }));
    updateCoverAccent(thumb);
    if (fullLyrics.open) renderFullLyrics();
  }

  const msg = $('#np-via');
  msg.className = 'np-via';
  if (p.status === 'loading') {
    msg.innerHTML = `<span class="spinner"></span>${esc(p.message || 'Loading…')}`;
  } else if (p.status === 'error') {
    msg.classList.add('error');
    msg.textContent = p.message || 'Playback failed';
  } else if (p.stalled) {
    msg.innerHTML = `<span class="spinner"></span>Buffering…`;
  } else if (p.version && track && p.version.matched_for) {
    // An upload standing in for the version the user picked (Spotify).
    const from = PLATFORMS[p.version.matched_for]?.label, via = PLATFORMS[p.version.platform]?.label;
    msg.title = p.version.isrc_match
      ? `${from} recording, streamed from ${via} (same recording, matched by ISRC)`
      : `Closest ${via} match for the ${from} track`;
    msg.replaceChildren(miniLogo(p.version.matched_for), h('span', {}, from), h('span', { class: 'via-sep' }, `via ${via}`));
  } else if (p.version && track) {
    msg.title = '';
    msg.replaceChildren(miniLogo(p.version.platform), h('span', {}, PLATFORMS[p.version.platform]?.label));
  } else {
    msg.textContent = '';
  }

  const playBtn = $('#btn-play');
  const want = p.status === 'loading' ? 'loading' : p.status === 'playing' ? 'pause' : 'play';
  if (playBtn.dataset.state !== want) {
    playBtn.dataset.state = want;
    playBtn.innerHTML = want === 'loading' ? '<span class="spinner"></span>' : icon(want);
  }

  renderProgress();

  const like = $('#np-like');
  const liked = track && state.favKeys.has(track.key);
  like.classList.toggle('on', !!liked);
  like.innerHTML = icon(liked ? 'heartFill' : 'heart');
  like.disabled = !track;

  const rep = $('#btn-repeat');
  rep.classList.toggle('on', p.repeat !== 'off');
  rep.innerHTML = icon(p.repeat === 'one' ? 'repeatOne' : 'repeat');
  rep.title = `Repeat: ${p.repeat}`;

  if (!volumeDragging) setVolumeUI(p.volume);
  document.title = track && p.status === 'playing' ? `${track.title} — ${track.artist || 'Tunewire'}` : 'Tunewire';
  if (p.status === 'playing' || p.status === 'error') state.loadingKey = null;
  // Walking every row is only needed when the playing track or status changes.
  const mark = `${track?.key}|${p.status}|${state.loadingKey}`;
  if (mark !== lastMark) {
    lastMark = mark;
    markPlaying();
    updateQueueIndicators();
  }
}

let lastMark = '';

/** Time, seek bar and buffering only — runs for every progress tick. */
function renderProgress() {
  const p = state.playback;
  if (!p) return;
  const dur = p.duration;
  if (!state.draggingProgress) {
    $('#t-pos').textContent = fmtTime(p.position);
    setProgress(dur ? p.position / dur : 0);
  }
  $('#t-dur').textContent = fmtTime(dur);
  $('#p-buffered').style.width = `${Math.min(1, p.buffered ?? 0) * 100}%`;
  progressEl.classList.toggle('disabled', !(p.status === 'playing' || p.status === 'paused') || !dur);
  progressEl.classList.toggle('stalled', !!p.stalled);
  const msg = $('#np-via');
  if (p.stalled && !msg.dataset.stalled) {
    msg.dataset.stalled = '1';
    renderPlayback();
  } else if (!p.stalled && msg.dataset.stalled) {
    delete msg.dataset.stalled;
    renderPlayback();
  }
}

function setProgress(frac) {
  frac = Math.max(0, Math.min(1, frac || 0));
  $('#p-played').style.width = `${frac * 100}%`;
  $('#p-knob').style.left = `${frac * 100}%`;
}

function fracFromEvent(e) {
  const r = progressEl.getBoundingClientRect();
  return Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
}

progressEl.addEventListener('pointerdown', (e) => {
  if (!state.playback?.duration) return;
  state.draggingProgress = true;
  progressEl.classList.add('dragging');
  progressEl.setPointerCapture(e.pointerId);
  const f = fracFromEvent(e);
  setProgress(f);
  $('#t-pos').textContent = fmtTime(f * state.playback.duration);
});
progressEl.addEventListener('pointermove', (e) => {
  const dur = state.playback?.duration;
  if (!dur) return;
  const f = fracFromEvent(e);
  const ht = $('#p-hover');
  ht.textContent = fmtTime(f * dur);
  ht.style.left = `${f * 100}%`;
  if (state.draggingProgress) {
    setProgress(f);
    $('#t-pos').textContent = fmtTime(f * dur);
  }
});
progressEl.addEventListener('pointerup', (e) => {
  if (!state.draggingProgress) return;
  state.draggingProgress = false;
  progressEl.classList.remove('dragging');
  const dur = state.playback?.duration;
  if (dur) seekTo(fracFromEvent(e) * dur);
});
progressEl.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
    e.preventDefault();
    e.stopPropagation();
    seekBy(e.key === 'ArrowLeft' ? -5 : 5);
  }
});

function seekTo(pos) {
  if (state.playback) state.playback.position = pos;
  call('seek', { position: Math.max(0, pos) }).catch((e) => toast(errMsg(e), 'error'));
}

function seekBy(delta) {
  const p = state.playback;
  if (!p || !p.duration || !(p.status === 'playing' || p.status === 'paused')) return;
  seekTo(Math.min(p.duration - 1, Math.max(0, p.position + delta)));
}

$('#btn-play').addEventListener('click', () => call('toggle_pause').catch((e) => toast(errMsg(e), 'error')));
$('#btn-next').addEventListener('click', () => call('next_track'));
$('#btn-prev').addEventListener('click', () => call('previous_track'));
$('#btn-stop').addEventListener('click', () => call('stop'));
$('#btn-repeat').addEventListener('click', () => {
  const order = ['off', 'all', 'one'];
  const next = order[(order.indexOf(state.playback?.repeat || 'off') + 1) % 3];
  call('set_repeat', { mode: next }).then(() => toast(`Repeat: ${next}`, 'info', 1200));
});
$('#np-like').addEventListener('click', () => {
  const t = state.playback?.current?.track;
  if (t) toggleFavorite(t);
});

// Volume ---------------------------------------------------------------------
const volumeEl = $('#volume');
let volumeDragging = false;
let lastNonZeroVolume = 0.7;

function setVolumeUI(v) {
  volumeEl.value = Math.round(v * 100);
  volumeEl.style.setProperty('--fill', `${v * 100}%`);
  $('#btn-mute').innerHTML = icon(v === 0 ? 'mute' : v < 0.45 ? 'volumeLow' : 'volume');
  if (v > 0) lastNonZeroVolume = v;
}

const sendVolume = throttle((v) => call('set_volume', { volume: v }).catch(() => {}), 60);
function setVolume(v) {
  v = Math.max(0, Math.min(1, v));
  setVolumeUI(v);
  if (state.playback) state.playback.volume = v;
  sendVolume(v);
}
volumeEl.addEventListener('input', () => { volumeDragging = true; setVolume(volumeEl.value / 100); });
volumeEl.addEventListener('change', () => { volumeDragging = false; setVolume(volumeEl.value / 100); });
volumeEl.addEventListener('wheel', (e) => { e.preventDefault(); setVolume(volumeEl.value / 100 + (e.deltaY < 0 ? 0.05 : -0.05)); }, { passive: false });
$('#btn-mute').addEventListener('click', () => {
  const v = Number(volumeEl.value) / 100;
  setVolume(v > 0 ? 0 : lastNonZeroVolume || 0.5);
});

$('#np-artist').addEventListener('click', () => {
  const t = state.playback?.current?.track;
  if (t?.artist) openArtist(t.artist, t);
});
