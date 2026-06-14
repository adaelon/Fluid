; Fluid key-line query — Rust (S4, ADR-0005/0009).
;
; A "key line" is a source line carrying semantic load. This query captures
; candidate nodes; parse.ts keeps only those inside a function body (重点行
; attach to one capsule), so out-of-function captures here are filtered out.
;
; KEPT: let bindings, assignments, value-bearing returns, branch/loop heads,
;       statement-position calls, and tail-expression calls (a block's final
;       call/macro with no trailing `;`, i.e. Rust's implicit return).
; SKIPPED (not matched here): use / attributes (#[...]) / fn/impl/struct/mod
;       headers / trivial identifier or reference tails.
; KNOWN v1 LIMIT: non-call tail expressions (e.g. `&self.root`) are intentionally
;       not key lines; tail method-chains are caught via (block (call_expression)).

(let_declaration) @key
(assignment_expression) @key
(compound_assignment_expr) @key
(return_expression (_)) @key
(if_expression) @key
(match_expression) @key
(for_expression) @key
(while_expression) @key
(loop_expression) @key

; calls in statement position (e.g. `out.push(...);`)
(expression_statement (call_expression) @key)
(expression_statement (macro_invocation) @key)

; tail-expression calls (block's final expr, no semicolon = implicit return)
(block (call_expression) @key)
(block (macro_invocation) @key)
