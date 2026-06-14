; Fluid key-line query — Python (S4, ADR-0005/0009).
;
; A "key line" is a source line carrying semantic load that a reader needs
; explained. This query captures candidate statements; parse.ts then keeps only
; the captures that fall inside a function body (重点行 attach to one capsule),
; so module-level and class-body captures here are filtered out downstream.
;
; KEPT: assignments, value-bearing returns, raise/assert, branch/loop/with heads,
;       standalone calls.
; SKIPPED (not matched here): import / decorator / def & class headers / pass /
;       try-except-else-finally scaffolding heads (their inner stmts still match).

(assignment) @key
(augmented_assignment) @key
(return_statement (_)) @key
(raise_statement) @key
(assert_statement) @key
(if_statement) @key
(elif_clause) @key
(for_statement) @key
(while_statement) @key
(with_statement) @key
(expression_statement (call)) @key
