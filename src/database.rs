use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use indicatif::ProgressBar;

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
            .args(["query", key, "/v", "InstallLocation"])
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
        if c_exists && c.file_name().and_then(|s| s.to_str()) == Some("Raw")
            && c.join("..").join("..").join("MTGA.exe").exists() {
            return Some(c.to_path_buf());
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

    let entries = crate::util::list_mtga_files(&raw_path);
    let total = entries.len();
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
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
        let conn = match crate::util::open_mtga_db(&path) {
            Some(c) => c,
            None => {
                pb.inc(1);
                continue;
            }
        };

        let tables = crate::util::get_table_names(&conn);
        if !tables.contains(&"Cards".to_string()) {
            pb.inc(1);
            continue;
        }

        if !tables.contains(&"Localizations_enUS".to_string()) {
            pb.inc(1);
            continue;
        }

        let loc_map = crate::util::load_loc_map(&conn);

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
        indicatif::ProgressStyle::default_bar()
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

fn saved_mtga_path_file(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("mtga_path.txt")
}

fn load_saved_mtga_path(data_dir: &Path) -> Option<std::path::PathBuf> {
    let path = saved_mtga_path_file(data_dir);
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() { None } else { Some(std::path::PathBuf::from(trimmed)) }
}

fn save_mtga_path(data_dir: &Path, raw_path: &Path) {
    let path_file = saved_mtga_path_file(data_dir);
    crate::util::ensure_parent(&path_file);
    let _ = std::fs::write(path_file, raw_path.to_string_lossy().as_ref());
}

fn prompt_mtga_path(data_dir: &Path) -> Option<std::path::PathBuf> {
    println!("\nMTGA installation not found at default locations.");
    println!("  Enter the path to your MTGA folder (e.g. D:/Games/MTGA)");
    let input = crate::util::prompt("  Path (or press Enter to download from Scryfall): ");
    if input.is_empty() {
        return None;
    }
    let p = std::path::PathBuf::from(&input);
    let raw = p.join("MTGA_Data").join("Downloads").join("Raw");
    if raw.exists() {
        save_mtga_path(data_dir, &p);
        Some(p)
    } else {
        println!("  Path not found: {}", raw.display());
        println!("  Make sure the game files are downloaded.");
        None
    }
}

pub fn load_card_database(data_dir: &Path, mtga_path: Option<&Path>) -> Lookup {
    // Priority: CLI arg > saved path > auto-detect > interactive > Scryfall
    let effective = mtga_path.map(|p| p.to_path_buf())
        .or_else(|| load_saved_mtga_path(data_dir));
    let mut lookup = load_local_mtga_database(effective.as_deref());

    if lookup.is_empty() {
        lookup = load_local_mtga_database(None);
    }

    if lookup.is_empty() {
        if let Some(root) = prompt_mtga_path(data_dir) {
            lookup = load_local_mtga_database(Some(&root));
        }
    }

    if lookup.is_empty() {
        println!("\n[Warn] Local database not found. Downloading from Scryfall...");
        lookup = fetch_scryfall_database();
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
