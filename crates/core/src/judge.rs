//! The judgeable events of a chart (each note and each chord) and the tally.

use crate::chart::Chart;

/// Seconds before an event's time that a matching sound still counts.
pub const EARLY: f64 = 0.15;
/// Seconds after an event's time that a matching sound still counts.
pub const LATE: f64 = 0.20;

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub time: f64,
    /// Expected pitches (MIDI), one per string played.
    pub midis: Vec<i32>,
    /// False for muted and harmonic notes: judged on attack, not pitch.
    pub pitched: bool,
    /// Index into `Chart::notes` or `Chart::chords`.
    pub index: usize,
    pub is_chord: bool,
    /// `None` until judged.
    pub result: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub hits: i64,
    pub misses: i64,
    pub streak: i64,
    pub best_streak: i64,
}

impl Tally {
    pub fn score(&self) -> i64 {
        crate::rules::score(self.hits, self.misses)
    }

    pub fn accuracy(&self) -> f64 {
        crate::rules::accuracy(self.hits, self.misses)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Judge {
    pub events: Vec<Event>,
    pub tally: Tally,
    /// Events judged since the caller last drained this (for hit effects).
    pub fresh: Vec<usize>,
}

impl Judge {
    pub fn new(chart: &Chart) -> Judge {
        let mut events: Vec<Event> = chart
            .notes
            .iter()
            .enumerate()
            .map(|(i, n)| Event {
                time: n.time,
                midis: vec![chart.midi(n)],
                pitched: !n.mute && !n.harmonic,
                index: i,
                is_chord: false,
                result: None,
            })
            .chain(chart.chords.iter().enumerate().map(|(i, c)| {
                let mut midis: Vec<i32> = c
                    .notes
                    .iter()
                    .filter(|n| !n.mute)
                    .map(|n| chart.midi(n))
                    .collect();
                midis.sort_unstable();
                midis.dedup();
                Event {
                    time: c.time,
                    pitched: !midis.is_empty() && !c.notes.iter().all(|n| n.harmonic),
                    midis,
                    index: i,
                    is_chord: true,
                    result: None,
                }
            }))
            .collect();
        events.sort_by(|a, b| a.time.total_cmp(&b.time));
        Judge {
            events,
            tally: Tally::default(),
            fresh: Vec::new(),
        }
    }

    /// Records the result of event `i` (once).
    pub fn record(&mut self, i: usize, hit: bool) {
        let Some(e) = self.events.get_mut(i) else {
            return;
        };
        if e.result.is_some() {
            return;
        }
        e.result = Some(hit);
        self.fresh.push(i);
        let t = &mut self.tally;
        if hit {
            t.hits += 1;
            t.streak += 1;
            t.best_streak = t.best_streak.max(t.streak);
        } else {
            t.misses += 1;
            t.streak = 0;
        }
    }

    /// Makes every event at or after `t` judgeable again (after a seek).
    pub fn reopen_from(&mut self, t: f64) {
        for e in &mut self.events {
            if e.time + LATE >= t {
                e.result = None;
            }
        }
        self.tally.streak = 0;
    }

    /// Index of the first event at or after `t`.
    pub fn first_at(&self, t: f64) -> usize {
        self.events.partition_point(|e| e.time < t)
    }

    /// Result of a note or chord by its chart index, for drawing.
    pub fn result_map(&self, chart: &Chart) -> (Vec<Option<bool>>, Vec<Option<bool>>) {
        let mut notes = vec![None; chart.notes.len()];
        let mut chords = vec![None; chart.chords.len()];
        for e in &self.events {
            if e.is_chord {
                chords[e.index] = e.result;
            } else {
                notes[e.index] = e.result;
            }
        }
        (notes, chords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Chord, Note};

    #[test]
    fn tally_and_order() {
        let chart = Chart {
            string_count: 4,
            bass: true,
            tuning: vec![0; 4],
            notes: vec![
                Note {
                    time: 2.0,
                    string: 0,
                    fret: 5,
                    ..Note::default()
                },
                Note {
                    time: 1.0,
                    string: 1,
                    fret: 0,
                    mute: true,
                    ..Note::default()
                },
            ],
            chords: vec![Chord {
                time: 1.5,
                template: 0,
                notes: vec![
                    Note {
                        time: 1.5,
                        string: 0,
                        fret: 5,
                        ..Note::default()
                    },
                    Note {
                        time: 1.5,
                        string: 1,
                        fret: 0,
                        ..Note::default()
                    },
                ],
            }],
            ..Chart::default()
        };
        let mut j = Judge::new(&chart);
        assert_eq!(j.events.len(), 3);
        assert_eq!(j.events[0].time, 1.0);
        assert!(!j.events[0].pitched);
        // Chord with A1 twice (5th fret E string and open A string) dedups.
        assert_eq!(j.events[1].midis, vec![33]);
        j.record(0, true);
        j.record(0, false); // ignored: already judged
        j.record(1, true);
        j.record(2, false);
        assert_eq!(
            j.tally,
            Tally {
                hits: 2,
                misses: 1,
                streak: 0,
                best_streak: 2
            }
        );
        assert_eq!(j.tally.score(), 133);
        j.reopen_from(1.4);
        assert_eq!(j.events[1].result, None);
        assert_eq!(j.events[0].result, Some(true));
    }
}
