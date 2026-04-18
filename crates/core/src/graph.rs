use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;

use crate::error::Error;
use crate::store::Store;
use crate::types::{NeighborEntry, RankEntry, Subgraph, SubgraphEdge, SubgraphNode};

pub struct KnowledgeGraph {
    graph: DiGraph<String, ()>,
    index: HashMap<String, NodeIndex>,
    stubs: HashSet<String>,
}

impl KnowledgeGraph {
    pub fn from_store(store: &Store) -> Result<Self, Error> {
        let mut graph = DiGraph::new();
        let mut index = HashMap::new();
        let mut stubs = HashSet::new();

        for (id, is_stub) in store.all_nodes_metadata()? {
            let ni = graph.add_node(id.clone());
            index.insert(id.clone(), ni);
            if is_stub {
                stubs.insert(id);
            }
        }

        for (source, target) in store.all_edges()? {
            if let (Some(&src_ni), Some(&tgt_ni)) = (index.get(&source), index.get(&target)) {
                graph.add_edge(src_ni, tgt_ni, ());
            }
        }

        Ok(Self { graph, index, stubs })
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    fn resolve_node_index(&self, id: &str) -> Result<NodeIndex, Error> {
        self.index
            .get(id)
            .copied()
            .ok_or_else(|| Error::NodeNotFound { id: id.to_string() })
    }

    pub fn neighbors(
        &self,
        id: &str,
        depth: usize,
        directed: bool,
    ) -> Result<Vec<NeighborEntry>, Error> {
        let start = self.resolve_node_index(id)?;
        let mut visited = HashSet::new();
        visited.insert(start);
        let mut queue = VecDeque::new();
        queue.push_back((start, 0usize));
        let mut result = Vec::new();

        while let Some((current, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            let next_depth = d + 1;
            let neighbors_iter: Box<dyn Iterator<Item = NodeIndex>> = if directed {
                Box::new(self.graph.neighbors_directed(current, Direction::Outgoing))
            } else {
                let out = self.graph.neighbors_directed(current, Direction::Outgoing);
                let inc = self.graph.neighbors_directed(current, Direction::Incoming);
                Box::new(out.chain(inc))
            };
            for neighbor in neighbors_iter {
                if visited.insert(neighbor) {
                    let neighbor_id = &self.graph[neighbor];
                    result.push(NeighborEntry {
                        id: neighbor_id.clone(),
                        depth: next_depth,
                    });
                    queue.push_back((neighbor, next_depth));
                }
            }
        }

        result.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.id.cmp(&b.id)));
        Ok(result)
    }

    pub fn path(
        &self,
        from: &str,
        to: &str,
        max_depth: usize,
        directed: bool,
    ) -> Result<Vec<Vec<String>>, Error> {
        let start = self.resolve_node_index(from)?;
        let end = self.resolve_node_index(to)?;

        if start == end {
            return Ok(vec![vec![self.graph[start].clone()]]);
        }

        let mut results = Vec::new();
        let mut path_stack = vec![start];
        let mut visited = HashSet::new();
        visited.insert(start);
        self.dfs_all_paths(start, end, max_depth, directed, &mut visited, &mut path_stack, &mut results);

        results.sort();
        Ok(results)
    }

    fn dfs_all_paths(
        &self,
        current: NodeIndex,
        target: NodeIndex,
        max_depth: usize,
        directed: bool,
        visited: &mut HashSet<NodeIndex>,
        path: &mut Vec<NodeIndex>,
        results: &mut Vec<Vec<String>>,
    ) {
        if path.len() - 1 >= max_depth {
            return;
        }

        let neighbors_iter: Box<dyn Iterator<Item = NodeIndex>> = if directed {
            Box::new(self.graph.neighbors_directed(current, Direction::Outgoing))
        } else {
            let out = self.graph.neighbors_directed(current, Direction::Outgoing);
            let inc = self.graph.neighbors_directed(current, Direction::Incoming);
            Box::new(out.chain(inc))
        };

        for neighbor in neighbors_iter {
            if neighbor == target {
                path.push(neighbor);
                let ids: Vec<String> = path.iter().map(|&ni| self.graph[ni].clone()).collect();
                results.push(ids);
                path.pop();
            } else if visited.insert(neighbor) {
                path.push(neighbor);
                self.dfs_all_paths(neighbor, target, max_depth, directed, visited, path, results);
                path.pop();
                visited.remove(&neighbor);
            }
        }
    }

    pub fn shared(
        &self,
        a: &str,
        b: &str,
        directed: bool,
    ) -> Result<Vec<String>, Error> {
        let a_ni = self.resolve_node_index(a)?;
        let b_ni = self.resolve_node_index(b)?;

        let neighbors_of = |ni: NodeIndex| -> HashSet<NodeIndex> {
            if directed {
                self.graph.neighbors_directed(ni, Direction::Outgoing).collect()
            } else {
                let out: HashSet<_> = self.graph.neighbors_directed(ni, Direction::Outgoing).collect();
                let inc: HashSet<_> = self.graph.neighbors_directed(ni, Direction::Incoming).collect();
                out.union(&inc).copied().collect()
            }
        };

        let a_set = neighbors_of(a_ni);
        let b_set = neighbors_of(b_ni);

        let mut common: Vec<String> = a_set
            .intersection(&b_set)
            .filter(|&&ni| ni != a_ni && ni != b_ni)
            .map(|&ni| self.graph[ni].clone())
            .collect();

        common.sort();
        Ok(common)
    }

    pub fn subgraph(
        &self,
        seeds: &[&str],
        depth: usize,
        directed: bool,
    ) -> Result<Subgraph, Error> {
        let mut included = HashSet::new();

        for &seed in seeds {
            let seed_ni = self.resolve_node_index(seed)?;
            included.insert(seed_ni);

            let mut queue = VecDeque::new();
            queue.push_back((seed_ni, 0usize));
            while let Some((current, d)) = queue.pop_front() {
                if d >= depth {
                    continue;
                }
                let next_depth = d + 1;
                let neighbors_iter: Box<dyn Iterator<Item = NodeIndex>> = if directed {
                    Box::new(self.graph.neighbors_directed(current, Direction::Outgoing))
                } else {
                    let out = self.graph.neighbors_directed(current, Direction::Outgoing);
                    let inc = self.graph.neighbors_directed(current, Direction::Incoming);
                    Box::new(out.chain(inc))
                };
                for neighbor in neighbors_iter {
                    if included.insert(neighbor) {
                        queue.push_back((neighbor, next_depth));
                    }
                }
            }
        }

        let mut nodes: Vec<SubgraphNode> = included
            .iter()
            .map(|&ni| {
                let id = self.graph[ni].clone();
                let is_stub = self.stubs.contains(&id);
                SubgraphNode { id, is_stub }
            })
            .collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));

        let mut edges: Vec<SubgraphEdge> = self
            .graph
            .edge_indices()
            .filter_map(|ei| {
                let (src, tgt) = self.graph.edge_endpoints(ei)?;
                if included.contains(&src) && included.contains(&tgt) {
                    Some(SubgraphEdge {
                        source: self.graph[src].clone(),
                        target: self.graph[tgt].clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        edges.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.target.cmp(&b.target)));

        Ok(Subgraph { nodes, edges })
    }

    pub fn degree_centrality(&self, top: usize) -> Vec<RankEntry> {
        let mut undirected_degree: HashMap<NodeIndex, usize> = HashMap::new();
        let mut seen_pairs: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();

        for ei in self.graph.edge_indices() {
            if let Some((src, tgt)) = self.graph.edge_endpoints(ei) {
                let pair = if src <= tgt { (src, tgt) } else { (tgt, src) };
                if seen_pairs.insert(pair) {
                    *undirected_degree.entry(src).or_insert(0) += 1;
                    if src != tgt {
                        *undirected_degree.entry(tgt).or_insert(0) += 1;
                    }
                }
            }
        }

        let total_degree: usize = undirected_degree.values().sum();
        if total_degree == 0 {
            return Vec::new();
        }

        let mut entries: Vec<RankEntry> = undirected_degree
            .into_iter()
            .filter(|&(_, deg)| deg > 0)
            .map(|(ni, deg)| RankEntry {
                id: self.graph[ni].clone(),
                score: deg as f64 / total_degree as f64,
            })
            .collect();

        entries.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap()
                .then_with(|| a.id.cmp(&b.id))
        });
        entries.truncate(top);
        entries
    }

    pub fn rank(&self, top: usize) -> Vec<RankEntry> {
        let mut undirected_adj: HashMap<NodeIndex, HashSet<NodeIndex>> = HashMap::new();
        let mut seen_pairs: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();

        for ei in self.graph.edge_indices() {
            if let Some((src, tgt)) = self.graph.edge_endpoints(ei) {
                if src == tgt {
                    continue;
                }
                let pair = if src <= tgt { (src, tgt) } else { (tgt, src) };
                if seen_pairs.insert(pair) {
                    undirected_adj.entry(src).or_default().insert(tgt);
                    undirected_adj.entry(tgt).or_default().insert(src);
                }
            }
        }

        let active: Vec<NodeIndex> = self
            .graph
            .node_indices()
            .filter(|ni| undirected_adj.get(ni).is_some_and(|s| !s.is_empty()))
            .collect();

        let n = active.len();
        if n == 0 {
            return Vec::new();
        }

        let idx_map: HashMap<NodeIndex, usize> = active
            .iter()
            .enumerate()
            .map(|(i, &ni)| (ni, i))
            .collect();

        let damping = 0.85_f64;
        let max_iter = 100;
        let epsilon = 1e-6_f64;

        let init = 1.0 / n as f64;
        let mut pr = vec![init; n];

        let degrees: Vec<usize> = active
            .iter()
            .map(|ni| undirected_adj.get(ni).map_or(0, |s| s.len()))
            .collect();

        let mut converged = false;

        for _ in 0..max_iter {
            let dangling_sum: f64 = active
                .iter()
                .enumerate()
                .filter(|&(i, _)| degrees[i] == 0)
                .map(|(i, _)| pr[i])
                .sum();

            let mut pr_new = vec![0.0_f64; n];
            let base = (1.0 - damping) / n as f64 + damping * dangling_sum / n as f64;

            for (i, &ni) in active.iter().enumerate() {
                pr_new[i] = base;
                if let Some(neighbors) = undirected_adj.get(&ni) {
                    for &neighbor in neighbors {
                        if let Some(&j) = idx_map.get(&neighbor) {
                            pr_new[i] += damping * pr[j] / degrees[j] as f64;
                        }
                    }
                }
            }

            let max_diff = pr
                .iter()
                .zip(pr_new.iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max);

            pr = pr_new;

            if max_diff < epsilon {
                converged = true;
                break;
            }
        }

        if !converged {
            return self.degree_centrality(top);
        }

        let mut entries: Vec<RankEntry> = active
            .iter()
            .enumerate()
            .map(|(i, &ni)| RankEntry {
                id: self.graph[ni].clone(),
                score: pr[i],
            })
            .collect();

        entries.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap()
                .then_with(|| a.id.cmp(&b.id))
        });
        entries.truncate(top);
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_graph(nodes: &[&str], edges: &[(&str, &str)]) -> KnowledgeGraph {
        build_graph_with_stubs(nodes, &[], edges)
    }

    fn build_graph_with_stubs(
        nodes: &[&str],
        stub_ids: &[&str],
        edges: &[(&str, &str)],
    ) -> KnowledgeGraph {
        let mut graph = DiGraph::new();
        let mut index = HashMap::new();
        let mut stubs = HashSet::new();

        for &id in nodes {
            let ni = graph.add_node(id.to_string());
            index.insert(id.to_string(), ni);
        }
        for &id in stub_ids {
            let ni = graph.add_node(id.to_string());
            index.insert(id.to_string(), ni);
            stubs.insert(id.to_string());
        }

        for &(src, tgt) in edges {
            let src_ni = index[src];
            let tgt_ni = index[tgt];
            graph.add_edge(src_ni, tgt_ni, ());
        }

        KnowledgeGraph { graph, index, stubs }
    }

    // --- Cycle 2: from_store ---

    #[test]
    fn from_store_builds_graph() {
        let store = crate::store::Store::open_memory().unwrap();
        let n = crate::types::ParsedNode {
            id: "a.md".into(),
            title: "A".into(),
            tags: vec![],
            frontmatter: serde_json::json!({}),
            first_paragraph: String::new(),
        };
        store.upsert_node(&n, 1).unwrap();
        let n2 = crate::types::ParsedNode {
            id: "b.md".into(),
            title: "B".into(),
            tags: vec![],
            frontmatter: serde_json::json!({}),
            first_paragraph: String::new(),
        };
        store.upsert_node(&n2, 1).unwrap();
        let n3 = crate::types::ParsedNode {
            id: "c.md".into(),
            title: "C".into(),
            tags: vec![],
            frontmatter: serde_json::json!({}),
            first_paragraph: String::new(),
        };
        store.upsert_node(&n3, 1).unwrap();
        store.upsert_stub("Ghost").unwrap();
        store.insert_edge("a.md", "b.md", "").unwrap();
        store.insert_edge("a.md", "Ghost", "").unwrap();

        let kg = KnowledgeGraph::from_store(&store).unwrap();
        assert_eq!(kg.node_count(), 4);
        assert_eq!(kg.edge_count(), 2);
        assert!(kg.stubs.contains("Ghost"));
        assert!(!kg.stubs.contains("a.md"));
    }

    #[test]
    fn from_store_empty_db() {
        let store = crate::store::Store::open_memory().unwrap();
        let kg = KnowledgeGraph::from_store(&store).unwrap();
        assert_eq!(kg.node_count(), 0);
        assert_eq!(kg.edge_count(), 0);
    }

    #[test]
    fn from_store_self_loop() {
        let store = crate::store::Store::open_memory().unwrap();
        let n = crate::types::ParsedNode {
            id: "a.md".into(),
            title: "A".into(),
            tags: vec![],
            frontmatter: serde_json::json!({}),
            first_paragraph: String::new(),
        };
        store.upsert_node(&n, 1).unwrap();
        store.insert_edge("a.md", "a.md", "").unwrap();

        let kg = KnowledgeGraph::from_store(&store).unwrap();
        assert_eq!(kg.edge_count(), 1);
    }

    // --- Cycle 3: neighbors ---

    #[test]
    fn neighbors_depth1_undirected() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("C", "A")]);
        let result = kg.neighbors("A", 1, false).unwrap();
        let ids: Vec<&str> = result.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["B", "C"]);
    }

    #[test]
    fn neighbors_depth1_directed() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("C", "A")]);
        let result = kg.neighbors("A", 1, true).unwrap();
        let ids: Vec<&str> = result.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["B"]);
    }

    #[test]
    fn neighbors_depth2() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("B", "C")]);
        let result = kg.neighbors("A", 2, false).unwrap();
        assert_eq!(result, vec![
            NeighborEntry { id: "B".into(), depth: 1 },
            NeighborEntry { id: "C".into(), depth: 2 },
        ]);
    }

    #[test]
    fn neighbors_self_loop_excluded() {
        let kg = build_graph(&["A", "B"], &[("A", "A"), ("A", "B")]);
        let result = kg.neighbors("A", 1, false).unwrap();
        let ids: Vec<&str> = result.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["B"]);
    }

    #[test]
    fn neighbors_depth0_returns_empty() {
        let kg = build_graph(&["A", "B"], &[("A", "B")]);
        let result = kg.neighbors("A", 0, false).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn neighbors_isolated_node() {
        let kg = build_graph(&["A"], &[]);
        let result = kg.neighbors("A", 1, false).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn neighbors_nonexistent_node() {
        let kg = build_graph(&["A"], &[]);
        let err = kg.neighbors("Z", 1, false).unwrap_err();
        assert!(matches!(err, Error::NodeNotFound { .. }));
    }

    #[test]
    fn neighbors_sorted_by_depth_then_id() {
        let kg = build_graph(&["A", "B", "C", "D"], &[("A", "C"), ("A", "B"), ("B", "D")]);
        let result = kg.neighbors("A", 2, false).unwrap();
        assert_eq!(result[0], NeighborEntry { id: "B".into(), depth: 1 });
        assert_eq!(result[1], NeighborEntry { id: "C".into(), depth: 1 });
        assert_eq!(result[2], NeighborEntry { id: "D".into(), depth: 2 });
    }

    // --- Cycle 4: path ---

    #[test]
    fn path_direct_link() {
        let kg = build_graph(&["A", "B"], &[("A", "B")]);
        let paths = kg.path("A", "B", 5, false).unwrap();
        assert_eq!(paths, vec![vec!["A", "B"]]);
    }

    #[test]
    fn path_two_hop() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("B", "C")]);
        let paths = kg.path("A", "C", 5, false).unwrap();
        assert!(paths.contains(&vec!["A".to_string(), "B".into(), "C".into()]));
    }

    #[test]
    fn path_multiple_sorted() {
        let kg = build_graph(&["A", "B", "C", "D"], &[("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")]);
        let paths = kg.path("A", "D", 5, false).unwrap();
        assert!(paths.len() >= 2);
        for i in 1..paths.len() {
            assert!(paths[i - 1] <= paths[i], "paths should be sorted lexicographically");
        }
    }

    #[test]
    fn path_max_depth_caps() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("B", "C")]);
        let paths = kg.path("A", "C", 1, false).unwrap();
        assert!(paths.is_empty(), "max_depth=1 should not find 2-hop path");
    }

    #[test]
    fn path_same_node() {
        let kg = build_graph(&["A"], &[]);
        let paths = kg.path("A", "A", 5, false).unwrap();
        assert_eq!(paths, vec![vec!["A"]]);
    }

    #[test]
    fn path_no_connection() {
        let kg = build_graph(&["A", "B"], &[]);
        let paths = kg.path("A", "B", 5, false).unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn path_nonexistent_node() {
        let kg = build_graph(&["A"], &[]);
        let err = kg.path("A", "Z", 5, false).unwrap_err();
        assert!(matches!(err, Error::NodeNotFound { .. }));
    }

    #[test]
    fn path_directed_respects_direction() {
        let kg = build_graph(&["A", "B"], &[("B", "A")]);
        let paths = kg.path("A", "B", 5, true).unwrap();
        assert!(paths.is_empty(), "directed: no path from A to B when only B→A exists");
    }

    #[test]
    fn path_cycle_avoidance() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("B", "C"), ("C", "A")]);
        let paths = kg.path("A", "C", 10, false).unwrap();
        for p in &paths {
            let unique: HashSet<&String> = p.iter().collect();
            assert_eq!(unique.len(), p.len(), "paths should be simple (no repeated nodes): {p:?}");
        }
    }

    // --- Cycle 5: shared ---

    #[test]
    fn shared_common_neighbors() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "C"), ("B", "C")]);
        let common = kg.shared("A", "B", false).unwrap();
        assert_eq!(common, vec!["C"]);
    }

    #[test]
    fn shared_no_common() {
        let kg = build_graph(&["A", "B", "C", "D"], &[("A", "C"), ("B", "D")]);
        let common = kg.shared("A", "B", false).unwrap();
        assert!(common.is_empty());
    }

    #[test]
    fn shared_excludes_a_and_b() {
        let kg = build_graph(&["A", "B"], &[("A", "B"), ("B", "A")]);
        let common = kg.shared("A", "B", false).unwrap();
        assert!(common.is_empty(), "a and b themselves should be excluded");
    }

    #[test]
    fn shared_nonexistent_node() {
        let kg = build_graph(&["A"], &[]);
        let err = kg.shared("A", "Z", false).unwrap_err();
        assert!(matches!(err, Error::NodeNotFound { .. }));
    }

    #[test]
    fn shared_directed() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "C"), ("B", "C")]);
        let common = kg.shared("A", "B", true).unwrap();
        assert_eq!(common, vec!["C"]);
    }

    // --- Cycle 6: subgraph ---

    #[test]
    fn subgraph_single_seed_depth0() {
        let kg = build_graph(&["A", "B"], &[("A", "B")]);
        let sg = kg.subgraph(&["A"], 0, false).unwrap();
        assert_eq!(sg.nodes.len(), 1);
        assert_eq!(sg.nodes[0].id, "A");
        assert!(sg.edges.is_empty());
    }

    #[test]
    fn subgraph_depth1_includes_neighbors() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("A", "C"), ("B", "C")]);
        let sg = kg.subgraph(&["A"], 1, false).unwrap();
        assert_eq!(sg.nodes.len(), 3);
        assert!(sg.edges.len() >= 2);
    }

    #[test]
    fn subgraph_multiple_seeds() {
        let kg = build_graph(&["A", "B", "C", "D"], &[("A", "B"), ("C", "D")]);
        let sg = kg.subgraph(&["A", "C"], 1, false).unwrap();
        assert_eq!(sg.nodes.len(), 4);
    }

    #[test]
    fn subgraph_stub_marking() {
        let kg = build_graph_with_stubs(&["A"], &["S"], &[("A", "S")]);
        let sg = kg.subgraph(&["A"], 1, false).unwrap();
        let stub_node = sg.nodes.iter().find(|n| n.id == "S").unwrap();
        assert!(stub_node.is_stub);
        let real_node = sg.nodes.iter().find(|n| n.id == "A").unwrap();
        assert!(!real_node.is_stub);
    }

    #[test]
    fn subgraph_induced_edges_only() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("B", "C")]);
        let sg = kg.subgraph(&["A"], 1, false).unwrap();
        assert!(sg.edges.iter().all(|e| {
            sg.nodes.iter().any(|n| n.id == e.source) && sg.nodes.iter().any(|n| n.id == e.target)
        }));
    }

    #[test]
    fn subgraph_nonexistent_seed() {
        let kg = build_graph(&["A"], &[]);
        let err = kg.subgraph(&["Z"], 1, false).unwrap_err();
        assert!(matches!(err, Error::NodeNotFound { .. }));
    }

    // --- Cycle 7: rank (PageRank) ---

    #[test]
    fn rank_single_isolate_returns_empty() {
        let kg = build_graph(&["A"], &[]);
        let result = kg.rank(10);
        assert!(result.is_empty());
    }

    #[test]
    fn rank_two_connected_nodes_equal() {
        let kg = build_graph(&["A", "B"], &[("A", "B")]);
        let result = kg.rank(10);
        assert_eq!(result.len(), 2);
        assert!((result[0].score - result[1].score).abs() < 1e-4);
        let sum: f64 = result.iter().map(|e| e.score).sum();
        assert!((sum - 1.0).abs() < 1e-4);
    }

    #[test]
    fn rank_triangle_all_equal() {
        let kg = build_graph(&["A", "B", "C"], &[("A", "B"), ("B", "C"), ("C", "A")]);
        let result = kg.rank(10);
        assert_eq!(result.len(), 3);
        for entry in &result {
            assert!((entry.score - 1.0 / 3.0).abs() < 1e-4);
        }
    }

    #[test]
    fn rank_star_center_highest() {
        let kg = build_graph(
            &["Center", "L1", "L2", "L3"],
            &[("Center", "L1"), ("Center", "L2"), ("Center", "L3")],
        );
        let result = kg.rank(10);
        assert_eq!(result[0].id, "Center");
        assert!(result[0].score > result[1].score);
        assert!((result[1].score - result[2].score).abs() < 1e-4);
        assert!((result[2].score - result[3].score).abs() < 1e-4);
    }

    #[test]
    fn rank_isolates_excluded() {
        let kg = build_graph(&["A", "B", "Isolated"], &[("A", "B")]);
        let result = kg.rank(10);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|e| e.id != "Isolated"));
    }

    #[test]
    fn rank_top_limits_output() {
        let kg = build_graph(
            &["A", "B", "C", "D"],
            &[("A", "B"), ("B", "C"), ("C", "D")],
        );
        let result = kg.rank(2);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn rank_sorted_descending() {
        let kg = build_graph(
            &["Center", "L1", "L2", "L3"],
            &[("Center", "L1"), ("Center", "L2"), ("Center", "L3")],
        );
        let result = kg.rank(10);
        for i in 1..result.len() {
            assert!(result[i - 1].score >= result[i].score);
        }
    }

    #[test]
    fn rank_scores_sum_to_one() {
        let kg = build_graph(
            &["A", "B", "C", "D"],
            &[("A", "B"), ("B", "C"), ("C", "D"), ("D", "A")],
        );
        let result = kg.rank(100);
        let sum: f64 = result.iter().map(|e| e.score).sum();
        assert!((sum - 1.0).abs() < 1e-4, "scores sum to {sum}");
    }

    #[test]
    fn rank_stubs_with_edges_participate() {
        let kg = build_graph_with_stubs(&["A"], &["Stub"], &[("A", "Stub")]);
        let result = kg.rank(10);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|e| e.id == "Stub"));
    }

    #[test]
    fn rank_empty_graph_returns_empty() {
        let kg = build_graph(&[], &[]);
        let result = kg.rank(10);
        assert!(result.is_empty());
    }

    // --- Cycle 8: degree_centrality ---

    #[test]
    fn degree_centrality_star_center_highest() {
        let kg = build_graph(
            &["Center", "L1", "L2", "L3"],
            &[("Center", "L1"), ("Center", "L2"), ("Center", "L3")],
        );
        let result = kg.degree_centrality(10);
        assert_eq!(result[0].id, "Center");
        assert!(result[0].score > result[1].score);
    }

    #[test]
    fn degree_centrality_isolates_excluded() {
        let kg = build_graph(&["A", "B", "Isolated"], &[("A", "B")]);
        let result = kg.degree_centrality(10);
        assert!(result.iter().all(|e| e.id != "Isolated"));
    }

    #[test]
    fn degree_centrality_scores_sum_to_one() {
        let kg = build_graph(
            &["A", "B", "C"],
            &[("A", "B"), ("B", "C")],
        );
        let result = kg.degree_centrality(10);
        let sum: f64 = result.iter().map(|e| e.score).sum();
        assert!((sum - 1.0).abs() < 1e-4, "scores sum to {sum}");
    }

    #[test]
    fn subgraph_deterministic_ordering() {
        let kg = build_graph(&["C", "A", "B"], &[("A", "B"), ("A", "C"), ("B", "C")]);
        let sg = kg.subgraph(&["A"], 1, false).unwrap();
        let ids: Vec<&str> = sg.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["A", "B", "C"]);
        for i in 1..sg.edges.len() {
            assert!(
                (sg.edges[i - 1].source.as_str(), sg.edges[i - 1].target.as_str())
                    <= (sg.edges[i].source.as_str(), sg.edges[i].target.as_str())
            );
        }
    }
}
