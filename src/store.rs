//! Leaderboard store: public highscores per song and arrangement, in SQLite
//! (`<CONFIG_DIR>/leaderboard.db`, same schema as RiffBoard 0.1). Visitors are
//! anonymous: a run records the typed name and a salted hash of the IP (for
//! moderation, never shown). Each name keeps its best run per board.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::util::{hex, now_secs, round_to};

pub const NAME_MAX: usize = 20;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS scores (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    filename    TEXT    NOT NULL,
    arrangement INTEGER NOT NULL,
    name        TEXT    NOT NULL,
    name_key    TEXT    NOT NULL,
    score       INTEGER NOT NULL,
    accuracy    REAL    NOT NULL,
    hits        INTEGER NOT NULL,
    misses      INTEGER NOT NULL,
    best_streak INTEGER NOT NULL,
    created_at  REAL    NOT NULL,
    ip_hash     TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS scores_board ON scores (filename, arrangement, score DESC);
CREATE INDEX IF NOT EXISTS scores_name ON scores (name_key);
CREATE TABLE IF NOT EXISTS banned_names (
    name_key   TEXT PRIMARY KEY,
    created_at REAL NOT NULL
);
";

fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(c as u32,
            0x00AD | 0x061C | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064
            | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0xE000..=0xF8FF | 0xD800..=0xDFFF)
}

/// Normalise a typed display name, or None if it isn't acceptable.
pub fn clean_name(raw: &str) -> Option<String> {
    let s: String = raw.nfkc().filter(|c| !is_invisible(*c)).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = s.chars().count();
    if !(1..=NAME_MAX).contains(&n) {
        return None;
    }
    s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == ' ' || c == '.' || c == '\'' || c == '-')
        .then_some(s)
}

/// Case/spacing-insensitive identity of a name (for bans and de-dup).
pub fn name_key(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !"_.-'".contains(*c))
        .collect()
}

pub struct Store {
    conn: Mutex<Connection>,
    salt: String,
}

impl Store {
    pub fn open(config_dir: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(config_dir.join("leaderboard.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        let salt_file = config_dir.join("leaderboard.salt");
        let salt = match std::fs::read_to_string(&salt_file) {
            Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => {
                let s = hex(&{
                    let mut b = [0u8; 32];
                    getrandom::fill(&mut b).expect("random");
                    b
                });
                write_private(&salt_file, &s);
                s
            }
        };
        Ok(Self {
            conn: Mutex::new(conn),
            salt,
        })
    }

    pub fn ip_hash(&self, ip: &str) -> String {
        let mut h = Sha256::new();
        h.update(format!("{}:{ip}", self.salt).as_bytes());
        hex(&h.finalize())[..32].to_string()
    }

    pub fn is_banned(&self, name: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT 1 FROM banned_names WHERE name_key = ?1",
            [name_key(name)],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &self,
        filename: &str,
        arrangement: i64,
        name: &str,
        score: i64,
        accuracy: f64,
        hits: i64,
        misses: i64,
        best_streak: i64,
        ip: &str,
    ) -> rusqlite::Result<i64> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO scores (filename, arrangement, name, name_key, score, accuracy, hits, misses, \
             best_streak, created_at, ip_hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![filename, arrangement, name, name_key(name), score, accuracy, hits, misses,
                    best_streak, now_secs(), self.ip_hash(ip)],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn delete(&self, id: i64) -> bool {
        let c = self.conn.lock().unwrap();
        c.execute("DELETE FROM scores WHERE id = ?1", [id])
            .unwrap_or(0)
            > 0
    }

    pub fn ban(&self, name: &str, purge: bool) -> usize {
        let key = name_key(name);
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT OR IGNORE INTO banned_names (name_key, created_at) VALUES (?1, ?2)",
            params![key, now_secs()],
        );
        if purge {
            c.execute("DELETE FROM scores WHERE name_key = ?1", [key])
                .unwrap_or(0)
        } else {
            0
        }
    }

    pub fn unban(&self, name: &str) -> bool {
        let c = self.conn.lock().unwrap();
        c.execute(
            "DELETE FROM banned_names WHERE name_key = ?1",
            [name_key(name)],
        )
        .unwrap_or(0)
            > 0
    }

    pub fn wipe(&self, filename: &str, arrangement: Option<i64>) -> usize {
        let c = self.conn.lock().unwrap();
        match arrangement {
            None => c.execute("DELETE FROM scores WHERE filename = ?1", [filename]),
            Some(a) => c.execute(
                "DELETE FROM scores WHERE filename = ?1 AND arrangement = ?2",
                params![filename, a],
            ),
        }
        .unwrap_or(0)
    }

    /// Best run per name, ranked by score (ties: earlier run wins).
    pub fn top(&self, filename: &str, arrangement: i64, limit: i64) -> Vec<Value> {
        let limit = limit.clamp(1, 100);
        let c = self.conn.lock().unwrap();
        let Ok(mut st) = c.prepare(
            "SELECT id, name, score, accuracy, hits, misses, best_streak, created_at FROM (
                 SELECT *, ROW_NUMBER() OVER (PARTITION BY name_key ORDER BY score DESC, created_at ASC) AS rn
                 FROM scores WHERE filename = ?1 AND arrangement = ?2
             ) WHERE rn = 1 ORDER BY score DESC, created_at ASC LIMIT ?3") else { return vec![] };
        let rows = st.query_map(params![filename, arrangement, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, f64>(7)?,
            ))
        });
        rows.map(|it| {
            it.flatten().enumerate().map(|(i, r)| json!({
            "rank": i + 1, "id": r.0, "name": r.1, "score": r.2, "accuracy": round_to(r.3, 4),
            "hits": r.4, "misses": r.5, "best_streak": r.6, "created_at": r.7,
        })).collect()
        })
        .unwrap_or_default()
    }

    pub fn rank_of(&self, filename: &str, arrangement: i64, id: i64) -> Option<i64> {
        self.top(filename, arrangement, 100)
            .iter()
            .find(|r| r["id"].as_i64() == Some(id))
            .and_then(|r| r["rank"].as_i64())
    }

    /// Per board: run count, best score and who holds it.
    pub fn summary(&self) -> Vec<(String, i64, i64, i64, String)> {
        let c = self.conn.lock().unwrap();
        let Ok(mut st) = c.prepare(
            "SELECT a.filename, a.arrangement, a.runs, a.top,
                    (SELECT name FROM scores t WHERE t.filename = a.filename AND t.arrangement = a.arrangement
                       AND t.score = a.top ORDER BY t.created_at ASC LIMIT 1)
             FROM (SELECT filename, arrangement, COUNT(*) AS runs, MAX(score) AS top
                   FROM scores GROUP BY filename, arrangement) a") else { return vec![] };
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            ))
        })
        .map(|it| it.flatten().collect())
        .unwrap_or_default()
    }

    pub fn recent(&self, limit: i64) -> Vec<Value> {
        let c = self.conn.lock().unwrap();
        let Ok(mut st) = c.prepare(
            "SELECT id, filename, arrangement, name, score, accuracy, created_at, ip_hash \
             FROM scores ORDER BY created_at DESC LIMIT ?1",
        ) else {
            return vec![];
        };
        st.query_map([limit.clamp(1, 500)], |r| Ok(json!({
            "id": r.get::<_, i64>(0)?, "filename": r.get::<_, String>(1)?, "arrangement": r.get::<_, i64>(2)?,
            "name": r.get::<_, String>(3)?, "score": r.get::<_, i64>(4)?,
            "accuracy": round_to(r.get::<_, f64>(5)?, 4), "created_at": r.get::<_, f64>(6)?,
            "ip_hash": r.get::<_, String>(7)?,
        }))).map(|it| it.flatten().collect()).unwrap_or_default()
    }
}

/// Write a file readable only by its owner (secrets: salt, admin token).
pub fn write_private(path: &Path, contents: &str) {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    if let Ok(mut f) = opts.open(path) {
        let _ = f.write_all(contents.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_lists_each_board_with_its_leader() {
        let dir = std::env::temp_dir().join(format!("rb-summary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir).unwrap();
        store.add("a.feedpak", 0, "Low", 100, 0.5, 5, 5, 2, "1.1.1.1").unwrap();
        store.add("a.feedpak", 0, "High", 900, 0.9, 9, 1, 7, "1.1.1.2").unwrap();
        store.add("b.feedpak", 1, "Solo", 50, 0.2, 2, 8, 1, "1.1.1.3").unwrap();
        let mut boards = store.summary();
        boards.sort();
        assert_eq!(
            boards,
            vec![
                ("a.feedpak".to_string(), 0, 2, 900, "High".to_string()),
                ("b.feedpak".to_string(), 1, 1, 50, "Solo".to_string()),
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("  Big   Dog ").as_deref(), Some("Big Dog"));
        assert_eq!(
            clean_name("Rif\u{202e}fer\u{200b}").as_deref(),
            Some("Riffer")
        );
        for bad in ["", " ", "<script>", "\u{202e}", "😀", &"x".repeat(21)] {
            assert!(clean_name(bad).is_none(), "{bad:?}");
        }
        assert_eq!(name_key(" Big.Dog_ "), "bigdog");
    }
}
