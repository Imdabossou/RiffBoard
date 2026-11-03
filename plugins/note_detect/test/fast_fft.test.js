'use strict';
// The fast real-input FFT behind _ndFftMagnitude must give the same spectrum
// as the reference complex FFT (_ndFftInPlace) it replaced.
const test = require('node:test');
const assert = require('node:assert');
const { loadDetectionCore } = require('./_loader');

const core = loadDetectionCore();

function reference(buffer, n) {
    const data = new Float64Array(2 * n);
    const L = buffer.length;
    for (let i = 0; i < L && i < n; i++) {
        const w = 0.5 * (1 - Math.cos(2 * Math.PI * i / (L - 1)));
        data[2 * i] = buffer[i] * w;
    }
    core.fftInPlace(data, 1);
    const out = new Float64Array((n >> 1) + 1);
    for (let k = 0; k < out.length; k++) out[k] = Math.hypot(data[2 * k], data[2 * k + 1]);
    return out;
}

function rng(seed) {
    let s = seed >>> 0;
    return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296 - 0.5; };
}

for (const [len, n] of [[8, 8], [5, 8], [64, 64], [100, 256], [4096, 16384], [16384, 16384], [3000, 32768]]) {
    test(`real FFT matches reference: ${len} samples, n=${n}`, () => {
        const r = rng(len * 31 + n);
        const buf = new Float32Array(len);
        for (let i = 0; i < len; i++) buf[i] = r() + 0.3 * Math.sin(i * 0.05);
        const want = reference(buf, n);
        const got = core.realFftMagnitude(buf, core.hannWindow(len), n, new Float32Array((n >> 1) + 1));
        let peak = 0;
        for (const v of want) peak = Math.max(peak, v);
        for (let k = 0; k < want.length; k++) {
            assert.ok(Math.abs(got[k] - want[k]) <= 1e-4 * peak + 1e-6,
                `bin ${k}: ${got[k]} vs ${want[k]}`);
        }
    });
}

test('fftMagnitude finds a sine at the right bin', () => {
    const sr = 48000, f = 110, len = 8192;
    const buf = new Float32Array(len);
    for (let i = 0; i < len; i++) buf[i] = Math.sin(2 * Math.PI * f * i / sr);
    const { magnitudes, binHz } = core.fftMagnitude(buf, sr);
    let best = 0;
    for (let k = 1; k < magnitudes.length; k++) if (magnitudes[k] > magnitudes[best]) best = k;
    assert.ok(Math.abs(best * binHz - f) <= binHz, `peak at ${best * binHz} Hz`);
});
