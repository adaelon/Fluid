// S-QWEB-2 deterministic checks. Run with:
//   node scripts/query-web-check.ts
// Node 24 strips the TypeScript annotations; no test framework or browser needed.

import { reduceQueryFrame, startQueryRequest } from '../src/queryState.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

console.log('=== cited query status -> evidence -> delta -> done ===')
let state = startQueryRequest()
state = reduceQueryFrame(state, {
  kind: 'status',
  reqId: 'q',
  phase: 'planning-web',
  message: '规划中',
})
state = reduceQueryFrame(state, {
  kind: 'status',
  reqId: 'q',
  phase: 'searching-web',
  message: '检索中',
})
check('searching status is visible', state.mode === 'streaming' && state.phase === 'searching-web')
state = reduceQueryFrame(state, {
  kind: 'evidence',
  reqId: 'q',
  status: 'web-cited',
  sources: [{ title: 'Rust docs', url: 'https://doc.rust-lang.org/' }],
})
state = reduceQueryFrame(state, {
  kind: 'status',
  reqId: 'q',
  phase: 'answering',
  message: '作答中',
})
state = reduceQueryFrame(state, { kind: 'delta', reqId: 'q', text: '第一段' })
state = reduceQueryFrame(state, { kind: 'delta', reqId: 'q', text: '第二段' })
state = reduceQueryFrame(state, { kind: 'done', reqId: 'q' })
check('delta chunks accumulate in order', state.answer === '第一段第二段')
check(
  'done preserves cited evidence and sources',
  state.mode === 'done' &&
    state.evidence?.status === 'web-cited' &&
    state.evidence.sources[0]?.url === 'https://doc.rust-lang.org/',
)

console.log('\n=== uncited and fallback evidence states ===')
let uncited = startQueryRequest()
uncited = reduceQueryFrame(uncited, {
  kind: 'evidence',
  reqId: 'qf',
  status: 'web-uncited',
})
check(
  'web-uncited remains a successful metadata state without sources',
  uncited.evidence?.status === 'web-uncited' && uncited.evidence.sources.length === 0,
)

let fallback = startQueryRequest()
fallback = reduceQueryFrame(fallback, {
  kind: 'status',
  reqId: 'q',
  phase: 'fallback',
  message: '改用本地上下文',
})
fallback = reduceQueryFrame(fallback, {
  kind: 'evidence',
  reqId: 'q',
  status: 'unverified',
  warning: '未核验：联网失败（联网检索超时）',
})
fallback = reduceQueryFrame(fallback, { kind: 'delta', reqId: 'q', text: '本地回答' })
check(
  'fallback warning survives local answer deltas',
  fallback.answer === '本地回答' && fallback.evidence?.warning?.includes('超时') === true,
)

console.log('\n=== terminal error preserves partial answer ===')
let failed = startQueryRequest()
failed = reduceQueryFrame(failed, { kind: 'delta', reqId: 'q', text: '已经收到的回答' })
failed = reduceQueryFrame(failed, { kind: 'error', reqId: 'q', message: '连接中断' })
check('error is terminal and visible', failed.mode === 'error' && failed.errorMessage === '连接中断')
check('error does not erase received answer', failed.answer === '已经收到的回答')

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query Web checks passed.')
