/*
 * RiffBoard — play modes and the song map.
 *
 * Every song starts in one of two modes, picked in a small chooser:
 *
 *   Score     the song plays straight through, start to finish. No seeking,
 *             looping, speed or difficulty changes, and pausing ends the run.
 *             A finished run can be posted to the song part's leaderboard.
 *   Practice  everything is available: pause, rewind, skip, loops, sections,
 *             slow-down, difficulty. Practice runs are never ranked.
 *
 * The song map is a strip across the top of the player showing the song's
 * sections and the playhead. In practice mode you can tap or drag it to jump
 * anywhere; in score mode it only shows progress.
 *
 * Enforcement is belt and braces: score mode hides the controls (CSS, see
 * riffboard.css "Play modes"), swallows the seek keys, treats any pause as
 * the end of the run, and leaderboard.js still voids a run on any seek, loop,
 * speed or difficulty change. The server re-checks elapsed time and note
 * counts on submit regardless.
 */
(function () {
    'use strict';

    // ── Pure helpers (also exported for node tests) ──────────────────────────
    const SECTION_KINDS = [
        ['intro', 'intro'], ['verse', 'verse'], ['prechorus', 'pre'], ['pre-chorus', 'pre'], ['pre', 'pre'],
        ['chorus', 'chorus'], ['bridge', 'bridge'], ['solo', 'solo'], ['breakdown', 'breakdown'],
        ['riff', 'riff'], ['outro', 'outro'], ['noguitar', 'rest'], ['silence', 'rest'],
    ];
    function sectionKind(name) {
        const low = String(name || '').toLowerCase();
        for (const [needle, kind] of SECTION_KINDS) if (low.includes(needle)) return kind;
        return 'other';
    }
    // "verse2" → "Verse 2", "pre_chorus" → "Pre chorus".
    function sectionLabel(name) {
        const s = String(name || '').replace(/[_-]+/g, ' ').replace(/([^\d\s])(\d+)$/, '$1 $2').replace(/\s+/g, ' ').trim();
        return s ? s.charAt(0).toUpperCase() + s.slice(1) : 'Section';
    }
    // Sections ({name, time}) → blocks with left/width as fractions of the
    // song. Unsorted, duplicate, negative or past-the-end times are tolerated.
    function layoutSections(sections, duration) {
        if (!Array.isArray(sections) || !(duration > 0)) return [];
        const list = sections
            .filter((s) => s && Number.isFinite(Number(s.time)))
            .map((s) => ({ name: s.name, time: Math.max(0, Math.min(duration, Number(s.time))) }))
            .sort((a, b) => a.time - b.time);
        const out = [];
        if (list.length && list[0].time > 0.5) list.unshift({ name: 'Start', time: 0 });
        for (let i = 0; i < list.length; i++) {
            const start = list[i].time;
            const end = i + 1 < list.length ? list[i + 1].time : duration;
            if (end - start <= 0) continue;
            out.push({
                start, end,
                left: start / duration, width: (end - start) / duration,
                label: sectionLabel(list[i].name), kind: sectionKind(list[i].name),
            });
        }
        return out;
    }
    function fmt(t) {
        t = Math.max(0, Math.floor(Number(t) || 0));
        return Math.floor(t / 60) + ':' + String(t % 60).padStart(2, '0');
    }

    if (typeof module !== 'undefined' && module.exports) {
        module.exports = { sectionKind, sectionLabel, layoutSections, fmt };
        return;
    }

    const sm = window.feedBack;
    if (!sm || typeof sm.on !== 'function') return;
    const rb = (window.riffboard = window.riffboard || {});
    const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => (
        { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
    const enc = encodeURIComponent;
    function decode(fn) { try { return decodeURIComponent(fn); } catch (_) { return fn; } }
    const $ = (id) => document.getElementById(id);

    // ── Mode state ───────────────────────────────────────────────────────────
    let mode = 'practice';
    let current = null;      // { filename (encoded), arrangement } of the loaded song
    let pendingMode = null;  // set just before we call through to playSong
    let savedMastery = null; // practice difficulty to restore after a score run

    function applyModeClass() {
        const p = $('player');
        if (p) {
            p.classList.toggle('rb-mode-score', mode === 'score');
            p.classList.toggle('rb-mode-practice', mode === 'practice');
        }
        renderPill();
        if (map) map.classList.toggle('rb-map-seekable', mode === 'practice');
    }
    function setMode(m) {
        mode = m === 'score' ? 'score' : 'practice';
        applyModeClass();
        if (mode === 'practice' && savedMastery != null && typeof window.setMastery === 'function') {
            const v = savedMastery; savedMastery = null;
            try { window.setMastery(v); } catch (_) { /* */ }
        }
        sm.emit('riffboard:mode', { mode });
    }
    rb.getMode = () => mode;

    // Score runs always start at full speed and difficulty with no loop.
    function normaliseForScore() {
        try { if (typeof window.setSpeed === 'function') window.setSpeed(1); } catch (_) { /* */ }
        try { if (sm.clearLoop) sm.clearLoop({ reason: 'riffboard-score-mode' }); } catch (_) { /* */ }
        try {
            const hw = window.highway;
            const m = hw && hw.getMastery ? hw.getMastery() : 1;
            if (m < 0.999 && typeof window.setMastery === 'function') {
                if (savedMastery == null) savedMastery = Math.round(m * 100);
                window.setMastery(100);
            }
        } catch (_) { /* */ }
    }

    // ── Chooser ──────────────────────────────────────────────────────────────
    const origPlaySong = window.playSong;
    // Go through the CURRENT window.playSong so plugin wrappers installed after
    // ours still run; our wrapper sees pendingMode and passes straight through.
    function startSong(filename, arrangement, options, m) {
        pendingMode = m;
        setMode(m);
        if (m === 'score') normaliseForScore();
        try {
            return window.playSong(filename, arrangement, options);
        } finally {
            // song:loading normally consumes it; don't let a failed start leak
            // the mode into the next unrelated play.
            setTimeout(() => { pendingMode = null; }, 0);
        }
    }
    rb.play = (filename, arrangement, m) => startSong(filename, arrangement, undefined, m === 'score' ? 'score' : 'practice');

    function closeChooser() {
        const c = $('rb-chooser');
        if (!c) return;
        if (c._onKey) document.removeEventListener('keydown', c._onKey, true);
        c.remove();
    }

    async function chooser(filename, arrangement, options) {
        closeChooser();
        const fn = decode(filename);
        const wrap = document.createElement('div');
        wrap.id = 'rb-chooser';
        wrap.className = 'feedBack-modal fixed inset-0 z-[260] flex items-center justify-center p-4 bg-black/70';
        wrap.innerHTML =
            '<div role="dialog" aria-modal="true" aria-labelledby="rb-chooser-title" class="w-full max-w-lg max-h-[92vh] overflow-y-auto bg-fb-card border border-fb-border rounded-xl shadow-2xl p-5 sm:p-6 text-fb-text">' +
            '<div class="flex items-center gap-4">' +
            '<img src="/api/song/' + esc(enc(fn)) + '/art" alt="" class="w-16 h-16 rounded-lg object-cover bg-fb-bg shrink-0" onerror="this.style.visibility=\'hidden\'">' +
            '<div class="min-w-0"><h2 id="rb-chooser-title" class="text-xl font-bold truncate">Loading…</h2>' +
            '<p id="rb-chooser-artist" class="text-sm text-fb-textDim truncate"></p></div></div>' +
            '<div id="rb-chooser-parts" class="mt-4 hidden"><div class="text-xs uppercase tracking-wider text-fb-textDim mb-2">Part</div>' +
            '<div class="flex flex-wrap gap-2" role="radiogroup" aria-label="Part"></div></div>' +
            '<div class="grid sm:grid-cols-2 gap-3 mt-5">' +
            '<button type="button" data-mode="score" class="rb-mode-card rb-mode-card-primary">' +
            '<span class="rb-mode-card-title">Play for score</span>' +
            '<span class="rb-mode-card-text">The whole song, start to finish, at full speed. No pausing or skipping. Your score goes on the leaderboard.</span></button>' +
            '<button type="button" data-mode="practice" class="rb-mode-card">' +
            '<span class="rb-mode-card-title">Practice</span>' +
            '<span class="rb-mode-card-text">Pause, rewind, jump to any section, loop parts and slow it down. Not ranked.</span></button>' +
            '</div>' +
            '<div class="mt-4 flex justify-between items-center gap-2">' +
            '<button type="button" data-board class="text-sm text-fb-primary hover:text-fb-primaryHi underline-offset-2 hover:underline">View leaderboard</button>' +
            '<button type="button" data-close class="px-4 py-2 rounded-md border border-fb-border hover:bg-fb-bg text-sm">Cancel</button></div></div>';
        document.body.appendChild(wrap);
        const onKey = (e) => { if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); closeChooser(); } };
        wrap._onKey = onKey;
        document.addEventListener('keydown', onKey, true);
        wrap.addEventListener('click', (e) => { if (e.target === wrap) closeChooser(); });
        wrap.querySelector('[data-close]').addEventListener('click', closeChooser);

        let arr = Number.isFinite(Number(arrangement)) && arrangement !== null && arrangement !== '' ? Number(arrangement) : undefined;
        wrap.querySelectorAll('[data-mode]').forEach((b) => b.addEventListener('click', () => {
            const m = b.getAttribute('data-mode');
            closeChooser();
            startSong(filename, arr, options, m);
        }));
        wrap.querySelector('[data-board]').addEventListener('click', () => {
            closeChooser();
            if (rb.openBoard) rb.openBoard(fn, arr || 0);
        });
        setTimeout(() => wrap.querySelector('[data-mode="score"]')?.focus(), 30);

        let meta = null;
        try { const r = await fetch('/api/song/' + enc(fn)); meta = r.ok ? await r.json() : null; } catch (_) { /* */ }
        if (!wrap.isConnected) return;
        wrap.querySelector('#rb-chooser-title').textContent = (meta && meta.title) || fn.split('/').pop();
        wrap.querySelector('#rb-chooser-artist').textContent = (meta && meta.artist) || '';
        const arrs = (meta && Array.isArray(meta.arrangements)) ? meta.arrangements : [];
        if (arrs.length > 1) {
            if (arr === undefined || !arrs.some((a) => a.index === arr)) arr = arrs[0].index;
            const box = wrap.querySelector('#rb-chooser-parts');
            const group = box.querySelector('[role=radiogroup]');
            const paint = () => group.querySelectorAll('button').forEach((b) => {
                const on = Number(b.dataset.arr) === arr;
                b.setAttribute('aria-checked', on ? 'true' : 'false');
                b.classList.toggle('rb-part-on', on);
            });
            group.innerHTML = arrs.map((a) => '<button type="button" role="radio" data-arr="' + Number(a.index) + '" class="rb-part">' +
                esc(a.name || ('Part ' + (Number(a.index) + 1))) + '</button>').join('');
            group.querySelectorAll('button').forEach((b) => b.addEventListener('click', () => { arr = Number(b.dataset.arr); paint(); }));
            paint();
            box.classList.remove('hidden');
        } else if (arrs.length === 1 && arr === undefined) {
            arr = arrs[0].index;
        }
    }

    if (typeof origPlaySong === 'function') {
        window.playSong = function (filename, arrangement, options) {
            // Calls we made ourselves (or a re-entrant plugin wrapper calling
            // back through) go straight through.
            if (pendingMode) {
                const m = pendingMode;
                setMode(m);
                return origPlaySong.call(window, filename, arrangement, options);
            }
            return chooser(filename, arrangement, options);
        };
    }

    sm.on('song:loading', (e) => {
        const d = (e && e.detail) || {};
        current = { filename: d.filename, arrangement: d.arrangement };
        // A load we didn't start (resume, play queue, a plugin) is practice.
        setMode(pendingMode || 'practice');
        pendingMode = null;
        hidePaused();
    });
    sm.on('song:ready', () => { if (mode === 'score') normaliseForScore(); buildMap(); });
    sm.on('song:arrangement-changed', (e) => {
        const d = (e && e.detail) || {};
        if (current && Number.isFinite(d.arrangement)) current.arrangement = d.arrangement;
        setTimeout(buildMap, 300);
    });

    // ── Score mode: no pausing ───────────────────────────────────────────────
    let ended = false;
    let runLive = false;   // score run has started playing
    sm.on('song:play', () => { ended = false; if (mode === 'score') runLive = true; });
    sm.on('song:resume', () => { if (mode === 'score') runLive = true; });
    sm.on('song:ended', () => { ended = true; runLive = false; });
    sm.on('song:stop', () => { runLive = false; hidePaused(); });
    sm.on('song:loading', () => { ended = false; runLive = false; });

    function nearEnd() {
        const a = $('audio');
        return !!(a && a.duration > 0 && a.currentTime >= a.duration - 0.75);
    }
    // Listen on the <audio> element too: upstream's togglePlay pauses it
    // without emitting song:pause, and the OS (phone call, lock screen) can
    // pause it behind our back.
    const audioEl = $('audio');
    if (audioEl) audioEl.addEventListener('pause', onPauseSignal);
    sm.on('song:pause', onPauseSignal);
    function onPauseSignal() {
        if (mode !== 'score' || !runLive) return;
        // The media element fires 'pause' just before 'ended'; give it a beat.
        setTimeout(() => {
            if (ended || mode !== 'score' || !runLive || nearEnd()) return;
            const a = $('audio');
            if (a && !a.paused) return;   // a seek/re-buffer blip, still playing
            runLive = false;
            if (rb.voidRun) rb.voidRun('You paused the song');
            showPaused();
        }, 350);
    }

    function hidePaused() { const p = $('rb-paused'); if (p) p.remove(); }
    function showPaused() {
        hidePaused();
        const box = document.createElement('div');
        box.id = 'rb-paused';
        box.className = 'feedBack-modal fixed inset-0 z-[255] flex items-center justify-center p-4 bg-black/60';
        box.innerHTML = '<div role="dialog" aria-modal="true" class="w-full max-w-md bg-fb-card border border-fb-border rounded-xl shadow-2xl p-6 text-fb-text">' +
            '<h2 class="text-xl font-bold">Score run stopped</h2>' +
            '<p class="mt-2 text-sm text-fb-textDim">Score runs are played straight through, so pausing ends this one. Start again, or carry on from here in practice mode.</p>' +
            '<div class="mt-6 flex flex-col sm:flex-row sm:justify-end gap-2">' +
            '<button data-act="exit" class="px-4 py-2 rounded-md border border-fb-border hover:bg-fb-bg">Leave song</button>' +
            '<button data-act="practice" class="px-4 py-2 rounded-md border border-fb-border hover:bg-fb-bg">Continue as practice</button>' +
            '<button data-act="restart" class="px-4 py-2 rounded-md bg-fb-primary hover:bg-fb-primaryHi text-white font-medium">Restart for score</button></div></div>';
        document.body.appendChild(box);
        box.querySelector('[data-act="restart"]').addEventListener('click', () => { hidePaused(); restartForScore(); });
        box.querySelector('[data-act="practice"]').addEventListener('click', () => {
            hidePaused();
            setMode('practice');
            if (typeof window.togglePlay === 'function') window.togglePlay();
        });
        box.querySelector('[data-act="exit"]').addEventListener('click', () => {
            hidePaused();
            if (typeof window.closeCurrentSong === 'function') window.closeCurrentSong();
            else if (window.showScreen) window.showScreen('v3-songs');
        });
        setTimeout(() => box.querySelector('[data-act="restart"]')?.focus(), 30);
    }

    function restartForScore() {
        if (!current || !current.filename) return;
        const info = (window.highway && window.highway.getSongInfo && window.highway.getSongInfo()) || {};
        const arr = Number.isFinite(info.arrangement_index) ? info.arrangement_index : current.arrangement;
        startSong(current.filename, arr == null ? undefined : arr, undefined, 'score');
    }
    rb.restartForScore = restartForScore;

    // Swallow the transport keys in score mode (Space still starts the song
    // and, once it's running, stops the run via the pause path above).
    const BLOCKED_KEYS = new Set(['ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageUp', 'PageDown']);
    document.addEventListener('keydown', (e) => {
        if (mode !== 'score') return;
        const p = $('player');
        if (!p || !p.classList.contains('active')) return;
        if (e.target && e.target.closest && e.target.closest('input, textarea, select, [contenteditable="true"]')) return;
        if (BLOCKED_KEYS.has(e.key)) { e.preventDefault(); e.stopImmediatePropagation(); }
    }, true);

    // ── Mode pill (player HUD) ───────────────────────────────────────────────
    function renderPill() {
        const hud = $('player-hud');
        if (!hud) return;
        let pill = $('rb-mode-pill');
        if (!pill) {
            pill = document.createElement('div');
            pill.id = 'rb-mode-pill';
            const anchor = $('hud-tuning-targets');
            if (anchor && anchor.parentNode) anchor.parentNode.appendChild(pill);
            else hud.appendChild(pill);
        }
        if (mode === 'score') {
            pill.className = 'rb-mode-pill rb-mode-pill-score';
            pill.innerHTML = '<span class="rb-mode-dot" aria-hidden="true"></span>Score run';
        } else {
            pill.className = 'rb-mode-pill rb-mode-pill-practice';
            pill.innerHTML = '<span class="rb-mode-dot" aria-hidden="true"></span>Practice' +
                '<button type="button" class="rb-mode-switch" title="Restart this song as a score run">Play for score</button>';
            pill.querySelector('button').addEventListener('click', restartForScore);
        }
    }

    // ── Song map ─────────────────────────────────────────────────────────────
    let map = null, mapFill = null, mapHead = null, mapTip = null;
    let blocks = [], blockEls = [], activeIdx = -1, duration = 0, dragging = false;
    let lastFrac = -1, lastNow = '', mapW = 0;
    // Width cached via ResizeObserver: reading clientWidth every tick forced a
    // synchronous layout while the highway was mid-frame.
    function mapWidth() { return mapW || (map ? (mapW = map.clientWidth) : 0); }

    function songDuration() {
        const info = (window.highway && window.highway.getSongInfo && window.highway.getSongInfo()) || {};
        const a = $('audio');
        return Number(info.duration) > 0 ? Number(info.duration) : (a && a.duration > 0 ? a.duration : 0);
    }
    function nowT() {
        const hw = window.highway;
        const t = hw && hw.getTime ? hw.getTime() : NaN;
        if (Number.isFinite(t)) return t;
        const a = $('audio');
        return a ? a.currentTime || 0 : 0;
    }

    function ensureMap() {
        if (map && map.isConnected) return map;
        const p = $('player');
        if (!p) return null;
        map = document.createElement('div');
        map.id = 'rb-song-map';
        map.className = 'rb-song-map rb-on-dark';
        map.setAttribute('role', 'slider');
        map.setAttribute('aria-label', 'Song position');
        map.setAttribute('aria-valuemin', '0');
        map.innerHTML = '<div class="rb-map-blocks"></div><div class="rb-map-fill"></div><div class="rb-map-head"></div><div class="rb-map-tip" hidden></div>';
        mapFill = map.querySelector('.rb-map-fill');
        mapHead = map.querySelector('.rb-map-head');
        mapTip = map.querySelector('.rb-map-tip');
        p.insertBefore(map, p.firstChild);
        p.classList.add('rb-has-map');
        if (typeof ResizeObserver === 'function') {
            new ResizeObserver((es) => {
                for (const e of es) mapW = e.contentRect.width;
                lastFrac = -1;
                tick();
            }).observe(map);
        }
        wireMap();
        applyModeClass();
        return map;
    }

    function buildMap() {
        if (!ensureMap()) return;
        duration = songDuration();
        const secs = (window.highway && window.highway.getSections && window.highway.getSections()) || [];
        blocks = layoutSections(secs, duration);
        if (!blocks.length && duration > 0) blocks = [{ start: 0, end: duration, left: 0, width: 1, label: '', kind: 'other' }];
        const host = map.querySelector('.rb-map-blocks');
        host.textContent = '';
        blockEls = blocks.map((b) => {
            const el = document.createElement('div');
            el.className = 'rb-map-block rb-sec-' + b.kind;
            el.style.left = (b.left * 100) + '%';
            el.style.width = (b.width * 100) + '%';
            el.title = b.label ? b.label + ' (' + fmt(b.start) + ')' : '';
            if (b.label && b.width > 0.04) {
                const s = document.createElement('span');
                s.textContent = b.label;
                el.appendChild(s);
            }
            host.appendChild(el);
            return el;
        });
        map.setAttribute('aria-valuemax', String(Math.round(duration)));
        activeIdx = -1;
        lastFrac = -1;
        tick();
    }

    function tick() {
        if (!map || !duration) return;
        const t = nowT();
        const f = Math.max(0, Math.min(1, t / duration));
        if (!dragging) {
            const q = Math.round(f * 2000) / 2000;   // skip sub-pixel no-op writes
            if (q !== lastFrac) {
                lastFrac = q;
                mapFill.style.transform = 'scaleX(' + q + ')';
                mapHead.style.transform = 'translateX(' + (q * mapWidth()) + 'px)';
            }
        }
        const now = String(Math.round(t));
        if (now !== lastNow) { lastNow = now; map.setAttribute('aria-valuenow', now); }
        let idx = -1;
        for (let i = 0; i < blocks.length; i++) { if (blocks[i].start <= t) idx = i; else break; }
        if (idx !== activeIdx) {
            if (blockEls[activeIdx]) blockEls[activeIdx].classList.remove('rb-map-now');
            if (blockEls[idx]) blockEls[idx].classList.add('rb-map-now');
            activeIdx = idx;
        }
    }
    // 4 Hz is plenty for a progress strip and costs next to nothing; skip the
    // work entirely when the player isn't on screen.
    setInterval(() => {
        const p = $('player');
        if (map && p && p.classList.contains('active') && !document.hidden) tick();
    }, 250);
    window.addEventListener('resize', () => { if (map) tick(); });

    function fracAt(clientX) {
        const r = map.getBoundingClientRect();
        return r.width > 0 ? Math.max(0, Math.min(1, (clientX - r.left) / r.width)) : 0;
    }
    function showTip(f) {
        const t = f * duration;
        let label = '';
        for (const b of blocks) if (b.start <= t) label = b.label;
        mapTip.textContent = (label ? label + ' · ' : '') + fmt(t);
        mapTip.hidden = false;
        mapTip.style.left = (f * 100) + '%';
    }
    function seekTo(f) {
        if (mode !== 'practice' || !duration) return;
        const t = f * duration;
        if (sm.seek) sm.seek(t, 'riffboard-song-map');
        tick();
    }
    function wireMap() {
        map.addEventListener('pointerdown', (e) => {
            if (mode !== 'practice' || !duration) return;
            dragging = true;
            try { map.setPointerCapture(e.pointerId); } catch (_) { /* */ }
            const f = fracAt(e.clientX);
            mapFill.style.transform = 'scaleX(' + f + ')';
            mapHead.style.transform = 'translateX(' + (f * mapWidth()) + 'px)';
            showTip(f);
            e.preventDefault();
        });
        map.addEventListener('pointermove', (e) => {
            if (mode !== 'practice' || !duration) return;
            const f = fracAt(e.clientX);
            if (dragging) {
                mapFill.style.transform = 'scaleX(' + f + ')';
                mapHead.style.transform = 'translateX(' + (f * mapWidth()) + 'px)';
            }
            if (e.pointerType === 'mouse' || dragging) showTip(f);
        });
        const finish = (e, commit) => {
            if (!dragging) { mapTip.hidden = true; return; }
            dragging = false;
            mapTip.hidden = true;
            if (commit) seekTo(fracAt(e.clientX));
        };
        map.addEventListener('pointerup', (e) => finish(e, true));
        map.addEventListener('pointercancel', (e) => finish(e, false));
        map.addEventListener('pointerleave', () => { if (!dragging) mapTip.hidden = true; });
        map.tabIndex = 0;
        map.addEventListener('keydown', (e) => {
            if (mode !== 'practice' || !duration) return;
            const step = e.shiftKey ? 15 : 5;
            let t = null;
            if (e.key === 'ArrowLeft') t = nowT() - step;
            else if (e.key === 'ArrowRight') t = nowT() + step;
            else if (e.key === 'Home') t = 0;
            if (t === null) return;
            e.preventDefault(); e.stopPropagation();
            seekTo(Math.max(0, Math.min(duration, t)) / duration);
        });
    }

    // Lets CSS stop painting the chrome hidden under the full-screen player.
    function syncPlaying() {
        const p = $('player');
        document.documentElement.classList.toggle('rb-playing', !!(p && p.classList.contains('active')));
    }
    sm.on('screen:changed', () => { syncPlaying(); setTimeout(syncPlaying, 0); });
    sm.on('song:loading', () => setTimeout(syncPlaying, 0));

    applyModeClass();
    syncPlaying();
})();
