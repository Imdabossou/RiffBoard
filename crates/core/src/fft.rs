//! A small radix-2 FFT for the note detector: Hann-windowed magnitude
//! spectrum of a real frame. All buffers are allocated once, in [`Fft::new`].

pub struct Fft {
    n: usize,
    window: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<u32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Fft {
    /// `n` must be a power of two.
    pub fn new(n: usize) -> Fft {
        assert!(n.is_power_of_two() && n >= 4);
        let bits = n.trailing_zeros();
        let tau = std::f64::consts::TAU;
        Fft {
            n,
            window: (0..n)
                .map(|i| (0.5 - 0.5 * (tau * i as f64 / n as f64).cos()) as f32)
                .collect(),
            cos: (0..n / 2)
                .map(|k| (tau * k as f64 / n as f64).cos() as f32)
                .collect(),
            sin: (0..n / 2)
                .map(|k| -(tau * k as f64 / n as f64).sin() as f32)
                .collect(),
            rev: (0..n as u32)
                .map(|i| i.reverse_bits() >> (32 - bits))
                .collect(),
            re: vec![0.0; n],
            im: vec![0.0; n],
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Magnitudes of bins `0..n/2` of `input` (length `n`) into `out`.
    pub fn magnitudes(&mut self, input: &[f32], out: &mut [f32]) {
        let n = self.n;
        for ((&x, &w), &j) in input.iter().zip(&self.window).zip(&self.rev) {
            self.re[j as usize] = x * w;
            self.im[j as usize] = 0.0;
        }
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let a = start + k;
                    let b = a + half;
                    let tr = self.re[b] * wr - self.im[b] * wi;
                    let ti = self.re[b] * wi + self.im[b] * wr;
                    self.re[b] = self.re[a] - tr;
                    self.im[b] = self.im[a] - ti;
                    self.re[a] += tr;
                    self.im[a] += ti;
                }
            }
            size *= 2;
        }
        for (k, o) in out.iter_mut().enumerate().take(n / 2) {
            *o = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_sine() {
        let n = 1024;
        let mut fft = Fft::new(n);
        let input: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 50.0 * i as f32 / n as f32).sin())
            .collect();
        let mut out = vec![0.0; n / 2];
        fft.magnitudes(&input, &mut out);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(peak, 50);
        // Hann window: a bin-centred sine has amplitude n/4.
        assert!((out[50] - n as f32 / 4.0).abs() < 1.0);
        assert!(out[200] < 1e-2);
    }
}
