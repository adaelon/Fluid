// S10b-cap: the per-file context snapshot the follow-up terminal sends with each
// question. Capsules live in the Editor's non-reactive GhostStore (ADR-0014); the
// Editor emits one of these up to App (@context) on every capsule arrival /
// activate / reset, App holds it in a ref and passes it down to QueryPanel.
// Pure (no Vue / CM6 / store deps) so the snapshot logic is unit-testable.

/** One function's name + its generated capsule summary (mirrors the backend
 *  `CapsuleSummary` in routes.rs — the `/api/query` request's `capsules` field). */
export interface CapsuleSummary {
  name: string
  summary: string
}

/** The whole-file query context: the function roster (names) plus a summary for
 *  each function whose capsule has already been generated. */
export interface QueryContext {
  roster: string[]
  capsules: CapsuleSummary[]
}

export const EMPTY_QUERY_CONTEXT: QueryContext = { roster: [], capsules: [] }

/** Build the snapshot QueryPanel sends with a follow-up: every function name in
 *  roster order, plus a `{name, summary}` entry for each function whose capsule
 *  has already arrived. Partial generation = only the settled ones are included
 *  (the roster is always complete; the file_summary backstop covers the rest). */
export function buildQueryContext(
  roster: { id: string; name: string }[],
  summaryOf: (fnId: string) => string | undefined,
): QueryContext {
  const capsules: CapsuleSummary[] = []
  for (const fn of roster) {
    const summary = summaryOf(fn.id)
    if (summary) capsules.push({ name: fn.name, summary })
  }
  return { roster: roster.map((r) => r.name), capsules }
}
