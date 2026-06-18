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

use std::collections::{BTreeMap, HashSet};

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

/// Prompt for explaining one MODULE-LEVEL declaration on demand (S-TS-3, 手动补行
/// 的声明粒度泛化). Unlike the line prompt it isn't inside a function — `decl_source`
/// is the declaration's own span, `kind` its coarse kind (const/let/type/interface/
/// enum) and `name` its identifier. Same `LineAnnotation` shape so the frontend
/// renders it as a trailing note on the declaration's first line.
pub fn build_explain_decl_prompt(
    name: &str,
    kind: &str,
    decl_source: &str,
    start_line: u32,
    ctx: &GenContext,
) -> (String, String) {
    let system = "你是 Fluid 的代码理解助手，面向零代码基础的读者。\
用户指定了一个模块顶层的【声明】(const/let/type/interface/enum 之一)，\
请用一句简体中文解释它是什么、用来做什么，避免逐字复述代码。\
给一个语义色温的十六进制颜色(color，如 #7ee787 表数据/常量、#f0883e 表类型/接口、#ff7b72 表特殊)。\
只输出一个 JSON 对象，禁止任何额外文字或 markdown 代码围栏。\
JSON 形如：{\"text\":\"...\",\"color\":\"#7ee787\"}";

    let mut user = String::new();
    if let Some(fs) = &ctx.file_summary {
        user.push_str(&format!("【文件摘要】{fs}\n"));
    }
    user.push_str(&format!("【声明种类】{kind}\n"));
    user.push_str(&format!("【声明名称】{name}\n"));
    user.push_str("【源码(带绝对行号)】\n");
    user.push_str(&number_lines(decl_source, start_line));

    (system.to_string(), user)
}

/// A focused function for a query: its source (zoomed to source granularity), the
/// 1-based start line for absolute numbering, and its name — the name lets the
/// degradation ladder prioritize this function and its neighbors' capsule
/// summaries when the context must be trimmed.
pub struct QueryFocus<'a> {
    pub source: &'a str,
    pub start_line: u32,
    pub name: &'a str,
}

/// Char-count proxy for the query context budget (ADR-0006 degradation ladder).
/// We carry no tokenizer (no extra dep), so the assembled context is bounded by
/// characters rather than true tokens — enough to deterministically trigger
/// degradation and keep the prompt bounded on large files. Set generously so
/// ordinary files never degrade; freely tunable (reverse cost is nil).
pub const QUERY_CONTEXT_BUDGET_CHARS: usize = 24_000;

/// Char-count cap on the function sources appended by on-demand fetch (S10a-追源,
/// ADR-0017). Bounds the phase-2 prompt so the round-trip can't reintroduce the
/// over-window blow-up the degradation ladder just avoided. Char proxy, like the
/// context budget — same rationale (no tokenizer dep).
pub const QUERY_FETCH_BUDGET_CHARS: usize = 12_000;

/// Build the (system, user) messages for a free-form follow-up question about the
/// current file (S10a query). ADR-0006 default tier: the *whole file is present at
/// summary granularity* (file summary + every function's capsule summary + edges +
/// cross-file one-liners) so the model keeps global sight, while only the focused
/// function is zoomed to *source granularity*. The answer is free-form markdown
/// (not JSON), streamed back token-by-token — there is no parse step.
///
/// **Over-window degradation (S10a-降级, ADR-0006 ladder):** the per-function
/// capsule summaries are the elastic part that blows up on large files. When the
/// assembled context would exceed `QUERY_CONTEXT_BUDGET_CHARS`, summaries are kept
/// greedily by priority — the focused function first, then its roster-neighbors
/// outward — until the budget is spent; the remaining (distant) functions degrade
/// to name-only (their names still appear in the roster line). The fixed spine
/// (file summary, roster, edges, callees, focus source, question) is never dropped.
/// Truncating the focus source and the model's on-demand source fetch are out of
/// scope (separate slice).
///
/// `capsules` is `(fn name, summary)` for the file's already-generated functions
/// (in roster/source order); `focus` is the focused function or `None` for a
/// file-level question. `extra_sources` is `(fn name, already-numbered source)` for
/// functions pulled back by on-demand fetch (S10a-追源, ADR-0017) — empty on the
/// single-call path.
pub fn build_query_prompt(
    question: &str,
    capsules: &[(String, String)],
    focus: Option<QueryFocus>,
    ctx: &GenContext,
    extra_sources: &[(String, String)],
) -> (String, String) {
    let system = "你是 Fluid 的代码理解助手，面向零代码基础的读者。\
基于下面给定的【当前文件上下文】回答用户的追问，用简体中文，可使用简单 markdown；\
需要数学公式时用 LaTeX（行内 $...$、块级 $$...$$）。\
只依据给定信息作答；信息不足时直说，不要臆造未给出的代码细节。";

    // The capsule summaries are elastic; the rest is the fixed spine. Measure the
    // spine, then fit summaries into the remaining budget by priority (focus +
    // neighbors outward). Unkept functions degrade to name-only via the roster line.
    let spine_len = query_spine_chars(question, ctx, focus.as_ref());
    let focus_name = focus.as_ref().map(|f| f.name);
    let included = select_capsule_summaries(
        capsules,
        focus_name,
        QUERY_CONTEXT_BUDGET_CHARS.saturating_sub(spine_len),
    );
    let degraded = included.len() < capsules.len();

    let mut user = String::new();
    if let Some(fs) = &ctx.file_summary {
        user.push_str(&format!("【文件摘要】{fs}\n"));
    }
    if !ctx.roster.is_empty() {
        user.push_str(&format!("【本文件函数清单】{}\n", ctx.roster.join(", ")));
    }
    if !included.is_empty() {
        let cs: Vec<String> = included
            .iter()
            .map(|&i| format!("{}: {}", capsules[i].0, capsules[i].1))
            .collect();
        user.push_str(&format!("【各函数摘要】{}\n", cs.join("; ")));
    }
    if degraded {
        user.push_str("（上下文超长，其余函数仅在上面的清单中列名、未含摘要）\n");
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
    if let Some(f) = &focus {
        user.push_str("【聚焦函数源码(带绝对行号)】\n");
        user.push_str(&number_lines(f.source, f.start_line));
        user.push('\n');
    }
    for (name, src) in extra_sources {
        user.push_str(&format!("【按需追加的函数源码: {name}(带绝对行号)】\n"));
        user.push_str(src);
        user.push('\n');
    }
    user.push_str(&format!("\n【用户问题】{question}\n"));

    (system.to_string(), user)
}

/// Build the (system, user) messages for the phase-1 *planning* call of on-demand
/// fetch (S10a-追源, ADR-0017). Same file context as the answer prompt, plus the
/// list of `fetchable` functions the model currently has *only the name* of, asking
/// it to name which ones' source it needs. The reply is a bare `{"need":[...]}` JSON
/// (parsed by `parse_fetch_plan`); a non-streaming call (`complete`).
pub fn build_query_planning_prompt(
    question: &str,
    capsules: &[(String, String)],
    focus: Option<QueryFocus>,
    ctx: &GenContext,
    fetchable: &[String],
) -> (String, String) {
    let system = "你是 Fluid 的代码理解助手。下面给出【当前文件上下文】与一份【可按需索取源码的函数清单】\
（这些函数你目前只有名字——或因上下文超长被省略了摘要源码、或定义在其他文件）。判断:要准确回答用户的追问，你还需要其中哪些函数的源码？\
只输出一个 JSON 对象 {\"need\":[\"函数名\", ...]}，不需要任何源码就返回 {\"need\":[]}；\
禁止任何额外文字或 markdown 代码围栏。";

    // Reuse the answer prompt's context body (summaries already degraded), then append
    // the name-only list and the question — the same situational picture the answer
    // call will see, so the plan is grounded in the real (trimmed) context.
    let (_, mut user) = build_query_prompt(question, capsules, focus, ctx, &[]);
    user.push_str(&format!(
        "\n【仅有名字的函数(可按需索取源码)】{}\n",
        fetchable.join(", ")
    ));
    (system.to_string(), user)
}

/// Approximate char count of the fixed (non-capsule-summary) parts of the query
/// user message — the spine that is never degraded. Used to size the budget left
/// for capsule summaries. Approximate by design (it's a proxy, not exact tokens);
/// the per-section constants cover the bracket labels and separators.
fn query_spine_chars(question: &str, ctx: &GenContext, focus: Option<&QueryFocus>) -> usize {
    let mut n = question.chars().count() + 16;
    if let Some(fs) = &ctx.file_summary {
        n += fs.chars().count() + 8;
    }
    if !ctx.roster.is_empty() {
        n += ctx.roster.iter().map(|r| r.chars().count() + 2).sum::<usize>() + 12;
    }
    for e in &ctx.edges {
        n += e.source.chars().count() + e.edge_type.chars().count() + e.target.chars().count() + 4;
    }
    for (k, v) in &ctx.callee_summaries {
        n += k.chars().count() + v.chars().count() + 2;
    }
    if let Some(f) = focus {
        // number_lines prefixes each line with "%4 | "; ~7 chars/line of overhead.
        let lines = f.source.lines().count();
        n += f.source.chars().count() + lines * 7 + 24;
    }
    n
}

/// Choose which capsule summaries fit within `budget` chars, prioritizing the
/// focused function and its neighbors (outward by index distance — capsules are in
/// source order), then the rest. Returns the kept indices in ascending (source)
/// order for stable rendering; unkept functions degrade to name-only. A function
/// whose `name` matches `focus_name` is the priority center; absent a focus (or if
/// the focused function has no capsule yet) priority is plain source order.
fn select_capsule_summaries(
    capsules: &[(String, String)],
    focus_name: Option<&str>,
    budget: usize,
) -> Vec<usize> {
    let center = focus_name.and_then(|name| capsules.iter().position(|(n, _)| n == name));
    let mut order: Vec<usize> = (0..capsules.len()).collect();
    if let Some(c) = center {
        order.sort_by_key(|&i| (i.abs_diff(c), i));
    }

    let mut kept: Vec<usize> = Vec::new();
    let mut used = 0usize;
    for i in order {
        // Mirror the rendered "name: summary" plus the "; " separator overhead.
        let cost = capsules[i].0.chars().count() + capsules[i].1.chars().count() + 4;
        if used + cost <= budget {
            kept.push(i);
            used += cost;
        }
    }
    kept.sort_unstable();
    kept
}

/// The names of functions whose capsule summary was dropped to name-only by the
/// degradation ladder for this query (S10a-降级) — i.e. the functions the model is
/// "blind" to and may need source for (S10a-追源 fetchable set). Empty when nothing
/// degraded (the single-call path). Uses the same budget logic as `build_query_prompt`
/// so the two agree on what was trimmed.
pub fn query_degraded_names(
    question: &str,
    capsules: &[(String, String)],
    focus: Option<&QueryFocus>,
    ctx: &GenContext,
) -> Vec<String> {
    let spine = query_spine_chars(question, ctx, focus);
    let focus_name = focus.map(|f| f.name);
    let kept: HashSet<usize> =
        select_capsule_summaries(capsules, focus_name, QUERY_CONTEXT_BUDGET_CHARS.saturating_sub(spine))
            .into_iter()
            .collect();
    capsules
        .iter()
        .enumerate()
        .filter(|(i, _)| !kept.contains(i))
        .map(|(_, (name, _))| name.clone())
        .collect()
}

/// Slice the sources of the functions the model asked for (S10a-追源 phase-2). Only
/// names in `fetchable` are honored (hallucination / non-degraded guard); each is
/// located in `roster_spans` by name, sliced from `file_source`, and numbered with
/// absolute line numbers. Deduplicated, and capped at `budget` chars total so the
/// enriched prompt stays bounded. Returns `(name, numbered source)` in request order.
pub fn slice_requested_sources(
    file_source: &str,
    roster_spans: &[FunctionSpan],
    need: &[String],
    fetchable: &[String],
    budget: usize,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut used = 0usize;
    for name in need {
        if !fetchable.iter().any(|f| f == name) {
            continue; // model named a function it can't fetch (kept-summary / nonexistent)
        }
        if !seen.insert(name.as_str()) {
            continue; // dedup
        }
        let Some(span) = roster_spans.iter().find(|s| &s.name == name) else {
            continue; // no span to slice (shouldn't happen if fetchable, but be safe)
        };
        let Some(src) = slice_span(file_source, span.line_range) else {
            continue; // stale line range
        };
        let numbered = number_lines(&src, span.line_range[0]);
        let cost = numbered.chars().count() + name.chars().count() + 4;
        if used + cost > budget {
            continue; // over budget — skip this one, a smaller later one may still fit
        }
        used += cost;
        out.push((name.clone(), numbered));
    }
    out
}

/// A cross-file callee the current file calls whose definition the graph can
/// locate (S10c, ADR-0007 修订). The model points at it by `name` during the
/// planning phase; the backend slices `line_range` out of `file_path`.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossFileTarget {
    /// Callee name (function or class) — what the model names in `{"need":[...]}`.
    pub name: String,
    /// Project-relative path of the file that defines it.
    pub file_path: String,
    /// 1-based inclusive `[start, end]` span of the definition in that file.
    pub line_range: [u32; 2],
}

/// Cross-file callees of `file_path` the graph can locate (S10c, ADR-0007 修订):
/// `calls` edges whose source node lives in `file_path` and whose target is a
/// `function` *or* `class` node in *another* file carrying a `line_range` (a class
/// instantiation is modeled as a `calls` edge to a `class` node). Excludes any name
/// already in the local `roster` (local precedence — keeps the model's plan
/// unambiguous: a named function resolves to exactly one pool, same-file or
/// cross-file). Deduplicated by name (first wins) so each fetchable name maps to a
/// single target. Empty without a graph, or when nodes are too sparse to locate
/// (no `line_range`) — the natural bound that keeps this from "opening everything".
pub fn cross_file_targets(
    graph: Option<&KnowledgeGraph>,
    file_path: &str,
    roster: &[String],
) -> Vec<CrossFileTarget> {
    let Some(g) = graph else { return Vec::new() };
    let local_ids: HashSet<&str> = g
        .nodes
        .iter()
        .filter(|n| n.file_path == file_path)
        .map(|n| n.id.as_str())
        .collect();

    let mut out: Vec<CrossFileTarget> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for e in &g.edges {
        if e.edge_type != "calls" || !local_ids.contains(e.source.as_str()) {
            continue;
        }
        let Some(t) = g.nodes.iter().find(|n| n.id == e.target) else {
            continue; // dangling edge target
        };
        // Accept both `function` and `class` definitions: `understand-anything`
        // models a Python class instantiation as a `calls` edge to a `class` node,
        // and classes are the majority node type — restricting to `function` here
        // silently dropped most cross-file "show me the implementation" callees.
        if !matches!(t.node_type.as_str(), "function" | "class") || t.file_path == file_path {
            continue; // only cross-file code definitions (function/class)
        }
        let Some(line_range) = t.line_range else {
            continue; // sparse node with no span — can't slice, leave name-only
        };
        if roster.iter().any(|r| r == &t.name) {
            continue; // name collides with a local function — local wins
        }
        if !seen.insert(t.name.as_str()) {
            continue; // dedup by name so the model's plan resolves to one target
        }
        out.push(CrossFileTarget {
            name: t.name.clone(),
            file_path: t.file_path.clone(),
            line_range,
        });
    }
    out
}

/// Slice the cross-file sources the model asked for (S10c phase-2). `sources` maps a
/// target's `file_path` → that file's full source (read by the caller, under the
/// lock — this stays IO-free). Only names present in `targets` are honored
/// (hallucination / non-cross-file guard); each is sliced, numbered with absolute
/// lines, and labeled `name @ path` so the model sees it came from another file.
/// Deduplicated, and capped at `budget` chars total (shared with same-file fetch so
/// the phase-2 prompt stays bounded). Returns `(label, numbered source)` in request
/// order.
pub fn slice_cross_file_sources(
    targets: &[CrossFileTarget],
    sources: &BTreeMap<String, String>,
    need: &[String],
    budget: usize,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut used = 0usize;
    for name in need {
        let Some(t) = targets.iter().find(|t| &t.name == name) else {
            continue; // not a fetchable cross-file callee
        };
        if !seen.insert(name.as_str()) {
            continue; // dedup
        }
        let Some(src) = sources.get(&t.file_path) else {
            continue; // caller didn't read this file (shouldn't happen)
        };
        let Some(sliced) = slice_span(src, t.line_range) else {
            continue; // stale / out-of-bounds line range
        };
        let numbered = number_lines(&sliced, t.line_range[0]);
        let label = format!("{} @ {}", t.name, t.file_path);
        let cost = numbered.chars().count() + label.chars().count() + 4;
        if used + cost > budget {
            continue; // over the shared budget — skip; a smaller later one may fit
        }
        used += cost;
        out.push((label, numbered));
    }
    out
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
    fn explain_decl_prompt_targets_the_declaration_not_a_line() {
        let ctx = assemble_gen_context(None, "a.ts", &[], &SharedContext::default());
        let (system, user) = build_explain_decl_prompt(
            "API_URL",
            "const",
            "export const API_URL = 'https://x'",
            4,
            &ctx,
        );
        // Decl-flavored system prompt, same JSON shape as lines.
        assert!(system.contains("模块顶层"));
        assert!(system.contains("const/let/type/interface/enum"));
        assert!(system.contains("{\"text\":"));
        // User message carries kind + name + numbered source at the decl's line.
        assert!(user.contains("【声明种类】const"));
        assert!(user.contains("【声明名称】API_URL"));
        assert!(user.contains("   4 | export const API_URL"));
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
            Some(QueryFocus { source: "def load():\n    return 1", start_line: 10, name: "load" }),
            &ctx,
            &[],
        );
        assert!(system.contains("当前文件上下文"));
        assert!(system.contains("LaTeX")); // 答案可含数学公式 (ADR-0008)
        assert!(user.contains("【文件摘要】配置加载模块"));
        assert!(user.contains("【本文件函数清单】load, save"));
        assert!(user.contains("【各函数摘要】load: 读配置; save: 写配置"));
        assert!(user.contains("【聚焦函数源码(带绝对行号)】"));
        assert!(user.contains("  10 | def load():"));
        assert!(user.contains("【用户问题】load 为什么要先校验？"));
        // Small context → no degradation note.
        assert!(!user.contains("上下文超长"));
    }

    #[test]
    fn query_prompt_omits_focus_and_capsules_when_absent() {
        let ctx = assemble_gen_context(None, "a.py", &[], &SharedContext::default());
        let (_, user) = build_query_prompt("这个文件是做什么的？", &[], None, &ctx, &[]);
        assert!(!user.contains("【各函数摘要】"));
        assert!(!user.contains("【聚焦函数源码"));
        assert!(!user.contains("上下文超长"));
        assert!(user.contains("【用户问题】这个文件是做什么的？"));
    }

    // — S10a-降级 over-window degradation ladder (ADR-0006) —

    /// `n` capsules named `fn{i}` each with a `len`-char summary marked `S{i}…`, in
    /// source order — bulky enough to blow the budget when `n*len` is large.
    fn bulky_capsules(n: usize, len: usize) -> Vec<(String, String)> {
        (0..n)
            .map(|i| {
                let summary = format!("S{i}{}", "摘".repeat(len));
                (format!("fn{i}"), summary)
            })
            .collect()
    }

    #[test]
    fn query_prompt_keeps_all_summaries_under_budget() {
        let names: Vec<String> = (0..5).map(|i| format!("fn{i}")).collect();
        let ctx = assemble_gen_context(None, "a.py", &names, &SharedContext::default());
        let capsules = bulky_capsules(5, 20); // tiny — well under budget
        let (_, user) = build_query_prompt("这个文件做什么？", &capsules, None, &ctx, &[]);
        for i in 0..5 {
            assert!(user.contains(&format!("fn{i}: S{i}")), "fn{i} summary should be present");
        }
        assert!(!user.contains("上下文超长"));
    }

    #[test]
    fn query_prompt_degrades_distant_summaries_when_over_budget() {
        let n = 60;
        let names: Vec<String> = (0..n).map(|i| format!("fn{i}")).collect();
        let ctx = assemble_gen_context(None, "a.py", &names, &SharedContext::default());
        // 60 × ~600-char summaries ≈ 36k chars > 24k budget → must degrade.
        let capsules = bulky_capsules(n, 600);
        let full: usize = capsules.iter().map(|(k, v)| k.chars().count() + v.chars().count()).sum();
        assert!(full > QUERY_CONTEXT_BUDGET_CHARS, "test precondition: summaries exceed budget");

        // Focus the middle function so its neighbors are prioritized.
        let (_, user) = build_query_prompt(
            "fn30 在做什么？",
            &capsules,
            Some(QueryFocus { source: "def fn30():\n    return 1", start_line: 1, name: "fn30" }),
            &ctx,
            &[],
        );

        // Degradation happened, and is announced.
        assert!(user.contains("上下文超长"), "degradation note expected");
        // Focused function's summary survives; the farthest function's summary does not.
        assert!(user.contains("fn30: S30"), "focused function summary must be kept");
        assert!(!user.contains("fn0: S0"), "distant function summary must degrade to name-only");
        // …but every function is still named in the roster line.
        assert!(user.contains("【本文件函数清单】"));
        assert!(user.contains("fn0, fn1"));
        // Assembled context stays bounded by the budget (+ small rendering slack).
        assert!(
            user.chars().count() <= QUERY_CONTEXT_BUDGET_CHARS + 2000,
            "assembled user message {} exceeds budget bound",
            user.chars().count()
        );
    }

    #[test]
    fn select_capsule_summaries_prioritizes_focus_neighbors() {
        let capsules = bulky_capsules(10, 100); // each ~100 chars
        // Budget for ~3 summaries (~104 each) centered on fn5.
        let kept = select_capsule_summaries(&capsules, Some("fn5"), 320);
        assert!(kept.contains(&5), "focus center kept");
        assert!(kept.contains(&4) || kept.contains(&6), "a neighbor kept");
        assert!(!kept.contains(&0), "distant function dropped");
        // Returned in ascending source order for stable rendering.
        let mut sorted = kept.clone();
        sorted.sort_unstable();
        assert_eq!(kept, sorted);
    }

    // — S10a-追源 on-demand source fetch (ADR-0017) —

    fn span(name: &str, lr: [u32; 2]) -> FunctionSpan {
        FunctionSpan { id: format!("{name}#1"), name: name.to_string(), line_range: lr }
    }

    #[test]
    fn query_degraded_names_lists_only_dropped_functions() {
        let names: Vec<String> = (0..60).map(|i| format!("fn{i}")).collect();
        let ctx = assemble_gen_context(None, "a.py", &names, &SharedContext::default());
        let capsules = bulky_capsules(60, 600); // > budget → degrades
        let focus = QueryFocus { source: "def fn30():\n    return 1", start_line: 1, name: "fn30" };
        let degraded = query_degraded_names("fn30 在做什么？", &capsules, Some(&focus), &ctx);
        assert!(!degraded.is_empty(), "large file should degrade some functions");
        assert!(degraded.contains(&"fn0".to_string()), "distant fn0 degraded to name-only");
        assert!(!degraded.contains(&"fn30".to_string()), "focused fn30 not degraded");
    }

    #[test]
    fn query_degraded_names_empty_under_budget() {
        let names: Vec<String> = (0..5).map(|i| format!("fn{i}")).collect();
        let ctx = assemble_gen_context(None, "a.py", &names, &SharedContext::default());
        let capsules = bulky_capsules(5, 20); // tiny — nothing degrades
        assert!(query_degraded_names("?", &capsules, None, &ctx).is_empty());
    }

    #[test]
    fn slice_requested_sources_slices_numbered_fetchable_within_budget() {
        let file = "def a():\n    return 1\ndef b():\n    return 2\ndef c():\n    return 3\n";
        let roster = vec![span("a", [1, 2]), span("b", [3, 4]), span("c", [5, 6])];
        let fetchable = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let got = slice_requested_sources(file, &roster, &["b".into(), "c".into()], &fetchable, 10_000);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "b");
        assert!(got[0].1.contains("   3 | def b():"));
        assert!(got[1].1.contains("   5 | def c():"));
    }

    #[test]
    fn slice_requested_sources_skips_non_fetchable_and_dedups() {
        let file = "def a():\n    return 1\ndef b():\n    return 2\n";
        let roster = vec![span("a", [1, 2]), span("b", [3, 4])];
        let fetchable = vec!["a".to_string()]; // only a is name-only/degraded
        // "b" not fetchable (kept-summary), "ghost" nonexistent, "a" requested twice.
        let need = vec!["b".into(), "ghost".into(), "a".into(), "a".into()];
        let got = slice_requested_sources(file, &roster, &need, &fetchable, 10_000);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "a");
    }

    #[test]
    fn slice_requested_sources_caps_at_budget() {
        let file = "def a():\n    return 1\ndef b():\n    return 2\n";
        let roster = vec![span("a", [1, 2]), span("b", [3, 4])];
        let fetchable = vec!["a".to_string(), "b".to_string()];
        // Budget too small for even one function's numbered source → nothing fits.
        let got = slice_requested_sources(file, &roster, &["a".into(), "b".into()], &fetchable, 3);
        assert!(got.is_empty());
    }

    #[test]
    fn planning_prompt_carries_context_and_fetchable_and_asks_for_need_json() {
        let names: Vec<String> = vec!["load".into(), "save".into(), "verify".into()];
        let ctx = assemble_gen_context(None, "a.py", &names, &SharedContext::default());
        let capsules = vec![("load".to_string(), "读配置".to_string())];
        let (system, user) = build_query_planning_prompt(
            "保存时如何校验？",
            &capsules,
            Some(QueryFocus { source: "def load():\n    return 1", start_line: 1, name: "load" }),
            &ctx,
            &["save".to_string(), "verify".to_string()],
        );
        assert!(system.contains("{\"need\":"));
        assert!(user.contains("【仅有名字的函数(可按需索取源码)】save, verify"));
        assert!(user.contains("【用户问题】保存时如何校验？"));
    }

    #[test]
    fn query_prompt_renders_extra_fetched_sources() {
        let ctx = assemble_gen_context(None, "a.py", &["a".into()], &SharedContext::default());
        let extra = vec![("save".to_string(), "   3 | def save():\n   4 |     pass".to_string())];
        let (_, user) = build_query_prompt("?", &[], None, &ctx, &extra);
        assert!(user.contains("【按需追加的函数源码: save(带绝对行号)】"));
        assert!(user.contains("   3 | def save():"));
    }

    // --- S10c: cross-file ephemeral fetch (ADR-0007 修订) ---

    fn fn_node(id: &str, name: &str, file: &str, lr: [u32; 2]) -> GraphNode {
        span_node(id, "function", name, file, lr)
    }

    fn class_node(id: &str, name: &str, file: &str, lr: [u32; 2]) -> GraphNode {
        span_node(id, "class", name, file, lr)
    }

    fn span_node(id: &str, ty: &str, name: &str, file: &str, lr: [u32; 2]) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            node_type: ty.to_string(),
            name: name.to_string(),
            file_path: file.to_string(),
            summary: String::new(),
            tags: vec![],
            complexity: None,
            line_range: Some(lr),
            language_notes: None,
        }
    }

    #[test]
    fn cross_file_targets_locates_cross_file_function_callees_with_spans() {
        let g = KnowledgeGraph {
            nodes: vec![
                node("file:a.py", "file", "a.py", ""),
                node("function:a.py:caller", "function", "a.py", ""),
                node("function:a.py:local2", "function", "a.py", ""),
                fn_node("function:b.py:encrypt", "encrypt", "b.py", [10, 20]),
                fn_node("function:b.py:sign", "sign", "b.py", [30, 40]),
                node("function:c.py:nolines", "function", "c.py", ""), // no line_range
            ],
            edges: vec![
                edge("function:a.py:caller", "function:b.py:encrypt", "calls"), // cross-file ✓
                edge("function:a.py:caller", "function:b.py:sign", "calls"),    // cross-file ✓
                edge("function:a.py:caller", "function:c.py:nolines", "calls"), // no span → drop
                edge("function:a.py:caller", "function:a.py:local2", "calls"),  // same-file → drop
                edge("function:b.py:x", "function:b.py:encrypt", "calls"),      // foreign src → drop
                edge("function:a.py:caller", "function:b.py:encrypt", "contains"), // wrong type
            ],
        };
        let targets = cross_file_targets(Some(&g), "a.py", &[]);
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["encrypt", "sign"]);
        let enc = targets.iter().find(|t| t.name == "encrypt").unwrap();
        assert_eq!(enc.file_path, "b.py");
        assert_eq!(enc.line_range, [10, 20]);
    }

    #[test]
    fn cross_file_targets_locates_cross_file_class_callees() {
        // Mirrors the real alphaGPT graph: a `class` node in this file `calls` a
        // `class` node defined in another file (a Python class instantiation —
        // `understand-anything` models the callee as a `class`, not `function`).
        // The callee class carries a span, so its source must be fetchable just
        // like a function; classes are the majority node type, so excluding them
        // silently broke S10c for most cross-file "show me the implementation"
        // questions.
        let g = KnowledgeGraph {
            nodes: vec![
                class_node("class:engine.py:AlphaEngine", "AlphaEngine", "engine.py", [1, 50]),
                class_node(
                    "class:alphagpt.py:NewtonSchulzLowRankDecay",
                    "NewtonSchulzLowRankDecay",
                    "alphagpt.py",
                    [8, 67],
                ),
            ],
            edges: vec![edge(
                "class:engine.py:AlphaEngine",
                "class:alphagpt.py:NewtonSchulzLowRankDecay",
                "calls",
            )],
        };
        let targets = cross_file_targets(Some(&g), "engine.py", &[]);
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["NewtonSchulzLowRankDecay"]);
        assert_eq!(targets[0].file_path, "alphagpt.py");
        assert_eq!(targets[0].line_range, [8, 67]);
    }

    #[test]
    fn cross_file_targets_excludes_roster_collisions_and_dedups_by_name() {
        let g = KnowledgeGraph {
            nodes: vec![
                node("function:a.py:caller", "function", "a.py", ""),
                fn_node("function:b.py:encrypt", "encrypt", "b.py", [1, 5]),
                fn_node("function:c.py:encrypt", "encrypt", "c.py", [2, 6]), // same name, other file
                fn_node("function:b.py:helper", "helper", "b.py", [9, 12]),
            ],
            edges: vec![
                edge("function:a.py:caller", "function:b.py:encrypt", "calls"),
                edge("function:a.py:caller", "function:c.py:encrypt", "calls"),
                edge("function:a.py:caller", "function:b.py:helper", "calls"),
            ],
        };
        // Local function also named "helper" (roster) → cross-file helper excluded;
        // "encrypt" deduped to the first target (b.py).
        let targets = cross_file_targets(Some(&g), "a.py", &["helper".to_string()]);
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["encrypt"]);
        assert_eq!(targets[0].file_path, "b.py");
    }

    #[test]
    fn cross_file_targets_empty_without_graph() {
        assert!(cross_file_targets(None, "a.py", &[]).is_empty());
    }

    #[test]
    fn slice_cross_file_sources_labels_with_path_and_numbers_absolute() {
        let targets = vec![CrossFileTarget {
            name: "encrypt".into(),
            file_path: "b.py".into(),
            line_range: [2, 3],
        }];
        let mut sources: BTreeMap<String, String> = BTreeMap::new();
        sources.insert("b.py".into(), "x=0\ndef encrypt():\n    return 1\n".into());
        let got = slice_cross_file_sources(&targets, &sources, &["encrypt".into()], 10_000);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "encrypt @ b.py"); // label tells the model it's cross-file
        assert!(got[0].1.contains("   2 | def encrypt():"));
        assert!(got[0].1.contains("   3 |     return 1"));
    }

    #[test]
    fn slice_cross_file_sources_guards_hallucination_dedup_and_budget() {
        let targets = vec![
            CrossFileTarget { name: "encrypt".into(), file_path: "b.py".into(), line_range: [1, 2] },
            CrossFileTarget { name: "missing".into(), file_path: "z.py".into(), line_range: [1, 2] },
        ];
        let mut sources: BTreeMap<String, String> = BTreeMap::new();
        sources.insert("b.py".into(), "def encrypt():\n    return 1\n".into());
        // "ghost" not a target (hallucination) → skip; "missing" has no read source → skip;
        // "encrypt" requested twice → dedup.
        let need = vec!["ghost".into(), "missing".into(), "encrypt".into(), "encrypt".into()];
        let got = slice_cross_file_sources(&targets, &sources, &need, 10_000);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "encrypt @ b.py");

        // Budget too small for even one numbered function → nothing fits.
        let tight = slice_cross_file_sources(&targets, &sources, &["encrypt".into()], 3);
        assert!(tight.is_empty());
    }
}
