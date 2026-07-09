use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

use rusqlite::Connection;

pub fn prompt(msg: &str) -> String {
    print!("{}", msg);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

pub fn ensure_parent(path: &Path) {
    let _ = std::fs::create_dir_all(path);
}

pub fn list_mtga_files(dir: &Path) -> Vec<std::fs::DirEntry> {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(d) => d.filter_map(|e| e.ok()).collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort_by_key(|e| std::fs::metadata(e.path()).ok().map(|m| m.len()).unwrap_or(0));
    entries.reverse();
    entries
}

pub fn open_mtga_db(path: &Path) -> Option<Connection> {
    if path.extension().and_then(|s| s.to_str()) != Some("mtga") {
        return None;
    }
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() < 500 * 1024 {
            return None;
        }
    }
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

pub fn get_table_names(conn: &Connection) -> Vec<String> {
    match conn.prepare("SELECT name FROM sqlite_master WHERE type='table'") {
        Ok(mut stmt) => stmt
            .query_map([], |row| row.get::<_, String>(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn load_loc_map(conn: &Connection) -> HashMap<i64, String> {
    let mut loc_map = HashMap::new();
    // Load plain text (Formatted = 0) first so it takes priority over formatted.
    if let Ok(mut stmt) = conn.prepare(
        "SELECT LocId, Loc FROM Localizations_enUS WHERE Formatted = 0",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let text: String = row.get(1)?;
            Ok((id, text))
        }) {
            for r in rows.flatten() {
                loc_map.insert(r.0, r.1);
            }
        }
    }
    // Also load formatted names (Formatted = 1) for LocIds that lack plain text.
    if let Ok(mut stmt) = conn.prepare(
        "SELECT LocId, Loc FROM Localizations_enUS WHERE Formatted = 1",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let text: String = row.get(1)?;
            Ok((id, text))
        }) {
            for r in rows.flatten() {
                let name = r.1;
                loc_map.entry(r.0).or_insert_with(|| {
                    name.replace("<nobr>", "").replace("</nobr>", "")
                });
            }
        }
    }
    loc_map
}
