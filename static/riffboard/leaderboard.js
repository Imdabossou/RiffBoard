/*
 * RiffBoard — leaderboards, end-of-run name entry, and the About page.
 *
 * Run lifecycle:
 *   song:loading      remember which song is coming
 *   first song:play   ask the server for a run ticket (POST /api/runs)
 *   note:hit/miss     tally the run (events from the note_detect plugin)
 *   seek/loop/speed/difficulty changes mark the run as practice (not ranked)
 *   song:ended        show the result; a ranked run can post a name + score
 *
 * The server re-checks everything it can (see lib/routers/scores.py); the
 * client-side rules here exist so honest players get a clear reason instead of
 * a rejected submission.
 */
(function () {
    'use strict';
    const sm = window.feedBack;
    if (!sm || typeof sm.on !== 'function') return;

    const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => (
        { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
    const enc = encodeURIComponent;
    const NAME_KEY = 'riffboard.name';
    const NAME_RE = /^[\p{L}\p{N} _.'-]{1,20}$/u;

    function decode(fn) { try { return decodeURIComponent(fn); } catch (_) { return fn; } }
    async function jget(u) { try { const r = await fetch(u); return r.ok ? r.json() : null; } catch (_) { return null; } }
    async function jpost(u, body) {
        try {
            const r = await fetch(u, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
            const d = await r.json().catch(() => ({}));
            return { ok: r.ok, status: r.status, data: d };
        } catch (_) { return { ok: false, status: 0, data: { error: 'Network error - check your connection.' } }; }
    }
    function savedName() { try { return localStorage.getItem(NAME_KEY) || ''; } catch (_) { return ''; } }
    function saveName(n) { try { localStorage.setItem(NAME_KEY, n); } catch (_) { /* */ } }
    function pct(acc) { return Math.floor((acc || 0) * 100) + '%'; }
    function scoreOf(h, m) { const a = h / Math.max(1, h + m); return Math.round(h * 100 * a); }

    // ── Run tracking ─────────────────────────────────────────────────────────
    let run = null;
    function newRun(filename) {
        run = {
            filename: decode(filename || ''), arrangement: null, runId: null, ticketPending: false,
            started: false, hits: 0, misses: 0, streak: 0, bestStreak: 0,
            practice: null,  // reason string once the run stops counting
        };
    }
    function practice(reason) { if (run && run.started && !run.practice) run.practice = reason; }
    function songInfo() { return (window.highway && window.highway.getSongInfo && window.highway.getSongInfo()) || {}; }

    async function requestTicket() {
        if (!run || run.ticketPending || run.runId) return;
        const info = songInfo();
        const arr = Number.isFinite(info.arrangement_index) ? info.arrangement_index : 0;
        run.arrangement = arr;
        run.ticketPending = true;
        const mine = run;
        const res = await jpost('/api/runs', { filename: mine.filename, arrangement: arr });
        mine.ticketPending = false;
        if (res.ok && res.data.run_id) mine.runId = res.data.run_id;
        else mine.ticketError = (res.data && res.data.error) || 'Leaderboard unavailable';
    }

    sm.on('song:loading', (e) => newRun(e && e.detail && e.detail.filename));
    sm.on('song:arrangement-changed', () => {
        if (!run) return;
        if (run.started) practice('You switched part mid-song');
        else { run.runId = null; run.ticketError = null; }
    });
    sm.on('song:play', () => {
        if (!run) return;
        if (!run.started) {
            run.started = true;
            requestTicket();
        }
    });
    sm.on('song:seek', (e) => {
        const d = (e && e.detail) || {};
        if (!run || !run.started) return;
        if (Number.isFinite(d.from) && Number.isFinite(d.to) && Math.abs(d.to - d.from) > 1 && d.from > 1) {
            practice('You skipped or rewound during the song');
        }
    });
    sm.on('loop:restart', () => practice('A loop was active'));
    sm.on('song:position-changed', () => {
        if (!run || !run.started || run.practice) return;
        const audio = document.getElementById('audio');
        if (audio && Math.abs((audio.playbackRate || 1) - 1) > 0.001) practice('The song was slowed down or sped up');
        const hw = window.highway;
        if (hw && hw.hasPhraseData && hw.hasPhraseData() && hw.getMastery && hw.getMastery() < 0.999) {
            practice('Difficulty was below 100%');
        }
    });
    sm.on('note:hit', () => {
        if (!run) return;
        run.hits++; run.streak++;
        if (run.streak > run.bestStreak) run.bestStreak = run.streak;
    });
    sm.on('note:miss', () => { if (run) { run.misses++; run.streak = 0; } });
    sm.on('song:ended', () => {
        if (!run || !run.started) return;
        const finished = run;
        run = null;
        showResult(finished);
    });
    sm.on('song:stop', () => { run = null; });

    // ── Modal helpers ────────────────────────────────────────────────────────
    function modal(html) {
        closeModal();
        const wrap = document.createElement('div');
        wrap.id = 'rb-modal';
        wrap.className = 'feedBack-modal fixed inset-0 z-[260] flex items-center justify-center p-4 bg-black/70';
        wrap.innerHTML = '<div role="dialog" aria-modal="true" class="w-full max-w-md max-h-[90vh] overflow-y-auto bg-fb-card border border-fb-border rounded-xl shadow-2xl p-6 text-fb-text fb-selectable">' + html + '</div>';
        wrap.addEventListener('click', (e) => { if (e.target === wrap) closeModal(); });
        document.body.appendChild(wrap);
        const onKey = (e) => { if (e.key === 'Escape') { e.stopPropagation(); closeModal(); } };
        wrap._onKey = onKey;
        document.addEventListener('keydown', onKey, true);
        wrap.querySelectorAll('[data-close]').forEach((b) => b.addEventListener('click', closeModal));
        return wrap;
    }
    function closeModal() {
        const m = document.getElementById('rb-modal');
        if (!m) return;
        if (m._onKey) document.removeEventListener('keydown', m._onKey, true);
        m.remove();
    }

    function boardTable(rows, highlightId) {
        if (!rows || !rows.length) return '<p class="text-sm text-fb-textDim py-4 text-center">No scores yet. Be the first!</p>';
        return '<table class="w-full text-sm"><thead><tr class="text-fb-textDim text-xs uppercase tracking-wider">' +
            '<th class="text-left py-2 w-8">#</th><th class="text-left py-2">Player</th><th class="text-right py-2">Accuracy</th><th class="text-right py-2">Score</th></tr></thead><tbody>' +
            rows.map((r) => '<tr class="border-t border-fb-border/50' + (r.id === highlightId ? ' bg-fb-primary/15' : '') + '">' +
                '<td class="py-2 ' + (r.rank <= 3 ? 'text-fb-gold font-bold' : 'text-fb-textDim') + '">' + r.rank + '</td>' +
                '<td class="py-2 font-medium truncate max-w-[10rem]">' + esc(r.name) + '</td>' +
                '<td class="py-2 text-right text-fb-textDim">' + pct(r.accuracy) + '</td>' +
                '<td class="py-2 text-right font-bold">' + Number(r.score).toLocaleString() + '</td></tr>').join('') +
            '</tbody></table>';
    }

    // ── End of run ───────────────────────────────────────────────────────────
    function showResult(r) {
        const judged = r.hits + r.misses;
        const info = songInfo();
        const title = esc(info.title || r.filename) + (info.artist ? ' <span class="text-fb-textDim font-normal">· ' + esc(info.artist) + '</span>' : '');
        if (!judged) {
            modal('<h2 class="text-xl font-bold">Song finished</h2><p class="mt-1 text-sm">' + title + '</p>' +
                '<p class="mt-4 text-sm text-fb-textDim">No notes were scored. To get on the leaderboard, turn on note detection and pick your input in <b>Settings → Mic</b>, then play again.</p>' +
                '<div class="mt-6 flex justify-end gap-2"><button data-mic class="px-4 py-2 rounded-md border border-fb-border hover:bg-fb-bg">Open mic settings</button>' +
                '<button data-close class="px-4 py-2 rounded-md bg-fb-primary hover:bg-fb-primaryHi text-white">Close</button></div>');
            document.querySelector('#rb-modal [data-mic]')?.addEventListener('click', () => {
                closeModal();
                window.showScreen && window.showScreen('settings');
                setTimeout(() => document.querySelector('#settings-tabbar [data-tab="mic"]')?.click(), 50);
            });
            return;
        }
        const acc = r.hits / judged;
        const reason = r.practice || (!r.runId ? (r.ticketError || 'The leaderboard could not be reached') : null);
        const head = '<h2 class="text-xl font-bold">' + (reason ? 'Practice run complete' : 'Nice playing!') + '</h2>' +
            '<p class="mt-1 text-sm">' + title + '</p>' +
            '<div class="grid grid-cols-3 gap-3 mt-5 text-center">' +
            '<div class="bg-fb-bg/60 rounded-lg p-3"><div class="text-2xl font-bold text-fb-gold">' + scoreOf(r.hits, r.misses).toLocaleString() + '</div><div class="text-xs text-fb-textDim">score</div></div>' +
            '<div class="bg-fb-bg/60 rounded-lg p-3"><div class="text-2xl font-bold">' + pct(acc) + '</div><div class="text-xs text-fb-textDim">accuracy</div></div>' +
            '<div class="bg-fb-bg/60 rounded-lg p-3"><div class="text-2xl font-bold">' + r.bestStreak + '</div><div class="text-xs text-fb-textDim">best streak</div></div></div>';
        if (reason) {
            modal(head + '<p class="mt-4 text-sm text-fb-textDim">This run doesn\'t count for the leaderboard: ' + esc(reason) +
                '. Play the whole song through at full speed and difficulty to post a score.</p>' +
                '<div class="mt-6 flex justify-end"><button data-close class="px-4 py-2 rounded-md bg-fb-primary hover:bg-fb-primaryHi text-white">Close</button></div>');
            return;
        }
        const m = modal(head +
            '<form id="rb-post" class="mt-5"><label for="rb-name" class="text-sm font-medium">Your name for the leaderboard</label>' +
            '<div class="flex gap-2 mt-2"><input id="rb-name" maxlength="20" autocomplete="nickname" required value="' + esc(savedName()) + '" ' +
            'class="flex-1 min-w-0 bg-fb-bg border border-fb-border rounded-md px-3 py-2 text-fb-text focus:border-fb-primary outline-none" placeholder="Name">' +
            '<button type="submit" class="px-4 py-2 rounded-md bg-fb-primary hover:bg-fb-primaryHi text-white font-medium">Post score</button></div>' +
            '<p id="rb-post-msg" class="text-xs text-fb-textDim mt-2">Shown publicly. Letters, numbers, spaces and . \' _ - only.</p></form>' +
            '<div class="mt-6 flex justify-end"><button data-close class="px-4 py-2 rounded-md border border-fb-border hover:bg-fb-bg">Skip</button></div>');
        const input = m.querySelector('#rb-name');
        setTimeout(() => input && input.focus(), 30);
        m.querySelector('#rb-post').addEventListener('submit', async (e) => {
            e.preventDefault();
            const msg = m.querySelector('#rb-post-msg');
            const name = input.value.trim().replace(/\s+/g, ' ');
            if (!NAME_RE.test(name)) { msg.textContent = 'Use 1-20 letters, numbers, spaces or . \' _ -'; msg.className = 'text-xs text-fb-low mt-2'; return; }
            const btn = m.querySelector('#rb-post button[type=submit]');
            btn.disabled = true; btn.textContent = 'Posting…';
            const res = await jpost('/api/scores', { run_id: r.runId, name, hits: r.hits, misses: r.misses, best_streak: r.bestStreak });
            if (!res.ok) {
                btn.disabled = false; btn.textContent = 'Post score';
                msg.textContent = res.data.error || 'Could not post your score.';
                msg.className = 'text-xs text-fb-low mt-2';
                if (res.status === 409 || res.status === 422) btn.disabled = true;
                return;
            }
            saveName(name);
            const d = res.data;
            modal('<h2 class="text-xl font-bold">' + (d.rank === 1 ? 'New top score!' : d.rank ? 'You\'re #' + d.rank + '!' : 'Score posted') + '</h2>' +
                '<p class="mt-1 text-sm">' + title + '</p>' +
                (d.rank ? '' : '<p class="mt-2 text-sm text-fb-textDim">Your personal best on this board is still higher.</p>') +
                '<div class="mt-4">' + boardTable(d.board, d.id) + '</div>' +
                '<div class="mt-6 flex justify-end"><button data-close class="px-4 py-2 rounded-md bg-fb-primary hover:bg-fb-primaryHi text-white">Done</button></div>');
            sm.emit('riffboard:score-posted', { filename: r.filename, arrangement: r.arrangement });
        });
    }

    // ── Single board popup ───────────────────────────────────────────────────
    async function openBoard(filename, arrangement) {
        const [board, meta] = await Promise.all([
            jget('/api/scores/' + enc(filename) + '?arrangement=' + (arrangement | 0) + '&limit=25'),
            jget('/api/song/' + enc(filename)),
        ]);
        const arrs = (meta && meta.arrangements) || [];
        const arrName = (arrs.find((a) => a.index === arrangement) || {}).name || '';
        const m = modal('<div class="flex items-start gap-4">' +
            '<img src="/api/song/' + esc(enc(filename)) + '/art" alt="" class="w-16 h-16 rounded object-cover bg-fb-bg" onerror="this.style.visibility=\'hidden\'">' +
            '<div class="min-w-0"><h2 class="text-xl font-bold truncate">' + esc((meta && meta.title) || filename) + '</h2>' +
            '<p class="text-sm text-fb-textDim truncate">' + esc((meta && meta.artist) || '') + (arrName ? ' · ' + esc(arrName) : '') + '</p></div></div>' +
            '<div class="mt-4">' + boardTable(board && board.scores) + '</div>' +
            '<div class="mt-6 flex justify-end gap-2"><button data-close class="px-4 py-2 rounded-md border border-fb-border hover:bg-fb-bg">Close</button>' +
            '<button data-play class="px-4 py-2 rounded-md bg-fb-primary hover:bg-fb-primaryHi text-white font-medium">Play this part</button></div>');
        m.querySelector('[data-play]').addEventListener('click', () => {
            closeModal();
            window.playSong && window.playSong(enc(filename), arrangement);
        });
    }

    // ── Leaderboards screen ──────────────────────────────────────────────────
    async function renderBoards() {
        const root = document.getElementById('v3-leaderboards');
        if (!root) return;
        const d = await jget('/api/leaderboards');
        const boards = (d && d.boards) || [];
        root.innerHTML = '<div class="max-w-5xl mx-auto px-4 md:px-8 pb-12">' +
            '<p class="text-fb-textDim mb-6">Every song part has its own board. Only full runs at normal speed count, and each name keeps its best score.</p>' +
            (boards.length
                ? '<div class="grid sm:grid-cols-2 gap-4">' + boards.map((b) =>
                    '<button data-board="' + esc(b.filename) + '" data-arr="' + b.arrangement + '" class="flex items-center gap-4 p-4 text-left bg-fb-card/80 hover:bg-fb-card rounded-lg border border-fb-border/50">' +
                    '<img src="' + esc(b.art_url) + '" alt="" loading="lazy" class="w-16 h-16 rounded object-cover bg-fb-bg" onerror="this.style.visibility=\'hidden\'">' +
                    '<div class="min-w-0 flex-1"><div class="font-semibold truncate">' + esc(b.title) + '</div>' +
                    '<div class="text-xs text-fb-textDim truncate">' + esc(b.artist) + (b.arrangement_name ? ' · ' + esc(b.arrangement_name) : '') + '</div>' +
                    '<div class="text-xs text-fb-textDim mt-1">' + b.runs + ' run' + (b.runs === 1 ? '' : 's') + '</div></div>' +
                    '<div class="text-right shrink-0"><div class="text-lg font-bold text-fb-gold">' + Number(b.top_score).toLocaleString() + '</div>' +
                    '<div class="text-xs text-fb-textDim truncate max-w-[7rem]">' + esc(b.top_name) + '</div></div></button>').join('') + '</div>'
                : '<div class="bg-fb-card/80 rounded-xl p-10 border border-fb-border/50 text-center">' +
                  '<div class="text-lg font-semibold">No scores yet</div>' +
                  '<p class="text-fb-textDim text-sm mt-1">Play a song all the way through to put the first name on the board.</p>' +
                  '<button data-go-songs class="mt-5 px-5 py-2 rounded-md bg-fb-primary hover:bg-fb-primaryHi text-white font-medium">Browse songs</button></div>') +
            '</div>';
        root.querySelectorAll('[data-board]').forEach((b) => b.addEventListener('click', () =>
            openBoard(b.getAttribute('data-board'), Number(b.getAttribute('data-arr')))));
        root.querySelector('[data-go-songs]')?.addEventListener('click', () => window.showScreen && window.showScreen('v3-songs'));
    }

    // ── About screen ─────────────────────────────────────────────────────────
    async function renderAbout() {
        const root = document.getElementById('v3-about');
        if (!root || root.dataset.rendered) return;
        const ver = await jget('/api/version');
        if (window.riffboardSite && !window.riffboardSite.loaded) await new Promise((r) => setTimeout(r, 300));
        const src = (ver && ver.source_url) || 'https://github.com/Imdabossou/RiffBoard';
        root.dataset.rendered = '1';
        const card = (h, body) => '<div class="bg-fb-card/80 rounded-xl p-6 border border-fb-border/50"><h2 class="text-lg font-bold mb-2">' + h + '</h2><div class="text-sm text-fb-textDim space-y-2">' + body + '</div></div>';
        const site = (window.riffboardSite && window.riffboardSite.data) || {};
        const siteCard = (site.about || site.links)
            ? card(esc(site.about_title || ('About ' + (site.name || 'the band'))),
                (site.about || []).map((p) => '<p>' + esc(p) + '</p>').join('') +
                (site.links ? '<p class="flex flex-wrap gap-x-4 gap-y-1">' + site.links.map((l) =>
                    '<a class="text-fb-primary hover:text-fb-primaryHi underline" href="' + esc(l.url) + '" target="_blank" rel="noopener noreferrer">' + esc(l.label) + '</a>').join('') + '</p>' : ''))
            : '';
        const siteName = site.name ? esc(site.name) + ' runs on RiffBoard: a' : 'A';
        root.innerHTML = '<div class="max-w-3xl mx-auto px-4 md:px-8 pb-12 space-y-4 fb-selectable">' + siteCard +
            card(site.name ? 'How it works' : 'What is RiffBoard?', '<p>' + siteName + ' place to play along to original songs by independent artists. Every song is charted by hand for guitar and bass. Plug in (or play near your microphone) and the notes you hit are scored as you play.</p>') +
            card('How scoring works', '<p>Note detection runs entirely in your browser: your audio never leaves your device. When a song ends, only the totals (notes hit, notes missed, best streak) are sent if you choose to post a score.</p>' +
                '<p>A run counts for the leaderboard when you play the whole song at normal speed and full difficulty, without skipping, rewinding or looping. Practise however you like; those runs just stay off the board.</p>') +
            card('Privacy', '<p>There are no accounts. Your settings, favourites and practice history are stored in this browser only. Posting a score stores the name you type, your score and the time. To limit abuse, the server also keeps a salted, one-way hash of your IP address with each score; it is never shown or shared.</p>') +
            card('Open source', '<p>RiffBoard is free software under the GNU AGPL v3.0. It is built on <a class="text-fb-primary hover:text-fb-primaryHi underline" href="https://github.com/got-feedback/feedBack" target="_blank" rel="noopener">fee[dB]ack</a> and its note-detection plugin, by their contributors.</p>' +
                '<p><a class="text-fb-primary hover:text-fb-primaryHi underline" href="' + esc(src) + '" target="_blank" rel="noopener">Source code</a>' +
                (ver && ver.version ? ' · version ' + esc(ver.version) : '') + '</p>') +
            '</div>';
    }

    sm.on('screen:changed', (e) => {
        const id = e && e.detail && e.detail.id;
        if (id === 'v3-leaderboards') renderBoards();
        else if (id === 'v3-about') renderAbout();
    });
    sm.on('riffboard:score-posted', () => {
        const active = document.querySelector('.screen.active');
        if (active && active.id === 'v3-leaderboards') renderBoards();
    });

    window.riffboardBoards = { open: openBoard, render: renderBoards };
})();
