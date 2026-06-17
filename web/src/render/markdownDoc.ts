// Document Render View: render a Markdown *file's* source to safe HTML and typeset
// its math, reusing the exact chain QueryPanel uses for answers — markdown-it
// (`html: false`, raw HTML escaped) → DOMPurify (defense-in-depth) → KaTeX. The
// heavy libs are pulled on demand (S11-lazy dynamic import()) so they stay out of
// the first-paint bundle; only a opened .md file triggers the fetch.
//
// Reuse note: this is a *new* module (no change to QueryPanel, which still inlines
// the same steps). Collapsing both onto this helper is a follow-up refactor, kept
// out of this slice to avoid a regression risk in the query path.

/** Render Markdown source to sanitized HTML (raw HTML escaped at the source). */
export async function renderDoc(src: string): Promise<string> {
  const [{ renderMarkdown }, { default: DOMPurify }] = await Promise.all([
    import('./markdown'),
    import('dompurify'),
  ])
  return DOMPurify.sanitize(renderMarkdown(src))
}

/** Typeset `$…$` / `$$…$$` math in an already-rendered element (KaTeX, in place). */
export async function typesetMath(el: HTMLElement): Promise<void> {
  const [{ default: renderMathInElement }] = await Promise.all([
    import('katex/contrib/auto-render'),
    import('katex/dist/katex.min.css'),
  ])
  renderMathInElement(el, {
    delimiters: [
      { left: '$$', right: '$$', display: true },
      { left: '$', right: '$', display: false },
      { left: '\\[', right: '\\]', display: true },
      { left: '\\(', right: '\\)', display: false },
    ],
    throwOnError: false,
  })
}
