mod cli;
mod envelope;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, error::ErrorKind};
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};
use crate::envelope::{Envelope, emit_stdout};

fn main() -> ExitCode {
    init_tracing();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return handle_clap_error(err),
    };

    match dispatch(cli) {
        Ok(()) => ExitCode::from(0),
        Err(err) => {
            let env: Envelope<()> = Envelope::err_from(&err);
            emit_stdout(&env);
            ExitCode::from(1)
        }
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
}

fn handle_clap_error(err: clap::Error) -> ExitCode {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print!("{err}");
            ExitCode::from(0)
        }
        _ => {
            let env: Envelope<()> =
                Envelope::err("unknown_subcommand", first_line(&err.to_string()));
            emit_stdout(&env);
            ExitCode::from(2)
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn dispatch(cli: Cli) -> Result<(), kg_core::Error> {
    match cli.command {
        Command::Parse { pretty } => cmd_parse(cli.vault, pretty),
        Command::Resolve { name } => cmd_resolve(cli.vault, &name),
        Command::Index => cmd_index(cli.vault, cli.data_dir),
        Command::Stats => cmd_stats(cli.vault, cli.data_dir),
        Command::Search { query, limit } => cmd_search(cli.vault, cli.data_dir, &query, limit),
        Command::Rank { top } => cmd_rank(cli.vault, cli.data_dir, top),
        Command::Neighbors { id, depth, directed } => cmd_neighbors(cli.vault, cli.data_dir, &id, depth, directed),
        Command::Path { from, to, max_depth, directed } => cmd_path(cli.vault, cli.data_dir, &from, &to, max_depth, directed),
        Command::Shared { a, b, directed } => cmd_shared(cli.vault, cli.data_dir, &a, &b, directed),
        Command::Subgraph { ids, depth, directed } => cmd_subgraph(cli.vault, cli.data_dir, &ids, depth, directed),
    }
}

fn cmd_resolve(vault: Option<PathBuf>, name: &str) -> Result<(), kg_core::Error> {
    let vault_path = require_vault(vault)?;

    let events = kg_core::parser::parse_vault(&vault_path)?;
    let nodes: Vec<_> = events
        .into_iter()
        .filter_map(|e| match e {
            kg_core::types::ParseEvent::Node(n) => Some(n),
            _ => None,
        })
        .collect();

    let matches = kg_core::resolve::resolve_name(name, &nodes);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for m in &matches {
        serde_json::to_writer(&mut out, m).expect("serialize");
        let _ = writeln!(out);
    }

    Ok(())
}

fn resolve_data_dir(vault: &PathBuf, data_dir: Option<PathBuf>) -> PathBuf {
    data_dir.unwrap_or_else(|| vault.join(".kg"))
}

fn require_vault(vault: Option<PathBuf>) -> Result<PathBuf, kg_core::Error> {
    vault.ok_or_else(|| kg_core::Error::VaultNotFound {
        path: "(provide --vault or set KG_VAULT_PATH)".into(),
    })
}

fn cmd_index(vault: Option<PathBuf>, data_dir: Option<PathBuf>) -> Result<(), kg_core::Error> {
    let vault_path = require_vault(vault)?;
    let dir = resolve_data_dir(&vault_path, data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| kg_core::Error::Io {
        source: e,
        path: dir.clone(),
    })?;
    let db_path = dir.join("kg.db");
    let mut store = kg_core::store::Store::open(&db_path)?;
    let summary = kg_core::indexer::index_vault(&vault_path, &mut store)?;
    println!("{}", serde_json::to_string(&summary).expect("serialize"));
    Ok(())
}

fn cmd_stats(vault: Option<PathBuf>, data_dir: Option<PathBuf>) -> Result<(), kg_core::Error> {
    let vault_path = require_vault(vault)?;
    let dir = resolve_data_dir(&vault_path, data_dir);
    let db_path = dir.join("kg.db");
    let store = kg_core::store::Store::open(&db_path)?;
    let stats = store.stats()?;
    println!("{}", serde_json::to_string(&stats).expect("serialize"));
    Ok(())
}

fn cmd_search(vault: Option<PathBuf>, data_dir: Option<PathBuf>, query: &str, limit: i64) -> Result<(), kg_core::Error> {
    let vault_path = require_vault(vault)?;
    let dir = resolve_data_dir(&vault_path, data_dir);
    let db_path = dir.join("kg.db");
    let store = kg_core::store::Store::open(&db_path)?;
    let results = store.search(query, limit)?;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for result in &results {
        serde_json::to_writer(&mut out, result).expect("serialize");
        let _ = writeln!(out);
    }

    Ok(())
}

fn open_graph(vault: Option<PathBuf>, data_dir: Option<PathBuf>) -> Result<kg_core::graph::KnowledgeGraph, kg_core::Error> {
    let vault_path = require_vault(vault)?;
    let dir = resolve_data_dir(&vault_path, data_dir);
    let db_path = dir.join("kg.db");
    let store = kg_core::store::Store::open(&db_path)?;
    kg_core::graph::KnowledgeGraph::from_store(&store)
}

fn open_store_and_graph(vault: Option<PathBuf>, data_dir: Option<PathBuf>) -> Result<(kg_core::store::Store, kg_core::graph::KnowledgeGraph), kg_core::Error> {
    let vault_path = require_vault(vault)?;
    let dir = resolve_data_dir(&vault_path, data_dir);
    let db_path = dir.join("kg.db");
    let store = kg_core::store::Store::open(&db_path)?;
    let kg = kg_core::graph::KnowledgeGraph::from_store(&store)?;
    Ok((store, kg))
}

fn cmd_rank(vault: Option<PathBuf>, data_dir: Option<PathBuf>, top: usize) -> Result<(), kg_core::Error> {
    let (store, kg) = open_store_and_graph(vault, data_dir)?;

    let fingerprint = store.graph_fingerprint()?;
    let cached_fp = store.get_meta("rank_cache_fingerprint")?;

    let all_entries: Vec<kg_core::types::RankEntry> = if cached_fp.as_deref() == Some(&fingerprint) {
        let data = store.get_meta("rank_cache_data")?.unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_else(|_| kg.rank(usize::MAX))
    } else {
        let entries = kg.rank(usize::MAX);
        if let Ok(json) = serde_json::to_string(&entries) {
            let _ = store.set_meta("rank_cache_fingerprint", &fingerprint);
            let _ = store.set_meta("rank_cache_data", &json);
        }
        entries
    };

    let titles = store.node_titles()?;
    let truncated: Vec<serde_json::Value> = all_entries
        .into_iter()
        .take(top)
        .map(|e| {
            let title = titles.get(&e.id).cloned().unwrap_or_default();
            serde_json::json!({ "id": e.id, "title": title, "score": e.score })
        })
        .collect();

    println!("{}", serde_json::to_string(&truncated).expect("serialize"));
    Ok(())
}

fn cmd_neighbors(vault: Option<PathBuf>, data_dir: Option<PathBuf>, id: &str, depth: usize, directed: bool) -> Result<(), kg_core::Error> {
    let kg = open_graph(vault, data_dir)?;
    let result = kg.neighbors(id, depth, directed)?;
    println!("{}", serde_json::to_string(&result).expect("serialize"));
    Ok(())
}

fn cmd_path(vault: Option<PathBuf>, data_dir: Option<PathBuf>, from: &str, to: &str, max_depth: usize, directed: bool) -> Result<(), kg_core::Error> {
    let kg = open_graph(vault, data_dir)?;
    let result = kg.path(from, to, max_depth, directed)?;
    println!("{}", serde_json::to_string(&result).expect("serialize"));
    Ok(())
}

fn cmd_shared(vault: Option<PathBuf>, data_dir: Option<PathBuf>, a: &str, b: &str, directed: bool) -> Result<(), kg_core::Error> {
    let kg = open_graph(vault, data_dir)?;
    let result = kg.shared(a, b, directed)?;
    println!("{}", serde_json::to_string(&result).expect("serialize"));
    Ok(())
}

fn cmd_subgraph(vault: Option<PathBuf>, data_dir: Option<PathBuf>, ids: &[String], depth: usize, directed: bool) -> Result<(), kg_core::Error> {
    let kg = open_graph(vault, data_dir)?;
    let seed_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    let result = kg.subgraph(&seed_refs, depth, directed)?;
    println!("{}", serde_json::to_string(&result).expect("serialize"));
    Ok(())
}

fn cmd_parse(vault: Option<PathBuf>, pretty: bool) -> Result<(), kg_core::Error> {
    let vault_path = require_vault(vault)?;

    let events = kg_core::parser::parse_vault(&vault_path)?;

    if pretty {
        let wrapper = serde_json::json!({
            "ok": true,
            "data": events,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&wrapper).expect("serialize")
        );
    } else {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for event in &events {
            serde_json::to_writer(&mut out, event).expect("serialize");
            let _ = writeln!(out);
        }
    }

    Ok(())
}
