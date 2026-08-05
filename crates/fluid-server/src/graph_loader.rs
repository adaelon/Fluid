//! Multi-scope understand-anything graph discovery and ownership.
//!
//! Every project or nested sub-project may own one graph. Within a scope the
//! current `.ua/knowledge-graph.json` wins, while the legacy
//! `.understand-anything/knowledge-graph.json` remains a compatibility fallback.
//! A malformed candidate never makes the graph mandatory and never hides valid
//! graphs in other scopes.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

const GRAPH_FILE: &str = "knowledge-graph.json";

/// A node in the understand-anything knowledge graph.
/// Field names are kept camelCase so the compatibility API can re-serialize the
/// graph exactly as the frontend expects (技术方案 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub name: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<String>,
    /// Present only on class/function nodes: [startLine, endLine].
    #[serde(rename = "lineRange", default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<[u32; 2]>,
    #[serde(
        rename = "languageNotes",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub language_notes: Option<String>,
}

/// A directed relationship between two nodes (calls/imports/contains/...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
}

/// The subset of an understand-anything graph Fluid consumes. Other top-level
/// fields (version/project/layers/tour) are deliberately ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

/// Which on-disk candidate supplied a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphOrigin {
    Ua,
    Legacy,
}

impl GraphOrigin {
    fn directory(self) -> &'static str {
        match self {
            Self::Ua => ".ua",
            Self::Legacy => ".understand-anything",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ua => ".ua",
            Self::Legacy => ".understand-anything",
        }
    }
}

/// One valid graph together with its scope and freshness identity.
#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    origin: GraphOrigin,
    scope_root: PathBuf,
    scope_path: String,
    path: PathBuf,
    content_hash: String,
    identity: String,
    graph: KnowledgeGraph,
}

impl GraphSnapshot {
    pub fn origin(&self) -> GraphOrigin {
        self.origin
    }

    /// Canonical absolute directory covered by this graph.
    pub fn scope_root(&self) -> &Path {
        &self.scope_root
    }

    /// Project-relative scope path using `/`; `.` denotes the project root.
    pub fn scope_path(&self) -> &str {
        &self.scope_path
    }

    /// Canonical absolute path of the selected graph file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stable hash of the graph file's exact bytes.
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Stable graph identity. Content changes keep the identity but update the
    /// content hash; switching `.ua`/legacy or scope changes the identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn graph(&self) -> &KnowledgeGraph {
        &self.graph
    }

    /// Convert a project-relative file path into this graph's scope-relative
    /// `filePath` representation. Returns `None` when the project path is outside
    /// this snapshot's scope.
    pub fn graph_relative_path(&self, project_path: &str) -> Option<String> {
        let project = normalize_relative(project_path)?;
        let scope = scope_components(&self.scope_path);
        let project_components: Vec<_> = project.components().collect();
        if project_components.len() < scope.len()
            || !project_components
                .iter()
                .zip(scope.iter())
                .all(|(left, right)| left.as_os_str() == right.as_os_str())
        {
            return None;
        }
        let relative = project_components[scope.len()..]
            .iter()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        Some(relative)
    }

    /// Convert a graph node's scope-relative `filePath` into a project-relative
    /// path suitable for `ProjectReader`.
    pub fn project_relative_path(&self, graph_path: &str) -> Option<String> {
        let graph_relative = normalize_relative(graph_path)?;
        let graph_relative = rel_to_unix(&graph_relative);
        if self.scope_path == "." {
            Some(graph_relative)
        } else if graph_relative.is_empty() {
            Some(self.scope_path.clone())
        } else {
            Some(format!("{}/{}", self.scope_path, graph_relative))
        }
    }
}

/// All valid graph snapshots under one canonical project root.
#[derive(Debug, Clone)]
pub struct GraphCatalog {
    project_root: PathBuf,
    snapshots: Vec<GraphSnapshot>,
    freshness_hash: String,
}

impl GraphCatalog {
    /// Discover every graph scope below `project_root`. The catalog is optional:
    /// an unreadable root or malformed graph yields an empty/partial catalog and
    /// warnings, never a server crash.
    pub fn discover(project_root: &Path) -> Self {
        let project_root = match project_root.canonicalize() {
            Ok(root) if root.is_dir() => root,
            Ok(root) => {
                eprintln!(
                    "warning: graph catalog root is not a directory: {}",
                    root.display()
                );
                return Self::empty(root);
            }
            Err(error) => {
                eprintln!(
                    "warning: failed to normalize graph catalog root {}: {error}",
                    project_root.display()
                );
                return Self::empty(project_root.to_path_buf());
            }
        };

        let mut scopes = discover_scope_roots(&project_root);
        scopes.sort_by_key(|scope| path_sort_key(scope));
        scopes.dedup();

        let mut snapshots = Vec::new();
        for scope_root in scopes {
            if let Some(snapshot) = load_first_valid(&project_root, &scope_root) {
                snapshots.push(snapshot);
            }
        }
        snapshots.sort_by(|left, right| left.identity.cmp(&right.identity));
        let freshness_hash = catalog_freshness_hash(&snapshots);

        Self {
            project_root,
            snapshots,
            freshness_hash,
        }
    }

    fn empty(project_root: PathBuf) -> Self {
        Self {
            project_root,
            snapshots: Vec::new(),
            freshness_hash: stable_hash(&[]),
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn snapshots(&self) -> &[GraphSnapshot] {
        &self.snapshots
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Hash of the discovered scope identities and their exact content hashes.
    /// Added/removed scopes and content changes all update this value.
    pub fn freshness_hash(&self) -> &str {
        &self.freshness_hash
    }

    /// Rediscover the catalog. Returns `true` when a scope was added/removed,
    /// candidate priority changed, or graph bytes changed.
    pub fn refresh(&mut self) -> bool {
        let next = Self::discover(&self.project_root);
        let changed = next.freshness_hash != self.freshness_hash;
        *self = next;
        changed
    }

    /// Compatibility projection for `GET /api/project/graph`: only the graph
    /// whose scope is exactly the project root is visible. Nested graphs are never
    /// merged into the legacy response.
    pub fn root_graph(&self) -> Option<&KnowledgeGraph> {
        self.root_snapshot().map(GraphSnapshot::graph)
    }

    pub fn root_snapshot(&self) -> Option<&GraphSnapshot> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.scope_root == self.project_root)
    }

    /// Resolve a project-relative or absolute file path to its deepest ancestor
    /// graph scope. Existing paths are canonicalized so a symlink escape cannot
    /// acquire a graph from inside the project.
    pub fn resolve_for_file(&self, file_path: &Path) -> Option<&GraphSnapshot> {
        let absolute = resolve_inside_root(&self.project_root, file_path)?;
        self.snapshots
            .iter()
            .filter(|snapshot| absolute.starts_with(&snapshot.scope_root))
            .max_by(|left, right| {
                left.scope_root
                    .components()
                    .count()
                    .cmp(&right.scope_root.components().count())
                    .then_with(|| left.identity.cmp(&right.identity))
            })
    }

    pub fn graph_for_file(&self, project_path: &str) -> Option<&GraphSnapshot> {
        self.resolve_for_file(Path::new(project_path))
    }

    /// Hash only graph identities plus nodes/edges relevant to the supplied
    /// project files. Unrelated scopes or unrelated nodes do not invalidate a
    /// future file-orientation cache.
    #[allow(dead_code)] // consumed by S-ORI-1; S-GRAPH-1 lands the stable identity first
    pub fn relevant_graph_set_hash(&self, project_paths: &[String]) -> String {
        let mut paths = project_paths.to_vec();
        paths.sort();
        paths.dedup();

        let mut parts = Vec::new();
        for project_path in paths {
            let Some(snapshot) = self.graph_for_file(&project_path) else {
                parts.push(format!("no-graph\0{project_path}"));
                continue;
            };
            let Some(graph_path) = snapshot.graph_relative_path(&project_path) else {
                parts.push(format!("invalid-path\0{project_path}"));
                continue;
            };

            let mut nodes: Vec<_> = snapshot
                .graph
                .nodes
                .iter()
                .filter(|node| {
                    normalized_graph_path(&node.file_path).as_deref() == Some(&graph_path)
                })
                .collect();
            nodes.sort_by(|left, right| left.id.cmp(&right.id));
            let ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
            let mut edges: Vec<_> = snapshot
                .graph
                .edges
                .iter()
                .filter(|edge| {
                    ids.contains(edge.source.as_str()) || ids.contains(edge.target.as_str())
                })
                .collect();
            edges.sort_by(|left, right| {
                (&left.source, &left.target, &left.edge_type).cmp(&(
                    &right.source,
                    &right.target,
                    &right.edge_type,
                ))
            });

            parts.push(format!("graph\0{}\0{}", snapshot.identity, graph_path));
            for node in nodes {
                parts.push(format!(
                    "node\0{}",
                    serde_json::to_string(node).expect("GraphNode serialization cannot fail")
                ));
            }
            for edge in edges {
                parts.push(format!(
                    "edge\0{}",
                    serde_json::to_string(edge).expect("GraphEdge serialization cannot fail")
                ));
            }
        }

        stable_hash_parts(parts.iter().map(String::as_bytes))
    }

    #[cfg(test)]
    pub(crate) fn from_root_graph_for_test(graph: KnowledgeGraph) -> Self {
        Self::from_scoped_graphs_for_test(vec![(".".into(), graph)])
    }

    #[cfg(test)]
    pub(crate) fn from_scoped_graphs_for_test(
        scoped_graphs: Vec<(String, KnowledgeGraph)>,
    ) -> Self {
        let project_root = std::env::current_dir()
            .and_then(|path| path.canonicalize())
            .expect("test current directory must exist");
        let mut snapshots = scoped_graphs
            .into_iter()
            .map(|(scope_path, graph)| {
                let scope_root = if scope_path == "." {
                    project_root.clone()
                } else {
                    project_root
                        .join(normalize_relative(&scope_path).expect("safe test graph scope path"))
                };
                GraphSnapshot {
                    origin: GraphOrigin::Ua,
                    path: scope_root.join(".ua").join(GRAPH_FILE),
                    content_hash: "test-content".into(),
                    identity: format!("scope:{scope_path}|origin:.ua"),
                    scope_root,
                    scope_path,
                    graph,
                }
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.identity.cmp(&right.identity));
        let freshness_hash = catalog_freshness_hash(&snapshots);
        Self {
            project_root,
            snapshots,
            freshness_hash,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        let project_root = std::env::current_dir()
            .and_then(|path| path.canonicalize())
            .expect("test current directory must exist");
        Self::empty(project_root)
    }
}

fn discover_scope_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut scopes = Vec::new();
    let walker = WalkDir::new(project_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_descend);

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("warning: graph discovery skipped an unreadable path: {error}");
                continue;
            }
        };
        if !entry.file_type().is_dir() {
            continue;
        }
        let Ok(scope_root) = entry.path().canonicalize() else {
            continue;
        };
        if !scope_root.starts_with(project_root) {
            continue;
        }
        if GraphOrigin::Ua
            .candidate(&scope_root)
            .try_exists()
            .unwrap_or(false)
            || GraphOrigin::Legacy
                .candidate(&scope_root)
                .try_exists()
                .unwrap_or(false)
        {
            scopes.push(scope_root);
        }
    }
    scopes
}

fn should_descend(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    !entry
        .file_name()
        .to_str()
        .is_some_and(|name| matches!(name, ".ua" | ".understand-anything"))
}

impl GraphOrigin {
    fn candidate(self, scope_root: &Path) -> PathBuf {
        scope_root.join(self.directory()).join(GRAPH_FILE)
    }
}

fn load_first_valid(project_root: &Path, scope_root: &Path) -> Option<GraphSnapshot> {
    for origin in [GraphOrigin::Ua, GraphOrigin::Legacy] {
        let candidate = origin.candidate(scope_root);
        if !candidate.try_exists().unwrap_or(false) {
            continue;
        }
        match load_snapshot(project_root, scope_root, origin, &candidate) {
            Ok(snapshot) => return Some(snapshot),
            Err(error) => eprintln!(
                "warning: ignored invalid {} graph at {}: {error:#}",
                origin.label(),
                candidate.display()
            ),
        }
    }
    None
}

fn load_snapshot(
    project_root: &Path,
    scope_root: &Path,
    origin: GraphOrigin,
    candidate: &Path,
) -> anyhow::Result<GraphSnapshot> {
    let path = candidate
        .canonicalize()
        .with_context(|| format!("cannot canonicalize {}", candidate.display()))?;
    if !path.starts_with(project_root) || !path.starts_with(scope_root) || !path.is_file() {
        bail!("graph path escapes its project/scope boundary");
    }

    let bytes = std::fs::read(&path)?;
    let text = decode_text(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    let mut graph: KnowledgeGraph = serde_json::from_str(text)?;
    normalize_and_validate_node_paths(scope_root, &mut graph)?;

    let relative_scope = scope_root
        .strip_prefix(project_root)
        .map_err(|_| anyhow!("scope is outside project root"))?;
    let scope_path = if relative_scope.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel_to_unix(relative_scope)
    };
    let identity = format!("scope:{scope_path}|origin:{}", origin.label());

    Ok(GraphSnapshot {
        origin,
        scope_root: scope_root.to_path_buf(),
        scope_path,
        path,
        content_hash: stable_hash(&bytes),
        identity,
        graph,
    })
}

fn normalize_and_validate_node_paths(
    scope_root: &Path,
    graph: &mut KnowledgeGraph,
) -> anyhow::Result<()> {
    for node in &mut graph.nodes {
        let relative = normalize_relative(&node.file_path)
            .ok_or_else(|| anyhow!("node {} has unsafe filePath {:?}", node.id, node.file_path))?;
        node.file_path = rel_to_unix(&relative);
        let joined = scope_root.join(&relative);
        if joined.exists() {
            let canonical = joined.canonicalize()?;
            if !canonical.starts_with(scope_root) {
                bail!("node {} filePath escapes graph scope", node.id);
            }
        } else if !joined.starts_with(scope_root) {
            bail!("node {} filePath escapes graph scope", node.id);
        }
    }
    Ok(())
}

fn resolve_inside_root(project_root: &Path, file_path: &Path) -> Option<PathBuf> {
    let joined = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        project_root.join(normalize_relative_path(file_path)?)
    };
    let resolved = if joined.exists() {
        joined.canonicalize().ok()?
    } else {
        joined
    };
    resolved.starts_with(project_root).then_some(resolved)
}

fn normalize_relative(value: &str) -> Option<PathBuf> {
    let portable = value.replace('\\', "/");
    normalize_relative_path(Path::new(&portable))
}

fn normalized_graph_path(value: &str) -> Option<String> {
    normalize_relative(value).map(|path| rel_to_unix(&path))
}

fn normalize_relative_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

fn scope_components(scope_path: &str) -> Vec<Component<'_>> {
    if scope_path == "." {
        Vec::new()
    } else {
        Path::new(scope_path).components().collect()
    }
}

fn rel_to_unix(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_sort_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn catalog_freshness_hash(snapshots: &[GraphSnapshot]) -> String {
    stable_hash_parts(snapshots.iter().flat_map(|snapshot| {
        [
            snapshot.identity.as_bytes(),
            snapshot.content_hash.as_bytes(),
        ]
    }))
}

/// Stable FNV-1a is sufficient here: this is a deterministic cache/freshness
/// identity, not a cryptographic authenticity boundary.
fn stable_hash(bytes: &[u8]) -> String {
    stable_hash_parts(std::iter::once(bytes))
}

fn stable_hash_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in (part.len() as u64).to_le_bytes().iter().chain(part.iter()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("{hash:016x}")
}

/// UTF-8 first; fall back to GBK only when the bytes are not valid UTF-8.
fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => {
            let (text, _, _) = encoding_rs::GBK.decode(bytes);
            text.into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "fluid-graph-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_graph(scope: &Path, origin: GraphOrigin, files: &[(&str, &str)]) {
        let directory = scope.join(origin.directory());
        fs::create_dir_all(&directory).unwrap();
        let nodes: Vec<_> = files
            .iter()
            .map(|(path, summary)| {
                serde_json::json!({
                    "id": format!("file:{path}"),
                    "type": "file",
                    "name": path,
                    "filePath": path,
                    "summary": summary
                })
            })
            .collect();
        fs::write(
            directory.join(GRAPH_FILE),
            serde_json::json!({ "nodes": nodes, "edges": [] }).to_string(),
        )
        .unwrap();
    }

    fn graph_summary(snapshot: &GraphSnapshot) -> &str {
        &snapshot.graph.nodes[0].summary
    }

    #[test]
    fn parses_minimal_graph_with_chinese_summary() {
        let json = r#"{
            "version": "1",
            "project": {"name": "x"},
            "nodes": [
                {"id":"file:a.py","type":"file","name":"a.py","filePath":"a.py",
                 "summary":"执行模块的配置类","tags":["config"],"complexity":"simple"},
                {"id":"class:a.py:C","type":"class","name":"C","filePath":"a.py",
                 "lineRange":[8,38],"summary":"类","tags":[]}
            ],
            "edges": [
                {"source":"file:a.py","target":"class:a.py:C","type":"contains","direction":"forward","weight":1}
            ]
        }"#;
        let graph: KnowledgeGraph = serde_json::from_str(json).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.nodes[0].summary, "执行模块的配置类");
        assert_eq!(graph.nodes[1].line_range, Some([8, 38]));
        assert_eq!(graph.edges[0].edge_type, "contains");
    }

    #[test]
    fn utf8_text_passes_through_unchanged() {
        let text = "执行模块";
        assert_eq!(decode_text(text.as_bytes()), text);
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn invalid_utf8_falls_back_to_gbk() {
        let gbk = [0xD6u8, 0xB4, 0xD0, 0xD0];
        assert!(std::str::from_utf8(&gbk).is_err());
        assert_eq!(decode_text(&gbk), "执行");
    }

    #[test]
    fn only_root_graph_is_discovered_and_projected() {
        let project = TempDir::new("root-only");
        write_graph(project.path(), GraphOrigin::Ua, &[("src/a.rs", "root")]);

        let catalog = GraphCatalog::discover(project.path());

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.root_graph().unwrap().nodes[0].summary, "root");
        assert_eq!(
            catalog
                .resolve_for_file(Path::new("src/a.rs"))
                .unwrap()
                .scope_path(),
            "."
        );
    }

    #[test]
    fn missing_graphs_yield_an_empty_optional_catalog() {
        let project = TempDir::new("missing");

        let catalog = GraphCatalog::discover(project.path());

        assert!(catalog.is_empty());
        assert!(catalog.root_graph().is_none());
        assert!(catalog.resolve_for_file(Path::new("src/a.rs")).is_none());
    }

    #[test]
    fn only_nested_graph_does_not_leak_into_root_projection() {
        let project = TempDir::new("nested-only");
        let nested = project.path().join("packages/app");
        write_graph(&nested, GraphOrigin::Ua, &[("src/app.rs", "nested")]);

        let catalog = GraphCatalog::discover(project.path());

        assert_eq!(catalog.len(), 1);
        assert!(catalog.root_graph().is_none());
        assert!(catalog.resolve_for_file(Path::new("README.md")).is_none());
        let snapshot = catalog
            .resolve_for_file(Path::new("packages/app/src/app.rs"))
            .unwrap();
        assert_eq!(snapshot.scope_path(), "packages/app");
        assert_eq!(
            snapshot
                .graph_relative_path("packages/app/src/app.rs")
                .as_deref(),
            Some("src/app.rs")
        );
        assert_eq!(
            snapshot.project_relative_path("src/app.rs").as_deref(),
            Some("packages/app/src/app.rs")
        );
    }

    #[test]
    fn node_paths_are_normalized_within_their_scope() {
        let project = TempDir::new("normalized-node-path");
        let nested = project.path().join("packages/app");
        write_graph(&nested, GraphOrigin::Ua, &[("./src\\app.rs", "nested")]);

        let catalog = GraphCatalog::discover(project.path());
        let snapshot = catalog
            .resolve_for_file(Path::new("packages/app/src/app.rs"))
            .unwrap();

        assert_eq!(snapshot.graph().nodes[0].file_path, "src/app.rs");
        assert_eq!(
            snapshot
                .project_relative_path(&snapshot.graph().nodes[0].file_path)
                .as_deref(),
            Some("packages/app/src/app.rs")
        );
    }

    #[test]
    fn nearest_ancestor_wins_and_missing_local_graph_inherits_parent() {
        let project = TempDir::new("nearest");
        write_graph(project.path(), GraphOrigin::Legacy, &[("root.rs", "root")]);
        let child = project.path().join("crates/child");
        write_graph(&child, GraphOrigin::Ua, &[("src/lib.rs", "child")]);

        let catalog = GraphCatalog::discover(project.path());

        assert_eq!(
            graph_summary(
                catalog
                    .resolve_for_file(Path::new("crates/child/src/lib.rs"))
                    .unwrap()
            ),
            "child"
        );
        assert_eq!(
            graph_summary(
                catalog
                    .resolve_for_file(Path::new("crates/other/src/lib.rs"))
                    .unwrap()
            ),
            "root"
        );
        assert_eq!(
            graph_summary(
                catalog
                    .resolve_for_file(Path::new("crates/child/deeper/file.rs"))
                    .unwrap()
            ),
            "child"
        );
    }

    #[test]
    fn sibling_scopes_are_all_discovered_in_deterministic_order() {
        let project = TempDir::new("siblings");
        write_graph(
            &project.path().join("zeta"),
            GraphOrigin::Ua,
            &[("z.rs", "z")],
        );
        write_graph(
            &project.path().join("alpha"),
            GraphOrigin::Ua,
            &[("a.rs", "a")],
        );

        let catalog = GraphCatalog::discover(project.path());
        let scopes: Vec<_> = catalog
            .snapshots()
            .iter()
            .map(GraphSnapshot::scope_path)
            .collect();

        assert_eq!(scopes, vec!["alpha", "zeta"]);
        assert_eq!(
            graph_summary(catalog.resolve_for_file(Path::new("zeta/z.rs")).unwrap()),
            "z"
        );
    }

    #[test]
    fn ua_wins_per_scope_and_corrupt_ua_falls_back_to_legacy() {
        let project = TempDir::new("priority");
        write_graph(project.path(), GraphOrigin::Legacy, &[("a.rs", "legacy")]);
        write_graph(project.path(), GraphOrigin::Ua, &[("a.rs", "ua")]);
        let catalog = GraphCatalog::discover(project.path());
        assert_eq!(catalog.root_snapshot().unwrap().origin(), GraphOrigin::Ua);
        assert_eq!(graph_summary(catalog.root_snapshot().unwrap()), "ua");

        fs::write(
            GraphOrigin::Ua.candidate(project.path()),
            "{ definitely broken",
        )
        .unwrap();
        let catalog = GraphCatalog::discover(project.path());
        assert_eq!(
            catalog.root_snapshot().unwrap().origin(),
            GraphOrigin::Legacy
        );
        assert_eq!(graph_summary(catalog.root_snapshot().unwrap()), "legacy");
    }

    #[test]
    fn corrupt_scope_does_not_hide_valid_sibling_scope() {
        let project = TempDir::new("partial");
        let broken = project.path().join("broken");
        fs::create_dir_all(broken.join(".ua")).unwrap();
        fs::write(broken.join(".ua").join(GRAPH_FILE), "not json").unwrap();
        let healthy = project.path().join("healthy");
        write_graph(&healthy, GraphOrigin::Ua, &[("ok.rs", "healthy")]);

        let catalog = GraphCatalog::discover(project.path());

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.snapshots()[0].scope_path(), "healthy");
    }

    #[test]
    fn unsafe_node_path_invalidates_candidate_and_all_missing_is_empty() {
        let project = TempDir::new("unsafe-node");
        write_graph(project.path(), GraphOrigin::Ua, &[("../escape.rs", "bad")]);

        let catalog = GraphCatalog::discover(project.path());

        assert!(catalog.is_empty());
        assert!(catalog.root_graph().is_none());
        assert!(catalog.resolve_for_file(Path::new("src/a.rs")).is_none());
    }

    #[test]
    fn refresh_detects_content_change_and_nearer_scope_add_remove() {
        let project = TempDir::new("refresh");
        write_graph(project.path(), GraphOrigin::Ua, &[("child/a.rs", "root")]);
        let mut catalog = GraphCatalog::discover(project.path());
        let initial_hash = catalog.root_snapshot().unwrap().content_hash().to_string();
        assert_eq!(
            graph_summary(catalog.resolve_for_file(Path::new("child/a.rs")).unwrap()),
            "root"
        );

        write_graph(
            project.path(),
            GraphOrigin::Ua,
            &[("child/a.rs", "root-v2")],
        );
        assert!(catalog.refresh());
        assert_ne!(
            catalog.root_snapshot().unwrap().content_hash(),
            initial_hash
        );

        let child = project.path().join("child");
        write_graph(&child, GraphOrigin::Ua, &[("a.rs", "child")]);
        assert!(catalog.refresh());
        assert_eq!(
            graph_summary(catalog.resolve_for_file(Path::new("child/a.rs")).unwrap()),
            "child"
        );

        fs::remove_dir_all(child.join(".ua")).unwrap();
        assert!(catalog.refresh());
        assert_eq!(
            graph_summary(catalog.resolve_for_file(Path::new("child/a.rs")).unwrap()),
            "root-v2"
        );
        assert!(!catalog.refresh());
    }

    #[test]
    fn relevant_hash_ignores_unrelated_nodes_but_tracks_relevant_slice() {
        let project = TempDir::new("relevant-hash");
        write_graph(
            project.path(),
            GraphOrigin::Ua,
            &[("a.rs", "A"), ("b.rs", "B")],
        );
        let mut catalog = GraphCatalog::discover(project.path());
        let files = vec!["a.rs".to_string()];
        let first = catalog.relevant_graph_set_hash(&files);

        write_graph(
            project.path(),
            GraphOrigin::Ua,
            &[("a.rs", "A"), ("b.rs", "B changed")],
        );
        assert!(catalog.refresh());
        assert_eq!(catalog.relevant_graph_set_hash(&files), first);

        write_graph(
            project.path(),
            GraphOrigin::Ua,
            &[("a.rs", "A changed"), ("b.rs", "B changed")],
        );
        assert!(catalog.refresh());
        assert_ne!(catalog.relevant_graph_set_hash(&files), first);
    }

    #[test]
    fn graph_directory_symlink_that_escapes_project_is_rejected() {
        let project = TempDir::new("symlink-project");
        let outside = TempDir::new("symlink-outside");
        write_graph(
            outside.path(),
            GraphOrigin::Ua,
            &[("outside.rs", "outside")],
        );
        let target = outside.path().join(".ua");
        let link = project.path().join(".ua");

        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&target, &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&target, &link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let linked = false;

        if !linked {
            return;
        }

        let catalog = GraphCatalog::discover(project.path());
        assert!(catalog.is_empty());
    }
}
