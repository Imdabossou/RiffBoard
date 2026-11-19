// RiffBoard menus: library, song page, results, leaderboards, settings.
// Plain DOM, rendered on navigation only; nothing here runs during play.

import { Session, loadWasm } from './player.js';

const $ = (sel, el = document) => el.querySelector(sel);
const view = $('#view');

/** Escapes untrusted text for HTML. */
function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

const encPath = (f) => f.split('/').map(encodeURIComponent).join('/');
const artUrl = (f) => `/api/song/${encPath(f)}/art`;
const fmtTime = (s) => { s = Math.max(0, Math.round(s || 0)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
const pct = (a) => `${Math.round((a || 0) * 100)}%`;
const armName = (a) => a.smart_name || a.name || `Part ${a.index + 1}`;

async function getJSON(url, opts) {
  const r = await fetch(url, opts);
  let body = null;
  try { body = await r.json(); } catch { /* not JSON */ }
  if (!r.ok) throw new Error((body && (body.error || body.detail)) || `Request failed (${r.status})`);
  return body;
}

// ── Settings (this browser only) ────────────────────────────────────────

const DEFAULTS = {
  speed: 11, lowStringBottom: false, lefty: false, sensitivity: 0.6, visualOffsetMs: 0, micOffsetMs: 0,
  quality: 'auto', micDeviceId: '', name: '',
};

function loadSettings() {
  try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem('rb-settings') || '{}') }; } catch { return { ...DEFAULTS }; }
}
function saveSettings(s) {
  try { localStorage.setItem('rb-settings', JSON.stringify(s)); } catch { /* storage blocked */ }
}
let settings = loadSettings();

// ── Theme ───────────────────────────────────────────────────────────────

const THEMES = ['system', 'light', 'dark'];
const ICONS = {
  system: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zm0 2v14a7 7 0 0 1 0-14z"/></svg>',
  light: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10zm0-5 1 3h-2zm0 20-1-3h2zM2 12l3-1v2zm20 0-3 1v-2zM4.9 4.9l2.8 1.4-1.4 1.4zm14.2 14.2-2.8-1.4 1.4-1.4zM4.9 19.1l1.4-2.8 1.4 1.4zM19.1 4.9l-1.4 2.8-1.4-1.4z"/></svg>',
  dark: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 15.3A8.5 8.5 0 0 1 8.7 4a8.5 8.5 0 1 0 11.3 11.3z"/></svg>',
};
function currentTheme() { return document.documentElement.dataset.theme || 'system'; }
function setTheme(t) {
  if (t === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = t;
  try { if (t === 'system') localStorage.removeItem('rb-theme'); else localStorage.setItem('rb-theme', t); } catch { /* blocked */ }
  const b = $('#theme-btn');
  b.innerHTML = ICONS[t];
  b.title = b.ariaLabel = `Theme: ${t}`;
}
$('#theme-btn').addEventListener('click', () => setTheme(THEMES[(THEMES.indexOf(currentTheme()) + 1) % 3]));
setTheme(currentTheme());

// ── Site branding ───────────────────────────────────────────────────────

let site = {};
async function loadSite() {
  try { site = await getJSON('/api/site'); } catch { site = {}; }
  if (site.name) {
    $('#brand-name').textContent = site.name;
    document.title = site.name;
  }
  if (site.favicon) {
    for (const l of document.querySelectorAll('link[rel="icon"]')) l.remove();
    const l = document.createElement('link');
    l.rel = 'icon'; l.href = site.favicon;
    document.head.append(l);
  }
  const img = document.createElement('img');
  img.src = site.logo || '/static/brand/riffboard-icon.svg';
  img.alt = '';
  $('#brand').prepend(img);
  try {
    const css = await (await fetch('/site.css')).text();
    if (css.includes('--fb-primary')) document.documentElement.classList.add('branded');
  } catch { /* unbranded */ }
}

// ── Router ──────────────────────────────────────────────────────────────

function route() {
  const h = decodeURIComponent(location.hash.slice(1) || '/');
  for (const a of document.querySelectorAll('[data-nav]')) a.removeAttribute('aria-current');
  if (h.startsWith('/song/')) return songPage(h.slice(6));
  if (h === '/boards') { navCurrent('boards'); return boardsPage(); }
  if (h === '/settings') { navCurrent('settings'); return settingsPage(); }
  navCurrent('songs');
  return libraryPage();
}
function navCurrent(n) { $(`[data-nav="${n}"]`)?.setAttribute('aria-current', 'page'); }
window.addEventListener('hashchange', () => { if (!session) route(); });

// ── Library ─────────────────────────────────────────────────────────────

let libraryCache = null;
async function libraryPage() {
  const hero = site.hero_image || '/static/brand/hero.png';
  view.innerHTML = `
    <div class="wrap">
      <section class="hero has-image"><div class="hero-bg" style="background-image:url('${esc(hero)}')"></div>
        <h1>${esc(site.hero_title || 'Play along. Top the board.')}</h1>
        <p class="dim">${esc(site.hero_text || 'Pick a song, plug in or use your mic, and play the whole thing to get on its leaderboard.')}</p>
        <p class="hero-actions"><a class="btn primary" href="#songs" id="start">Start playing</a><a class="btn" href="#/boards">Leaderboards</a></p>
      </section>
      <section class="steps" aria-label="How it works">
        <div class="card"><b>1</b><h2>Pick a song</h2><p class="dim">Choose a part: lead, rhythm or bass.</p></div>
        <div class="card"><b>2</b><h2>Plug in</h2><p class="dim">An audio interface or your phone or laptop mic. Headphones help.</p></div>
        <div class="card"><b>3</b><h2>Post your score</h2><p class="dim">Play the whole song and put your name on its board.</p></div>
      </section>
      <h2 id="songs">Songs</h2>
      <input class="search" type="search" id="q" placeholder="Search songs" aria-label="Search songs">
      <div class="grid" id="grid"><p class="dim">Loading songs...</p></div>
      ${aboutCard()}
    </div>`;
  $('#start').addEventListener('click', (e) => { e.preventDefault(); $('#songs').scrollIntoView({ behavior: 'smooth' }); $('#q').focus({ preventScroll: true }); });
  try {
    libraryCache ||= (await getJSON('/api/library?size=100&sort=artist')).songs || [];
  } catch (e) {
    $('#grid').innerHTML = `<p class="error">${esc(e.message)}</p>`;
    return;
  }
  const draw = (q) => {
    q = q.trim().toLowerCase();
    const songs = libraryCache.filter((s) => !q || `${s.title} ${s.artist} ${s.album}`.toLowerCase().includes(q));
    $('#grid').innerHTML = songs.length ? songs.map((s) => `
      <a class="song" href="#/song/${encodeURIComponent(s.filename)}">
        <img class="cover" src="${artUrl(s.filename)}" alt="" loading="lazy">
        <div class="meta"><div class="title">${esc(s.title)}</div>
        <div class="sub">${esc(s.artist)} · ${fmtTime(s.duration)}</div></div>
      </a>`).join('') : '<p class="dim">No songs found.</p>';
  };
  draw('');
  $('#q').addEventListener('input', (e) => draw(e.target.value));
  // Warm up the game engine while the visitor browses.
  loadWasm().catch(() => {});
}

function aboutCard() {
  const about = site.about || [];
  const links = site.links || [];
  if (!about.length && !links.length) return '';
  return `<section class="card about">
    <h2>${esc(site.about_title || `About ${site.name || 'us'}`)}</h2>
    ${about.map((p) => `<p>${esc(p)}</p>`).join('')}
    ${links.length ? `<p class="links">${links.map((l) => `<a class="btn" href="${esc(l.url)}" rel="noopener noreferrer" target="_blank">${esc(l.label)}</a>`).join('')}</p>` : ''}
  </section>`;
}

// ── Song page ───────────────────────────────────────────────────────────

async function songPage(filename) {
  view.innerHTML = '<div class="wrap"><p class="dim" style="padding:28px 0">Loading...</p></div>';
  let song;
  try {
    song = await getJSON(`/api/song/${encPath(filename)}`);
  } catch (e) {
    view.innerHTML = `<div class="wrap"><p class="error" style="padding:28px 0">${esc(e.message)}</p></div>`;
    return;
  }
  const arrs = (song.arrangements || []).slice().sort((a, b) => a.index - b.index);
  let arr = Number(sessionStorage.getItem(`rb-arr:${filename}`) ?? arrs[0]?.index ?? 0);
  if (!arrs.some((a) => a.index === arr)) arr = arrs[0]?.index ?? 0;
  view.innerHTML = `
    <div class="wrap song-page">
      <div><img class="cover" src="${artUrl(filename)}" alt=""></div>
      <div>
        <h1>${esc(song.title)}</h1>
        <p class="dim">${esc(song.artist)}${song.album ? ` · ${esc(song.album)}` : ''} · ${fmtTime(song.duration)}</p>
        ${arrs.length > 1 ? `<h2 style="margin-top:20px">Part</h2>
        <div class="choices" role="radiogroup" aria-label="Part">${arrs.map((a) => `
          <label class="choice"><input type="radio" name="arr" value="${a.index}" ${a.index === arr ? 'checked' : ''}><span>${esc(armName(a))}</span></label>`).join('')}
        </div>` : `<p class="dim">${esc(arrs[0] ? armName(arrs[0]) : '')}</p>`}
        <div class="modes">
          <button class="btn primary" id="play-score" type="button">Play for score</button>
          <button class="btn" id="play-practice" type="button">Practice</button>
        </div>
        <p class="dim small">Score mode listens through your microphone and needs the whole song. Headphones help: the detector should hear your instrument, not the backing track. Practice plays the song without judging, and lets you jump around.</p>
        <h2 style="margin-top:24px">Leaderboard</h2>
        <div id="board"><p class="dim">Loading...</p></div>
      </div>
    </div>`;
  const loadBoard = async () => {
    try {
      const b = await getJSON(`/api/scores/${encPath(filename)}?arrangement=${arr}&limit=10`);
      $('#board').innerHTML = boardTable(b.scores);
    } catch (e) { $('#board').innerHTML = `<p class="error">${esc(e.message)}</p>`; }
  };
  for (const r of document.querySelectorAll('input[name="arr"]')) {
    r.addEventListener('change', () => {
      arr = Number(r.value);
      try { sessionStorage.setItem(`rb-arr:${filename}`, String(arr)); } catch { /* blocked */ }
      loadBoard();
    });
  }
  $('#play-score').addEventListener('click', () => startPlay(filename, arr, 'score', song));
  $('#play-practice').addEventListener('click', () => startPlay(filename, arr, 'practice', song));
  loadBoard();
}

function boardTable(scores, highlightId) {
  if (!scores || !scores.length) return '<p class="dim">No scores yet. Be the first.</p>';
  return `<table class="board"><thead><tr><th>#</th><th>Name</th><th class="num">Score</th><th class="num">Accuracy</th></tr></thead><tbody>${
    scores.map((s) => `<tr class="${s.id === highlightId ? 'me' : ''}"><td class="${s.rank === 1 ? 'rank1' : ''}">${s.rank}</td><td>${esc(s.name)}</td><td class="num">${Number(s.score).toLocaleString()}</td><td class="num">${pct(s.accuracy)}</td></tr>`).join('')
  }</tbody></table>`;
}

// ── Play ────────────────────────────────────────────────────────────────

let session = null;
const playEl = $('#play');
const overlay = $('#play-overlay');

function showOverlay(html) {
  overlay.innerHTML = `<div class="panel">${html}</div>`;
  overlay.hidden = false;
  const first = overlay.querySelector('[autofocus], .btn.primary, .btn');
  first?.focus();
}
function hideOverlay() { overlay.hidden = true; overlay.innerHTML = ''; }

async function startPlay(filename, arr, mode, song) {
  if (session) return;
  playEl.hidden = false;
  document.body.style.overflow = 'hidden';
  const s = new Session($('#stage'), settings);
  session = s;
  s.mode = mode;
  s.filename = filename;
  s.arrangement = arr;
  s.song = song;
  const stage = (t) => showOverlay(`<div class="spinner"></div><p class="dim" style="text-align:center;margin:0">${esc(t)}...</p>`);
  try {
    await s.load(filename, arr, stage);
    if (mode === 'score') {
      stage('Asking for the microphone');
      try {
        await s.enableMic();
      } catch {
        showOverlay(`<h2>No microphone</h2><p class="dim">Score mode needs your microphone to hear you play. Allow it in your browser, or practise without scoring.</p>
          <div class="row"><button class="btn" data-act="quit" type="button">Back</button><button class="btn primary" data-act="practice" type="button">Practice instead</button></div>`);
        return;
      }
      stage('Getting your run ready');
      try {
        const r = await getJSON('/api/runs', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ filename, arrangement: arr }) });
        s.runId = r.run_id;
      } catch (e) {
        s.runError = e.message;
      }
    }
  } catch (e) {
    showOverlay(`<h2>Could not start</h2><p class="dim">${esc(e.message)}</p><button class="btn" data-act="quit" type="button">Back</button>`);
    return;
  }
  hideOverlay();
  s.onEnd = () => showResults(s);
  s.play(-s.leadIn);
}

function pausePlay() {
  const s = session;
  if (!s || s.ended || s.paused || !overlay.hidden) return;
  s.pause();
  const sections = s.mode === 'practice' && s.info.sections.length > 1
    ? `<p class="dim small" style="margin:4px 0 0">Jump to</p><div class="sections">${s.info.sections.map((x, i) => `<button class="btn" type="button" data-act="jump" data-i="${i}">${esc(x.name || `Part ${i + 1}`)}</button>`).join('')}</div>`
    : '';
  showOverlay(`<h2>Paused</h2>
    <p class="dim">${esc(s.info.title)} · ${fmtTime(Math.max(0, s.time()))} / ${fmtTime(s.endT)}</p>
    <button class="btn primary" type="button" data-act="resume">Resume</button>
    ${s.mode === 'practice' ? '<button class="btn" type="button" data-act="restart">Restart</button>' : ''}
    ${sections}
    <button class="btn" type="button" data-act="quit">Quit</button>`);
}

function resumePlay() {
  const s = session;
  hideOverlay();
  // Rewind a moment so the player can find the groove again.
  const t = s.time();
  s.play(s.mode === 'practice' ? Math.max(-1, t - 1.5) : t);
  if (s.mode === 'practice') s.game.seek(Math.max(-1, t - 1.5));
}

function endPlay() {
  session?.destroy();
  session = null;
  hideOverlay();
  playEl.hidden = true;
  document.body.style.overflow = '';
  route();
}

function showResults(s) {
  const t = s.tally();
  const scored = s.mode === 'score';
  const canPost = scored && s.runId && t.hits + t.misses > 0;
  showOverlay(`<h2>${esc(s.info.title)}</h2>
    ${scored ? `<div class="big">${Number(t.score).toLocaleString()}</div>
    <div class="stats">
      <div class="stat"><span class="dim small">Accuracy</span><b>${pct(t.accuracy)}</b></div>
      <div class="stat"><span class="dim small">Best streak</span><b>${t.best_streak}</b></div>
      <div class="stat"><span class="dim small">Hits</span><b>${t.hits}</b></div>
      <div class="stat"><span class="dim small">Misses</span><b>${t.misses}</b></div>
    </div>` : '<p class="dim">Practice run finished.</p>'}
    ${canPost ? `<form id="post" class="row" autocomplete="on">
        <input type="text" id="name" name="nickname" maxlength="24" placeholder="Your name" aria-label="Your name" value="${esc(settings.name)}" autofocus>
        <button class="btn primary" type="submit" style="flex:0 0 auto">Post</button>
      </form><p class="dim small" id="post-msg" style="margin:0"></p>` : ''}
    ${scored && !s.runId ? `<p class="dim small">This run can't go on the leaderboard${s.runError ? `: ${esc(s.runError)}` : ''}.</p>` : ''}
    <div id="result-board"></div>
    <div class="row"><button class="btn" type="button" data-act="again">Play again</button><button class="btn" type="button" data-act="quit">Done</button></div>`);
  $('#post')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const name = $('#name').value.trim();
    const btn = e.target.querySelector('button');
    btn.disabled = true;
    try {
      const r = await getJSON('/api/scores', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ run_id: s.runId, name, hits: t.hits, misses: t.misses, best_streak: t.best_streak }),
      });
      settings.name = name;
      saveSettings(settings);
      $('#post').remove();
      $('#post-msg').textContent = r.rank ? `You placed #${r.rank}.` : 'Score posted.';
      $('#result-board').innerHTML = boardTable(r.board, r.id);
      s.runId = null;
    } catch (err) {
      btn.disabled = false;
      $('#post-msg').textContent = err.message;
    }
  });
}

overlay.addEventListener('click', (e) => {
  const b = e.target.closest('[data-act]');
  if (!b || !session) return;
  const s = session;
  switch (b.dataset.act) {
    case 'resume': resumePlay(); break;
    case 'restart': hideOverlay(); s.game.seek(-s.leadIn); s.play(-s.leadIn); break;
    case 'jump': {
      const t = s.info.sections[Number(b.dataset.i)].time - 1.5;
      hideOverlay(); s.game.seek(t); s.play(t); break;
    }
    case 'again': {
      const { filename, arrangement, mode, song } = s;
      session.destroy(); session = null; hideOverlay();
      startPlay(filename, arrangement, mode, song);
      break;
    }
    case 'practice': {
      const { filename, arrangement, song } = s;
      session.destroy(); session = null; hideOverlay();
      startPlay(filename, arrangement, 'practice', song);
      break;
    }
    case 'quit': endPlay(); break;
  }
});

$('#pause-btn').addEventListener('click', pausePlay);
document.addEventListener('keydown', (e) => {
  if (!session || e.target.closest('input, textarea, select')) return;
  if (e.key === ' ' || e.key === 'Escape' || e.key === 'p') {
    e.preventDefault();
    if (!overlay.hidden && $('[data-act="resume"]', overlay)) resumePlay();
    else pausePlay();
  }
});
document.addEventListener('visibilitychange', () => { if (document.hidden) pausePlay(); });

// ── Leaderboards ────────────────────────────────────────────────────────

async function boardsPage() {
  view.innerHTML = '<div class="wrap" style="padding-top:24px"><h1>Leaderboards</h1><div class="boards" id="boards"><p class="dim">Loading...</p></div></div>';
  try {
    const { boards } = await getJSON('/api/leaderboards');
    $('#boards').innerHTML = boards.length ? boards.map((b) => `
      <a class="board-row" href="#/song/${encodeURIComponent(b.filename)}">
        <img src="${esc(b.art_url)}" alt="" loading="lazy">
        <div><div style="font-weight:700">${esc(b.title)}</div><div class="dim small">${esc(b.artist)} · ${esc(b.arrangement_name)} · ${b.runs} ${b.runs === 1 ? 'run' : 'runs'}</div></div>
        <div class="top-score"><div style="font-weight:700">${Number(b.top_score).toLocaleString()}</div><div class="dim small">${esc(b.top_name)}</div></div>
      </a>`).join('') : '<p class="dim">No scores yet. Pick a song and be the first.</p>';
  } catch (e) { $('#boards').innerHTML = `<p class="error">${esc(e.message)}</p>`; }
}

// ── Settings ────────────────────────────────────────────────────────────

async function settingsPage() {
  const range = (id, label, min, max, step, val, fmt, help) => `
    <div class="setting card"><label for="${id}">${label}</label>
      <div class="row"><input type="range" id="${id}" min="${min}" max="${max}" step="${step}" value="${val}"><output for="${id}">${fmt(val)}</output></div>
      ${help ? `<span class="dim small">${help}</span>` : ''}</div>`;
  view.innerHTML = `<div class="wrap"><div class="settings">
    <h1>Settings</h1>
    <p class="dim" style="margin:0">Saved in this browser only.</p>
    ${range('speed', 'Note speed', 6, 22, 1, settings.speed, (v) => `${v}`, 'How fast notes travel down the highway.')}
    <div class="setting card"><label for="hand">Handedness</label>
      <select id="hand"><option value="right">Right-handed</option><option value="left">Left-handed (mirrored highway)</option></select></div>
    <div class="setting card"><label for="strings">String order</label>
      <select id="strings"><option value="top">Lowest string at the top</option><option value="bottom">Lowest string at the bottom (like tab)</option></select></div>
    ${range('sens', 'Detection leniency', 0, 1, 0.05, settings.sensitivity, (v) => `${Math.round(v * 100)}%`, 'Higher accepts quieter or less clean notes; lower is stricter about the right fret.')}
    ${range('mic-off', 'Microphone delay', -150, 150, 5, settings.micOffsetMs, (v) => `${v} ms`, 'If correct notes count as late, raise this; if they count as early, lower it.')}
    ${range('vis-off', 'Picture offset', -100, 100, 5, settings.visualOffsetMs, (v) => `${v} ms`, 'Moves the notes against the audio, for screens that lag.')}
    <div class="setting card"><label for="quality">Graphics</label>
      <select id="quality"><option value="auto">Automatic</option><option value="sharp">Sharpest</option><option value="fast">Fastest (lower resolution)</option></select></div>
    <div class="setting card"><label for="mic">Microphone</label>
      <select id="mic"><option value="">Default</option></select>
      <span class="dim small" id="mic-help">The browser shows device names once you have allowed the microphone.</span></div>
  </div></div>`;
  $('#strings').value = settings.lowStringBottom ? 'bottom' : 'top';
  $('#hand').value = settings.lefty ? 'left' : 'right';
  $('#hand').addEventListener('change', (e) => { settings.lefty = e.target.value === 'left'; saveSettings(settings); });
  $('#quality').value = settings.quality;
  const bind = (id, key, conv) => {
    const el = $(`#${id}`);
    el.addEventListener('input', () => {
      settings[key] = conv(el.value);
      const out = el.parentElement.querySelector('output');
      if (out) out.textContent = id === 'sens' ? `${Math.round(settings[key] * 100)}%` : id === 'speed' ? `${settings[key]}` : `${settings[key]} ms`;
      saveSettings(settings);
    });
  };
  bind('speed', 'speed', Number);
  bind('sens', 'sensitivity', Number);
  bind('mic-off', 'micOffsetMs', Number);
  bind('vis-off', 'visualOffsetMs', Number);
  $('#strings').addEventListener('change', (e) => { settings.lowStringBottom = e.target.value === 'bottom'; saveSettings(settings); });
  $('#quality').addEventListener('change', (e) => { settings.quality = e.target.value; saveSettings(settings); });
  const mic = $('#mic');
  mic.addEventListener('change', () => { settings.micDeviceId = mic.value; saveSettings(settings); });
  try {
    const devs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'audioinput' && d.deviceId && d.deviceId !== 'default');
    for (const d of devs) {
      const o = document.createElement('option');
      o.value = d.deviceId; o.textContent = d.label || 'Microphone';
      mic.append(o);
    }
    mic.value = settings.micDeviceId;
    if (devs.some((d) => d.label)) $('#mic-help').remove();
  } catch { /* no device list */ }
}

// Songs without cover art get a plain placeholder instead of a broken image.
const NO_ART = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="#8884"/><path fill="#8888" d="M15 5v9.2a3 3 0 1 1-2-2.8V7h-3V5z"/></svg>')}`;
document.addEventListener('error', (e) => {
  const img = e.target;
  if (img instanceof HTMLImageElement && img.matches('.cover, .board-row img') && img.src !== NO_ART) img.src = NO_ART;
}, true);

// ── Start ───────────────────────────────────────────────────────────────

// AGPL: every page offers its source code.
getJSON('/api/version').then((v) => {
  $('#foot').innerHTML = `RiffBoard ${esc(v.version)} · <a href="${esc(v.source_url)}" rel="noopener">Source code</a> · <a href="${esc(v.license_url)}" rel="noopener">AGPL-3.0 licence</a>`;
}).catch(() => {});

loadSite().finally(route);
