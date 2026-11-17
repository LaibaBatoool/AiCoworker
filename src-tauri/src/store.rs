//! Central activity store: one SQLite database OUTSIDE every workspace,
//! %APPDATA%\AiCoworker\aicoworker.db.
//!
//! Why (finding 43): the old audit log lived in <workspace>/.aicoworker,
//! where the agent's own tools could reach it. The JSONL file is kept as a
//! readable mirror, but this database is the record we trust.
//!
//! Tamper evidence, not tamper proofing. Every row stores
//!   hash = SHA-256(previous row's hash + this row's fields)
//! so changing, deleting or inserting a row breaks the chain. A script that
//! runs as the same Windows user CAN still open the file and recompute the
//! whole chain, so the app also keeps the hash of the last row IT wrote in
//! memory and checks, before every new write, that the database still ends
//! with that row. Any change made while the app is running is caught at the
//! next logged action. Limit: a full, correctly re-hashed rewrite made while
//! the app is closed is not detectable without an outside anchor.

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const GENESIS: &str = "GENESIS";

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StoreRow {
    pub id: i64,
    pub timestamp_unix: i64,
    pub workspace: String,
    pub tier: String,
    pub action: String,
    pub success: bool,
    pub detail: String,
    pub hash: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StoreStatus {
    pub path: String,
    pub enabled: bool,
    pub rows: i64,
    /// Every row's hash matches its content and the row before it.
    pub chain_ok: bool,
    /// The database still ends with the last row this app process wrote.
    pub head_ok: bool,
    /// Human-readable problems (chain breaks, head mismatches, open errors).
    pub problems: Vec<String>,
    /// Tamper alerts raised during this app session (also stored as rows).
    pub alerts: Vec<String>,
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
    /// (id, hash) of the last row this process wrote or verified at open.
    head: Option<(i64, String)>,
    alerts: Vec<String>,
    /// Chain breaks already reported, so one break raises one alert.
    known_breaks: Vec<String>,
}

/// The canonical text that gets hashed. JSON array = unambiguous field
/// boundaries (no "ab"+"c" == "a"+"bc" tricks).
fn row_hash(prev: &str, id: i64, ts: i64, ws: &str, tier: &str, action: &str, success: bool, detail: &str) -> String {
    let canon = serde_json::json!([prev, id, ts, ws, tier, action, success, detail]).to_string();
    let digest = Sha256::digest(canon.as_bytes());
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("cannot open {}: {}", path.display(), e))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS audit (
                 id INTEGER PRIMARY KEY,
                 timestamp_unix INTEGER NOT NULL,
                 workspace TEXT NOT NULL,
                 tier TEXT NOT NULL,
                 action TEXT NOT NULL,
                 success INTEGER NOT NULL,
                 detail TEXT NOT NULL,
                 prev_hash TEXT NOT NULL,
                 hash TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS audit_ws ON audit(workspace, id);",
        )
        .map_err(|e| format!("cannot create tables: {}", e))?;
        let mut store = Store { conn, path: path.to_path_buf(), head: None, alerts: Vec::new(), known_breaks: Vec::new() };
        let problems = store.verify_chain();
        store.head = store.last_row()?;
        store.known_breaks = problems.clone();
        if !problems.is_empty() {
            store.alert(format!("Audit chain broken when the app started: {}", problems.join("; ")));
        }
        Ok(store)
    }

    fn last_row(&self) -> Result<Option<(i64, String)>, String> {
        self.conn
            .query_row("SELECT id, hash FROM audit ORDER BY id DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Walks every row in order. Returns one message per broken link
    /// (capped), empty = intact.
    pub fn verify_chain(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut stmt = match self.conn.prepare(
            "SELECT id, timestamp_unix, workspace, tier, action, success, detail, prev_hash, hash FROM audit ORDER BY id",
        ) {
            Ok(s) => s,
            Err(e) => return vec![format!("cannot read audit table: {}", e)],
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)? != 0,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        });
        let rows = match rows {
            Ok(r) => r,
            Err(e) => return vec![format!("cannot read audit table: {}", e)],
        };
        let mut expected_prev = GENESIS.to_string();
        for row in rows {
            let (id, ts, ws, tier, action, success, detail, prev, hash) = match row {
                Ok(v) => v,
                Err(e) => {
                    problems.push(format!("unreadable row: {}", e));
                    break;
                }
            };
            if prev != expected_prev {
                problems.push(format!("row #{} does not follow the row before it (a row was deleted or inserted)", id));
            } else if row_hash(&prev, id, ts, &ws, &tier, &action, success, &detail) != hash {
                problems.push(format!("row #{} was changed after it was written", id));
            }
            // continue from the STORED hash so one edit reports one problem
            expected_prev = hash;
            if problems.len() >= 5 {
                problems.push("… more problems not listed".into());
                break;
            }
        }
        problems
    }

    /// The database must still end with the row this process last wrote.
    fn check_head(&self) -> Option<String> {
        let now = self.last_row().ok().flatten();
        if now == self.head {
            return None;
        }
        Some(match (&self.head, &now) {
            (Some((hid, _)), None) => format!("all rows were deleted (the app last wrote row #{})", hid),
            (None, Some((nid, _))) => format!("row #{} appeared that this app did not write", nid),
            (Some((hid, _)), Some((nid, _))) if nid < hid => {
                format!("rows after #{} were removed (the app last wrote row #{})", nid, hid)
            }
            (Some((hid, _)), Some((nid, _))) if nid > hid => {
                format!("rows #{}..#{} were added outside the app", hid + 1, nid)
            }
            (Some((hid, _)), Some(_)) => format!("row #{} was changed or the log was rewritten", hid),
            (None, None) => unreachable!(),
        })
    }

    fn alert(&mut self, msg: String) {
        self.alerts.push(msg.clone());
        // the alert itself goes into the chain, so it can't be silently lost either
        let _ = self.insert("app", "system", "AUDIT_TAMPER_DETECTED", false, &msg);
    }

    fn insert(&mut self, workspace: &str, tier: &str, action: &str, success: bool, detail: &str) -> Result<i64, String> {
        let prev = self.last_row()?.map(|(_, h)| h).unwrap_or_else(|| GENESIS.to_string());
        let id = self.last_row()?.map(|(i, _)| i + 1).unwrap_or(1);
        let ts = now_unix();
        let hash = row_hash(&prev, id, ts, workspace, tier, action, success, detail);
        self.conn
            .execute(
                "INSERT INTO audit (id, timestamp_unix, workspace, tier, action, success, detail, prev_hash, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, ts, workspace, tier, action, success as i64, detail, prev, hash],
            )
            .map_err(|e| format!("cannot write audit row: {}", e))?;
        self.head = Some((id, hash));
        Ok(id)
    }

    /// Records one action. First checks (a) the database still ends with our
    /// last row and (b) the whole chain is intact. If either shows a change
    /// made outside the app, a tamper alert row is written BEFORE the entry.
    /// Full verification per action costs a few ms for thousands of rows.
    pub fn record(&mut self, workspace: &str, tier: &str, action: &str, success: bool, detail: &str) -> Result<i64, String> {
        let head_problem = self.check_head();
        let chain = self.verify_chain();
        let new_breaks: Vec<String> = chain.iter().filter(|p| !self.known_breaks.contains(p)).cloned().collect();
        if head_problem.is_some() || !new_breaks.is_empty() {
            let mut parts: Vec<String> = head_problem.into_iter().collect();
            parts.extend(new_breaks);
            let msg = format!("Audit log was modified outside the app: {}", parts.join("; "));
            self.head = self.last_row()?; // continue the chain from what is there now
            self.alert(msg);
            self.known_breaks = chain;
        }
        self.insert(workspace, tier, action, success, detail)
    }

    pub fn status(&self) -> StoreStatus {
        let rows = self.conn.query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0)).unwrap_or(-1);
        let mut problems = self.verify_chain();
        let chain_ok = problems.is_empty();
        let head_problem = self.check_head();
        if let Some(p) = &head_problem {
            problems.push(format!("Modified outside the app: {}", p));
        }
        StoreStatus {
            path: self.path.display().to_string(),
            enabled: true,
            rows,
            chain_ok,
            head_ok: head_problem.is_none(),
            problems,
            alerts: self.alerts.clone(),
        }
    }

    /// Newest first. `workspace` = None for all workspaces.
    pub fn recent(&self, workspace: Option<&str>, limit: i64) -> Result<Vec<StoreRow>, String> {
        let sql = "SELECT id, timestamp_unix, workspace, tier, action, success, detail, hash FROM audit
                   WHERE (?1 IS NULL OR workspace = ?1) ORDER BY id DESC LIMIT ?2";
        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![workspace, limit.clamp(1, 1000)], |r| {
                Ok(StoreRow {
                    id: r.get(0)?,
                    timestamp_unix: r.get(1)?,
                    workspace: r.get(2)?,
                    tier: r.get(3)?,
                    action: r.get(4)?,
                    success: r.get::<_, i64>(5)? != 0,
                    detail: r.get(6)?,
                    hash: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
}

// ---------- the one store the app uses ----------

static STORE: Mutex<Option<Result<Store, String>>> = Mutex::new(None);

/// AICOWORKER_DB=off disables the store (benchmark runs, so test actions
/// don't fill the user's history); AICOWORKER_DB=<path> uses another file.
fn configured_path() -> Option<PathBuf> {
    match std::env::var("AICOWORKER_DB") {
        Ok(v) if v.eq_ignore_ascii_case("off") => None,
        Ok(v) if !v.trim().is_empty() => Some(PathBuf::from(v)),
        _ => {
            let base = std::env::var("APPDATA").or_else(|_| std::env::var("XDG_CONFIG_HOME")).unwrap_or_else(|_| ".".into());
            Some(PathBuf::from(base).join("AiCoworker").join("aicoworker.db"))
        }
    }
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> Result<T, String> {
    // unit tests of other modules call log_action; they must never touch the real database
    if cfg!(test) {
        return Err("store disabled in tests".into());
    }
    let path = configured_path().ok_or_else(|| "store disabled (AICOWORKER_DB=off)".to_string())?;
    let mut guard = STORE.lock().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        *guard = Some(Store::open(&path));
    }
    match guard.as_mut().unwrap() {
        Ok(store) => Ok(f(store)),
        Err(e) => Err(e.clone()),
    }
}

/// Called from permission::log_action for every mutating/privileged action.
pub fn record(workspace: &str, tier: &str, action: &str, success: bool, detail: &str) {
    match with_store(|s| s.record(workspace, tier, action, success, detail)) {
        Ok(Ok(_)) => {}
        Ok(Err(e)) | Err(e) => {
            if !cfg!(test) && !e.contains("disabled") {
                eprintln!("Warning: audit database: {}", e);
            }
        }
    }
}

pub fn status() -> StoreStatus {
    match with_store(|s| s.status()) {
        Ok(st) => st,
        Err(e) => StoreStatus {
            path: configured_path().map(|p| p.display().to_string()).unwrap_or_default(),
            enabled: false,
            rows: 0,
            chain_ok: false,
            head_ok: false,
            problems: vec![e],
            alerts: vec![],
        },
    }
}

pub fn recent(workspace: Option<&str>, limit: i64) -> Result<Vec<StoreRow>, String> {
    with_store(|s| s.recent(workspace, limit))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aicw_store_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("aicoworker.db")
    }

    /// A second connection = "a script running as the same user".
    fn outsider(path: &Path) -> Connection {
        Connection::open(path).unwrap()
    }

    #[test]
    fn records_and_reads_back_with_a_valid_chain() {
        let p = temp_db("basic");
        let mut s = Store::open(&p).unwrap();
        s.record("C:\\ws", "mutating", "write_file: a.txt", true, "").unwrap();
        s.record("C:\\ws", "privileged", "delete_file: b.txt", false, "declined").unwrap();
        s.record("C:\\other", "mutating", "write_file: c.txt", true, "").unwrap();
        let st = s.status();
        assert!(st.chain_ok && st.head_ok && st.problems.is_empty(), "{:?}", st);
        assert_eq!(st.rows, 3);
        let ws_rows = s.recent(Some("C:\\ws"), 50).unwrap();
        assert_eq!(ws_rows.len(), 2);
        assert_eq!(ws_rows[0].action, "delete_file: b.txt", "newest first");
        assert_eq!(s.recent(None, 50).unwrap().len(), 3);
        // reopening (= app restart) keeps the chain valid
        drop(s);
        let s2 = Store::open(&p).unwrap();
        assert!(s2.status().chain_ok && s2.alerts.is_empty());
    }

    #[test]
    fn an_edited_row_is_detected() {
        let p = temp_db("edit");
        let mut s = Store::open(&p).unwrap();
        for i in 0..3 {
            s.record("ws", "privileged", &format!("delete_file: f{}.txt", i), true, "").unwrap();
        }
        outsider(&p).execute("UPDATE audit SET action = 'list_directory: .' WHERE id = 2", []).unwrap();
        let st = s.status();
        assert!(!st.chain_ok);
        assert!(st.problems.iter().any(|m| m.contains("row #2 was changed")), "{:?}", st.problems);
        // the next action writes an alert row first, then the action
        s.record("ws", "mutating", "write_file: x.txt", true, "").unwrap();
        assert_eq!(s.alerts.len(), 1);
        let rows = s.recent(None, 10).unwrap();
        assert_eq!(rows[1].action, "AUDIT_TAMPER_DETECTED");
        assert!(rows[1].detail.contains("row #2 was changed"), "{}", rows[1].detail);
        // the same break doesn't raise a new alert on every later action
        s.record("ws", "mutating", "write_file: y.txt", true, "").unwrap();
        assert_eq!(s.alerts.len(), 1);
    }

    #[test]
    fn deleted_rows_are_detected() {
        let p = temp_db("delete");
        let mut s = Store::open(&p).unwrap();
        for i in 0..4 {
            s.record("ws", "privileged", &format!("act {}", i), true, "").unwrap();
        }
        // a row in the middle
        outsider(&p).execute("DELETE FROM audit WHERE id = 2", []).unwrap();
        assert!(s.status().problems.iter().any(|m| m.contains("row #3 does not follow")));
        // the newest row: the chain of the remaining rows is still valid,
        // only the in-memory head notices
        let p2 = temp_db("delete_tail");
        let mut t = Store::open(&p2).unwrap();
        for i in 0..3 {
            t.record("ws", "privileged", &format!("act {}", i), true, "").unwrap();
        }
        outsider(&p2).execute("DELETE FROM audit WHERE id = 3", []).unwrap();
        let st = t.status();
        assert!(st.chain_ok, "remaining rows are a valid chain");
        assert!(!st.head_ok);
        assert!(st.problems.iter().any(|m| m.contains("rows after #2 were removed")), "{:?}", st.problems);
        // everything
        outsider(&p2).execute("DELETE FROM audit", []).unwrap();
        assert!(t.status().problems.iter().any(|m| m.contains("all rows were deleted")));
    }

    /// The attack a hash chain alone can't stop: rewrite the whole log and
    /// recompute every hash. The in-memory head catches it while the app runs.
    #[test]
    fn a_fully_rehashed_rewrite_is_caught_by_the_head_check() {
        let p = temp_db("rehash");
        let mut s = Store::open(&p).unwrap();
        s.record("ws", "privileged", "execute_command: del secrets.txt", true, "").unwrap();
        s.record("ws", "mutating", "write_file: notes.txt", true, "").unwrap();
        {
            let c = outsider(&p);
            c.execute("DELETE FROM audit", []).unwrap();
            let h1 = row_hash(GENESIS, 1, 100, "ws", "mutating", "write_file: notes.txt", true, "");
            c.execute(
                "INSERT INTO audit VALUES (1, 100, 'ws', 'mutating', 'write_file: notes.txt', 1, '', ?1, ?2)",
                params![GENESIS, h1],
            )
            .unwrap();
        }
        let st = s.status();
        assert!(st.chain_ok, "the forged log is internally consistent");
        assert!(!st.head_ok, "but it does not end with the row the app wrote");
        s.record("ws", "safe", "next", true, "").unwrap();
        assert_eq!(s.alerts.len(), 1, "{:?}", s.alerts);
    }

    #[test]
    fn rows_added_outside_the_app_are_detected() {
        let p = temp_db("insert");
        let mut s = Store::open(&p).unwrap();
        s.record("ws", "mutating", "a", true, "").unwrap();
        let (id, prev) = s.head.clone().unwrap();
        let h = row_hash(&prev, id + 1, 1, "ws", "safe", "fake", true, "");
        outsider(&p)
            .execute("INSERT INTO audit VALUES (?1, 1, 'ws', 'safe', 'fake', 1, '', ?2, ?3)", params![id + 1, prev, h])
            .unwrap();
        assert!(s.status().problems.iter().any(|m| m.contains("were added outside the app")));
    }

    #[test]
    fn a_broken_chain_at_startup_raises_an_alert() {
        let p = temp_db("startup");
        {
            let mut s = Store::open(&p).unwrap();
            s.record("ws", "mutating", "a", true, "").unwrap();
            s.record("ws", "mutating", "b", true, "").unwrap();
        }
        outsider(&p).execute("UPDATE audit SET success = 0 WHERE id = 1", []).unwrap();
        let s = Store::open(&p).unwrap();
        assert_eq!(s.alerts.len(), 1);
        assert!(s.alerts[0].contains("when the app started"));
        // the alert row is chained on, so the status still reports the old break
        assert!(s.status().problems.iter().any(|m| m.contains("row #1 was changed")));
        assert!(s.status().head_ok);
    }
}