/*
 * RiffBoard — per-site branding (GET /api/site, from CONFIG_DIR/site/site.json
 * on the server). Colours arrive separately as /site.css so there is no flash;
 * this script applies the text and images: page title, favicon, sidebar
 * wordmark/logo, and exposes the rest (hero copy, about text, links) to the
 * home and about pages through window.riffboardSite.
 *
 * All values are plain text or same-origin/http(s) URLs validated server-side;
 * they are still only ever inserted with textContent or as attribute values.
 */
(function () {
    'use strict';
    const site = { data: {}, loaded: false };
    window.riffboardSite = site;

    function applyBrand() {
        const d = site.data;
        const brand = document.getElementById('v3-brand');
        if (!brand || !(d.name || d.logo)) return;
        const a = brand.querySelector('a') || brand;
        if (d.logo) {
            a.textContent = '';
            const img = document.createElement('img');
            img.src = d.logo;
            img.alt = d.name || '';
            img.style.cssText = 'max-width:100%;max-height:4.5rem;display:block';
            a.appendChild(img);
        } else {
            const label = a.querySelector('span');
            if (label) label.textContent = d.name;
        }
        a.setAttribute('aria-label', (d.name || 'Home') + ' home');
    }

    function applyHead() {
        const d = site.data;
        if (d.name) {
            document.title = d.tagline ? d.name + ' · ' + d.tagline : d.name;
            const meta = document.querySelector('meta[name="description"]');
            if (meta && d.tagline) meta.setAttribute('content', d.name + ': ' + d.tagline);
        }
        if (d.favicon) {
            document.querySelectorAll('link[rel="icon"],link[rel="apple-touch-icon"]').forEach((l) => l.setAttribute('href', d.favicon));
        }
    }

    function applyAll() {
        applyHead();
        applyBrand();
        const title = document.getElementById('v3-topbar-title');
        const active = document.querySelector('.screen.active');
        if (title && active && active.id === 'v3-home' && site.data.name) {
            title.textContent = 'Welcome to ' + site.data.name;
        }
        if (window.v3Dashboard && typeof window.v3Dashboard.render === 'function') window.v3Dashboard.render();
    }

    fetch('/api/site').then((r) => (r.ok ? r.json() : {})).catch(() => ({})).then((d) => {
        site.data = d && typeof d === 'object' ? d : {};
        site.loaded = true;
        if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', applyAll, { once: true });
        else applyAll();
        if (window.feedBack && typeof window.feedBack.emit === 'function') window.feedBack.emit('riffboard:site', site.data);
    });

    // The shell rebuilds the sidebar on boot and the topbar title on every
    // navigation; re-apply after those.
    document.addEventListener('DOMContentLoaded', () => {
        setTimeout(() => { if (site.loaded) applyBrand(); }, 0);
        if (window.feedBack && typeof window.feedBack.on === 'function') {
            window.feedBack.on('screen:changed', (e) => {
                if (e && e.detail && e.detail.id === 'v3-home' && site.data.name) {
                    const t = document.getElementById('v3-topbar-title');
                    if (t) t.textContent = 'Welcome to ' + site.data.name;
                }
            });
        }
    });
})();
