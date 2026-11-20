//! RiffBoard's game rules, shared by the server and the browser game.
//!
//! Everything here is plain Rust with no I/O, so it is unit-tested natively
//! and compiled unchanged into the wasm game:
//!
//! - [`chart`]: the chart model: read from packs by the server, sent to the
//!   game as JSON.
//! - [`judge`]: the list of judgeable events and the running tally.
//! - [`rules`]: what a leaderboard run must satisfy, and the score formula.
//!   The server checks submitted runs with exactly this code.
//! - [`pitch`]: string/fret/tuning to pitch.
//! - [`detect`]: microphone note detection against the expected notes.

pub mod chart;
pub mod detect;
pub mod fft;
pub mod judge;
pub mod pitch;
pub mod rules;
