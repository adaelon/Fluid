// S-WANCHOR-MD1 deterministic checks. Run with:
//   node web/scripts/workspace-markdown-anchor-check.ts
// The content identity/fallback math and MarkdownView ownership wiring are
// checked without requiring a browser DOM.

import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import {
  MAX_MARKDOWN_ANCHOR_OCCURRENCE,
  MAX_MARKDOWN_ANCHOR_OFFSET_PX,
  captureMarkdownReadingAnchor,
  correctedMarkdownAnchorScrollTop,
  digestMarkdownBlockText,
  indexMarkdownContentBlocks,
  normalizeMarkdownBlockText,
  normalizeMarkdownReadingAnchor,
  resolveMarkdownReadingAnchor,
} from '../src/readingAnchor.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

console.log('=== visible block normalization and stable identity ===')
check(
  'visible whitespace collapses while empty decoration stays anchorless',
  normalizeMarkdownBlockText('  Heading\u00a0\n  text  ') === 'Heading text'
    && normalizeMarkdownBlockText('\n\t ') === '',
)
check(
  'block identity is a deterministic SHA-256 digest rather than saved source text',
  digestMarkdownBlockText('abc')
    === 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
    && digestMarkdownBlockText('  abc\n') === digestMarkdownBlockText('abc')
    && digestMarkdownBlockText('') === null,
)
const longUnicodeBlock = '重复内容🙂  '.repeat(40)
const normalizedLongUnicodeBlock = normalizeMarkdownBlockText(longUnicodeBlock)
check(
  'multi-block UTF-8 hashing agrees with the platform SHA-256 oracle',
  digestMarkdownBlockText(longUnicodeBlock)
    === createHash('sha256').update(normalizedLongUnicodeBlock, 'utf8').digest('hex'),
)

const indexed = indexMarkdownContentBlocks([
  'Title',
  'repeat me',
  '   ',
  'repeat\n me',
  'const value = 1',
  'Column A Column B',
])
check(
  'heading, paragraph, code and table text receive stable identities',
  indexed[0]?.blockDigest === digestMarkdownBlockText('Title')
    && indexed[1]?.blockDigest === digestMarkdownBlockText('repeat me')
    && indexed[4]?.blockDigest === digestMarkdownBlockText('const value = 1')
    && indexed[5]?.blockDigest === digestMarkdownBlockText('Column A Column B'),
)
check(
  'empty blocks are skipped and equal normalized text uses zero-based occurrence',
  indexed[2] === null
    && indexed[1]?.occurrence === 0
    && indexed[3]?.occurrence === 1,
)

console.log('\n=== capture normalization and persisted-state bounds ===')
const captured = captureMarkdownReadingAnchor({
  blockDigest: indexed[3]!.blockDigest,
  occurrence: indexed[3]!.occurrence,
  offsetPx: -12.5,
  scrollTop: 450,
  scrollHeight: 2400,
  clientHeight: 600,
})
check(
  'capture stores block identity, viewport offset and current scroll ratio',
  JSON.stringify(captured) === JSON.stringify({
    kind: 'markdown',
    blockDigest: indexed[3]!.blockDigest,
    occurrence: 1,
    offsetPx: -12.5,
    scrollRatio: 0.25,
  }),
)
check(
  'capture clamps finite offset and scroll ratio to backend-compatible bounds',
  captureMarkdownReadingAnchor({
    blockDigest: indexed[0]!.blockDigest,
    occurrence: 0,
    offsetPx: MAX_MARKDOWN_ANCHOR_OFFSET_PX * 2,
    scrollTop: 500,
    scrollHeight: 100,
    clientHeight: 100,
  })?.offsetPx === MAX_MARKDOWN_ANCHOR_OFFSET_PX
    && captureMarkdownReadingAnchor({
      blockDigest: indexed[0]!.blockDigest,
      occurrence: 0,
      offsetPx: 0,
      scrollTop: -50,
      scrollHeight: 100,
      clientHeight: 50,
    })?.scrollRatio === 0,
)
check(
  'NaN, malformed digest and out-of-range occurrence degrade to no anchor',
  captureMarkdownReadingAnchor({
    blockDigest: indexed[0]!.blockDigest,
    occurrence: 0,
    offsetPx: Number.NaN,
    scrollTop: 0,
    scrollHeight: 100,
    clientHeight: 50,
  }) === null
    && normalizeMarkdownReadingAnchor({
      kind: 'markdown',
      blockDigest: 'contains whitespace',
      occurrence: 0,
      offsetPx: 0,
      scrollRatio: 0,
    }) === null
    && normalizeMarkdownReadingAnchor({
      kind: 'markdown',
      blockDigest: indexed[0]!.blockDigest,
      occurrence: MAX_MARKDOWN_ANCHOR_OCCURRENCE + 1,
      offsetPx: 0,
      scrollRatio: 0,
    }) === null,
)

console.log('\n=== repeated content resolution and ratio fallback ===')
const exact = resolveMarkdownReadingAnchor(captured, indexed)
check(
  'digest plus occurrence resolves the exact repeated content block',
  JSON.stringify(exact) === JSON.stringify({
    mode: 'block',
    blockIndex: 3,
    offsetPx: -12.5,
  }),
)
const missing = resolveMarkdownReadingAnchor({
  kind: 'markdown',
  blockDigest: digestMarkdownBlockText('removed paragraph')!,
  occurrence: 0,
  offsetPx: 8,
  scrollRatio: 0.75,
}, indexed)
check(
  'a missing content identity resolves only to its bounded scroll ratio',
  JSON.stringify(missing) === JSON.stringify({ mode: 'ratio', scrollRatio: 0.75 }),
)
check(
  'block correction preserves the saved sticky-viewport offset',
  correctedMarkdownAnchorScrollTop(exact, {
    scrollTop: 400,
    currentOffsetPx: 20,
    maxScrollTop: 1000,
  }) === 432.5,
)
check(
  'ratio fallback uses the current extent and clamps invalid geometry safely',
  correctedMarkdownAnchorScrollTop(missing, {
    scrollTop: 0,
    currentOffsetPx: null,
    maxScrollTop: 800,
  }) === 600
    && correctedMarkdownAnchorScrollTop(missing, {
      scrollTop: 0,
      currentOffsetPx: null,
      maxScrollTop: Number.NaN,
    }) === null,
)

console.log('\n=== MarkdownView ownership, readiness and cancellation wiring ===')
const viewSource = readFileSync(new URL('../src/MarkdownView.vue', import.meta.url), 'utf8')
const correctionBlock = viewSource.slice(
  viewSource.indexOf('function scheduleReadingAnchorCorrection('),
  viewSource.indexOf('function restoreReadingAnchor('),
)
const renderBlock = viewSource.slice(
  viewSource.indexOf('async function renderActive('),
  viewSource.indexOf('function teardownStream('),
)
check(
  'MarkdownView publishes anchors and exposes capture, restore and cancellation',
  viewSource.includes("'reading-anchor': [MarkdownReadingAnchor]")
    && viewSource.includes('captureReadingAnchor,')
    && viewSource.includes('restoreReadingAnchor,')
    && viewSource.includes('cancelReadingAnchorRestore,'),
)
check(
  'only the frozen visible content block tags participate in anchor identity',
  viewSource.includes("const MARKDOWN_CONTENT_BLOCK_SELECTOR = 'h1,h2,h3,h4,h5,h6,p,li,pre,blockquote,table'")
    && viewSource.includes('querySelectorAll<HTMLElement>(MARKDOWN_CONTENT_BLOCK_SELECTOR)'),
)
check(
  'capture measures content against the sticky document header bottom',
  viewSource.includes("head.value?.getBoundingClientRect().bottom")
    && viewSource.includes('block.rect.top - contentViewportTop'),
)
check(
  'restore cannot choose block or ratio fallback before async render and KaTeX finish',
  correctionBlock.includes('if (!renderedText')
    && renderBlock.indexOf('await typesetMath(article.value)') >= 0
    && renderBlock.indexOf('scheduleReadingAnchorCorrection()')
      > renderBlock.indexOf('renderedText = buildRenderedTextIndex(article.value)'),
)
check(
  'render/file generations and restore sequence reject stale animation callbacks',
  correctionBlock.includes('sequence !== readingAnchorRestoreSequence')
    && correctionBlock.includes('request !== renderRequest')
    && correctionBlock.includes('filePath !== props.path'),
)
check(
  'scroll and post-layout resize emit/correct anchors without replaying after user input',
  viewSource.includes("addEventListener('scroll', scheduleReadingAnchorEmit")
    && viewSource.includes('scheduleReadingAnchorCorrection()')
    && viewSource.includes("addEventListener('wheel', cancelReadingAnchorRestore")
    && viewSource.includes("addEventListener('touchstart', cancelReadingAnchorRestore")
    && viewSource.includes("addEventListener('pointerdown', onReadingAnchorPointerDown")
    && viewSource.includes("addEventListener('keydown', onReadingAnchorKeyDown"),
)
check(
  'file switches, translation mode and explicit find navigation cancel old restore',
  viewSource.includes('function reset(): void {\n  cancelReadingAnchorRestore()')
    && viewSource.includes('async function showOriginal(): Promise<void> {\n  cancelReadingAnchorRestore()')
    && viewSource.includes('function showChinese(): void {\n  cancelReadingAnchorRestore()')
    && viewSource.includes('if (scrollCurrent) cancelReadingAnchorRestore()')
    && viewSource.includes('function moveFind(direction: InFileFindDirection): void {\n  cancelReadingAnchorRestore()'),
)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll workspace markdown-anchor checks passed.')
