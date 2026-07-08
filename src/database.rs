use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardInfo {
    pub name: String,
    #[serde(default)]
    pub set: String,
    #[serde(default)]
    pub collector_number: String,
}

pub type Lookup = HashMap<u32, CardInfo>;

fn read_registry_install_path() -> Option<String> {
    let keys = [
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 2141910",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 2141910",
    ];
    for key in &keys {
        let output = std::process::Command::new("reg")
            .args(&["query", key, "/v", "InstallLocation"])
            .output()
            .ok()?;
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let trimmed = line.trim();
            if let Some(pos) = trimmed.find("REG_SZ") {
                let path = trimmed[pos + 6..].trim();
                if !path.is_empty() {
                    return Some(path.to_string());
                }
            }
        }
    }
    None
}

fn raw_path_from_root(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let raw = root.join("MTGA_Data").join("Downloads").join("Raw");
    if raw.exists() {
        Some(raw)
    } else {
        None
    }
}

fn find_local_mtga_path(custom: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    // 1. Custom path from CLI args
    if let Some(c) = custom {
        let c_exists = c.exists();
        if c_exists && c.join("MTGA.exe").exists() {
            if let Some(raw) = raw_path_from_root(c) {
                return Some(raw);
            }
        }
        if c_exists && c.file_name().and_then(|s| s.to_str()) == Some("Raw") {
            if c.join("..").join("..").join("MTGA.exe").exists() {
                return Some(c.to_path_buf());
            }
        }
        if c_exists {
            return Some(c.to_path_buf());
        }
    }

    let standard_roots = [
        r"C:\Program Files (x86)\Steam\steamapps\common\MTGA",
        r"D:\Program Files (x86)\Steam\steamapps\common\MTGA",
        r"C:\Program Files\Steam\steamapps\common\MTGA",
        r"C:\Program Files\Wizards of the Coast\MTGA",
        r"C:\Program Files (x86)\Wizards of the Coast\MTGA",
        r"D:\SteamLibrary\steamapps\common\MTGA",
        r"E:\SteamLibrary\steamapps\common\MTGA",
        r"F:\SteamLibrary\steamapps\common\MTGA",
        r"G:\SteamLibrary\steamapps\common\MTGA",
        r"C:\MTGA",
        r"D:\MTGA",
        r"C:\Games\MTGA",
        r"D:\Games\MTGA",
    ];

    for root in &standard_roots {
        let p = std::path::Path::new(root);
        if p.join("MTGA.exe").exists() && p.join("MTGA_Data").exists() {
            if let Some(raw) = raw_path_from_root(p) {
                return Some(raw);
            }
        }
    }

    if let Some(reg_path) = read_registry_install_path() {
        let p = std::path::Path::new(&reg_path);
        if p.join("MTGA.exe").exists() {
            if let Some(raw) = raw_path_from_root(p) {
                return Some(raw);
            }
        }
    }

    None
}

fn load_local_mtga_database(mtga_path: Option<&std::path::Path>) -> Lookup {
    let raw_path = match find_local_mtga_path(mtga_path) {
        Some(p) => p,
        None => {
            println!("Local MTGA installation not found.");
            return Lookup::new();
        }
    };

    println!("Scanning local MTGA files in {}...", raw_path.display());

    let mut lookup = Lookup::new();

    let mut entries: Vec<_> = match std::fs::read_dir(&raw_path) {
        Ok(d) => d.filter_map(|e| e.ok()).collect(),
        Err(_) => return lookup,
    };
    entries.sort_by_key(|e| std::fs::metadata(e.path()).ok().map(|m| m.len()).unwrap_or(0));
    entries.reverse();

    let total = entries.len();
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:>10} [{bar:30}] {percent:>3}% {msg}")
            .unwrap()
            .progress_chars("█░"),
    );
    pb.set_prefix("Local DB:");

    for entry in entries.iter() {
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        let display = if fname_str.len() > 10 {
            format!("{}...", &fname_str[..10])
        } else {
            fname_str.to_string()
        };
        pb.set_message(format!("Checking {}...", display));

        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("mtga") {
            pb.inc(1);
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() < 500 * 1024 {
                pb.inc(1);
                continue;
            }
        }

        let conn = match rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(_) => {
                pb.inc(1);
                continue;
            }
        };

        let tables: Vec<String> = match conn.prepare("SELECT name FROM sqlite_master WHERE type='table'") {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default(),
            Err(_) => {
                pb.inc(1);
                continue;
            }
        };

        if !tables.contains(&"Cards".to_string()) {
            pb.inc(1);
            continue;
        }

        let has_new_loc = tables.contains(&"Localizations_enUS".to_string());
        let has_old_loc = tables.contains(&"Localizations".to_string());
        if !has_new_loc && !has_old_loc {
            pb.inc(1);
            continue;
        }

        let mut loc_map: HashMap<i64, String> = HashMap::new();

        // New schema: Localizations_enUS (LocId, Formatted, Loc)
        if has_new_loc {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT LocId, Loc FROM Localizations_enUS WHERE Formatted = 1",
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
        }

        // Old schema fallback: Localizations (Id, Text)
        if loc_map.is_empty() && has_old_loc {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT Id, Text FROM Localizations WHERE Format LIKE '%en-US%' OR Format IS NULL",
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
            if loc_map.is_empty() {
                if let Ok(mut stmt) = conn.prepare("SELECT Id, Text FROM Localizations") {
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
            }
        }

        let cols: Vec<String> = match conn.prepare("PRAGMA table_info(Cards)") {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get::<_, String>(1))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default(),
            Err(_) => {
                pb.inc(1);
                continue;
            }
        };
        let has_set = cols.contains(&"ExpansionCode".to_string());
        let has_cn = cols.contains(&"CollectorNumber".to_string());

        let query = if has_set && has_cn {
            "SELECT GrpId, TitleId, ExpansionCode, CollectorNumber FROM Cards"
        } else if has_set {
            "SELECT GrpId, TitleId, ExpansionCode, NULL FROM Cards"
        } else if has_cn {
            "SELECT GrpId, TitleId, NULL, CollectorNumber FROM Cards"
        } else {
            "SELECT GrpId, TitleId, NULL, NULL FROM Cards"
        };

        if let Ok(mut stmt) = conn.prepare(query) {
            if let Ok(rows) = stmt.query_map([], |row| {
                let grp_id: i64 = row.get(0)?;
                let title_id: i64 = row.get(1)?;
                let set_code: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
                let cn: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
                Ok((grp_id, title_id, set_code, cn))
            }) {
                for r in rows.flatten() {
                    let grp_id = r.0 as u32;
                    let title_id = r.1;
                    if let Some(name) = loc_map.get(&title_id) {
                        lookup.insert(
                            grp_id,
                            CardInfo {
                                name: name.clone(),
                                set: r.2.to_uppercase(),
                                collector_number: r.3,
                            },
                        );
                    }
                }
            }
        }

        if lookup.len() > 1000 {
            pb.finish_with_message("Done");
            println!("  Loaded {} cards locally.", lookup.len());
            return lookup;
        }

        pb.inc(1);
    }

    pb.finish_with_message("Done");
    println!("  Loaded {} cards locally.", lookup.len());
    lookup
}

fn fetch_scryfall_database() -> Lookup {
    println!("Fetching card data from Scryfall API...");
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("MTGA-Collection-Extractor/2.0"),
    );

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .default_headers(headers.clone())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            println!("Failed to create HTTP client: {}", e);
            return Lookup::new();
        }
    };

    let bulk_meta: serde_json::Value = match client
        .get("https://api.scryfall.com/bulk-data/default-cards")
        .send()
    {
        Ok(r) => match r.json() {
            Ok(v) => v,
            Err(e) => {
                println!("Scryfall metadata parse failed: {}", e);
                return Lookup::new();
            }
        },
        Err(e) => {
            println!("Scryfall metadata request failed: {}", e);
            return Lookup::new();
        }
    };

    let download_uri = match bulk_meta["download_uri"].as_str() {
        Some(u) => u.to_string(),
        None => {
            println!("Scryfall response missing download_uri");
            return Lookup::new();
        }
    };

    let mut response = match client
        .get(&download_uri)
        .timeout(std::time::Duration::from_secs(300))
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            println!("Scryfall card data download failed: {}", e);
            return Lookup::new();
        }
    };

    let total_size = response.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:>10} [{bar:30}] {bytes:>7}/{total_bytes} {msg}")
            .unwrap()
            .progress_chars("█░"),
    );
    pb.set_prefix("Scryfall:");
    pb.set_message("Downloading...");

    use std::io::Read;
    let mut raw = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = match response.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                println!("\nRead error: {}", e);
                return Lookup::new();
            }
        };
        raw.extend_from_slice(&buf[..n]);
        pb.inc(n as u64);
    }
    pb.finish_with_message(format!("{:.1} MB", raw.len() as f64 / 1_000_000.0));

    if raw.is_empty() {
        println!("Downloaded empty response.");
        return Lookup::new();
    }

    let cards_data: Vec<serde_json::Value> = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => {
            println!("Scryfall card data parse failed: {}", e);
            return Lookup::new();
        }
    };

    let mut lookup = Lookup::new();
    for card in &cards_data {
        if let Some(arena_id) = card["arena_id"].as_u64() {
            let aid = arena_id as u32;
            let name = card["name"].as_str().unwrap_or("Unknown").to_string();
            let set = card["set"].as_str().unwrap_or("").to_uppercase();
            let cn = card["collector_number"].as_str().unwrap_or("").to_string();
            lookup.insert(
                aid,
                CardInfo {
                    name,
                    set,
                    collector_number: cn,
                },
            );
        }
    }

    println!("Loaded {} cards from Scryfall.", lookup.len());
    lookup
}

fn prompt(msg: &str) -> String {
    print!("{}", msg);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

fn saved_mtga_path_file(lookup_path: &Path) -> std::path::PathBuf {
    lookup_path.parent().unwrap_or(Path::new("data")).join("mtga_path.txt")
}

fn load_saved_mtga_path(lookup_path: &Path) -> Option<std::path::PathBuf> {
    let path = saved_mtga_path_file(lookup_path);
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() { None } else { Some(std::path::PathBuf::from(trimmed)) }
}

fn save_mtga_path(lookup_path: &Path, raw_path: &Path) {
    if let Some(parent) = lookup_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(saved_mtga_path_file(lookup_path), raw_path.to_string_lossy().as_ref());
}

fn prompt_mtga_path(lookup_path: &Path) -> Option<std::path::PathBuf> {
    println!("\nMTGA installation not found at default locations.");
    println!("  Enter the path to your MTGA folder (e.g. D:/Games/MTGA)");
    let input = prompt("  Path (or press Enter to download from Scryfall): ");
    if input.is_empty() {
        return None;
    }
    let p = std::path::PathBuf::from(&input);
    let raw = p.join("MTGA_Data").join("Downloads").join("Raw");
    if raw.exists() {
        save_mtga_path(lookup_path, &p);
        Some(p)
    } else {
        println!("  Path not found: {}", raw.display());
        println!("  Make sure the game files are downloaded.");
        None
    }
}

pub fn load_card_database(lookup_path: &Path, mtga_path: Option<&Path>) -> Lookup {
    if lookup_path.exists() {
        println!("Loading cached database...");
        match std::fs::File::open(lookup_path) {
            Ok(f) => {
                let data: HashMap<String, CardInfo> = match serde_json::from_reader(f) {
                    Ok(d) => d,
                    Err(_) => {
                        println!("Cache corrupted.");
                        return Lookup::new();
                    }
                };
                let mut lookup = Lookup::new();
                for (k, v) in &data {
                    if let Ok(id) = k.parse::<u32>() {
                        lookup.insert(id, v.clone());
                    }
                }
                if !lookup.is_empty() {
                    println!("Loaded {} cards from cache.", lookup.len());
                    return lookup;
                }
            }
            Err(_) => {
                println!("Cache corrupted.");
            }
        }
    }

    let mut lookup = load_local_mtga_database(mtga_path);

    if lookup.is_empty() && mtga_path.is_none() {
        // Try saved path from previous interactive session
        if let Some(saved) = load_saved_mtga_path(lookup_path) {
            lookup = load_local_mtga_database(Some(&saved));
        }
    }

    if lookup.is_empty() && mtga_path.is_none() {
        // Interactive prompt
        if let Some(root) = prompt_mtga_path(lookup_path) {
            lookup = load_local_mtga_database(Some(&root));
        }
    }

    if lookup.is_empty() {
        println!("\n[Warn] Local database not found. Downloading from Scryfall...");
        lookup = fetch_scryfall_database();
    }

    if !lookup.is_empty() {
        let string_keyed: HashMap<String, &CardInfo> =
            lookup.iter().map(|(k, v)| (k.to_string(), v)).collect();
        if let Some(parent) = lookup_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(f) = std::fs::File::create(lookup_path) {
            if serde_json::to_writer(f, &string_keyed).is_ok() {
                println!("Database cached.");
            }
        }
    }

    lookup
}

pub fn load_local_name_fallback(mtga_path: Option<&std::path::Path>) -> HashMap<u32, String> {
    let raw_path = match find_local_mtga_path(mtga_path) {
        Some(p) => p,
        None => return HashMap::new(),
    };

    let mut entries: Vec<_> = match std::fs::read_dir(&raw_path) {
        Ok(d) => d.filter_map(|e| e.ok()).collect(),
        Err(_) => return HashMap::new(),
    };
    entries.sort_by_key(|e| std::fs::metadata(e.path()).ok().map(|m| m.len()).unwrap_or(0));
    entries.reverse();

    let mut fallback = HashMap::new();

    for entry in &entries {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("mtga") {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() < 500 * 1024 {
                continue;
            }
        }

        let conn = match rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let tables: Vec<String> = match conn.prepare("SELECT name FROM sqlite_master WHERE type='table'") {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default(),
            Err(_) => continue,
        };

        if !tables.contains(&"Cards".to_string()) {
            continue;
        }

        let has_new_loc = tables.contains(&"Localizations_enUS".to_string());
        let has_old_loc = tables.contains(&"Localizations".to_string());
        if !has_new_loc && !has_old_loc {
            continue;
        }

        let mut loc_map: HashMap<i64, String> = HashMap::new();

        // New schema: Localizations_enUS (LocId, Formatted, Loc)
        if has_new_loc {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT LocId, Loc FROM Localizations_enUS WHERE Formatted = 1",
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
        }

        // Old schema fallback: Localizations (Id, Text)
        if loc_map.is_empty() && has_old_loc {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT Id, Text FROM Localizations WHERE Format LIKE '%en-US%' OR Format IS NULL",
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
            if loc_map.is_empty() {
                if let Ok(mut stmt) = conn.prepare("SELECT Id, Text FROM Localizations") {
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
            }
        }

        if let Ok(mut stmt) = conn.prepare("SELECT GrpId, TitleId FROM Cards") {
            if let Ok(rows) = stmt.query_map([], |row| {
                let grp_id: i64 = row.get(0)?;
                let title_id: i64 = row.get(1)?;
                Ok((grp_id, title_id))
            }) {
                for r in rows.flatten() {
                    if let Some(name) = loc_map.get(&r.1) {
                        fallback.entry(r.0 as u32).or_insert_with(|| name.clone());
                    }
                }
            }
        }

        if fallback.len() > 1000 {
            return fallback;
        }
    }

    fallback
}

pub fn build_name_index(db: &Lookup) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for (id, info) in db {
        map.insert(info.name.to_lowercase(), *id);
    }
    map
}
