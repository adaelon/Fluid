//! Document translation (文档翻译): turn an English Markdown document into Simplified
//! Chinese as a bypass product, without translating code. The deterministic half
//! lives here (B2: deterministic tooling protects structure, the LLM only does
//! prose); the LLM call + caching live in routes.rs / cache_store.rs.
//!
//! Strategy: replace every fenced code block (``` / ~~~) with a sentinel line
//! `[[FLUID_CODE_BLOCK_n]]`, send the masked Markdown to the model (one call) with a
//! strict instruction to translate prose, keep Markdown structure, and never touch
//! the sentinels, then restore the original code blocks verbatim. Code bytes thus
//! never reach the model — they cannot be altered. Inline `` `code` `` is left to the
//! model instruction (a smaller risk than whole code blocks).

/// Sentinel standing in for the n-th protected code block. ASCII, placeholder-shaped
/// so the model leaves it alone; restored 1:1 afterwards.
fn sentinel(idx: usize) -> String {
    format!("[[FLUID_CODE_BLOCK_{idx}]]")
}

/// Replace each fenced code block (including its fence lines) with a sentinel line,
/// returning the masked document and the extracted blocks in order. A fence is any
/// line whose trimmed start is ``` or ~~~ ; it toggles code mode. An unterminated
/// fence protects the remainder of the document (never translate code).
pub fn protect_code(src: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut blocks: Vec<String> = Vec::new();
    let mut in_code = false;
    let mut current = String::new();

    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let is_fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if is_fence {
            current.push_str(line);
            if in_code {
                // Closing fence: emit the collected block as one sentinel line.
                let idx = blocks.len();
                blocks.push(std::mem::take(&mut current));
                out.push_str(&sentinel(idx));
                out.push('\n');
                in_code = false;
            } else {
                in_code = true; // opening fence
            }
        } else if in_code {
            current.push_str(line);
        } else {
            out.push_str(line);
        }
    }
    // Unterminated fence → keep what we collected as a final protected block so code
    // is never sent to the model.
    if in_code && !current.is_empty() {
        let idx = blocks.len();
        blocks.push(current);
        out.push_str(&sentinel(idx));
        out.push('\n');
    }
    (out, blocks)
}

/// Put the original code blocks back where their sentinels are. A sentinel the model
/// dropped/altered simply isn't matched (its block is then absent and the literal
/// `[[FLUID_CODE_BLOCK_n]]` would remain visible — detectable in eye-verify).
pub fn restore_code(translated: &str, blocks: &[String]) -> String {
    let mut out = translated.to_string();
    for (i, block) in blocks.iter().enumerate() {
        // The sentinel sits on its own line with a trailing '\n'; the block already
        // carries its own newlines, so trim one trailing '\n' to avoid a blank line.
        out = out.replace(&sentinel(i), block.trim_end_matches('\n'));
    }
    out
}

/// Split masked Markdown into chunks no larger than `budget` chars, packing whole
/// blocks (separated by blank lines) greedily. A long document must be translated in
/// pieces — one giant request overruns the model's output limit / times out and the
/// gateway returns 500 (the bug this fixes). Block boundaries fall on blank lines, so
/// a paragraph / list / table / sentinel is never cut mid-way. A single block larger
/// than `budget` becomes its own (over-budget) chunk rather than being sliced — we
/// never cut structure. **Lossless**: `split_chunks(m, _).concat() == m`.
pub fn split_chunks(masked: &str, budget: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for block in split_blocks(masked) {
        // Starting a new chunk would only help if the current one isn't empty;
        // an over-budget single block still goes in alone.
        if !cur.is_empty() && cur.len() + block.len() > budget {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.push_str(&block);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// Split text into blocks on blank-line boundaries, keeping every byte (each block
/// carries its own trailing blank line). Concatenating the blocks reproduces `text`.
fn split_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        cur.push_str(line);
        if line.trim().is_empty() {
            blocks.push(std::mem::take(&mut cur)); // blank line ends a block
        }
    }
    if !cur.is_empty() {
        blocks.push(cur); // trailing block with no terminating blank line
    }
    blocks
}

/// Build the (system, user) prompt for one whole-document translation. The user
/// message is the *masked* Markdown (code blocks already replaced by sentinels).
pub fn build_translate_prompt(masked: &str) -> (String, String) {
    let system = "你是专业技术文档翻译。把用户给出的 Markdown 文本从英文翻译成简体中文。严格遵守:\
1) 完整保留所有 Markdown 标记(标题 #、列表、表格、链接、强调、引用等)的结构与语法不变;\
2) 形如 [[FLUID_CODE_BLOCK_0]] 的占位符必须原样逐字符保留,绝不改动、翻译或删除——它代表一段被保护的代码;\
3) 行内代码(反引号包裹)、URL、文件路径、命令、代码标识符、专有名词保持原文不译;\
4) 只翻译自然语言叙述文字,不增删、不重排、不解释,直接输出翻译后的完整 Markdown。"
        .to_string();
    (system, masked.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_a_fenced_block_and_restores_it_verbatim() {
        let src = "# Title\n\nSome prose.\n\n```bash\nfluid /path\nrm -rf x\n```\n\nMore prose.\n";
        let (masked, blocks) = protect_code(src);
        // The code is gone from what the model sees, replaced by one sentinel.
        assert!(!masked.contains("rm -rf x"));
        assert!(masked.contains("[[FLUID_CODE_BLOCK_0]]"));
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("rm -rf x"));
        // Prose stays.
        assert!(masked.contains("Some prose."));

        // Simulate the model translating only prose + keeping the sentinel intact.
        let translated = masked
            .replace("# Title", "# 标题")
            .replace("Some prose.", "一些散文。")
            .replace("More prose.", "更多散文。");
        let restored = restore_code(&translated, &blocks);
        assert!(restored.contains("```bash\nfluid /path\nrm -rf x\n```"));
        assert!(restored.contains("# 标题"));
        assert!(!restored.contains("FLUID_CODE_BLOCK"));
    }

    #[test]
    fn handles_multiple_blocks_keyed_in_order() {
        let src = "a\n```\none\n```\nb\n~~~\ntwo\n~~~\nc\n";
        let (masked, blocks) = protect_code(src);
        assert_eq!(blocks.len(), 2);
        assert!(masked.contains("[[FLUID_CODE_BLOCK_0]]"));
        assert!(masked.contains("[[FLUID_CODE_BLOCK_1]]"));
        assert!(blocks[0].contains("one"));
        assert!(blocks[1].contains("two"));
        // Round-trip with no prose change restores both blocks exactly.
        let restored = restore_code(&masked, &blocks);
        assert!(restored.contains("```\none\n```"));
        assert!(restored.contains("~~~\ntwo\n~~~"));
    }

    #[test]
    fn unterminated_fence_protects_the_rest() {
        // No closing fence: everything from the fence on is treated as code, never
        // sent to the model.
        let src = "intro\n```\ncode line\nstill code\n";
        let (masked, blocks) = protect_code(src);
        assert_eq!(blocks.len(), 1);
        assert!(!masked.contains("code line"));
        assert!(masked.contains("intro"));
        assert!(blocks[0].contains("still code"));
    }

    #[test]
    fn no_code_is_a_passthrough() {
        let src = "# Just prose\n\nNo code here at all.\n";
        let (masked, blocks) = protect_code(src);
        assert!(blocks.is_empty());
        assert_eq!(masked, src);
    }

    #[test]
    fn split_chunks_is_lossless_and_respects_budget() {
        // Several blank-line-separated paragraphs; a small budget forces multiple
        // chunks. Concatenation must reproduce the input exactly (no byte lost/added).
        let masked = "# Title\n\npara one is here.\n\npara two is here.\n\npara three.\n";
        let chunks = split_chunks(masked, 20);
        assert!(chunks.len() > 1, "small budget should split into several chunks");
        assert_eq!(chunks.concat(), masked, "split must be lossless");
        // Each chunk ends on a block boundary, so none starts mid-paragraph.
        for c in &chunks {
            assert!(!c.is_empty());
        }
    }

    #[test]
    fn split_chunks_keeps_an_oversized_block_whole() {
        // A single paragraph larger than the budget is not sliced — structure is
        // never cut; it becomes its own over-budget chunk.
        let big = "x".repeat(100);
        let masked = format!("{big}\n\nsmall.\n");
        let chunks = split_chunks(&masked, 10);
        assert_eq!(chunks.concat(), masked);
        assert!(chunks[0].contains(&big), "oversized block stays in one chunk");
    }

    #[test]
    fn split_chunks_small_doc_is_one_chunk() {
        let masked = "# Tiny\n\njust a bit.\n";
        let chunks = split_chunks(masked, 4000);
        assert_eq!(chunks, vec![masked.to_string()]);
    }

    #[test]
    fn split_chunks_does_not_cut_a_sentinel_block() {
        // The sentinel line for a protected code block must survive splitting intact
        // so restore_code can match it.
        let masked = "intro.\n\n[[FLUID_CODE_BLOCK_0]]\n\noutro.\n";
        let chunks = split_chunks(masked, 12);
        assert_eq!(chunks.concat(), masked);
        assert!(
            chunks.iter().any(|c| c.contains("[[FLUID_CODE_BLOCK_0]]")),
            "a sentinel must stay whole within one chunk"
        );
    }
}
