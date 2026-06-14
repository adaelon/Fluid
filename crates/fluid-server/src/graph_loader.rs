//! GraphLoader — reads the optional `.understand-anything/knowledge-graph.json`.
//!
//! S2 scope: load nodes/edges, decode text correctly, expose the graph (or
//! `None` when absent/malformed). understand-anything is an *optional* enhancement
//! (ADR-0011): a missing or broken graph must never crash the server.
//!
//! Encoding note: the real alphaGPT graph is valid UTF-8 Chinese — the "GBK
//! mojibake" in the design docs was a console-rendering artifact, not on-disk
//! corruption. So we decode UTF-8 first and only fall back to GBK if the bytes
//! are genuinely not valid UTF-8, never transcoding valid UTF-8.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// A node in the understand-anything knowledge graph.
/// Field names are kept camelCase so the API re-serializes them as the frontend
/// expects (技术方案 §3).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// The subset of the graph Fluid consumes: nodes + edges. Other top-level fields
/// (version/project/layers/tour) are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

pub struct GraphLoader {
    graph: Option<KnowledgeGraph>,
}

impl GraphLoader {
    /// Load the graph from `<root>/.understand-anything/knowledge-graph.json`.
    /// Absent or malformed → `None` (server keeps running, ADR-0011).
    pub fn load(project_root: &Path) -> Self {
        let path = project_root
            .join(".understand-anything")
            .join("knowledge-graph.json");
        if !path.exists() {
            return Self { graph: None };
        }
        match read_graph(&path) {
            Ok(g) => Self { graph: Some(g) },
            Err(e) => {
                eprintln!("warning: failed to load knowledge graph: {e}");
                Self { graph: None }
            }
        }
    }

    pub fn graph(&self) -> Option<&KnowledgeGraph> {
        self.graph.as_ref()
    }
}

fn read_graph(path: &Path) -> anyhow::Result<KnowledgeGraph> {
    let bytes = std::fs::read(path)?;
    let text = decode_text(&bytes);
    let text = text.trim_start_matches('\u{feff}'); // strip UTF-8 BOM if present
    let graph: KnowledgeGraph = serde_json::from_str(text)?;
    Ok(graph)
}

/// UTF-8 first; fall back to GBK only when the bytes are not valid UTF-8.
fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GBK.decode(bytes);
            cow.into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let g: KnowledgeGraph = serde_json::from_str(json).unwrap();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.nodes[0].summary, "执行模块的配置类");
        assert_eq!(g.nodes[1].line_range, Some([8, 38]));
        assert_eq!(g.edges[0].edge_type, "contains");
    }

    #[test]
    fn utf8_text_passes_through_unchanged() {
        let s = "执行模块";
        assert_eq!(decode_text(s.as_bytes()), s);
    }

    #[test]
    #[allow(invalid_from_utf8)] // intentional: these bytes are deliberately not UTF-8
    fn invalid_utf8_falls_back_to_gbk() {
        // GBK bytes for 执行 (0xD6 0xB4 0xD0 0xD0) — not valid UTF-8.
        let gbk = [0xD6u8, 0xB4, 0xD0, 0xD0];
        assert!(std::str::from_utf8(&gbk).is_err());
        assert_eq!(decode_text(&gbk), "执行");
    }

    #[test]
    fn missing_graph_yields_none() {
        let loader = GraphLoader::load(Path::new("/nonexistent/project/path/xyz"));
        assert!(loader.graph().is_none());
    }
}
