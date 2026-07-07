use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    pub arena_id: u32,
    pub quantity: u32,
    pub name: String,
}

pub type AnchorList = Vec<Anchor>;

fn prompt(msg: &str) -> String {
    print!("{}", msg);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

pub fn load_anchors(anchor_path: &Path) -> Option<AnchorList> {
    if !anchor_path.exists() {
        return None;
    }
    let file = std::fs::File::open(anchor_path).ok()?;
    let saved: Vec<serde_json::Value> = serde_json::from_reader(file).ok()?;
    let anchors: AnchorList = saved
        .into_iter()
        .filter_map(|v| {
            let arr = v.as_array()?;
            Some(Anchor {
                arena_id: arr.get(0)?.as_u64()? as u32,
                quantity: arr.get(1)?.as_u64()? as u32,
                name: arr.get(2)?.as_str()?.to_string(),
            })
        })
        .collect();
    if anchors.is_empty() {
        None
    } else {
        Some(anchors)
    }
}

pub fn save_anchors(anchor_path: &Path, anchors: &AnchorList) {
    let data: Vec<Vec<serde_json::Value>> = anchors
        .iter()
        .map(|a| {
            vec![
                serde_json::json!(a.arena_id),
                serde_json::json!(a.quantity),
                serde_json::json!(a.name),
            ]
        })
        .collect();
    if let Some(parent) = anchor_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(f) = std::fs::File::create(anchor_path) {
        let _ = serde_json::to_writer_pretty(f, &data);
    }
}

pub fn interactive_anchors(name_to_id: &HashMap<String, u32>, anchor_path: &Path) -> AnchorList {
    if let Some(saved) = load_anchors(anchor_path) {
        println!("\n[Previous Anchors Found]");
        for (i, a) in saved.iter().enumerate() {
            println!("  {}. {} (x{})", i + 1, a.name, a.quantity);
        }
        let response = prompt("  Use these? [Y/n]: ").trim().to_lowercase();
        if response.is_empty() || response == "y" || response == "yes" {
            return saved;
        }
    }

    println!(
        "\n[Setup] Enter 5 unique owned cards (Rares/Mythics best) to calibrate scanner."
    );

    let mut anchors = AnchorList::new();
    while anchors.len() < 5 {
        println!("\nCard #{} (Enter empty to finish):", anchors.len() + 1);
        let name_input = prompt("  Name: ");

        if name_input.is_empty() {
            if !anchors.is_empty() {
                break;
            }
            println!("  Required.");
            continue;
        }

        let search = name_input.to_lowercase();
        let cid = name_to_id.get(&search).copied();

        let (final_cid, final_name) = match cid {
            Some(id) => (id, name_input),
            None => {
                let names: Vec<&String> = name_to_id.keys().collect();
                let fuzzy = fuzzy_find(&search, &names, 5, 0.5);

                if fuzzy.is_empty() {
                    println!("  Not found. Check spelling.");
                    continue;
                }

                let chosen = if fuzzy.len() == 1 {
                    println!("  Assuming: {}", title_case(&fuzzy[0]));
                    fuzzy[0].clone()
                } else {
                    println!("  Did you mean?");
                    for (i, m) in fuzzy.iter().enumerate() {
                        println!("    {}. {}", i + 1, title_case(m));
                    }
                    let sel = prompt("  Select #: ");
                    let idx: usize = match sel.parse::<usize>() {
                        Ok(n) if n >= 1 && n <= fuzzy.len() => n - 1,
                        _ => continue,
                    };
                    fuzzy[idx].clone()
                };

                match name_to_id.get(&chosen.to_lowercase()).copied() {
                    Some(id) => (id, chosen),
                    None => continue,
                }
            }
        };

        let qty_input = prompt(&format!("  Quantity of '{}': ", final_name));
        let qty: u32 = match qty_input.parse() {
            Ok(n) if n >= 1 => n,
            _ => {
                println!("  Invalid quantity.");
                continue;
            }
        };

        anchors.push(Anchor {
            arena_id: final_cid,
            quantity: qty,
            name: final_name,
        });
    }

    if !anchors.is_empty() {
        save_anchors(anchor_path, &anchors);
    }

    anchors
}

fn fuzzy_find(query: &str, candidates: &[&String], max_results: usize, cutoff: f64) -> Vec<String> {
    use strsim::normalized_levenshtein;

    let mut scored: Vec<(f64, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let score = normalized_levenshtein(query, c);
            if score >= cutoff {
                Some((score, *c))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(max_results);
    scored.into_iter().map(|(_, s)| s.clone()).collect()
}

fn title_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut new_word = true;
    for c in s.chars() {
        if c.is_whitespace() {
            new_word = true;
            result.push(c);
        } else if new_word {
            result.extend(c.to_uppercase());
            new_word = false;
        } else {
            result.extend(c.to_lowercase());
        }
    }
    result
}
