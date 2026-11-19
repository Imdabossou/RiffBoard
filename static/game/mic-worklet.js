// Microphone capture for the note detector. Runs on the audio thread: it only
// copies input into ~21 ms blocks and posts them, stamped with the audio
// clock, to the page. Blocks are handed back by the page and reused, so
// steady play creates no garbage.
const BLOCK = 1024;

class RiffBoardMic extends AudioWorkletProcessor {
  constructor() {
    super();
    this.free = [];
    this.buf = null;
    this.n = 0;
    this.t = 0;
    this.port.onmessage = (e) => {
      if (e.data instanceof Float32Array) this.free.push(e.data);
    };
  }

  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (!ch) return true;
    let i = 0;
    while (i < ch.length) {
      if (!this.buf) {
        this.buf = this.free.pop() || new Float32Array(BLOCK);
        this.n = 0;
        // Audio-clock time of this block's first sample.
        this.t = currentTime + i / sampleRate;
      }
      const take = Math.min(BLOCK - this.n, ch.length - i);
      this.buf.set(ch.subarray(i, i + take), this.n);
      this.n += take;
      i += take;
      if (this.n === BLOCK) {
        this.port.postMessage({ buf: this.buf, t: this.t }, [this.buf.buffer]);
        this.buf = null;
      }
    }
    return true;
  }
}

registerProcessor('riffboard-mic', RiffBoardMic);
