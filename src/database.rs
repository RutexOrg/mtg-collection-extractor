use std::collections::HashMap;
use std::fs;
use std::iter::once;
use std::path::{Path, PathBuf};

use indicatif::ProgressBar;
use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardInfo {
    pub name: String,
    #[serde(default)]
    pub set: String,
    #[serde(default)]
    pub collector_number: String,
    #[serde(default)]
    pub rarity: u8,
}

pub type Lookup = HashMap<u32, CardInfo>;

fn read_registry_install_path() -> Option<String> {
    let key_paths = [
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Steam App 2141910",
        "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Steam App 2141910",
    ];
    for key in &key_paths {
        let key_wide: Vec<u16> = key.encode_utf16().chain(once(0)).collect();
        let value_wide: Vec<u16> = "InstallLocation\0".encode_utf16().collect();
        let mut hkey = HKEY::default();

        let result = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR::from_raw(key_wide.as_ptr()),
                0,
                KEY_READ,
                &mut hkey,
            )
        };
        if result != ERROR_SUCCESS {
            continue;
        }

        let mut data_size: u32 = 0;
        let result = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR::from_raw(value_wide.as_ptr()),
                None,
                None,
                None,
                Some(&mut data_size as *mut u32),
            )
        };
        if result == ERROR_SUCCESS && data_size > 0 {
            let len = (data_size / 2) as usize;
            let mut buf = vec![0u16; len];
            let result = unsafe {
                RegQueryValueExW(
                    hkey,
                    PCWSTR::from_raw(value_wide.as_ptr()),
                    None,
                    None,
                    Some(buf.as_mut_ptr() as *mut u8),
                    Some(&mut data_size as *mut u32),
                )
            };
            if result == ERROR_SUCCESS {
                let null_pos = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let path = String::from_utf16_lossy(&buf[..null_pos]);
                let _ = unsafe { RegCloseKey(hkey) };
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
        let _ = unsafe { RegCloseKey(hkey) };
    }
    None
}

fn raw_path_from_root(root: &Path) -> Option<PathBuf> {
    let raw = root.join("MTGA_Data").join("Downloads").join("Raw");
    if raw.exists() {
        Some(raw)
    } else {
        None
    }
}

fn find_local_mtga_path(custom: Option<&Path>, process_path: Option<&Path>) -> Option<PathBuf> {
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

    // 2. Path from running MTGA.exe process
    if let Some(p) = process_path {
        if let Some(parent) = p.parent() {
            if let Some(raw) = raw_path_from_root(parent) {
                return Some(raw);
            }
        }
    }

    // 3. Registry (WinAPI)
    if let Some(reg_path) = read_registry_install_path() {
        let p = Path::new(&reg_path);
        if p.join("MTGA.exe").exists() {
            if let Some(raw) = raw_path_from_root(p) {
                return Some(raw);
            }
        }
    }

    None
}

fn load_local_mtga_database(mtga_path: Option<&Path>, process_path: Option<&Path>) -> Lookup {
    let raw_path = match find_local_mtga_path(mtga_path, process_path) {
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

        pb.println(format!("  Reading: {}", entry.path().display()));
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
        let has_rarity = cols.contains(&"Rarity".to_string());

        let set_col = if has_set { "ExpansionCode" } else { "NULL" };
        let cn_col = if has_cn { "CollectorNumber" } else { "NULL" };
        let rarity_col = if has_rarity { "Rarity" } else { "0" };
        let query = format!(
            "SELECT GrpId, TitleId, {}, {}, {} FROM Cards",
            set_col, cn_col, rarity_col
        );

        if let Ok(mut stmt) = conn.prepare(&query) {
            if let Ok(rows) = stmt.query_map([], |row| {
                let grp_id: i64 = row.get(0)?;
                let title_id: i64 = row.get(1)?;
                let set_code: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
                let cn: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
                let rarity: u8 = row.get::<_, Option<i64>>(4)?.unwrap_or(0) as u8;
                Ok((grp_id, title_id, set_code, cn, rarity))
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
                                rarity: r.4,
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

fn saved_mtga_path_file(data_dir: &Path) -> PathBuf {
    data_dir.join("mtga_path.txt")
}

fn load_saved_mtga_path(data_dir: &Path) -> Option<PathBuf> {
    let path = saved_mtga_path_file(data_dir);
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() { None } else { Some(PathBuf::from(trimmed)) }
}

fn save_mtga_path(data_dir: &Path, raw_path: &Path) {
    let path_file = saved_mtga_path_file(data_dir);
    crate::util::ensure_parent(&path_file);
    let _ = fs::write(path_file, raw_path.to_string_lossy().as_ref());
}

fn prompt_mtga_path(data_dir: &Path) -> Option<PathBuf> {
    println!("\nMTGA installation not found at default locations.");
    println!("  Enter the path to your MTGA folder (e.g. D:/Games/MTGA)");
    let input = crate::util::prompt("  Path (or press Enter to cancel): ");
    if input.is_empty() {
        return None;
    }
    let p = PathBuf::from(&input);
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

pub fn load_card_database(data_dir: &Path, mtga_path: Option<&Path>, process_path: Option<&Path>) -> Lookup {
    let effective = mtga_path.map(|p| p.to_path_buf())
        .or_else(|| load_saved_mtga_path(data_dir));
    let lookup = load_local_mtga_database(effective.as_deref(), process_path);

    if lookup.is_empty() {
        if let Some(root) = prompt_mtga_path(data_dir) {
            return load_local_mtga_database(Some(&root), None);
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

pub fn build_display_names(db: &Lookup) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for info in db.values() {
        map.insert(info.name.to_lowercase(), info.name.clone());
    }
    map
}
