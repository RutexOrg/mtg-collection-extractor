use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::database::Lookup;

#[derive(Debug, Clone)]
pub struct CollectionEntry {
    pub count: u32,
    pub name: String,
    pub set: String,
    pub cn: String,
}

pub fn extract_collection(
    raw: &HashMap<u32, u32>,
    db: &Lookup,
) -> Vec<CollectionEntry> {
    let mut merged: HashMap<(String, String), CollectionEntry> = HashMap::new();

    for (cid, qty) in raw {
        if let Some(info) = db.get(cid) {
            let key = (info.name.clone(), info.set.clone());
            let entry = merged.entry(key).or_insert(CollectionEntry {
                count: 0,
                name: info.name.clone(),
                set: info.set.clone(),
                cn: info.collector_number.clone(),
            });
            entry.count += qty;
        }
    }

    let mut list: Vec<CollectionEntry> = merged.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.set.cmp(&b.set)));
    list
}

fn ensure_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
}

pub fn export_txt(path: &Path, entries: &[CollectionEntry]) {
    ensure_parent(path);
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
    ensure_parent(path);
    let data: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
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
    ensure_parent(path);
    let mut wtr = csv::Writer::from_path(path).expect("Failed to create CSV writer");
    let _ = wtr.write_record(&["Count", "Name", "Edition", "Condition", "Language", "Foil", "Tag"]);
    for e in entries {
        let _ = wtr.write_record(&[
            e.count.to_string(),
            e.name.clone(),
            e.set.clone(),
            "Near Mint".to_string(),
            "English".to_string(),
            String::new(),
            String::new(),
        ]);
    }
    let _ = wtr.flush();
}

pub fn do_export(cfg: &Config, raw: &HashMap<u32, u32>, db: &Lookup) {
    let entries = extract_collection(raw, db);
    println!("\n[Success] Found {} unique entries.", entries.len());

    export_txt(&cfg.output_txt, &entries);
    export_json(&cfg.output_json, &entries);
    export_csv(&cfg.output_csv, &entries);

    println!("\nExport complete!");
    println!("Files saved to: {}", cfg.output_dir.display());

    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(&cfg.output_txt)
        .spawn();
}
