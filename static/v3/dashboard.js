/*
 * RiffBoard — Home (#v3-home).
 *
 * Hero + "continue / pick a song" card, a three-step "how it works" row, the
 * newest songs, the current leaderboard toppers, and the visitor's own recent
 * plays (kept in this browser by static/riffboard/local-api.js). Every widget
 * fetches independently and degrades to a placeholder.
 */
(function () {
    'use strict';
    const sm = window.feedBack;
    const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => (
        { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
    const enc = encodeURIComponent;
    const jget = async (u) => { try { const r = await fetch(u); return r.ok ? r.json() : null; } catch (e) { return null; } };
    const go = (id) => window.showScreen && window.showScreen(id);

    function artUrl(song) {
        const v = song.mtime ? ('?v=' + Math.floor(song.mtime)) : '';
        return '/api/song/' + enc(song.filename) + '/art' + v;
    }
    function art(url, cls) {
        return '<img src="' + esc(url) + '" alt="" loading="lazy" class="' + (cls || '') +
            '" onerror="this.style.visibility=\'hidden\'">';
    }

    async function randomSong() {
        const stats = await jget('/api/library/stats');
        const total = (stats && (stats.total_songs ?? stats.total)) || 0;
        if (!total) return null;
        const data = await jget('/api/library?size=1&page=' + Math.floor(Math.random() * total));
        return data && data.songs && data.songs[0] ? data.songs[0] : null;
    }

    function songTile(song, attrs) {
        return '<button ' + attrs + ' class="group text-left">' +
            '<div class="relative aspect-square rounded-lg overflow-hidden bg-fb-card border border-fb-border/50">' +
            art(song.art_url || artUrl(song), 'w-full h-full object-cover group-hover:scale-105 transition-transform') +
            '</div>' +
            '<div class="mt-2 text-sm font-semibold text-fb-text truncate">' + esc(song.title || song.filename) + '</div>' +
            '<div class="text-xs text-fb-textDim truncate">' + esc(song.artist || '') + '</div></button>';
    }

    function step(n, title, body) {
        return '<div class="bg-fb-card/80 rounded-lg p-5 border border-fb-border/50">' +
            '<div class="w-8 h-8 rounded-full bg-fb-primary/20 text-fb-primary font-bold flex items-center justify-center">' + n + '</div>' +
            '<div class="mt-3 font-semibold text-fb-text">' + title + '</div>' +
            '<div class="mt-1 text-sm text-fb-textDim">' + body + '</div></div>';
    }

    let renderSeq = 0;
    async function render() {
        const root = document.getElementById('v3-home');
        if (!root) return;
        const seq = ++renderSeq;
        const [cont, latest, boards, recent, stats] = await Promise.all([
            jget('/api/session/continue'),
            jget('/api/library?size=8&sort=recent'),
            jget('/api/leaderboards'),
            jget('/api/stats/recent?limit=6'),
            jget('/api/library/stats'),
        ]);
        const pick = (cont && cont.filename) ? null : await randomSong();
        if (seq !== renderSeq) return;
        const songCount = (stats && (stats.total_songs ?? stats.total)) || 0;

        let sideCard;
        if (cont && cont.filename) {
            sideCard = '<button id="rb-continue" class="rb-on-dark group relative text-left rounded-xl overflow-hidden border border-fb-border/50 bg-fb-card aspect-square self-start flex flex-col justify-end">' +
                art(cont.art_url, 'absolute inset-0 w-full h-full object-cover opacity-70 group-hover:opacity-90 transition-opacity') +
                '<div class="relative p-5 bg-gradient-to-t from-black/90 to-transparent">' +
                '<div class="text-xs uppercase tracking-wider text-fb-primaryHi font-semibold">Continue playing</div>' +
                '<div class="text-xl font-bold text-white mt-1">' + esc(cont.title) + '</div>' +
                '<div class="text-sm text-gray-300">' + esc(cont.artist) + '</div></div></button>';
        } else if (pick) {
            sideCard = '<button id="rb-pick" class="rb-on-dark group relative text-left rounded-xl overflow-hidden border border-fb-border/50 bg-fb-card aspect-square self-start flex flex-col justify-end">' +
                art(artUrl(pick), 'absolute inset-0 w-full h-full object-cover opacity-70 group-hover:opacity-90 transition-opacity') +
                '<div class="relative p-5 bg-gradient-to-t from-black/90 to-transparent">' +
                '<div class="text-xs uppercase tracking-wider text-fb-primaryHi font-semibold">Try this one</div>' +
                '<div class="text-xl font-bold text-white mt-1">' + esc(pick.title) + '</div>' +
                '<div class="text-sm text-gray-300">' + esc(pick.artist) + '</div></div></button>';
        } else {
            sideCard = '<div class="rounded-xl border border-fb-border/50 bg-fb-card/60 aspect-square self-start flex flex-col items-center justify-center text-center p-6">' +
                '<div class="text-fb-textDim text-sm">No songs yet. Check back soon!</div></div>';
        }

        const latestSongs = (latest && latest.songs) || [];
        const latestSection = latestSongs.length
            ? '<h2 class="text-xl font-bold text-fb-text mt-10 mb-4">Newest songs</h2>' +
              '<div class="grid grid-cols-2 sm:grid-cols-4 gap-4">' +
              latestSongs.map((s) => songTile(s, 'data-play="' + esc(s.filename) + '"')).join('') + '</div>'
            : '';

        const topBoards = ((boards && boards.boards) || []).slice().sort((a, b) => b.runs - a.runs).slice(0, 5);
        const boardSection = topBoards.length
            ? '<div class="flex items-center justify-between mt-10 mb-4"><h2 class="text-xl font-bold text-fb-text">Top of the boards</h2>' +
              '<button id="rb-all-boards" class="text-sm text-fb-primary hover:text-fb-primaryHi">All leaderboards</button></div>' +
              '<div class="bg-fb-card/80 rounded-lg border border-fb-border/50 divide-y divide-fb-border/50">' +
              topBoards.map((b) =>
                  '<button data-board="' + esc(b.filename) + '" data-arr="' + b.arrangement + '" class="w-full flex items-center gap-4 p-3 text-left hover:bg-fb-card">' +
                  art(b.art_url, 'w-12 h-12 rounded object-cover bg-fb-cardMuted') +
                  '<div class="min-w-0 flex-1"><div class="font-semibold text-fb-text truncate">' + esc(b.title) +
                  (b.arrangement_name ? ' <span class="text-xs text-fb-textDim font-normal">· ' + esc(b.arrangement_name) + '</span>' : '') + '</div>' +
                  '<div class="text-xs text-fb-textDim truncate">' + esc(b.artist) + '</div></div>' +
                  '<div class="text-right shrink-0"><div class="text-fb-gold font-bold">' + Number(b.top_score).toLocaleString() + '</div>' +
                  '<div class="text-xs text-fb-textDim">' + esc(b.top_name) + '</div></div></button>').join('') +
              '</div>'
            : '';

        const recentRows = Array.isArray(recent) ? recent : [];
        const recentSection = recentRows.length
            ? '<h2 class="text-xl font-bold text-fb-text mt-10 mb-4">Jump back in</h2>' +
              '<div class="grid grid-cols-3 sm:grid-cols-6 gap-4">' +
              recentRows.map((r) => songTile(r, 'data-play="' + esc(r.filename) + '" data-arr="' + (r.arrangement ?? '') + '"')).join('') + '</div>'
            : '';

        root.innerHTML =
            '<div class="max-w-7xl mx-auto px-4 md:px-8 pb-12">' +
            '<div class="grid lg:grid-cols-3 gap-6">' +
            '<div class="rb-on-dark lg:col-span-2 relative rounded-xl overflow-hidden min-h-[360px] md:min-h-[440px] flex items-center bg-fb-bg">' +
            '<img src="/static/v3/brand/hero.png" alt="" aria-hidden="true" class="absolute inset-y-0 right-0 h-full w-2/3 object-cover object-right" onerror="this.style.display=\'none\'">' +
            '<div class="absolute inset-0 rb-hero-fade"></div>' +
            '<div class="relative p-6 md:p-8 max-w-md">' +
            '<h3 class="text-3xl md:text-4xl font-bold leading-tight text-white">Play along. Top the board.</h3>' +
            '<p class="text-gray-300 mt-3">Original songs from independent artists, charted for guitar and bass. Plug in or use your mic, play the whole song, and post your score.</p>' +
            '<div class="flex flex-wrap gap-3 mt-6">' +
            '<button id="rb-start" class="bg-fb-primary hover:bg-fb-primaryHi text-white px-6 py-2.5 rounded-md font-medium shadow-lg shadow-fb-primary/20">Start playing</button>' +
            '<button id="rb-boards" class="border border-gray-400 hover:border-white text-white px-6 py-2.5 rounded-md font-medium">Leaderboards</button>' +
            '</div>' +
            (songCount ? '<p class="text-xs text-gray-400 mt-4">' + songCount + ' song' + (songCount === 1 ? '' : 's') + ' to play</p>' : '') +
            '</div></div>' +
            sideCard +
            '</div>' +
            '<div class="grid md:grid-cols-3 gap-4 mt-6">' +
            step(1, 'Pick a song', 'Browse the library and choose a part: lead, rhythm or bass.') +
            step(2, 'Plug in', 'Use an audio interface or your mic. <button id="rb-setup" class="text-fb-primary hover:text-fb-primaryHi">Set up your input</button> once and it\'s remembered on this device.') +
            step(3, 'Post your score', 'Play the whole song at full speed and enter your name to land on the leaderboard.') +
            '</div>' +
            recentSection + latestSection + boardSection +
            '</div>';

        root.querySelector('#rb-start')?.addEventListener('click', () => {
            if (cont && cont.filename) resume(cont);
            else go('v3-songs');
        });
        root.querySelector('#rb-boards')?.addEventListener('click', () => go('v3-leaderboards'));
        root.querySelector('#rb-all-boards')?.addEventListener('click', () => go('v3-leaderboards'));
        root.querySelector('#rb-setup')?.addEventListener('click', () => {
            go('settings');
            setTimeout(() => document.querySelector('#settings-tabbar [data-tab="mic"]')?.click(), 50);
        });
        root.querySelector('#rb-continue')?.addEventListener('click', () => resume(cont));
        root.querySelector('#rb-pick')?.addEventListener('click', () => window.playSong && window.playSong(enc(pick.filename)));
        root.querySelectorAll('[data-play]').forEach((b) => b.addEventListener('click', () => {
            if (!window.playSong) return;
            const arr = b.getAttribute('data-arr');
            window.playSong(enc(b.getAttribute('data-play')), arr === '' || arr == null ? undefined : Number(arr));
        }));
        root.querySelectorAll('[data-board]').forEach((b) => b.addEventListener('click', () => {
            if (window.riffboardBoards) window.riffboardBoards.open(b.getAttribute('data-board'), Number(b.getAttribute('data-arr')));
        }));
        if (sm && typeof sm.emit === 'function') sm.emit('v3:dashboard-rendered', {});
    }

    function resume(cont) {
        if (!window.playSong) return;
        const opts = cont.last_position > 0 ? { resume: { position: cont.last_position } } : undefined;
        window.playSong(enc(cont.filename), cont.arrangement, opts);
    }

    window.v3Dashboard = { render: render };
    if (sm && typeof sm.on === 'function') {
        sm.on('screen:changed', (e) => { if (e && e.detail && e.detail.id === 'v3-home') render(); });
    }
    function boot() { render(); }
    if (document.readyState !== 'complete') document.addEventListener('DOMContentLoaded', boot, { once: true });
    else boot();
})();
