mod config;
mod database;
mod anchor;
mod memory;
mod export;
mod util;

use std::collections::HashMap;
use std::thread;

use indicatif::{ProgressBar, ProgressStyle};

fn main() {
    let cfg = match config::Config::from_cli() {
        Some(c) => c,
        None => return,
    };
    println!("MTGA Collection Extractor");
    println!("Output Folder: {}\n", cfg.output_dir.display());

    let mem_source = match memory::MemorySource::from_process() {
        Some(m) => m,
        None => {
            println!("MTG Arena not running. Open game and 'Collections' tab first.");
            wait_exit();
            return;
        }
    };

    let db = database::load_card_database(&cfg.data_dir, cfg.mtga_path.as_deref(), mem_source.process_path.as_deref());
    if db.is_empty() {
        println!("Database init failed.");
        wait_exit();
        return;
    }

    {
        let n = if cfg.threads > 0 {
            cfg.threads
        } else {
            let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
            if cores <= 2 { 1 } else { cores - 2 }
        };
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .ok();
    }

    let name_to_id = database::build_name_index(&db);
    let display_names = database::build_display_names(&db);
    let anchors = anchor::interactive_anchors(&name_to_id, &display_names, &cfg.anchor_file);
    if anchors.is_empty() {
        return;
    }

    println!("\nScanning memory for collection data...");
    let total_regions = mem_source.region_infos.len();

    let pb = ProgressBar::new(total_regions as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:>10} [{bar:30}] {pos:>4}/{len} {msg}")
            .unwrap()
            .progress_chars("█░"),
    );
    pb.set_prefix("Scan:");

    let patterns: Vec<Vec<u8>> = anchors
        .iter()
        .map(|a| make_pattern(a.arena_id, a.quantity))
        .collect();

    pb.reset_elapsed();
    pb.set_length(total_regions as u64);
    pb.set_position(0);
    pb.set_message("Scanning...");

    let results = mem_source.multi_pattern_scan(&patterns, &pb);
    pb.finish_with_message("Done");

    let mut all_matches = Vec::new();
    for (i, mut matches) in results.into_iter().enumerate() {
        if !matches.is_empty() {
            println!("  ✓ {}", anchors[i].name);
            all_matches.append(&mut matches);
        } else {
            println!("  ✗ {}", anchors[i].name);
        }
    }

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

    let mut candidates: Vec<HashMap<u32, u32>> = Vec::new();
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
        .max_by_key(|b| b.keys().filter(|k| db.contains_key(k)).count())
        .unwrap_or_default();

    export::do_export(&cfg, &collection, &db);

    wait_exit();
}

fn make_pattern(arena_id: u32, quantity: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&arena_id.to_le_bytes());
    bytes.extend_from_slice(&quantity.to_le_bytes());
    bytes
}

fn wait_exit() {
    println!();
    util::prompt("Press Enter to exit...");
}
