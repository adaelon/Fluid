// S11-md: render a follow-up answer (free-form Markdown, may contain LaTeX) to
// HTML (ADR-0008). This module is the *pure* half — Markdown → HTML string only,
// with raw-HTML passthrough disabled (`html: false`) so any `<script>`/event-
// handler in the model output is escaped at the source. No DOMPurify / KaTeX /
// DOM imports here, so it stays unit-testable under plain Node. The browser side
// (QueryPanel) layers DOMPurify (defense-in-depth) + KaTeX auto-render on top.

import MarkdownIt from 'markdown-it'

// `html: false` escapes raw HTML (primary XSS defense, node-testable). `linkify`
// turns bare URLs into links; markdown-it's default validateLink already blocks
// javascript:/data:/vbscript: hrefs. `$...$` math is left as literal text for
// KaTeX auto-render to transform in the DOM afterwards.
const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: true,
})

/** Render Markdown source to an HTML string (raw HTML escaped). Pure: no DOM. */
export function renderMarkdown(src: string): string {
  return md.render(src)
}
