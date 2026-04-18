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
    /// Resolve a name against the vault's node set.
    Resolve {
        /// The name to resolve (node ID, title, alias, or substring).
        name: String,
    },
    /// Index (or re-index) a vault into a local SQLite database.
    Index,
    /// Show statistics about the indexed knowledge graph.
    Stats,
    /// Full-text search across indexed nodes.
    Search {
        /// The search query (FTS5 syntax).
        query: String,
        /// Maximum number of results.
        #[arg(long, default_value = "20")]
        limit: i64,
    },
    /// Find neighbors of a node via BFS traversal.
    Neighbors {
        /// Node ID (relative path from vault root).
        id: String,
        /// Maximum BFS depth.
        #[arg(long, default_value = "1")]
        depth: usize,
        /// Only follow outgoing edges.
        #[arg(long)]
        directed: bool,
    },
    /// Find all simple paths between two nodes.
    Path {
        /// Source node ID.
        from: String,
        /// Target node ID.
        to: String,
        /// Maximum path length in edges.
        #[arg(long, default_value = "5")]
        max_depth: usize,
        /// Only follow outgoing edges.
        #[arg(long)]
        directed: bool,
    },
    /// Find shared neighbors of two nodes.
    Shared {
        /// First node ID.
        a: String,
        /// Second node ID.
        b: String,
        /// Only follow outgoing edges.
        #[arg(long)]
        directed: bool,
    },
    /// Extract an induced subgraph around seed nodes.
    Subgraph {
        /// Seed node IDs.
        ids: Vec<String>,
        /// BFS expansion depth from each seed.
        #[arg(long, default_value = "1")]
        depth: usize,
        /// Only follow outgoing edges.
        #[arg(long)]
        directed: bool,
    },
}
