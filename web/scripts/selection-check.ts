// S-SEL-2 deterministic checks. Run with:
//   node scripts/selection-check.ts
// Node 24 strips the TypeScript annotations; no test framework or browser needed.

import {
  reduceSelectionFrame,
  selectionToUtf8ByteRange,
  startSelectionRequest,
  type SelectionViewState,
} from '../src/selectionState.ts'
import type { SelectionExplanation } from '../src/ghostTypes.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

console.log('=== CodeMirror UTF-16 offsets -> UTF-8 byte ranges ===')
const ascii = selectionToUtf8ByteRange('const value = 1', 6, 11)
check('ASCII offsets remain byte-identical', ascii?.startByte === 6 && ascii.endByte === 11)

const cjk = selectionToUtf8ByteRange('a名称z', 1, 3)
check('CJK selection counts three bytes per character', cjk?.startByte === 1 && cjk.endByte === 7)

const emoji = selectionToUtf8ByteRange('a名😀z', 2, 4)
check('emoji UTF-16 pair maps to four UTF-8 bytes', emoji?.startByte === 4 && emoji.endByte === 8)
check('surrogate-splitting range is rejected', selectionToUtf8ByteRange('a😀z', 1, 2) === null)
check('multiline range is rejected', selectionToUtf8ByteRange('one\ntwo', 0, 5) === null)
check('whitespace-only range is rejected', selectionToUtf8ByteRange('a   b', 1, 4) === null)

console.log('\n=== selection frame reducer ===')
const explanation: SelectionExplanation = {
  selectedText: 'value',
  kind: '变量',
  meaning: '一个局部值',
  roleHere: '参与当前计算',
  evidenceStatus: 'project-source',
}
let state = startSelectionRequest()
state = reduceSelectionFrame(state, { kind: 'status', reqId: 's1', phase: 'planning-web', message: '规划中' })
check('status enters loading phase', state.mode === 'loading' && state.phase === 'planning-web')
state = reduceSelectionFrame(state, { kind: 'cache-hit', reqId: 's1' })
check('cache hit is retained', state.mode === 'loading' && state.cacheHit)
state = reduceSelectionFrame(state, { kind: 'result', reqId: 's1', explanation })
check('result keeps cache flag and explanation', state.mode === 'result' && state.cacheHit && state.explanation === explanation)
const afterDone = reduceSelectionFrame(state, { kind: 'done', reqId: 's1' })
check('done does not erase the visible result', afterDone === state)

const failed: SelectionViewState = reduceSelectionFrame(startSelectionRequest(), {
  kind: 'error',
  reqId: 's2',
  message: '连接失败',
})
check('error is terminal and visible', failed.mode === 'error' && failed.message === '连接失败')

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll selection checks passed.')
