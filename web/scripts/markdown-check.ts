// S11-md validation (B2, deterministic). Exercises the pure Markdown→HTML core —
// renderMarkdown — with assertions, no DOM / DOMPurify / KaTeX / browser.
// Run with: node scripts/markdown-check.ts  (Node 24 strips TS types).
//
// DOMPurify sanitize + KaTeX auto-render run in the DOM and are browser-verified
// (A2); this script locks markdown-it's structure + the html:false escaping that
// is the primary XSS defense (raw HTML in model output never reaches innerHTML).

import { renderMarkdown } from '../src/render/markdown.ts'

let failures = 0
function check(label: string, cond: boolean): void {
  if (cond) {
    console.log(`  PASS  ${label}`)
  } else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

console.log('=== Markdown structure ===')
check('heading → <h1>', renderMarkdown('# Title').includes('<h1>Title</h1>'))
check('bold → <strong>', renderMarkdown('**hi**').includes('<strong>hi</strong>'))
check('inline code → <code>', renderMarkdown('`x`').includes('<code>x</code>'))
const list = renderMarkdown('- a\n- b')
check('bullet list → <ul><li>', list.includes('<ul>') && list.includes('<li>a</li>'))
const fence = renderMarkdown('```\nfoo\n```')
check('fenced code → <pre><code>', fence.includes('<pre><code>') && fence.includes('foo'))

console.log('\n=== Raw HTML is escaped (primary XSS defense, html:false) ===')
const xss = renderMarkdown('<script>alert(1)</script>')
check('no live <script> tag', !xss.includes('<script>'))
check('angle brackets escaped', xss.includes('&lt;script&gt;'))
const img = renderMarkdown('<img src=x onerror=alert(1)>')
check('no raw <img onerror>', !img.includes('<img'))

console.log('\n=== LaTeX delimiters left intact for KaTeX (not mangled by md) ===')
const inline = renderMarkdown('complexity is $O(n^2)$ here')
check('inline $…$ preserved verbatim', inline.includes('$O(n^2)$'))
const block = renderMarkdown('$$\\sum_{i=1}^n i$$')
check('block $$…$$ preserved verbatim', block.includes('$$\\sum_{i=1}^n i$$'))

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll markdown checks passed.')
