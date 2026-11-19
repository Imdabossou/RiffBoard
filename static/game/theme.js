// Applies the remembered light/dark choice before first paint (no flash).
// No stored choice = follow the system.
try {
  const t = localStorage.getItem('rb-theme');
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
} catch { /* storage blocked: follow the system */ }
