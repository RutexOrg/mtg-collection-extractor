use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::database::Lookup;

#[derive(Debug, Clone)]
pub struct CollectionEntry {
    pub card_id: u32,
    pub count: u32,
    pub name: String,
    pub set: String,
    pub cn: String,
}

#[derive(Debug, Clone)]
pub struct UnknownEntry {
    pub arena_id: u32,
    pub count: u32,
    pub name: Option<String>,
}

pub fn extract_collection(
    raw: &HashMap<u32, u32>,
    db: &Lookup,
) -> (Vec<CollectionEntry>, Vec<UnknownEntry>) {
    let mut merged: HashMap<(String, String), CollectionEntry> = HashMap::new();
    let mut unknown: HashMap<u32, u32> = HashMap::new();

    for (cid, qty) in raw {
        if let Some(info) = db.get(cid) {
            let key = (info.name.clone(), info.set.clone());
            let entry = merged.entry(key).or_insert(CollectionEntry {
                card_id: *cid,
                count: 0,
                name: info.name.clone(),
                set: info.set.clone(),
                cn: info.collector_number.clone(),
            });
            entry.count += qty;
        } else {
            *unknown.entry(*cid).or_insert(0) += qty;
        }
    }

    let mut list: Vec<CollectionEntry> = merged.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.set.cmp(&b.set)));

    let mut unknown_list: Vec<UnknownEntry> = unknown
        .into_iter()
        .map(|(arena_id, count)| UnknownEntry {
            arena_id,
            count,
            name: None,
        })
        .collect();
    unknown_list.sort_by_key(|e| e.arena_id);

    (list, unknown_list)
}

pub fn export_txt(path: &Path, entries: &[CollectionEntry]) {
    let mut output = String::new();
    for e in entries {
        if e.set.is_empty() {
            output.push_str(&format!("{} {}\n", e.count, e.name));
        } else {
            output.push_str(&format!("{} {} ({})\n", e.count, e.name, e.set));
        }
    }
    let _ = std::fs::write(path, output);
}

pub fn export_json(path: &Path, entries: &[CollectionEntry]) {
    let data: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "card_id": e.card_id,
                "count": e.count,
                "name": e.name,
                "set": e.set,
                "cn": e.cn,
            })
        })
        .collect();
    if let Ok(f) = std::fs::File::create(path) {
        let _ = serde_json::to_writer_pretty(f, &data);
    }
}

pub fn export_csv(path: &Path, entries: &[CollectionEntry]) {

    let mut wtr = csv::Writer::from_path(path).expect("Failed to create CSV writer");
    let _ = wtr.write_record(["card_id", "count", "name", "set", "cn"]);
    for e in entries {
        let _ = wtr.write_record(&[e.card_id.to_string(), e.count.to_string(), e.name.clone(), e.set.clone(), e.cn.clone()]);
    }
    let _ = wtr.flush();
}

pub fn export_unknown_txt(path: &Path, entries: &[UnknownEntry]) {

    let mut output = String::new();
    for e in entries {
        let label = match &e.name {
            Some(n) => n.clone(),
            None => format!("arena_id:{}", e.arena_id),
        };
        output.push_str(&format!("{} {}\n", e.count, label));
    }
    let _ = std::fs::write(path, output);
}

pub fn export_unknown_json(path: &Path, entries: &[UnknownEntry]) {

    let data: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            let mut obj = serde_json::json!({
                "count": e.count,
                "arena_id": e.arena_id,
            });
            if let Some(ref name) = e.name {
                obj["name"] = serde_json::json!(name);
            }
            obj
        })
        .collect();
    if let Ok(f) = std::fs::File::create(path) {
        let _ = serde_json::to_writer_pretty(f, &data);
    }
}

pub fn export_unknown_csv(path: &Path, entries: &[UnknownEntry]) {

    let mut wtr = csv::Writer::from_path(path).expect("Failed to create CSV writer");
    let _ = wtr.write_record(["Count", "Name", "ArenaID"]);
    for e in entries {
        let _ = wtr.write_record(&[
            e.count.to_string(),
            e.name.clone().unwrap_or_default(),
            e.arena_id.to_string(),
        ]);
    }
    let _ = wtr.flush();
}

pub fn do_export(cfg: &Config, raw: &HashMap<u32, u32>, db: &Lookup) {
    let (entries, unknown) = extract_collection(raw, db);
    crate::util::ensure_parent(&cfg.output_dir);
    println!("\n[Success] Found {} unique entries.", entries.len());

    export_txt(&cfg.output_txt, &entries);
    export_json(&cfg.output_json, &entries);
    export_csv(&cfg.output_csv, &entries);

    if !unknown.is_empty() {
        println!("[Warn] {} cards not found in database (unknown).", unknown.len());
        export_unknown_txt(&cfg.output_unknown_txt, &unknown);
        export_unknown_json(&cfg.output_unknown_json, &unknown);
        export_unknown_csv(&cfg.output_unknown_csv, &unknown);
    }

    println!("\nExport complete!");
    println!("Files saved to: {}", cfg.output_dir.display());

    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(&cfg.output_txt)
        .spawn();
}
