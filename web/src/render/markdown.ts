// S11-md: render a follow-up answer (free-form Markdown, may contain LaTeX) to
// HTML (ADR-0008). This module is the *pure* half — Markdown → HTML string only,
// with raw-HTML passthrough disabled (`html: false`) so any `<script>`/event-
// handler in the model output is escaped at the source. No DOMPurify / KaTeX /
// DOM imports here, so it stays unit-testable under plain Node. The browser side
// (QueryPanel) layers DOMPurify (defense-in-depth) + KaTeX auto-render on top.

import MarkdownIt from 'markdown-it'
import type { CodeEvidenceRef } from '../ghostTypes'

// `html: false` escapes raw HTML (primary XSS defense, node-testable). `linkify`
// turns bare URLs into links; markdown-it's default validateLink already blocks
// javascript:/data:/vbscript: hrefs. `$...$` math is left as literal text for
// KaTeX auto-render to transform in the DOM afterwards.
const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: true,
})

type QueryEvidenceEnv = { queryEvidenceIds?: Set<string> }

// Turn a bare, backend-known [E#] citation into a local fragment link. Explicit
// Markdown links and code spans/fences keep their normal semantics; unknown IDs
// remain plain text so the model cannot manufacture a clickable source anchor.
md.inline.ruler.before('emphasis', 'fluid_query_evidence', (state, silent) => {
  const match = /^\[(E[1-9]\d*)\]/.exec(state.src.slice(state.pos))
  if (!match) return false
  const next = state.src[state.pos + match[0].length]
  if (next === '(' || next === '[') return false

  if (!silent) {
    const known = (state.env as QueryEvidenceEnv).queryEvidenceIds?.has(match[1]) === true
    if (known) {
      const open = state.push('link_open', 'a', 1)
      open.attrSet('href', `#fluid-evidence-${match[1]}`)
      open.attrSet('class', 'query-code-evidence-link')
      const text = state.push('text', '', 0)
      text.content = match[0]
      state.push('link_close', 'a', -1)
    } else {
      const text = state.push('text', '', 0)
      text.content = match[0]
    }
  }
  state.pos += match[0].length
  return true
})

/** Render Markdown source to an HTML string (raw HTML escaped). Pure: no DOM. */
export function renderMarkdown(src: string): string {
  return md.render(src)
}

/** Query-answer variant: only E# values present in the backend QueryMap become
 * clickable. The URL is a same-document fragment intercepted by QueryPanel; no
 * project path or source text is placed in an external URL. */
export function renderQueryMarkdown(src: string, evidence: CodeEvidenceRef[]): string {
  return md.render(src, {
    queryEvidenceIds: new Set(evidence.map((reference) => reference.id)),
  } satisfies QueryEvidenceEnv)
}
