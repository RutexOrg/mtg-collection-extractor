mod config;
mod database;
mod anchor;
mod memory;
mod export;

use std::io::{self, Write};
use indicatif::{ProgressBar, ProgressStyle};

fn main() {
    println!("MTGA Collection Extractor");

    let cfg = config::Config::from_cli();
    println!("Output Folder: {}\n", cfg.output_dir.display());

    // Load card database
    let db = database::load_card_database(&cfg.lookup_file, cfg.mtga_path.as_deref());
    if db.is_empty() {
        println!("Database init failed.");
        wait_exit();
        return;
    }

    if cfg.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cfg.threads)
            .build_global()
            .ok();
    }

    let mem_source = match memory::MemorySource::from_process() {
        Some(m) => m,
        None => {
            println!("MTG Arena not running. Open game and 'Collections' tab first.");
            wait_exit();
            return;
        }
    };

    let name_to_id = database::build_name_index(&db);
    let anchors = anchor::interactive_anchors(&name_to_id, &cfg.anchor_file);
    if anchors.is_empty() {
        return;
    }

    // Pattern scan for each anchor
    println!("\nScanning memory for collection data...");
    let total_anchors = anchors.len();
    let total_regions = mem_source.region_infos.len();

    let pb = ProgressBar::new(total_regions as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:>10} [{bar:30}] {pos:>4}/{len} {msg}")
            .unwrap()
            .progress_chars("█░"),
    );
    pb.set_prefix("Scan:");

    let mut all_matches = Vec::new();
    for (i, anchor) in anchors.iter().enumerate() {
        let display = if anchor.name.len() > 24 {
            format!("{}..", &anchor.name[..24])
        } else {
            anchor.name.clone()
        };

        pb.reset_elapsed();
        pb.set_length(total_regions as u64);
        pb.set_position(0);
        pb.set_message(format!("{}/{} {}", i + 1, total_anchors, display));

        let pattern = make_pattern(anchor.arena_id, anchor.quantity);
        let matches = mem_source.pattern_scan(&pattern, &pb);

        pb.set_length(total_anchors as u64);
        pb.set_position(i as u64 + 1);

        if !matches.is_empty() {
            all_matches.extend(matches);
            pb.println(format!("  ✓ {}", display));
            if anchor.quantity > 1 {
                pb.set_message("Done");
                break;
            }
        } else {
            pb.println(format!("  ✗ {}", display));
        }
    }
    pb.finish_with_message("Done");

    if all_matches.is_empty() {
        println!("\nScanner failed to locate collection from anchors.");
        wait_exit();
        return;
    }

    // Find blocks around matches
    let block_pb = ProgressBar::new(all_matches.len() as u64);
    block_pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:>10} [{bar:30}] {pos:>4}/{len} {msg}")
            .unwrap()
            .progress_chars("█░"),
    );
    block_pb.set_prefix("Blocks:");
    block_pb.set_message("scanning...");

    let mut candidates: Vec<std::collections::HashMap<u32, u32>> = Vec::new();
    for m in &all_matches {
        let blocks = mem_source.find_blocks(*m, cfg.offset_back, cfg.read_size);
        candidates.extend(blocks);
        block_pb.inc(1);
    }
    block_pb.finish_with_message("Done");

    if candidates.is_empty() {
        println!("No valid data blocks found.");
        wait_exit();
        return;
    }

    let collection = candidates
        .into_iter()
        .max_by_key(|b| b.len())
        .unwrap_or_default();

    let name_fallback = database::load_local_name_fallback(cfg.mtga_path.as_deref());
    export::do_export(&cfg, &collection, &db, &name_fallback);

    wait_exit();
}

fn prompt(msg: &str) -> String {
    print!("{}", msg);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

fn make_pattern(arena_id: u32, quantity: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&arena_id.to_le_bytes());
    bytes.extend_from_slice(&quantity.to_le_bytes());
    bytes
}

fn wait_exit() {
    println!();
    prompt("Press Enter to exit...");
}
