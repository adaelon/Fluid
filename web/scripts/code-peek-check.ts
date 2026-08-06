// S-QPEEK-1 deterministic checks. Run with:
//   node scripts/code-peek-check.ts
// Node 24 strips TypeScript annotations; no Vue, DOM or backend is needed.

import {
  codePeekCenterLine,
  codePeekDocumentLineCount,
  completeCodePeekRequest,
  disposeCodePeekState,
  failCodePeekRequest,
  idleCodePeekState,
  selectCodePeekTarget,
  startCodePeekRequest,
  validateCodePeekRange,
} from '../src/codePeekState.ts'
import type { CodeEvidenceRef } from '../src/ghostTypes.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function target(
  filePath: string,
  startLine: number,
  endLine: number,
  id = 'E1',
): CodeEvidenceRef {
  return { id, filePath, startLine, endLine }
}

console.log('=== 1-based inclusive evidence range validation ===')
const fourLines = 'first\nsecond\nthird\nfourth'
check('document line count includes every source line', codePeekDocumentLineCount(fourLines) === 4)
check('empty documents still have one CodeMirror line', codePeekDocumentLineCount('') === 1)
check('a trailing newline creates the final empty line', codePeekDocumentLineCount('first\n') === 2)
check('single-line range is valid', validateCodePeekRange(target('a.ts', 2, 2), 4).ok)
check('multi-line range is valid', validateCodePeekRange(target('a.ts', 2, 3), 4).ok)
check('first line is valid', validateCodePeekRange(target('a.ts', 1, 1), 4).ok)
check('last line is valid', validateCodePeekRange(target('a.ts', 4, 4), 4).ok)
check('single-line range centers on itself', codePeekCenterLine(target('a.ts', 3, 3)) === 3)
check('even multi-line range uses a stable whole center line', codePeekCenterLine(target('a.ts', 2, 3)) === 2)
check('zero-based start is rejected', !validateCodePeekRange(target('a.ts', 0, 1), 4).ok)
check('reversed range is rejected', !validateCodePeekRange(target('a.ts', 3, 2), 4).ok)
check('fractional line is rejected', !validateCodePeekRange(target('a.ts', 1.5, 2), 4).ok)
check('range beyond the document is rejected', !validateCodePeekRange(target('a.ts', 4, 5), 4).ok)

console.log('\n=== ready state reuses source for same-file range changes ===')
const firstTarget = target('src/a.ts', 1, 1)
let state = startCodePeekRequest(1, firstTarget)
state = completeCodePeekRequest(state, 1, fourLines, 'ts')
check('matching load becomes ready', state.mode === 'ready' && state.source === fourLines)
const sameFile = selectCodePeekTarget(state, target('src/a.ts', 2, 4, 'E2'), 2)
check(
  'same-file target stays ready without a new request',
  sameFile.mode === 'ready' && sameFile.source === fourLines && sameFile.target.id === 'E2',
)
const invalidSameFile = selectCodePeekTarget(state, target('src/a.ts', 3, 8, 'E3'), 2)
check(
  'same-file invalid range fails visibly before rendering',
  invalidSameFile.mode === 'error' && invalidSameFile.message.includes('4'),
)

console.log('\n=== cross-file request ids isolate slow responses ===')
const crossFile = selectCodePeekTarget(state, target('src/b.ts', 1, 1, 'E4'), 2)
check('cross-file target starts a fresh load', crossFile.mode === 'loading' && crossFile.requestId === 2)
const afterSlowA = completeCodePeekRequest(crossFile, 1, 'stale', 'ts')
check('slow prior response is ignored by identity', afterSlowA === crossFile)
const readyB = completeCodePeekRequest(crossFile, 2, 'only line', 'ts')
check(
  'matching cross-file response becomes ready',
  readyB.mode === 'ready' && readyB.target.filePath === 'src/b.ts' && readyB.source === 'only line',
)

console.log('\n=== errors retry cleanly and disposal blocks write-back ===')
const loading = startCodePeekRequest(3, target('src/error.ts', 1, 1, 'E5'))
const failed = failCodePeekRequest(loading, 3, 'network down')
check('matching failure is visible', failed.mode === 'error' && failed.message === 'network down')
const retried = selectCodePeekTarget(failed, failed.target, 4)
check('retry starts the same target under a new request id', retried.mode === 'loading' && retried.requestId === 4)
const afterStaleError = failCodePeekRequest(retried, 3, 'old error')
check('stale failure cannot replace the retry', afterStaleError === retried)
const disposed = disposeCodePeekState(retried)
const afterDisposedLoad = completeCodePeekRequest(disposed, 4, 'late source', 'ts')
check('late completion after disposal cannot write back', afterDisposedLoad === disposed && disposed.mode === 'idle')

const untouchedIdle = selectCodePeekTarget(idleCodePeekState(), firstTarget, 5)
check('idle selection starts loading', untouchedIdle.mode === 'loading' && untouchedIdle.requestId === 5)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll code-peek checks passed.')
