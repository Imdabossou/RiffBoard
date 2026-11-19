//! Pitch of a string/fret on a tuned instrument.

/// MIDI note of each open string in standard tuning, lowest string first.
pub fn base_midis(string_count: usize, bass: bool) -> &'static [i32] {
    // Same table as the server's scale-degree marks (src/chart.rs).
    match (string_count, bass) {
        (5, true) => &[23, 28, 33, 38, 43],
        (n, true) if n <= 4 => &[28, 33, 38, 43],
        (7, _) => &[35, 40, 45, 50, 55, 59, 64],
        (8, _) => &[30, 35, 40, 45, 50, 55, 59, 64],
        _ => &[40, 45, 50, 55, 59, 64],
    }
}

/// MIDI note of `fret` on `string` (0 = lowest), given per-string tuning
/// offsets in semitones and a capo.
pub fn midi(
    string: usize,
    fret: i32,
    tuning: &[i32],
    capo: i32,
    bass: bool,
    strings: usize,
) -> i32 {
    let base = base_midis(strings, bass);
    let root = base
        .get(string)
        .copied()
        .unwrap_or(*base.last().unwrap_or(&40));
    root + tuning.get(string).copied().unwrap_or(0) + capo + fret.max(0)
}

/// Frequency in Hz of a (fractional) MIDI note, with a cent offset.
pub fn freq(midi: f64, cents: f64) -> f64 {
    440.0 * 2f64.powf((midi - 69.0 + cents / 100.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bass_and_guitar() {
        // Bass E standard: open E string is E1 (28), 5th fret is A1 (33).
        assert_eq!(midi(0, 0, &[0; 4], 0, true, 4), 28);
        assert_eq!(midi(0, 5, &[0; 4], 0, true, 4), 33);
        // Drop D lowers the E string by two.
        assert_eq!(midi(0, 0, &[-2, 0, 0, 0], 0, true, 4), 26);
        // Guitar high E string, 12th fret with capo 2.
        assert_eq!(midi(5, 12, &[0; 6], 2, false, 6), 78);
        assert!((freq(69.0, 0.0) - 440.0).abs() < 1e-9);
        assert!((freq(28.0, 0.0) - 41.2034).abs() < 1e-3);
    }
}
