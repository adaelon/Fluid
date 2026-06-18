; Fluid key-line query — TypeScript (S-TS-2, ADR-0005/0009).
;
; A "key line" is a source line carrying semantic load. This query captures
; candidate nodes; parse.ts keeps only those inside a function body (重点行
; attach to one capsule via innermostHost), so module/class-level captures here
; are filtered out downstream.
;
; KEPT: const/let initializers, re-assignments (incl. compound), value-bearing
;       returns, throw, branch/loop/switch heads, statement-position calls and
;       awaited calls.
; SKIPPED (not matched here): import / export bare / type & interface decls /
;       function & class & method headers / decorators / empty `return;`.
; KNOWN v1 LIMIT: an arrow/function assigned to a const (`const f = () => {}`) is
;       both a roster function AND a lexical_declaration, so its definition line
;       gets a (redundant) key-line note. Acceptable; avoids dropping the far more
;       common `const x = compute()` initializer.

; assignments / declarations with an initializer
(lexical_declaration) @key
(expression_statement (assignment_expression)) @key
(expression_statement (augmented_assignment_expression)) @key

; value-bearing return + throw
(return_statement (_)) @key
(throw_statement) @key

; branch / loop / switch heads
(if_statement) @key
(for_statement) @key
(for_in_statement) @key
(while_statement) @key
(switch_statement) @key

; calls in statement position (e.g. `doThing();`, `await save();`)
(expression_statement (call_expression) @key)
(expression_statement (await_expression) @key)
