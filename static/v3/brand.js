/*
 * RiffBoard brand helpers (from fee[dB]ack v0.3.0).
 *
 * Single source for the wordmark so the sidebar, topbar (prompt 12), modals
 * and onboarding (prompt 15) render it identically. The `[dB]` is a literal
 * bracketed decibel pun, accent-colored (design/01-design-system.md §4).
 * Vanilla JS, no framework (constitution P-II).
 */
(function () {
    'use strict';

    // Inner markup of the wordmark (no wrapper element) so callers can choose
    // the tag/size. `fee` + `ack` use the primary text color; `[dB]` is sky.
    const WORDMARK_INNER = 'Riff<span class="text-fb-primary">Board</span>';

    /**
     * Returns the styled RiffBoard wordmark as an HTML string.
     * @param {Object} [opts]
     * @param {string} [opts.size='text-xl'] Tailwind text-size utility.
     * @param {boolean} [opts.mono=false]    Monochrome (single color, keeps
     *                                        the bracket characters) for small
     *                                        or single-tone contexts.
     * @param {string} [opts.extra='']       Extra classes on the wrapper.
     */
    function wordmarkHTML(opts) {
        opts = opts || {};
        const size = opts.size || 'text-xl';
        const extra = opts.extra || '';
        const inner = opts.mono ? 'RiffBoard' : WORDMARK_INNER;
        return '<span class="font-extrabold tracking-tight text-fb-text ' +
            size + ' ' + extra + '">' + inner + '</span>';
    }

    /** Replaces an element's contents with the wordmark. */
    function renderWordmark(el, opts) {
        if (!el) return;
        el.innerHTML = wordmarkHTML(opts);
    }

    window.fbBrand = {
        wordmarkHTML: wordmarkHTML,
        renderWordmark: renderWordmark,
        ICON_SVG: '/static/v3/brand/riffboard-icon.svg',
        FAVICON_SVG: '/static/v3/brand/favicon.svg',
    };
})();
