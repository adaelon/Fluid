// Fuzzy subsequence matcher for the command palette (U4). Pure + deterministic so
// the ranking is locked by scripts/fuzzy-check.ts (B2); the palette UI itself is
// browser-verified (A2). A query matches when its chars appear in order (not
// necessarily contiguous) in the target, case-insensitively. Higher score = better.

/** Chars that begin a "segment" — a match right after one (or at index 0) scores
 *  a bonus so `eng` ranks `engine.py` above an incidental mid-word hit. */
const SEPARATORS = new Set(['/', '\\', '_', '-', '.', ' '])

/** Score `query` against `text`, or `null` when `text` is not a supersequence of
 *  `query`. An empty query matches everything with score 0 (preserves input order
 *  via the caller's index tiebreak). */
export function fuzzyMatch(query: string, text: string): number | null {
  if (query === '') return 0
  const q = query.toLowerCase()
  const t = text.toLowerCase()
  let qi = 0
  let score = 0
  let prev = -2 // index of the previous matched char (for the consecutive bonus)
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] !== q[qi]) continue
    let bonus = 1
    if (ti === prev + 1) bonus += 3 // consecutive run
    if (ti === 0 || SEPARATORS.has(t[ti - 1])) bonus += 2 // start of a segment
    score += bonus
    prev = ti
    qi++
  }
  if (qi < q.length) return null // ran out of text before matching every query char
  return score - text.length * 0.01 // tiny nudge toward shorter targets on ties
}

/** Filter + rank `items` by `query` over `key(item)`. Stable: equal scores keep
 *  input order. Capped at `limit` to bound the rendered list. */
export function fuzzyFilter<T>(
  query: string,
  items: readonly T[],
  key: (item: T) => string,
  limit = 50,
): T[] {
  const hits: { item: T; score: number; idx: number }[] = []
  items.forEach((item, idx) => {
    const score = fuzzyMatch(query, key(item))
    if (score !== null) hits.push({ item, score, idx })
  })
  hits.sort((a, b) => b.score - a.score || a.idx - b.idx)
  return hits.slice(0, limit).map((h) => h.item)
}
