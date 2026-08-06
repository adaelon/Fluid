// S-QTRACE-1 deterministic checks. Run with:
//   node scripts/query-trace-check.ts
// Node 24 strips TypeScript annotations; no Vue, browser, socket or provider needed.

import {
  alignQueryTrace,
  appendCompletedQueryTurn,
  currentQueryScope,
  selectedQueryScope,
  startQueryRequest,
  startQueryTrace,
  reduceQueryFrame,
} from '../src/queryState.ts'
import type { QueryMap } from '../src/ghostTypes.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

const map: QueryMap = {
  actors: [{ id: 'file', name: '当前文件', role: '本次追问范围。', boundary: 'inside-file' }],
  direction: [],
  coreFunctionIds: [],
  supportingFunctionIds: [],
  walkthrough: {
    title: '直接作用',
    input: 'fixture',
    steps: [{ text: '核对当前源码。', evidenceIds: [] }],
  },
  evidence: [],
}

console.log('=== first question and continuous follow-ups ===')
const current = currentQueryScope('src/a.ts', 'orientation-a1')
let trace = startQueryTrace(current, '为什么这里要排队？')
check('first question becomes the immutable original question', trace.originalQuestion === '为什么这里要排队？')
check('no incomplete turn is recorded before done', trace.turns.length === 0)

trace = appendCompletedQueryTurn(trace, current, '为什么这里要排队？', '为了限制并发。')
trace = appendCompletedQueryTurn(trace, current, '如果不限制会怎样？', '供应商会被突发请求压垮。')
check('completed turns retain question-answer pairs in order', trace.turns.length === 2)
check('a follow-up never replaces the original question', trace.originalQuestion === '为什么这里要排队？')
check('latest correction is retained verbatim', trace.turns[1]?.answer.includes('突发请求') === true)

console.log('\n=== scope and revision isolation ===')
check('same scope and revision retain the trace', alignQueryTrace(trace, current) === trace)
check(
  'orientation revision change clears the trace',
  alignQueryTrace(trace, currentQueryScope('src/a.ts', 'orientation-a2')) === null,
)
check(
  'file switch clears the trace',
  alignQueryTrace(trace, currentQueryScope('src/b.ts', 'orientation-b1')) === null,
)
const selectedA = selectedQueryScope(['src/b.ts', 'src/a.ts', 'src/a.ts'])
const selectedB = selectedQueryScope(['src/a.ts', 'src/b.ts'])
check('selected scope identity sorts and deduplicates paths', selectedA.scopeKey === selectedB.scopeKey)
const selectedUnicode = selectedQueryScope(['\u{e000}.py', '\u{10000}.py'])
check(
  'selected scope uses the wire-compatible UTF-16 path order',
  selectedUnicode.scopeKey === 'selected:["\u{10000}.py","\u{e000}.py"]',
)
check('switching current/selected scope clears the trace', alignQueryTrace(trace, selectedA) === null)

console.log('\n=== stale stream isolation ===')
let state = startQueryRequest('q-2')
const beforeStale = state
state = reduceQueryFrame(state, { kind: 'delta', reqId: 'q-1', text: '旧回答' })
check('a stale request id cannot append answer text', state === beforeStale && state.answer === '')
state = reduceQueryFrame(state, { kind: 'map', reqId: 'q-2', map })
state = reduceQueryFrame(state, { kind: 'delta', reqId: 'q-2', text: '当前回答' })
check('the active request id still streams normally', state.answer === '当前回答')

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query trace checks passed.')
