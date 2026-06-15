//! ContextAssembler — builds the shared context injected into a per-function
//! generation request, and the prompt sent to the LLM (S6).
//!
//! Per ADR-0004, each function-capsule request carries: the file-level summary,
//! the file's full function roster, relevant edges (calls/imports), and one-liner
//! summaries of cross-file callees (ADR-0007/0011). Source priority is: the
//! request body (what the frontend's tree-sitter pass already computed) → the
//! understand-anything graph (when present) → omitted.
//!
//! S6 scope: prefer-request-then-graph assembly + prompt construction. The
//! §5 fallbacks that cost extra LLM calls (LLM_summarizeFile, LLM_oneLine for
//! callees) are deferred — when neither request nor graph supplies them, they are
//! simply omitted so S6 stays a single LLM call per function.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph_loader::{GraphEdge, KnowledgeGraph};

/// A function as located by the frontend's tree-sitter pass (技术方案 §3).
/// `lineRange` is 1-based inclusive `[start, end]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionSpan {
    pub id: String,
    pub name: String,
    #[serde(rename = "lineRange")]
    pub line_range: [u32; 2],
}

/// Optional shared context the client may pre-fill (all fields fall back to the
/// graph or are omitted). Mirrors `shared` in the `/api/generate` contract.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SharedContext {
    #[serde(rename = "fileSummary")]
    pub file_summary: Option<String>,
    pub edges: Option<Vec<GraphEdge>>,
    #[serde(rename = "calleeSummaries")]
    pub callee_summaries: Option<BTreeMap<String, String>>,
}

/// The assembled context handed to the prompt builder.
pub struct GenContext {
    pub file_summary: Option<String>,
    pub roster: Vec<String>,
    pub edges: Vec<GraphEdge>,
    pub callee_summaries: BTreeMap<String, String>,
}

/// Assemble generation context: request value wins, else graph, else empty/omitted
/// (技术方案 §5, S6 minimal — no extra LLM calls).
pub fn assemble_gen_context(
    graph: Option<&KnowledgeGraph>,
    file_path: &str,
    roster: &[String],
    shared: &SharedContext,
) -> GenContext {
    let file_summary = shared
        .file_summary
        .clone()
        .or_else(|| graph.and_then(|g| file_summary_from_graph(g, file_path)));

    let edges = shared
        .edges
        .clone()
        .unwrap_or_else(|| graph.map(|g| edges_for_file(g, file_path)).unwrap_or_default());

    let callee_summaries = shared.callee_summaries.clone().unwrap_or_default();

    GenContext {
        file_summary,
        roster: roster.to_vec(),
        edges,
        callee_summaries,
    }
}

/// The summary of the `file` node for `file_path`, if the graph has one.
fn file_summary_from_graph(g: &KnowledgeGraph, file_path: &str) -> Option<String> {
    g.nodes
        .iter()
        .find(|n| n.node_type == "file" && n.file_path == file_path && !n.summary.is_empty())
        .map(|n| n.summary.clone())
}

/// Edges whose source node lives in `file_path` and is a calls/imports relation —
/// the meso context for functions in this file (ADR-0004).
fn edges_for_file(g: &KnowledgeGraph, file_path: &str) -> Vec<GraphEdge> {
    let local_ids: std::collections::HashSet<&str> = g
        .nodes
        .iter()
        .filter(|n| n.file_path == file_path)
        .map(|n| n.id.as_str())
        .collect();

    g.edges
        .iter()
        .filter(|e| {
            matches!(e.edge_type.as_str(), "calls" | "imports")
                && local_ids.contains(e.source.as_str())
        })
        .cloned()
        .collect()
}

/// Slice a 1-based inclusive line range out of a source string. Returns `None`
/// if the range is empty or out of bounds.
pub fn slice_span(source: &str, line_range: [u32; 2]) -> Option<String> {
    let [start, end] = line_range;
    if start == 0 || end < start {
        return None;
    }
    let lines: Vec<&str> = source.lines().collect();
    let (s, e) = (start as usize - 1, end as usize - 1);
    if e >= lines.len() {
        return None;
    }
    Some(lines[s..=e].join("\n"))
}

/// Build the (system, user) messages for a single function's generation.
/// The function source is presented with absolute line numbers so the model can
/// attach line annotations by number (技术方案 §7.3, key lines).
pub fn build_gen_prompt(
    func: &FunctionSpan,
    fn_source: &str,
    key_lines: &[u32],
    ctx: &GenContext,
) -> (String, String) {
    let system = "你是 Fluid 的代码理解助手，面向零代码基础的读者。\
针对给定的【单个函数】，用简体中文生成语义投影。\
要求：summary 讲清这个函数“做什么、为什么”，避免逐字复述代码；\
io 用一句话抽象输入与输出；complexity 取 simple/moderate/complex 之一；\
signature 给出函数签名。\
对【需要标注的重点行】各写一句话注释（text），并给一个语义色温的十六进制颜色（color，如 #7ee787 表正常流、#f0883e 表分支、#ff7b72 表异常/return）。\
只输出一个 JSON 对象，禁止任何额外文字或 markdown 代码围栏。\
JSON 形如：{\"capsule\":{\"signature\":\"...\",\"summary\":\"...\",\"complexity\":\"simple\",\"io\":\"...\"},\"lines\":[{\"lineNumber\":12,\"text\":\"...\",\"color\":\"#7ee787\"}]}";

    let mut user = String::new();
    if let Some(fs) = &ctx.file_summary {
        user.push_str(&format!("【文件摘要】{fs}\n"));
    }
    if !ctx.roster.is_empty() {
        user.push_str(&format!("【本文件函数清单】{}\n", ctx.roster.join(", ")));
    }
    if !ctx.edges.is_empty() {
        let rels: Vec<String> = ctx
            .edges
            .iter()
            .map(|e| format!("{}-{}->{}", e.source, e.edge_type, e.target))
            .collect();
        user.push_str(&format!("【相关关系(calls/imports)】{}\n", rels.join("; ")));
    }
    if !ctx.callee_summaries.is_empty() {
        let cs: Vec<String> = ctx
            .callee_summaries
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        user.push_str(&format!("【被调对象一句话摘要】{}\n", cs.join("; ")));
    }

    user.push_str(&format!("\n【目标函数】{}\n", func.name));
    if key_lines.is_empty() {
        user.push_str("【需要标注的重点行】无（lines 返回空数组）\n");
    } else {
        let ks: Vec<String> = key_lines.iter().map(|n| n.to_string()).collect();
        user.push_str(&format!("【需要标注的重点行(行号)】{}\n", ks.join(", ")));
    }
    user.push_str("【源码(带绝对行号)】\n");
    user.push_str(&number_lines(fn_source, func.line_range[0]));

    (system.to_string(), user)
}

/// Build the (system, user) messages for explaining ONE arbitrary line (S9 manual
/// line fill). Unlike `build_gen_prompt` this asks for a single annotation on the
/// target line, returned as a bare `{text, color}` JSON object. The enclosing
/// function source is shown with absolute line numbers so the model can ground the
/// target line in its local context.
pub fn build_explain_line_prompt(
    func: &FunctionSpan,
    fn_source: &str,
    line_number: u32,
    ctx: &GenContext,
) -> (String, String) {
    let system = "你是 Fluid 的代码理解助手，面向零代码基础的读者。\
用户指定了某个函数内的【某一行】，请用一句简体中文解释这一行在做什么、为什么，\
结合所在函数的上下文，但避免逐字复述代码。\
给一个语义色温的十六进制颜色（color，如 #7ee787 表正常流、#f0883e 表分支、#ff7b72 表异常/return）。\
只输出一个 JSON 对象，禁止任何额外文字或 markdown 代码围栏。\
JSON 形如：{\"text\":\"...\",\"color\":\"#7ee787\"}";

    let mut user = String::new();
    if let Some(fs) = &ctx.file_summary {
        user.push_str(&format!("【文件摘要】{fs}\n"));
    }
    user.push_str(&format!("【所在函数】{}\n", func.name));
    user.push_str(&format!("【目标行号】{line_number}\n"));
    user.push_str("【源码(带绝对行号)】\n");
    user.push_str(&number_lines(fn_source, func.line_range[0]));

    (system.to_string(), user)
}

/// Build the (system, user) messages for a free-form follow-up question about the
/// current file (S10a query). ADR-0006 default tier: the *whole file is present at
/// summary granularity* (file summary + every function's capsule summary + edges +
/// cross-file one-liners) so the model keeps global sight, while only the focused
/// function is zoomed to *source granularity*. The answer is free-form markdown
/// (not JSON), streamed back token-by-token — there is no parse step.
///
/// `capsules` is `(fn name, summary)` for the file's already-generated functions;
/// `focus` is the source (with its 1-based start line) of the function the user is
/// focused on, or `None` for a file-level question.
pub fn build_query_prompt(
    question: &str,
    capsules: &[(String, String)],
    focus: Option<(&str, u32)>,
    ctx: &GenContext,
) -> (String, String) {
    let system = "你是 Fluid 的代码理解助手，面向零代码基础的读者。\
基于下面给定的【当前文件上下文】回答用户的追问，用简体中文，可使用简单 markdown。\
只依据给定信息作答；信息不足时直说，不要臆造未给出的代码细节。";

    let mut user = String::new();
    if let Some(fs) = &ctx.file_summary {
        user.push_str(&format!("【文件摘要】{fs}\n"));
    }
    if !ctx.roster.is_empty() {
        user.push_str(&format!("【本文件函数清单】{}\n", ctx.roster.join(", ")));
    }
    if !capsules.is_empty() {
        let cs: Vec<String> = capsules
            .iter()
            .map(|(name, summary)| format!("{name}: {summary}"))
            .collect();
        user.push_str(&format!("【各函数摘要】{}\n", cs.join("; ")));
    }
    if !ctx.edges.is_empty() {
        let rels: Vec<String> = ctx
            .edges
            .iter()
            .map(|e| format!("{}-{}->{}", e.source, e.edge_type, e.target))
            .collect();
        user.push_str(&format!("【相关关系(calls/imports)】{}\n", rels.join("; ")));
    }
    if !ctx.callee_summaries.is_empty() {
        let cs: Vec<String> = ctx
            .callee_summaries
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        user.push_str(&format!("【跨文件被调摘要】{}\n", cs.join("; ")));
    }
    if let Some((src, start)) = focus {
        user.push_str("【聚焦函数源码(带绝对行号)】\n");
        user.push_str(&number_lines(src, start));
        user.push('\n');
    }
    user.push_str(&format!("\n【用户问题】{question}\n"));

    (system.to_string(), user)
}

/// Prefix each line with its absolute line number, e.g. `  12 | code`.
fn number_lines(src: &str, start_line: u32) -> String {
    src.lines()
        .enumerate()
        .map(|(i, line)| format!("{:>4} | {}", start_line + i as u32, line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_loader::{GraphNode, KnowledgeGraph};

    fn node(id: &str, ty: &str, file: &str, summary: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            node_type: ty.to_string(),
            name: id.to_string(),
            file_path: file.to_string(),
            summary: summary.to_string(),
            tags: vec![],
            complexity: None,
            line_range: None,
            language_notes: None,
        }
    }

    fn edge(src: &str, tgt: &str, ty: &str) -> GraphEdge {
        GraphEdge {
            source: src.to_string(),
            target: tgt.to_string(),
            edge_type: ty.to_string(),
            direction: None,
            weight: None,
        }
    }

    #[test]
    fn slice_span_extracts_inclusive_1based_range() {
        let src = "a\nb\nc\nd\n";
        assert_eq!(slice_span(src, [2, 3]).as_deref(), Some("b\nc"));
        assert_eq!(slice_span(src, [1, 1]).as_deref(), Some("a"));
    }

    #[test]
    fn slice_span_rejects_out_of_bounds_and_empty() {
        let src = "a\nb\n";
        assert_eq!(slice_span(src, [0, 1]), None);
        assert_eq!(slice_span(src, [2, 1]), None);
        assert_eq!(slice_span(src, [1, 9]), None);
    }

    #[test]
    fn request_values_win_over_graph() {
        let g = KnowledgeGraph {
            nodes: vec![node("file:a.py", "file", "a.py", "图谱给的摘要")],
            edges: vec![],
        };
        let shared = SharedContext {
            file_summary: Some("请求给的摘要".into()),
            edges: None,
            callee_summaries: None,
        };
        let ctx = assemble_gen_context(Some(&g), "a.py", &["f".into()], &shared);
        assert_eq!(ctx.file_summary.as_deref(), Some("请求给的摘要"));
    }

    #[test]
    fn falls_back_to_graph_summary_and_filters_edges_by_file() {
        let g = KnowledgeGraph {
            nodes: vec![
                node("file:a.py", "file", "a.py", "执行模块的配置类"),
                node("function:a.py:f", "function", "a.py", ""),
                node("function:b.py:g", "function", "b.py", ""),
            ],
            edges: vec![
                edge("function:a.py:f", "function:b.py:g", "calls"), // local source → kept
                edge("function:b.py:g", "function:a.py:f", "calls"), // foreign source → dropped
                edge("function:a.py:f", "file:a.py", "contains"),    // wrong type → dropped
            ],
        };
        let ctx = assemble_gen_context(Some(&g), "a.py", &[], &SharedContext::default());
        assert_eq!(ctx.file_summary.as_deref(), Some("执行模块的配置类"));
        assert_eq!(ctx.edges.len(), 1);
        assert_eq!(ctx.edges[0].edge_type, "calls");
        assert_eq!(ctx.edges[0].source, "function:a.py:f");
    }

    #[test]
    fn no_graph_yields_empty_context_no_panic() {
        let ctx = assemble_gen_context(None, "a.py", &["f".into(), "g".into()], &SharedContext::default());
        assert!(ctx.file_summary.is_none());
        assert!(ctx.edges.is_empty());
        assert_eq!(ctx.roster, vec!["f".to_string(), "g".to_string()]);
    }

    #[test]
    fn prompt_numbers_lines_from_absolute_start() {
        let func = FunctionSpan {
            id: "f#10".into(),
            name: "f".into(),
            line_range: [10, 11],
        };
        let ctx = assemble_gen_context(None, "a.py", &["f".into()], &SharedContext::default());
        let (system, user) = build_gen_prompt(&func, "def f():\n    return 1", &[11], &ctx);
        assert!(system.contains("只输出一个 JSON"));
        assert!(user.contains("  10 | def f():"));
        assert!(user.contains("  11 |     return 1"));
        assert!(user.contains("【需要标注的重点行(行号)】11"));
    }

    #[test]
    fn explain_line_prompt_numbers_lines_and_targets_the_line() {
        let func = FunctionSpan {
            id: "f#10".into(),
            name: "f".into(),
            line_range: [10, 12],
        };
        let ctx = assemble_gen_context(None, "a.py", &["f".into()], &SharedContext::default());
        let (system, user) =
            build_explain_line_prompt(&func, "def f():\n    y = 1\n    return y", 11, &ctx);
        assert!(system.contains("某一行"));
        assert!(system.contains("{\"text\":"));
        assert!(user.contains("【所在函数】f"));
        assert!(user.contains("【目标行号】11"));
        assert!(user.contains("  11 |     y = 1"));
    }

    #[test]
    fn query_prompt_carries_layered_context_and_focus_source() {
        let g = KnowledgeGraph {
            nodes: vec![node("file:a.py", "file", "a.py", "配置加载模块")],
            edges: vec![],
        };
        let ctx = assemble_gen_context(Some(&g), "a.py", &["load".into(), "save".into()], &SharedContext::default());
        let capsules = vec![
            ("load".to_string(), "读配置".to_string()),
            ("save".to_string(), "写配置".to_string()),
        ];
        let (system, user) = build_query_prompt(
            "load 为什么要先校验？",
            &capsules,
            Some(("def load():\n    return 1", 10)),
            &ctx,
        );
        assert!(system.contains("当前文件上下文"));
        assert!(user.contains("【文件摘要】配置加载模块"));
        assert!(user.contains("【本文件函数清单】load, save"));
        assert!(user.contains("【各函数摘要】load: 读配置; save: 写配置"));
        assert!(user.contains("【聚焦函数源码(带绝对行号)】"));
        assert!(user.contains("  10 | def load():"));
        assert!(user.contains("【用户问题】load 为什么要先校验？"));
    }

    #[test]
    fn query_prompt_omits_focus_and_capsules_when_absent() {
        let ctx = assemble_gen_context(None, "a.py", &[], &SharedContext::default());
        let (_, user) = build_query_prompt("这个文件是做什么的？", &[], None, &ctx);
        assert!(!user.contains("【各函数摘要】"));
        assert!(!user.contains("【聚焦函数源码"));
        assert!(user.contains("【用户问题】这个文件是做什么的？"));
    }
}
