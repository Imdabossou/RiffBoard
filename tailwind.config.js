/**
 * Tailwind CSS build config for RiffBoard (from feedBack core).
 *
 * Palette colours are CSS variables (static/riffboard/theme.css) so the light
 * and dark themes swap at runtime without a second stylesheet.
 *
 * Replaces the Play CDN (`cdn.tailwindcss.com`) JIT runtime that
 * previously scanned the DOM ~1.8x/sec on the main thread, causing
 * sustained frame drops with the 3D highway. See feedBack-desktop#110.
 *
 * Regenerate `static/tailwind.min.css` with:
 *   bash scripts/build-tailwind.sh
 *
 * The generated CSS is committed; there is no build step at serve time.
 */
module.exports = {
    content: [
        './static/**/*.{html,js}',
        // fee[dB]ack v0.3.0 shell + screens — the only UI since the classic v2
        // shell was removed. Subsumed by the recursive ./static/** glob above,
        // but listed explicitly so the v3 tree's Tailwind coverage is obvious.
        './static/v3/**/*.{html,js}',
        // One recursive plugin glob subsumes the previous four narrow ones
        // (static/**, screen.js, settings.html, *.html) and additionally
        // scans plugin .js files that aren't screen.js (e.g.
        // plugins/app_tour_*/script.js), closing a silent coverage hole.
        './plugins/**/*.{js,html}',
        // highway_3d owns its styles via the `styles` capability (it ships
        // plugins/highway_3d/assets/plugin.css, preflight off), so core no
        // longer scans it — its arbitrary values live in its own sheet, not
        // core's. Pilot for decentralizing all bundled plugins' CSS.
        '!./plugins/highway_3d/**',
    ],
    theme: {
        extend: {
            colors: {
                dark: { 900: 'rgb(var(--dark-900) / <alpha-value>)', 800: 'rgb(var(--dark-800) / <alpha-value>)', 700: 'rgb(var(--dark-700) / <alpha-value>)', 600: 'rgb(var(--dark-600) / <alpha-value>)', 500: 'rgb(var(--dark-500) / <alpha-value>)' },
                gray: Object.fromEntries([50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950].map((n) => [n, `rgb(var(--gray-${n}) / <alpha-value>)`])),
                accent: { DEFAULT: 'rgb(var(--rb-link) / <alpha-value>)', light: 'rgb(var(--rb-linkHi) / <alpha-value>)', dark: 'rgb(var(--rb-linkLo) / <alpha-value>)' },
                gold: '#e8c040',
                // fee[dB]ack v0.3.0 palette (additive). Namespaced under `fb`
                // so it never clashes with the legacy `accent`/`dark`/`gold`
                // tokens the 0.2.9 stylesheet still builds against.
                // See ~/Repositories/feedBack-feedback-v030/design/01-design-system.md §1.
                fb: Object.fromEntries(["bg", "sidebar", "card", "cardMuted", "primary", "primaryHi", "accent", "text", "textDim", "border", "good", "mid", "low", "gold"].map((k) => [k, `rgb(var(--fb-${k}) / <alpha-value>)`])),
            },
            fontFamily: {
                // Shared token used by BOTH the v2 (legacy) and v3 bodies. Keep it
                // Inter so the default v2 UI is unchanged; v3 overrides font-display
                // to Rubik in static/v3/v3.css (loaded only by the v3 page).
                display: ['"Inter"', 'system-ui', 'sans-serif'],
            },
        },
    },
    safelist: [
        // Dynamically-built class names that don't appear textually in
        // any source file the content globs cover.
        { pattern: /^(bg|text|border|ring)-(red|green|amber|yellow|blue|indigo|purple|pink|gray|slate)-(50|100|200|300|400|500|600|700|800|900)$/ },
        { pattern: /^(bg|text|border)-(dark|accent)(-.+)?$/ },
        'text-gold', 'bg-gold', 'border-gold',
    ],
    plugins: [],
};
