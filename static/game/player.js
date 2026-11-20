// The play session: chart loading, Web Audio playback, the song clock, the
// microphone, and the frame loop that drives the wasm game.
//
// Smoothness rules (see CLAUDE.md "Game client"):
// - song time comes from the audio output clock, interpolated per frame;
// - the frame loop allocates nothing and touches no DOM;
// - everything on screen during play is drawn by the game into one canvas.

import init, { Game } from './pkg/riffboard_game.js';

let wasmReady = null;
export function loadWasm() {
  wasmReady ||= init({ module_or_path: new URL('./pkg/riffboard_game_bg.wasm', import.meta.url) });
  return wasmReady;
}

/** Fetches one arrangement's chart (JSON text, parsed by the game). */
export async function fetchChart(filename, arrangement) {
  const path = filename.split('/').map(encodeURIComponent).join('/');
  const r = await fetch(`/api/chart/${path}?arrangement=${encodeURIComponent(arrangement)}`);
  if (!r.ok) {
    let msg = 'Could not load the chart';
    try { msg = (await r.json()).error || msg; } catch { /* not JSON */ }
    throw new Error(msg);
  }
  return r.text();
}

/**
 * Song clock: maps the audio output clock to song seconds and smooths it per
 * frame, so the highway moves evenly even though the audio clock itself
 * advances in render-quantum steps.
 */
class Clock {
  constructor(ctx) {
    this.ctx = ctx;
    this.start = 0; // ctx time at which song time 0 is rendered
    this.shown = 0;
    this.lastPerf = 0;
    this.running = false;
    this.pausedAt = 0;
  }

  /** Song time being heard right now (unsmoothed). */
  raw(perfNow) {
    const ctx = this.ctx;
    const ts = ctx.getOutputTimestamp ? ctx.getOutputTimestamp() : null;
    let heard;
    if (ts && ts.performanceTime > 0 && ts.contextTime > 0) {
      heard = ts.contextTime + (perfNow - ts.performanceTime) / 1000;
    } else {
      heard = ctx.currentTime - (ctx.outputLatency || ctx.baseLatency || 0);
    }
    return heard - this.start;
  }

  /** Smoothed song time for drawing at `perfNow` (rAF timestamp). */
  now(perfNow) {
    if (!this.running) return this.pausedAt;
    const raw = this.raw(perfNow);
    const dt = this.lastPerf ? (perfNow - this.lastPerf) / 1000 : 0;
    this.lastPerf = perfNow;
    const pred = this.shown + dt;
    const err = raw - pred;
    // Snap on big jumps (start, seek, audio glitch), otherwise slew gently.
    this.shown = Math.abs(err) > 0.05 ? raw : pred + err * 0.08;
    return this.shown;
  }
}

export class Session {
  /**
   * @param {HTMLCanvasElement} canvas
   * @param {object} opts {speed, lowStringBottom, sensitivity, visualOffsetMs, micOffsetMs, quality, micDeviceId}
   */
  constructor(canvas, opts) {
    this.canvas = canvas;
    this.opts = opts;
    this.ctx = null;
    this.game = null;
    this.buffers = [];
    this.sources = [];
    this.raf = 0;
    this.ended = false;
    this.onEnd = null;
    this.mic = null;
    this.cssW = canvas.clientWidth || 1;
    this.cssH = canvas.clientHeight || 1;
    this.dpr = 1;
    this.endT = 0;
    this.leadIn = 3;
    this.loop = this.loop.bind(this);
    this.ro = new ResizeObserver(([e]) => {
      this.cssW = e.contentRect.width || 1;
      this.cssH = e.contentRect.height || 1;
    });
    this.ro.observe(canvas);
    this.updateDpr();
  }

  updateDpr() {
    const d = window.devicePixelRatio || 1;
    const q = this.opts.quality;
    this.dpr = q === 'fast' ? 1 : q === 'sharp' ? Math.min(d, 3) : Math.min(d, 2);
  }

  /** Loads wasm, chart and audio. `onStage(text)` reports progress. */
  async load(filename, arrangement, onStage) {
    onStage('Starting the game');
    await loadWasm();
    this.game = new Game(this.canvas);
    onStage('Loading the chart');
    this.info = JSON.parse(this.game.load(await fetchChart(filename, arrangement)));
    this.game.set_speed(this.opts.speed);
    this.game.set_low_string_bottom(!!this.opts.lowStringBottom);
    this.game.set_lefty(!!this.opts.lefty);
    if (this.opts.accent) this.game.set_accent(...this.opts.accent, ...this.opts.accent2);
    this.game.set_sensitivity(this.opts.sensitivity);

    if (this.opts.backdropUrl) {
      try {
        const img = new Image();
        img.src = this.opts.backdropUrl;
        await img.decode();
        this.game.set_backdrop(img);
      } catch { /* no backdrop: plain dark stage */ }
    }

    this.ctx = new AudioContext({ latencyHint: 'interactive' });
    this.clock = new Clock(this.ctx);
    let stems = this.info.stems.filter((s) => s.default);
    if (!stems.length && this.info.stems.length) stems = [this.info.stems[0]];
    const urls = stems.length ? stems.map((s) => s.url) : (this.info.audio_url ? [this.info.audio_url] : []);
    if (!urls.length) throw new Error('This song has no audio');
    onStage('Loading the audio');
    this.buffers = await Promise.all(urls.map(async (u) => {
      const r = await fetch(u);
      if (!r.ok) throw new Error('Could not load the audio');
      return this.ctx.decodeAudioData(await r.arrayBuffer());
    }));
    const audioEnd = Math.max(...this.buffers.map((b) => b.duration));
    this.endT = Math.max(this.info.last_event + 1.5, Math.min(audioEnd, (this.info.duration || audioEnd) + 0.5));
    this.leadIn = this.info.first_event < 3 ? 3 - Math.max(0, this.info.first_event) + 0.5 : 1;
    this.master = this.ctx.createGain();
    this.master.connect(this.ctx.destination);
  }

  /** Asks for the microphone and starts judging. Throws if refused. */
  async enableMic() {
    const constraints = {
      audio: {
        echoCancellation: false, noiseSuppression: false, autoGainControl: false,
        ...(this.opts.micDeviceId ? { deviceId: { exact: this.opts.micDeviceId } } : {}),
      },
    };
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia(constraints);
    } catch (e) {
      if (this.opts.micDeviceId) stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false } });
      else throw e;
    }
    await this.ctx.audioWorklet.addModule(new URL('./mic-worklet.js', import.meta.url));
    const src = this.ctx.createMediaStreamSource(stream);
    const node = new AudioWorkletNode(this.ctx, 'riffboard-mic', { numberOfInputs: 1, numberOfOutputs: 0 });
    src.connect(node);
    const track = stream.getAudioTracks()[0];
    const inLat = (track.getSettings && track.getSettings().latency) || 0.01;
    const outLat = this.ctx.outputLatency || this.ctx.baseLatency || 0;
    // Mic samples are stamped with the render clock; what the player heard
    // (and played along to) was rendered `outLat` earlier, and the sound
    // reaches the worklet `inLat` after it was played.
    this.micShift = outLat + inLat + (this.opts.micOffsetMs || 0) / 1000;
    node.port.onmessage = (e) => {
      const { buf, t } = e.data;
      if (this.clock.running) this.game.mic(buf, t - this.clock.start - this.micShift);
      node.port.postMessage(buf, [buf.buffer]);
    };
    this.game.enable_mic(this.ctx.sampleRate);
    this.mic = { stream, node, src };
  }

  /** Starts (or resumes) playback at song time `from`. */
  play(from) {
    const ctx = this.ctx;
    if (ctx.state === 'suspended') ctx.resume();
    this.stopSources();
    const at = ctx.currentTime + 0.08;
    const start = at - from; // ctx time of song time 0
    this.clock.start = start;
    this.clock.running = true;
    this.clock.lastPerf = 0;
    this.clock.shown = from;
    for (const b of this.buffers) {
      const s = ctx.createBufferSource();
      s.buffer = b;
      s.connect(this.master);
      if (from >= 0) s.start(at, from);
      else s.start(start, 0);
      this.sources.push(s);
    }
    if (!this.raf) this.raf = requestAnimationFrame(this.loop);
  }

  stopSources() {
    for (const s of this.sources) { try { s.stop(); } catch { /* already stopped */ } s.disconnect(); }
    this.sources = [];
  }

  pause() {
    if (!this.clock.running) return;
    this.clock.pausedAt = this.clock.raw(performance.now());
    this.clock.running = false;
    this.stopSources();
  }

  get paused() { return !this.clock.running; }

  time() { return this.clock.running ? this.clock.raw(performance.now()) : this.clock.pausedAt; }

  /** Practice: jump to song time `t` (keeps paused state). */
  seek(t) {
    t = Math.max(-this.leadIn, Math.min(t, this.endT));
    this.game.seek(t);
    if (this.clock.running) this.play(t);
    else this.clock.pausedAt = t;
  }

  loop(perfNow) {
    this.raf = requestAnimationFrame(this.loop);
    const t = this.clock.now(perfNow) + (this.opts.visualOffsetMs || 0) / 1000;
    this.game.frame(t, this.cssW, this.cssH, this.dpr);
    if (!this.ended && this.clock.running && t >= this.endT) {
      this.ended = true;
      this.pause();
      this.game.finish();
      this.onEnd?.();
    }
  }

  tally() {
    const [hits, misses, streak, best, score, accuracy] = this.game.tally();
    return { hits, misses, streak, best_streak: best, score, accuracy };
  }

  destroy() {
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.stopSources();
    this.ro.disconnect();
    if (this.mic) {
      this.mic.stream.getTracks().forEach((t) => t.stop());
      this.mic.node.port.onmessage = null;
      this.mic.src.disconnect();
    }
    this.ctx?.close();
    this.game?.free();
    this.game = null;
  }
}
