//! Percent-encoding sets matching Python's `urllib.parse.quote`, so URLs the
//! server hands to the browser are byte-identical to the original server's.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};

/// `quote(s, safe="")`: everything but ASCII alphanumerics and `_.-~`.
pub const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

/// `quote(s)` (default `safe="/"`).
pub const PATH: &AsciiSet = &COMPONENT.remove(b'/');

pub fn quote_component(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, COMPONENT).to_string()
}

pub fn quote_path(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, PATH).to_string()
}
