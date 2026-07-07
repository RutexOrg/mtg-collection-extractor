use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

fn find_local_mtga_path() -> Option<std::path::PathBuf> {
    let paths = [
        r"C:\Program Files (x86)\Steam\steamapps\common\MTGA\MTGA_Data\Downloads\Raw",
        r"C:\Program Files\Wizards of the Coast\MTGA\MTGA_Data\Downloads\Raw",
        r"C:\Program Files (x86)\Wizards of the Coast\MTGA\MTGA_Data\Downloads\Raw",
    ];
    for p in &paths {
        let path = std::path::Path::new(p);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }
    None
}

fn load_local_mtga_database() -> Lookup {
    let raw_path = match find_local_mtga_path() {
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

        if !tables.contains(&"Cards".to_string()) || !tables.contains(&"Localizations".to_string()) {
            pb.inc(1);
            continue;
        }

        let mut loc_map: HashMap<i64, String> = HashMap::new();
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
        reqwest::header::HeaderValue::from_static("MTGA-Collection-Exporter/2.0"),
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

pub fn load_card_database(lookup_path: &Path) -> Lookup {
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

    let mut lookup = load_local_mtga_database();

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

pub fn build_name_index(db: &Lookup) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for (id, info) in db {
        map.insert(info.name.to_lowercase(), *id);
    }
    map
}
