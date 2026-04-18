use std::collections::HashSet;
use std::path::Path;

use ignore::WalkBuilder;
use serde::Serialize;
use tracing::{info, debug};

use crate::error::Error;
use crate::parser;
use crate::resolve::{self, LinkResolution};
use crate::store::Store;
use crate::types::ParseEvent;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexSummary {
    pub added: usize,
    pub changed: usize,
    pub deleted: usize,
    pub stubs: usize,
}

pub fn collect_vault_files(vault: &Path) -> Result<Vec<(String, i64)>, Error> {
    if !vault.is_dir() {
        return Err(Error::VaultNotFound {
            path: vault.to_path_buf(),
        });
    }

    let mut files = Vec::new();
    let walker = WalkBuilder::new(vault).build();

    for entry in walker {
        let entry = entry.map_err(|e| Error::Io {
            source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
            path: vault.to_path_buf(),
        })?;

        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let rel = path
            .strip_prefix(vault)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let meta = std::fs::metadata(path).map_err(|e| Error::Io {
            source: e,
            path: path.to_path_buf(),
        })?;
        let mtime = meta
            .modified()
            .map_err(|e| Error::Io {
                source: e,
                path: path.to_path_buf(),
            })?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        files.push((rel, mtime));
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    debug!(files = files.len(), "collected vault files");
    Ok(files)
}

pub fn index_vault(vault: &Path, store: &mut Store) -> Result<IndexSummary, Error> {
    info!(vault = %vault.display(), "indexing vault");
    let fs_files = collect_vault_files(vault)?;
    let synced = store.all_synced_paths()?;
    let synced_set: HashSet<&str> = synced.iter().map(|s| s.as_str()).collect();
    let fs_set: HashSet<&str> = fs_files.iter().map(|(p, _)| p.as_str()).collect();

    let mut new_files = Vec::new();
    let mut changed_files = Vec::new();
    let deleted: Vec<String> = synced
        .iter()
        .filter(|p| !fs_set.contains(p.as_str()))
        .cloned()
        .collect();

    for (path, mtime) in &fs_files {
        if !synced_set.contains(path.as_str()) {
            new_files.push((path.clone(), *mtime));
        } else {
            let stored_mtime = store.get_sync_mtime(path)?;
            if stored_mtime.map_or(true, |sm| *mtime > sm) {
                changed_files.push((path.clone(), *mtime));
            }
        }
    }

    debug!(added = new_files.len(), changed = changed_files.len(), removed = deleted.len(), "index diff computed");

    if new_files.is_empty() && changed_files.is_empty() && deleted.is_empty() {
        info!("vault unchanged, skipping reindex");
        return Ok(IndexSummary {
            added: 0,
            changed: 0,
            deleted: 0,
            stubs: 0,
        });
    }

    store.begin_transaction()?;

    for id in &deleted {
        store.delete_node(id)?;
    }

    let mut all_edges = Vec::new();

    for (path, mtime) in &new_files {
        let abs_path = vault.join(path);
        let (node, edges) = parser::parse_file(vault, &abs_path)?;
        store.upsert_node(&node, *mtime)?;
        all_edges.extend(edges);
    }

    for (path, mtime) in &changed_files {
        let abs_path = vault.join(path);
        let (node, edges) = parser::parse_file(vault, &abs_path)?;
        store.upsert_node(&node, *mtime)?;
        all_edges.extend(edges);
    }

    // Re-parse unchanged files for edge resolution (we need ALL edges)
    for (path, _mtime) in &fs_files {
        if new_files.iter().any(|(p, _)| p == path)
            || changed_files.iter().any(|(p, _)| p == path)
        {
            continue;
        }
        let abs_path = vault.join(path);
        let (_node, edges) = parser::parse_file(vault, &abs_path)?;
        all_edges.extend(edges);
    }

    let events = parser::parse_vault(vault)?;
    let nodes: Vec<_> = events
        .into_iter()
        .filter_map(|e| match e {
            ParseEvent::Node(n) => Some(n),
            _ => None,
        })
        .collect();

    let resolved = resolve::resolve_edges(&nodes, &all_edges);

    store.replace_all_edges(&resolved)?;

    let mut stub_count = 0;
    for edge in &resolved {
        if let LinkResolution::Unresolved = &edge.resolution {
            store.upsert_stub(&edge.target_raw)?;
            stub_count += 1;
        }
    }

    store.commit()?;

    let added = new_files.len();
    let changed = changed_files.len();
    let removed = deleted.len();
    info!(added, changed, removed, "indexing complete");

    Ok(IndexSummary {
        added,
        changed,
        deleted: removed,
        stubs: stub_count,
    })
}
