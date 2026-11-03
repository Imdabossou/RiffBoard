/*
 * RiffBoard — per-visitor state, kept in this browser.
 *
 * The upstream frontend (fee[dB]ack) stores a player's settings, favourites,
 * loops, practice stats and "continue playing" on the server, in one shared
 * database. On a public site that would mean every visitor shares (and can
 * rewrite) one profile. Rather than rewrite the whole frontend, this script
 * wraps window.fetch and answers those per-user endpoints from localStorage.
 * Everything else goes to the server untouched.
 *
 * Settings are the one exception that still talks to the server: the server
 * owns the validation and instrument-profile rules, so GET/POST /api/settings
 * become a stateless GET /api/settings?state=…&patch=… whose result
 * (`_state`) is what we keep here. The server stores nothing.
 *
 * Must load before every other script (see static/v3/index.html).
 */
(function () {
    'use strict';
    if (window.riffboardLocal) return;
    // Marker other modules (and the bundled plugins) check to know they're
    // running in the public RiffBoard build.
    window.riffboard = window.riffboard || {};

    const realFetch = window.fetch.bind(window);
    const NS = 'rb:';

    // ── Storage helpers (never throw: private mode / blocked storage) ────────
    const mem = {};
    function load(key, fallback) {
        try {
            const raw = localStorage.getItem(NS + key);
            if (raw == null) return key in mem ? mem[key] : fallback;
            return JSON.parse(raw);
        } catch (_) {
            return key in mem ? mem[key] : fallback;
        }
    }
    function save(key, value) {
        mem[key] = value;
        try { localStorage.setItem(NS + key, JSON.stringify(value)); } catch (_) { /* quota/private */ }
    }

    function json(body, status) {
        return new Response(JSON.stringify(body), {
            status: status || 200,
            headers: { 'Content-Type': 'application/json' },
        });
    }

    async function readBody(input, init) {
        try {
            if (init && init.body != null) {
                return typeof init.body === 'string' ? JSON.parse(init.body) : null;
            }
            if (input instanceof Request) {
                const t = await input.clone().text();
                return t ? JSON.parse(t) : null;
            }
        } catch (_) { /* not JSON */ }
        return null;
    }

    function decodePath(p) {
        try { return decodeURIComponent(p); } catch (_) { return p; }
    }

    // ── Song metadata cache (for "recent" / "continue" cards) ────────────────
    const songMeta = new Map();
    async function metaFor(filename) {
        if (songMeta.has(filename)) return songMeta.get(filename);
        let meta = null;
        try {
            const r = await realFetch('/api/song/' + encodeURIComponent(filename));
            if (r.ok) meta = await r.json();
        } catch (_) { /* offline */ }
        songMeta.set(filename, meta);
        return meta;
    }
    function artUrl(filename) { return '/api/song/' + encodeURIComponent(filename) + '/art'; }

    // ── Settings ─────────────────────────────────────────────────────────────
    async function settingsCall(patch) {
        const state = load('settings', {});
        const qs = new URLSearchParams({ state: JSON.stringify(state) });
        if (patch) qs.set('patch', JSON.stringify(patch));
        const r = await realFetch('/api/settings?' + qs.toString());
        const data = await r.json().catch(() => ({}));
        if (!r.ok) return { error: data.error || 'Could not save that setting' };
        if (data._state) save('settings', data._state);
        delete data._state;
        return data;
    }

    // ── Practice stats (personal, this device) ───────────────────────────────
    function statsRows() { return load('stats', {}); }
    function statKey(fn, arr) { return fn + '\u0000' + (arr | 0); }
    function recordStats(body) {
        if (!body || !body.filename) return { error: 'filename required' };
        const fn = decodePath(String(body.filename));
        const arr = Number.isFinite(Number(body.arrangement)) ? Number(body.arrangement) | 0 : 0;
        const rows = statsRows();
        const k = statKey(fn, arr);
        const now = Date.now() / 1000;
        const row = rows[k] || {
            filename: fn, arrangement: arr, plays: 0, best_score: 0, best_accuracy: 0,
            last_score: null, last_accuracy: null, last_position: null, seconds_total: 0,
            last_played_at: null, updated_at: null,
        };
        const scored = typeof body.score === 'number' && typeof body.accuracy === 'number';
        if (scored) {
            row.plays += 1;
            row.best_score = Math.max(row.best_score, body.score | 0);
            row.best_accuracy = Math.max(row.best_accuracy, Math.max(0, Math.min(1, body.accuracy)));
            row.last_score = body.score | 0;
            row.last_accuracy = body.accuracy;
            row.last_played_at = now;
        }
        const pos = body.lastPlayPosition != null ? body.lastPlayPosition : body.last_position;
        if (Number.isFinite(pos) && pos >= 0) {
            row.last_position = pos;
            row.last_played_at = now;
        }
        if (Number.isFinite(body.seconds) && body.seconds > 0) {
            row.seconds_total += Math.min(body.seconds, 7200);
        }
        row.updated_at = now;
        rows[k] = row;
        save('stats', rows);
        return { stats: row, progress: null };
    }
    function songStats(fn) {
        const arr = Object.values(statsRows()).filter((r) => r.filename === fn);
        return {
            filename: fn,
            best_accuracy: arr.reduce((m, a) => Math.max(m, a.best_accuracy || 0), 0),
            best_score: arr.reduce((m, a) => Math.max(m, a.best_score || 0), 0),
            plays: arr.reduce((m, a) => m + (a.plays || 0), 0),
            arrangements: arr,
        };
    }
    async function withSongMeta(rows) {
        const out = [];
        for (const r of rows) {
            const m = await metaFor(r.filename);
            if (!m || m.error) continue;  // song no longer in the library
            out.push({
                ...r, title: m.title || r.filename, artist: m.artist || '',
                tuning_name: m.tuning_name || '', duration: m.duration || 0,
                art_url: artUrl(r.filename),
            });
        }
        return out;
    }

    // ── Favourites ───────────────────────────────────────────────────────────
    function favSet() { return new Set(load('favorites', [])); }
    function markFavorites(payload) {
        const favs = favSet();
        const mark = (s) => { if (s && typeof s === 'object' && 'filename' in s) s.favorite = favs.has(s.filename); };
        if (payload && Array.isArray(payload.songs)) payload.songs.forEach(mark);
        return payload;
    }
    async function favoriteLibrary(url) {
        // The server's own favourites table is unused; filter locally instead.
        const want = new URLSearchParams(url.search);
        const page = Math.max(0, parseInt(want.get('page') || '0', 10) || 0);
        const size = Math.max(1, Math.min(100, parseInt(want.get('size') || '24', 10) || 24));
        want.delete('favorites'); want.delete('after');
        want.set('size', '100');
        const favs = favSet();
        const all = [];
        for (let p = 0; p < 50; p++) {
            want.set('page', String(p));
            const r = await realFetch('/api/library?' + want.toString());
            if (!r.ok) return r;
            const d = await r.json();
            (d.songs || []).forEach((s) => { if (favs.has(s.filename)) all.push(s); });
            if (!d.songs || d.songs.length < 100 || (p + 1) * 100 >= (d.total || 0)) break;
        }
        all.forEach((s) => { s.favorite = true; });
        return json({ songs: all.slice(page * size, page * size + size), total: all.length,
                      page, size, next_cursor: null });
    }

    // ── Loops ────────────────────────────────────────────────────────────────
    function loopsAll() { return load('loops', []); }

    // ── Tuner plugin config (was tuner.json on the server) ───────────────────
    const TUNER_DEFAULTS = {
        lastTuning: 'Standard', lastInstrument: 'guitar-6', freeTune: false,
        customTunings: {}, visualizationMode: 'default', audioInputMode: 'auto',
        autoOpenOnTuningChange: false,
    };

    // ── Per-song personal notes/tags/difficulty ──────────────────────────────
    function userMeta() { return load('user-meta', {}); }
    function cleanUserMeta(b) {
        const out = {};
        if (b && 'user_difficulty' in b) {
            const d = b.user_difficulty;
            out.user_difficulty = (d == null || d === '') ? null : Math.max(1, Math.min(5, d | 0));
        }
        if (b && typeof b.notes === 'string') out.notes = b.notes.slice(0, 2000);
        if (b && Array.isArray(b.tags)) out.tags = b.tags.filter((t) => typeof t === 'string').map((t) => t.slice(0, 40)).slice(0, 20);
        return out;
    }

    // ── Routes ───────────────────────────────────────────────────────────────
    // [method, regex on pathname, handler(match, url, body)]
    const NOT_HERE = { error: 'Not available on RiffBoard' };
    const routes = [
        ['GET', /^\/api\/settings$/, () => settingsCall(null).then((d) => json(d))],
        ['POST', /^\/api\/settings$/, async (_m, _u, body) => {
            const d = await settingsCall(body || {});
            return json(d.error ? d : { message: 'Settings saved' });
        }],
        ['POST', /^\/api\/settings\/reset$/, (_m, _u, body) => {
            const keys = (body && Array.isArray(body.keys)) ? body.keys : [];
            const st = load('settings', {});
            keys.forEach((k) => { delete st[k]; });
            save('settings', st);
            return json({ message: 'Settings reset', reset: keys });
        }],

        ['GET', /^\/api\/plugins\/tuner\/config$/, () => json({ ...TUNER_DEFAULTS, ...load('tuner', {}) })],
        ['POST', /^\/api\/plugins\/tuner\/config$/, (_m, _u, body) => {
            if (body && typeof body === 'object') {
                const cur = load('tuner', {});
                for (const k of Object.keys(TUNER_DEFAULTS)) if (k in body) cur[k] = body[k];
                save('tuner', cur);
            }
            return json({ ok: true });
        }],

        ['GET', /^\/api\/loops$/, (_m, url) => {
            const fn = decodePath(url.searchParams.get('filename') || '');
            return json(loopsAll().filter((l) => l.filename === fn)
                .map(({ id, name, start, end }) => ({ id, name, start, end })));
        }],
        ['POST', /^\/api\/loops$/, (_m, _u, body) => {
            if (!body || !body.filename || !Number.isFinite(body.start) || !Number.isFinite(body.end)) {
                return json({ error: 'Missing fields' });
            }
            const all = loopsAll();
            const id = all.reduce((m, l) => Math.max(m, l.id), 0) + 1;
            const name = String(body.name || 'Loop ' + id).slice(0, 80);
            all.push({ id, filename: decodePath(String(body.filename)), name, start: body.start, end: body.end });
            save('loops', all);
            return json({ ok: true, name });
        }],
        ['DELETE', /^\/api\/loops\/(\d+)$/, (m) => {
            save('loops', loopsAll().filter((l) => l.id !== Number(m[1])));
            return json({ ok: true });
        }],

        ['POST', /^\/api\/favorites\/toggle$/, (_m, _u, body) => {
            const fn = body && body.filename ? decodePath(String(body.filename)) : '';
            if (!fn) return json({ error: 'No filename' });
            const favs = favSet();
            const on = !favs.has(fn);
            if (on) favs.add(fn); else favs.delete(fn);
            save('favorites', [...favs]);
            return json({ favorite: on });
        }],
        ['POST', /^\/api\/saved\/toggle$/, () => json({ saved: false })],

        ['POST', /^\/api\/stats$/, (_m, _u, body) => json(recordStats(body))],
        ['GET', /^\/api\/stats\/best$/, () => {
            const out = {};
            Object.values(statsRows()).forEach((r) => {
                if (r.plays > 0) out[r.filename] = Math.max(out[r.filename] || 0, r.best_accuracy || 0);
            });
            return json(out);
        }],
        ['GET', /^\/api\/stats\/recent$/, async (_m, url) => {
            const limit = Math.max(1, Math.min(100, parseInt(url.searchParams.get('limit') || '12', 10) || 12));
            const rows = Object.values(statsRows()).filter((r) => r.last_played_at)
                .sort((a, b) => b.last_played_at - a.last_played_at).slice(0, limit);
            return json(await withSongMeta(rows));
        }],
        ['GET', /^\/api\/stats\/top$/, async (_m, url) => {
            const limit = Math.max(1, Math.min(50, parseInt(url.searchParams.get('limit') || '5', 10) || 5));
            const by = {};
            Object.values(statsRows()).forEach((r) => {
                if (!r.plays) return;
                const b = by[r.filename] || (by[r.filename] = { filename: r.filename, best_score: 0, best_accuracy: 0, plays: 0 });
                b.best_score = Math.max(b.best_score, r.best_score);
                b.best_accuracy = Math.max(b.best_accuracy, r.best_accuracy);
                b.plays += r.plays;
            });
            const rows = Object.values(by).sort((a, b) => b.best_score - a.best_score).slice(0, limit);
            return json(await withSongMeta(rows));
        }],
        ['GET', /^\/api\/stats\/(.+)$/, (m) => json(songStats(decodePath(m[1])))],
        ['GET', /^\/api\/session\/continue$/, async () => {
            const row = Object.values(statsRows()).filter((r) => r.last_position > 0 && r.last_played_at)
                .sort((a, b) => b.last_played_at - a.last_played_at)[0];
            if (!row) return json(null);
            const [full] = await withSongMeta([row]);
            return json(full || null);
        }],

        ['GET', /^\/api\/song\/(.+)\/user-meta$/, (m) => {
            const fn = decodePath(m[1]);
            return json({ filename: fn, user_difficulty: null, notes: '', tags: [], ...(userMeta()[fn] || {}) });
        }],
        ['PUT', /^\/api\/song\/(.+)\/user-meta$/, (m, _u, body) => {
            const fn = decodePath(m[1]);
            const all = userMeta();
            all[fn] = { ...(all[fn] || {}), ...cleanUserMeta(body) };
            save('user-meta', all);
            return json({ ok: true, ...all[fn] });
        }],
        ['POST', /^\/api\/songs\/user-meta\/batch$/, (_m, _u, body) => {
            const all = userMeta();
            const files = (body && Array.isArray(body.filenames)) ? body.filenames.slice(0, 1000) : [];
            const add = cleanUserMeta({ tags: (body && body.add_tags) || [] }).tags;
            const remove = new Set(cleanUserMeta({ tags: (body && body.remove_tags) || [] }).tags);
            files.forEach((f) => {
                const fn = decodePath(String(f));
                const cur = { ...(all[fn] || {}) };
                if (body && 'set_difficulty' in body) cur.user_difficulty = cleanUserMeta({ user_difficulty: body.set_difficulty }).user_difficulty;
                const tags = new Set(cur.tags || []);
                add.forEach((t) => tags.add(t));
                remove.forEach((t) => tags.delete(t));
                cur.tags = [...tags].slice(0, 20);
                all[fn] = cur;
            });
            save('user-meta', all);
            return json({ ok: true, updated: files.length });
        }],
        ['GET', /^\/api\/tags$/, () => {
            const count = {};
            Object.values(userMeta()).forEach((m) => (m.tags || []).forEach((t) => { count[t] = (count[t] || 0) + 1; }));
            return json({ tags: Object.entries(count).sort((a, b) => b[1] - a[1]).map(([tag, n]) => ({ tag, count: n })) });
        }],

        // Upstream features that don't exist here: answer instantly with an
        // empty/neutral value so the UI degrades instead of erroring.
        ['GET', /^\/api\/(playlists|collections|wanted)$/, () => json([])],
        ['GET', /^\/api\/playlists\/.*$/, () => json(NOT_HERE, 404)],
        ['GET', /^\/api\/(profile|progression|profile\/progress|shop)$/, () => json({})],
        ['GET', /^\/api\/enrichment\/status$/, () => json({ running: false, total: 0, matched: 0, enabled: false })],
        ['GET', /^\/api\/library\/practice-suggestions$/, () => json([])],
        ['GET', /^\/api\/plugins\/updates$/, () => json({})],
        ['GET', /^\/api\/audio-effects\/.*$/, () => json([])],
        ['GET', /^\/api\/plugins\/minigames\/.*$/, () => json({})],
        ['GET', /^\/api\/plugins\/note_detect\/config$/, () => json({})],
    ];

    async function shimFetch(input, init) {
        let url;
        try {
            url = new URL(typeof input === 'string' ? input : (input && input.url) || String(input), location.href);
        } catch (_) {
            return realFetch(input, init);
        }
        if (url.origin !== location.origin || !url.pathname.startsWith('/api/')) {
            return realFetch(input, init);
        }
        const method = ((init && init.method) || (input instanceof Request ? input.method : 'GET') || 'GET').toUpperCase();
        for (const [m, re, handler] of routes) {
            if (m !== method) continue;
            const match = url.pathname.match(re);
            if (!match) continue;
            const body = (method === 'GET' || method === 'DELETE') ? null : await readBody(input, init);
            try {
                return await handler(match, url, body);
            } catch (e) {
                return json({ error: 'local storage error' }, 500);
            }
        }
        if (method === 'GET' && url.pathname === '/api/library') {
            if (url.searchParams.get('favorites') === '1') return favoriteLibrary(url);
            const r = await realFetch(input, init);
            if (!r.ok) return r;
            return json(markFavorites(await r.json()));
        }
        return realFetch(input, init);
    }

    window.fetch = shimFetch;
    window.riffboardLocal = {
        resetAll() {
            if (!window.confirm('Reset all your RiffBoard settings, favourites and practice stats on this device?')) return;
            try { localStorage.clear(); } catch (_) { /* */ }
            try { sessionStorage.clear(); } catch (_) { /* */ }
            location.reload();
        },
        _realFetch: realFetch,
    };
})();
