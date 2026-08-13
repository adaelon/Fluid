// S-WANCHOR-CM1 deterministic checks. Run with:
//   node web/scripts/workspace-code-anchor-check.ts
// The geometry math and Editor ownership wiring are checked without a browser.

import { readFileSync } from 'node:fs'
import {
  MAX_CODE_ANCHOR_OFFSET_PX,
  MAX_CODE_ANCHOR_TOTAL_LINES,
  captureCodeReadingAnchor,
  correctedCodeAnchorScrollTop,
  normalizeCodeReadingAnchor,
  resolveCodeReadingAnchor,
} from '../src/readingAnchor.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

console.log('=== capture normalization and finite bounds ===')
const captured = captureCodeReadingAnchor({
  topLine: 42,
  offsetPx: -7.25,
  totalLines: 120,
})
check(
  'a measured top line, viewport offset and line count form a code anchor',
  JSON.stringify(captured) === JSON.stringify({
    kind: 'code',
    topLine: 42,
    offsetPx: -7.25,
    totalLines: 120,
  }),
)
check(
  'large positive finite offsets clamp to the persisted-state bound',
  captureCodeReadingAnchor({
    topLine: 1,
    offsetPx: MAX_CODE_ANCHOR_OFFSET_PX * 2,
    totalLines: 1,
  })?.offsetPx === MAX_CODE_ANCHOR_OFFSET_PX,
)
check(
  'large negative finite offsets clamp symmetrically',
  captureCodeReadingAnchor({
    topLine: 1,
    offsetPx: -MAX_CODE_ANCHOR_OFFSET_PX * 2,
    totalLines: 1,
  })?.offsetPx === -MAX_CODE_ANCHOR_OFFSET_PX,
)
check(
  'NaN and infinite offsets degrade to no anchor',
  captureCodeReadingAnchor({ topLine: 1, offsetPx: Number.NaN, totalLines: 1 }) === null
    && captureCodeReadingAnchor({ topLine: 1, offsetPx: Number.POSITIVE_INFINITY, totalLines: 1 }) === null,
)
check(
  'zero, fractional and reversed line identities are rejected',
  captureCodeReadingAnchor({ topLine: 0, offsetPx: 0, totalLines: 1 }) === null
    && captureCodeReadingAnchor({ topLine: 1.5, offsetPx: 0, totalLines: 2 }) === null
    && captureCodeReadingAnchor({ topLine: 3, offsetPx: 0, totalLines: 2 }) === null,
)
check(
  'the frontend total-line ceiling stays aligned with persisted validation',
  MAX_CODE_ANCHOR_TOTAL_LINES === 100_000_000
    && captureCodeReadingAnchor({
      topLine: 1,
      offsetPx: 0,
      totalLines: MAX_CODE_ANCHOR_TOTAL_LINES + 1,
    }) === null,
)
check(
  'runtime normalization rejects non-code and malformed payloads',
  normalizeCodeReadingAnchor(null) === null
    && normalizeCodeReadingAnchor({ kind: 'markdown' }) === null
    && normalizeCodeReadingAnchor({
      kind: 'code',
      topLine: 1,
      offsetPx: Number.NaN,
      totalLines: 1,
    }) === null,
)

console.log('\n=== unchanged and changed-document targeting ===')
const exact = resolveCodeReadingAnchor(captured, 120)
check(
  'an unchanged line count restores the exact saved line and offset',
  JSON.stringify(exact) === JSON.stringify({
    lineNumber: 42,
    offsetPx: -7.25,
    lineCountChanged: false,
  }),
)
const shortened = resolveCodeReadingAnchor({
  kind: 'code',
  topLine: 90,
  offsetPx: -3,
  totalLines: 100,
}, 40)
check(
  'a shorter document clamps to its last line without pixel-ratio fallback',
  JSON.stringify(shortened) === JSON.stringify({
    lineNumber: 40,
    offsetPx: -3,
    lineCountChanged: true,
  }),
)
const lengthened = resolveCodeReadingAnchor({
  kind: 'code',
  topLine: 90,
  offsetPx: -3,
  totalLines: 100,
}, 160)
check(
  'a longer document keeps the saved line number nearby',
  JSON.stringify(lengthened) === JSON.stringify({
    lineNumber: 90,
    offsetPx: -3,
    lineCountChanged: true,
  }),
)
check(
  'invalid current document geometry degrades without throwing',
  resolveCodeReadingAnchor(captured, 0) === null
    && resolveCodeReadingAnchor(captured, Number.NaN) === null,
)

console.log('\n=== measured scroll correction ===')
check(
  'currentOffset - savedOffset is added to scrollTop',
  correctedCodeAnchorScrollTop({
    scrollTop: 400,
    currentOffsetPx: -2,
    savedOffsetPx: -10,
    maxScrollTop: 1000,
  }) === 408,
)
check(
  'correction clamps at the scroll origin',
  correctedCodeAnchorScrollTop({
    scrollTop: 2,
    currentOffsetPx: -50,
    savedOffsetPx: 0,
    maxScrollTop: 1000,
  }) === 0,
)
check(
  'correction clamps at the current scroll extent after reflow',
  correctedCodeAnchorScrollTop({
    scrollTop: 980,
    currentOffsetPx: 80,
    savedOffsetPx: 0,
    maxScrollTop: 1000,
  }) === 1000,
)
check(
  'non-finite geometry produces no scroll command',
  correctedCodeAnchorScrollTop({
    scrollTop: 10,
    currentOffsetPx: Number.NaN,
    savedOffsetPx: 0,
    maxScrollTop: 100,
  }) === null,
)

console.log('\n=== CodeMirror ownership and cancellation wiring ===')
const editorSource = readFileSync(new URL('../src/Editor.vue', import.meta.url), 'utf8')
const restoreBlock = editorSource.slice(
  editorSource.indexOf('function restoreReadingAnchor('),
  editorSource.indexOf('function onReadingAnchorPointerDown('),
)
const capsuleStartBlock = editorSource.slice(
  editorSource.indexOf('async function startCapsulesAfterOrientation('),
  editorSource.indexOf('function requestOrientation('),
)
check(
  'Editor publishes code anchors and exposes capture, restore and cancellation',
  editorSource.includes("'reading-anchor': [CodeReadingAnchor]")
    && editorSource.includes('captureReadingAnchor,')
    && editorSource.includes('restoreReadingAnchor,')
    && editorSource.includes('cancelReadingAnchorRestore,'),
)
check(
  'capture uses the viewport top and CodeMirror line-block geometry',
  editorSource.includes('lineBlockAtHeight(')
    && editorSource.includes('.documentTop')
    && editorSource.includes('.lineBlockAt(')
    && editorSource.includes('getBoundingClientRect().top'),
)
check(
  'restore corrects scrollTop in a CodeMirror measure cycle',
  editorSource.includes('.requestMeasure({')
    && editorSource.includes('correctedCodeAnchorScrollTop({')
    && editorSource.includes('.scrollDOM.scrollTop ='),
)
check(
  'restore does not dispatch a selection or start generation work',
  restoreBlock.length > 0
    && !restoreBlock.includes('.dispatch(')
    && !restoreBlock.includes('selection')
    && !restoreBlock.includes('scheduler'),
)
check(
  'viewport changes emit fresh anchors and geometry changes replay correction',
  editorSource.includes('if (u.viewportChanged) scheduleReadingAnchorEmit()')
    && editorSource.includes('if (u.geometryChanged) scheduleReadingAnchorCorrection()'),
)
check(
  'capsule scheduling gives a pending restored anchor first correction priority',
  capsuleStartBlock.indexOf('scheduleReadingAnchorCorrection()') >= 0
    && capsuleStartBlock.indexOf('scheduleReadingAnchorCorrection()')
      < capsuleStartBlock.indexOf('scheduler.start(ids, viewportDist())'),
)
check(
  'find and evidence navigation cancel delayed restoration before scrolling',
  editorSource.includes('cancelReadingAnchorRestore()\n  const selection = EditorSelection.single')
    && editorSource.includes('cancelReadingAnchorRestore()\n  suppressFindStateEmit = true')
    && editorSource.includes('cancelReadingAnchorRestore()\n      editor.dispatch({'),
)
check(
  'user wheel, touch, scrollbar and keyboard navigation cancel correction',
  editorSource.includes("addEventListener('wheel', cancelReadingAnchorRestore")
    && editorSource.includes("addEventListener('touchstart', cancelReadingAnchorRestore")
    && editorSource.includes("addEventListener('pointerdown', onReadingAnchorPointerDown")
    && editorSource.includes("addEventListener('keydown', onReadingAnchorKeyDown"),
)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll workspace code-anchor checks passed.')
