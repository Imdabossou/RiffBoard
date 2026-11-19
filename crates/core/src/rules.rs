//! Leaderboard rules: when a run counts, and how it is scored.
//!
//! The server calls [`check_result`] on every submitted score; the game uses
//! [`score`] for the number it shows, so the two can never disagree.

/// A run must last at least this fraction of the chart (up to its last note).
pub const MIN_ELAPSED_FRACTION: f64 = 0.95;
/// Seconds of slack on that elapsed-time check (network and start-up delays).
pub const ELAPSED_GRACE_S: f64 = 3.0;
/// At most this many judged events per judgeable event (+2), for rounding.
pub const MAX_JUDGED_SLACK: f64 = 1.05;
/// At least this fraction of the chart must have been judged.
pub const MIN_JUDGED_FRACTION: f64 = 0.5;

/// `round(hits * 100 * accuracy)`, half up, where accuracy = hits / judged.
pub fn score(hits: i64, misses: i64) -> i64 {
    let judged = (hits + misses) as f64;
    let accuracy = hits as f64 / judged.max(1.0);
    (hits as f64 * 100.0 * accuracy + 0.5).floor() as i64
}

/// Hits as a fraction of judged events (0 when nothing was judged).
pub fn accuracy(hits: i64, misses: i64) -> f64 {
    hits as f64 / ((hits + misses) as f64).max(1.0)
}

/// Checks a submitted run. Returns `(score, accuracy)` or the reason it is refused.
///
/// `elapsed` is the time since the run ticket was issued, `judgeable` the
/// number of notes + chords in the arrangement, `last_note` its last event time.
pub fn check_result(
    elapsed: f64,
    judgeable: usize,
    last_note: f64,
    hits: i64,
    misses: i64,
) -> Result<(i64, f64), &'static str> {
    if elapsed + ELAPSED_GRACE_S < last_note * MIN_ELAPSED_FRACTION {
        return Err("that run finished too quickly to count");
    }
    let judged = (hits + misses) as f64;
    if judged > judgeable as f64 * MAX_JUDGED_SLACK + 2.0 {
        return Err("more notes judged than the chart has");
    }
    if judged < judgeable as f64 * MIN_JUDGED_FRACTION {
        return Err("only full runs count for the leaderboard");
    }
    Ok((score(hits, misses), accuracy(hits, misses)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_formula() {
        assert_eq!(score(0, 0), 0);
        assert_eq!(score(10, 0), 1000);
        assert_eq!(score(9, 1), 810);
        assert_eq!(score(1, 2), 33); // 1 * 100 * 0.333.. = 33.3
        assert_eq!(score(1, 1), 50);
    }

    #[test]
    fn checks() {
        assert!(check_result(10.0, 100, 100.0, 90, 10).is_err()); // too quick
        assert!(check_result(100.0, 100, 100.0, 200, 0).is_err()); // too many
        assert!(check_result(100.0, 100, 100.0, 20, 10).is_err()); // too few
        let (s, a) = check_result(92.5, 100, 100.0, 90, 10).unwrap();
        assert_eq!(s, 8100);
        assert!((a - 0.9).abs() < 1e-9);
    }
}
