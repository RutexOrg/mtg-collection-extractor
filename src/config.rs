use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "MTGA Collection Extractor", version = "2.0", about = "Export MTGA card collection")]
pub struct Cli {
    #[arg(long, default_value = "1048576", help = "Bytes to read before anchor address")]
    pub offset_back: usize,

    #[arg(long, default_value = "4194304", help = "Total bytes to read around anchor")]
    pub read_size: usize,

    #[arg(long, help = "Custom output directory")]
    pub output_dir: Option<PathBuf>,

    #[arg(long, default_value = "0", help = "Number of scan threads (0 = all CPU cores)")]
    pub threads: usize,

    #[arg(long, help = "Custom MTGA installation path (e.g. D:/Games/MTGA)")]
    pub mtga_path: Option<PathBuf>,
}

pub struct Config {
    pub offset_back: usize,
    pub read_size: usize,
    pub output_dir: PathBuf,
    pub lookup_file: PathBuf,
    pub anchor_file: PathBuf,
    pub output_json: PathBuf,
    pub output_txt: PathBuf,
    pub output_csv: PathBuf,
    pub output_unknown_txt: PathBuf,
    pub output_unknown_json: PathBuf,
    pub output_unknown_csv: PathBuf,
    pub threads: usize,
    pub mtga_path: Option<PathBuf>,
}

impl Config {
    pub fn from_cli() -> Self {
        let cli = Cli::parse();
        let script_dir = cli
            .output_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let data_dir = script_dir.join("data");
        let output_dir = data_dir.join("output");

        Config {
            offset_back: cli.offset_back,
            read_size: cli.read_size,
            threads: cli.threads,
            output_dir: output_dir.clone(),
            lookup_file: data_dir.join("arena_id_lookup.json"),
            anchor_file: data_dir.join("last_anchors.json"),
            output_json: output_dir.join("mtga_collection.json"),
            output_txt: output_dir.join("mtga_collection.txt"),
            output_csv: output_dir.join("mtga_collection.csv"),
            output_unknown_txt: output_dir.join("mtga_collection_unknown.txt"),
            output_unknown_json: output_dir.join("mtga_collection_unknown.json"),
            output_unknown_csv: output_dir.join("mtga_collection_unknown.csv"),
            mtga_path: cli.mtga_path,
        }
    }
}
