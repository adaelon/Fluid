// S-QMAP-1 deterministic checks. Run with:
//   node scripts/query-map-check.ts
// Node 24 strips TypeScript annotations; no Vue, browser, socket or provider needed.

import { reduceQueryFrame, startQueryRequest } from '../src/queryState.ts'
import {
  queryAnswerEvidenceCitations,
  queryMapUnknownEvidenceIds,
} from '../src/queryEvidence.ts'
import { renderQueryMarkdown } from '../src/render/markdown.ts'
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
  actors: [
    { id: 'caller', name: 'Caller', role: 'Starts work.', boundary: 'project' },
    { id: 'worker', name: 'Worker', role: 'Finishes work.', boundary: 'inside-file' },
  ],
  direction: [
    {
      fromActorId: 'caller',
      via: 'dispatch',
      payload: 'Request',
      toActorId: 'worker',
      why: 'The worker needs the request.',
      evidenceIds: ['E1'],
    },
  ],
  coreFunctionIds: ['dispatch#1'],
  supportingFunctionIds: ['record_metric#8'],
  walkthrough: {
    title: 'One request',
    input: 'request-1',
    steps: [{ text: 'Caller dispatches request-1.', evidenceIds: ['E1'] }],
  },
  evidence: [{ id: 'E1', filePath: 'src/a.ts', startLine: 1, endLine: 4, symbol: 'dispatch' }],
}

console.log('=== map is a hard precondition for answer deltas ===')
let state = startQueryRequest('q-2')
const beforeStale = state
state = reduceQueryFrame(state, { kind: 'map', reqId: 'q-1', map })
check('stale map cannot cross into the active request', state === beforeStale && state.map === null)
state = reduceQueryFrame(state, { kind: 'map', reqId: 'q-2', map })
check('matching map is retained before answer text', state.map === map && state.answer === '')
state = reduceQueryFrame(state, { kind: 'delta', reqId: 'q-2', text: '回答 [E1]' })
check('delta is accepted after map', state.answer === '回答 [E1]')

let malformed = startQueryRequest('q-3')
malformed = reduceQueryFrame(malformed, { kind: 'delta', reqId: 'q-3', text: '抢跑回答' })
check(
  'delta before map becomes a visible protocol error',
  malformed.mode === 'error' && malformed.answer === '' && malformed.errorMessage.includes('方向图'),
)
const terminalMalformed = malformed
malformed = reduceQueryFrame(malformed, { kind: 'map', reqId: 'q-3', map })
check('a protocol error cannot be revived by later frames', malformed === terminalMalformed)

let lateEvidence = startQueryRequest('q-4')
lateEvidence = reduceQueryFrame(lateEvidence, { kind: 'map', reqId: 'q-4', map })
lateEvidence = reduceQueryFrame(lateEvidence, { kind: 'delta', reqId: 'q-4', text: '回答' })
lateEvidence = reduceQueryFrame(lateEvidence, {
  kind: 'evidence',
  reqId: 'q-4',
  status: 'unverified',
})
check(
  'evidence after a delta becomes a visible protocol error',
  lateEvidence.mode === 'error' && lateEvidence.errorMessage.includes('证据'),
)

let duplicateEvidence = startQueryRequest('q-5')
duplicateEvidence = reduceQueryFrame(duplicateEvidence, { kind: 'map', reqId: 'q-5', map })
duplicateEvidence = reduceQueryFrame(duplicateEvidence, {
  kind: 'evidence',
  reqId: 'q-5',
  status: 'unverified',
})
duplicateEvidence = reduceQueryFrame(duplicateEvidence, {
  kind: 'evidence',
  reqId: 'q-5',
  status: 'web-uncited',
})
check(
  'duplicate evidence becomes a visible protocol error',
  duplicateEvidence.mode === 'error' && duplicateEvidence.errorMessage.includes('证据'),
)

console.log('\n=== evidence references and Markdown links ===')
check('a valid map has no dangling E#', queryMapUnknownEvidenceIds(map).length === 0)
const dangling: QueryMap = {
  ...map,
  direction: [{ ...map.direction[0], evidenceIds: ['E1', 'E9'] }],
}
check('dangling map E# is reported', queryMapUnknownEvidenceIds(dangling).join(',') === 'E9')

const rendered = renderQueryMarkdown('结论 [E1]，未知 [E9]，代码 `[E9]`。', map.evidence)
check('known E# becomes a clickable local evidence link', rendered.includes('href="#fluid-evidence-E1"'))
check('unknown E# is not turned into a link', !rendered.includes('href="#fluid-evidence-E9"'))
const citations = queryAnswerEvidenceCitations('结论 [E1]，未知 [E9]，代码 `[E8]`。', map.evidence)
check('known E# enters trace metadata', citations.knownIds.join(',') === 'E1')
check('unknown prose E# is visible but code literals are ignored', citations.unknownIds.join(',') === 'E9')

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query map checks passed.')
