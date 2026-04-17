use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "kg", version, about = "knowledge-graph CLI")]
pub struct Cli {
    /// Path to the Obsidian-style vault to operate on.
    #[arg(long, global = true, env = "KG_VAULT_PATH")]
    pub vault: Option<PathBuf>,

    /// Directory where kg stores its database and caches.
    #[arg(long, global = true, env = "KG_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Parse a vault into nodes and edges.
    Parse {
        /// Wrap output in an envelope and pretty-print.
        #[arg(long)]
        pretty: bool,
    },
}
