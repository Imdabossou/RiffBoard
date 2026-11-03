/*
 * RiffBoard — player screen controls that work on a phone.
 *
 *  - Back button (top left): leaves the song, same as Esc.
 *  - Settings cog (bottom left): opens the player tools rail (visuals, audio,
 *    mixer, lyrics, advanced). Upstream revealed that rail only when the mouse
 *    hovered an invisible strip along the left edge, which touch screens can't
 *    do and mouse users never discover.
 *  - Mic toggle (next to the Practice / Score pill): turns note detection on or
 *    off, so practice works without a microphone. Shows "unavailable" when the
 *    browser can't open one (e.g. a plain http:// page).
 */
(function () {
    'use strict';
    const sm = window.feedBack;
    if (!sm || typeof sm.on !== 'function') return;
    const $ = (id) => document.getElementById(id);
    const player = () => $('player');
    const playerActive = () => { const p = player(); return !!(p && p.classList.contains('active')); };

    const ICON_BACK = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/></svg>';
    const ICON_COG = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12,15.5A3.5,3.5 0 0,1 8.5,12A3.5,3.5 0 0,1 12,8.5A3.5,3.5 0 0,1 15.5,12A3.5,3.5 0 0,1 12,15.5M19.43,12.97C19.47,12.65 19.5,12.33 19.5,12C19.5,11.67 19.47,11.34 19.43,11L21.54,9.37C21.73,9.22 21.78,8.95 21.66,8.73L19.66,5.27C19.54,5.05 19.27,4.96 19.05,5.05L16.56,6.05C16.04,5.66 15.5,5.32 14.87,5.07L14.5,2.42C14.46,2.18 14.25,2 14,2H10C9.75,2 9.54,2.18 9.5,2.42L9.13,5.07C8.5,5.32 7.96,5.66 7.44,6.05L4.95,5.05C4.73,4.96 4.46,5.05 4.34,5.27L2.34,8.73C2.21,8.95 2.27,9.22 2.46,9.37L4.57,11C4.53,11.34 4.5,11.67 4.5,12C4.5,12.33 4.53,12.65 4.57,12.97L2.46,14.63C2.27,14.78 2.21,15.05 2.34,15.27L4.34,18.73C4.46,18.95 4.73,19.03 4.95,18.95L7.44,17.94C7.96,18.34 8.5,18.68 9.13,18.93L9.5,21.58C9.54,21.82 9.75,22 10,22H14C14.25,22 14.46,21.82 14.5,21.58L14.87,18.93C15.5,18.67 16.04,18.34 16.56,17.94L19.05,18.95C19.27,19.03 19.54,18.95 19.66,18.73L21.66,15.27C21.78,15.05 21.73,14.78 21.54,14.63L19.43,12.97Z"/></svg>';
    const ICON_MIC = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12,2A3,3 0 0,1 15,5V11A3,3 0 0,1 12,14A3,3 0 0,1 9,11V5A3,3 0 0,1 12,2M19,11C19,14.53 16.39,17.44 13,17.93V21H11V17.93C7.61,17.44 5,14.53 5,11H7A5,5 0 0,0 12,16A5,5 0 0,0 17,11H19Z"/></svg>';

    // ── Back ────────────────────────────────────────────────────────────────
    function leaveSong() {
        if (typeof window.requestExitSong === 'function') window.requestExitSong();
        else if (typeof window.closeCurrentSong === 'function') window.closeCurrentSong();
        else if (window.showScreen) window.showScreen('v3-songs');
    }

    // ── Tools rail ──────────────────────────────────────────────────────────
    function setTools(open) {
        const p = player();
        if (!p) return;
        p.classList.toggle('rb-tools-open', !!open);
        const b = $('rb-tools-btn');
        if (b) b.setAttribute('aria-expanded', open ? 'true' : 'false');
    }
    function toolsOpen() { const p = player(); return !!(p && p.classList.contains('rb-tools-open')); }

    // Short labels under the rail icons (touch screens have no tooltips), and a
    // lyrics icon for the lyrics toggle, which upstream drew as a microphone.
    const RAIL_LABELS = { viz: 'Visuals', audio: 'Input', mixer: 'Mixer', panes: 'Panes', plugins: 'Plugins', advanced: 'More' };
    const ICON_LYRICS = '<svg class="v3-rail-svg" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h16v2H4V5m0 4h10v2H4V9m0 4h16v2H4v-2m0 4h10v2H4v-2z"/></svg>';
    function labelRail() {
        const rail = $('v3-player-rail');
        if (!rail) return;
        rail.querySelectorAll('[data-rail]').forEach((b) => {
            const l = RAIL_LABELS[b.getAttribute('data-rail')];
            if (l && b.dataset.rbLabel !== l) b.dataset.rbLabel = l;
        });
        const lyr = rail.querySelector('[data-rail-action="lyrics"]');
        if (lyr && lyr.dataset.rbLabel !== 'Lyrics') {
            lyr.dataset.rbLabel = 'Lyrics';
            const svg = lyr.querySelector('svg');
            if (svg) svg.outerHTML = ICON_LYRICS;
        }
        const sp = $('section-practice-pill');
        if (sp && sp.dataset.rbLabel !== 'Sections') sp.dataset.rbLabel = 'Sections';
    }

    // ── Mic ─────────────────────────────────────────────────────────────────
    function micStatus() {
        const nd = window.noteDetect;
        if (!nd || typeof nd.getMicStatus !== 'function') return null;
        try { return nd.getMicStatus(); } catch (_) { return null; }
    }
    function renderMic() {
        const b = $('rb-mic-btn');
        if (!b) return;
        const st = micStatus();
        let state = 'off', label = 'Mic off', title = 'Note detection is off. Tap to turn it on.';
        if (!st) { b.hidden = true; return; }
        b.hidden = false;
        if (st.enabled) { state = 'on'; label = 'Mic on'; title = 'Note detection is listening. Tap to play without the mic.'; }
        else if (st.blocked) { state = 'blocked'; label = 'No mic'; title = 'The microphone is unavailable: ' + st.blocked + ' Tap to try again.'; }
        if (b.dataset.state !== state) {
            b.dataset.state = state;
            b.innerHTML = ICON_MIC + '<span>' + label + '</span>';
        }
        if (b.title !== title) { b.title = title; b.setAttribute('aria-label', title); }
        b.setAttribute('aria-pressed', state === 'on' ? 'true' : 'false');
    }
    async function toggleMic() {
        const nd = window.noteDetect;
        if (!nd || typeof nd.toggle !== 'function') return;
        const b = $('rb-mic-btn');
        if (b) b.disabled = true;
        try { await nd.toggle(); } catch (_) { /* surfaced by note_detect */ }
        if (b) b.disabled = false;
        renderMic();
    }

    // ── Mount ───────────────────────────────────────────────────────────────
    function mount() {
        const p = player();
        if (!p || $('rb-back-btn')) return;
        p.classList.add('rb-player-ui');

        const back = document.createElement('button');
        back.id = 'rb-back-btn';
        back.type = 'button';
        back.className = 'rb-round-btn rb-back-btn';
        back.title = 'Back to songs';
        back.setAttribute('aria-label', 'Back to songs');
        back.innerHTML = ICON_BACK;
        back.addEventListener('click', (e) => { e.stopPropagation(); leaveSong(); });
        p.appendChild(back);

        const cog = document.createElement('button');
        cog.id = 'rb-tools-btn';
        cog.type = 'button';
        cog.className = 'rb-round-btn rb-tools-btn';
        cog.title = 'Player settings';
        cog.setAttribute('aria-label', 'Player settings');
        cog.setAttribute('aria-controls', 'v3-player-rail');
        cog.setAttribute('aria-expanded', 'false');
        cog.innerHTML = ICON_COG;
        cog.addEventListener('click', (e) => { e.stopPropagation(); setTools(!toolsOpen()); });
        p.appendChild(cog);

        // Close the rail on a tap/click anywhere outside it (and outside its
        // popovers), and on Esc before Esc means "leave the song".
        document.addEventListener('click', (e) => {
            if (!toolsOpen()) return;
            const t = e.target;
            if (t && t.closest && (t.closest('#v3-railzone') || t.closest('#rb-tools-btn') || t.closest('.feedBack-modal'))) return;
            setTools(false);
        });
        document.addEventListener('keydown', (e) => {
            if (e.key !== 'Escape' || !toolsOpen() || !playerActive()) return;
            if (p.classList.contains('pop-open')) return;   // player-chrome closes the popover first
            e.preventDefault();
            e.stopImmediatePropagation();
            setTools(false);
        }, true);

        mountMic();
        labelRail();
        const rail = $('v3-player-rail');
        // The section-practice control is moved into the rail later; label it then.
        if (rail && typeof MutationObserver === 'function') new MutationObserver(labelRail).observe(rail, { childList: true });
    }

    function mountMic() {
        if ($('rb-mic-btn')) return;
        const pill = $('rb-mode-pill');
        const host = pill ? pill.parentNode : $('player-hud');
        if (!host) return;
        const b = document.createElement('button');
        b.id = 'rb-mic-btn';
        b.type = 'button';
        b.className = 'rb-mic-btn';
        b.hidden = true;
        b.addEventListener('click', (e) => { e.stopPropagation(); toggleMic(); });
        if (pill && pill.nextSibling) host.insertBefore(b, pill.nextSibling);
        else host.appendChild(b);
        renderMic();
    }

    sm.on('screen:changed', () => {
        setTimeout(() => {
            mount();
            if (!playerActive()) setTools(false);
            renderMic();
        }, 0);
    });
    sm.on('song:loading', () => { setTools(false); setTimeout(() => { mount(); mountMic(); renderMic(); }, 0); });
    sm.on('song:ready', renderMic);
    sm.on('notedetect:mic-unavailable', renderMic);
    sm.on('riffboard:mode', () => setTimeout(() => { mountMic(); renderMic(); }, 0));
    // Detection can come up a moment after the song starts (permission prompt,
    // device open); a cheap 1 Hz refresh while the player is open catches it.
    setInterval(() => { if (playerActive() && !document.hidden) renderMic(); }, 1000);

    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount, { once: true });
    else mount();
})();
