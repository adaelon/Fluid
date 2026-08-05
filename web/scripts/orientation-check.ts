// S-ORI-4 deterministic checks. Run with:
//   node scripts/orientation-check.ts
// Node 24 strips the TypeScript annotations; no browser or provider needed.

import type { FileOrientationCard } from '../src/ghostTypes.ts'
import {
  orientationCanActivate,
  reduceOrientationFrame,
  startOrientationRequest,
} from '../src/orientationState.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function card(filePath: string): FileOrientationCard {
  return {
    schemaVersion: 1,
    orientationId: `orientation:${filePath}`,
    filePath,
    purpose: '解释一个真实文件的职责与方向。',
    actors: [
      { id: 'caller', name: 'Caller', role: '发起请求的参与者。', boundary: 'project' },
      { id: 'worker', name: 'Worker', role: '处理请求的参与者。', boundary: 'inside-file' },
    ],
    types: [{ name: 'Request', ownerActorId: 'caller', meaning: '待处理请求。' }],
    coreFlows: [
      {
        id: 'request-flow',
        name: '请求流',
        kind: 'request',
        why: '把请求交给处理者。',
        steps: [
          {
            fromActorId: 'caller',
            via: 'run',
            payload: 'Request',
            toActorId: 'worker',
            why: '触发实际处理。',
            evidenceIds: ['E1'],
          },
        ],
      },
    ],
    supportingCapabilities: [],
    functionRoles: [
      {
        fnId: 'run#1',
        lane: 'core',
        flowIds: ['request-flow'],
        stage: '处理请求',
        receivesFromActorIds: ['caller'],
        consumes: ['Request'],
        sendsToActorIds: [],
        produces: [],
        why: '完成文件主要职责。',
        evidenceIds: ['E1'],
      },
    ],
    walkthrough: {
      title: '一次请求',
      input: 'Request(id=req-1)',
      steps: [{ text: 'Caller 调用 Worker.run。', evidenceIds: ['E1'] }],
    },
    invariants: [{ text: '请求只处理一次。', evidenceIds: ['E1'] }],
    evidence: [{ id: 'E1', filePath, startLine: 1, endLine: 1, symbol: 'run' }],
    coverage: { mode: 'full-source', omittedFunctionIds: [] },
  }
}

console.log('=== cache miss planning -> card -> done gate ===')
let state = startOrientationRequest('ori-1', 'src/a.ts')
check('request starts in connecting state', state.mode === 'loading' && state.phase === 'connecting')
state = reduceOrientationFrame(state, {
  kind: 'status',
  reqId: 'ori-1',
  phase: 'planning-source',
  message: '规划源码',
})
check('planning-source is visible', state.mode === 'loading' && state.phase === 'planning-source')
state = reduceOrientationFrame(state, {
  kind: 'status',
  reqId: 'ori-1',
  phase: 'orienting',
  message: '生成卡片',
})
state = reduceOrientationFrame(state, { kind: 'card', reqId: 'ori-1', card: card('src/a.ts') })
check('card alone keeps capsule gate closed', !orientationCanActivate(state, 'src/a.ts'))
state = reduceOrientationFrame(state, { kind: 'done', reqId: 'ori-1' })
check('matching card plus done opens capsule gate', orientationCanActivate(state, 'src/a.ts'))

console.log('\n=== cache hit and terminal errors ===')
let hit = startOrientationRequest('ori-hit', 'src/a.ts')
hit = reduceOrientationFrame(hit, { kind: 'cache-hit', reqId: 'ori-hit' })
check('cache-hit marker survives until the card', hit.cacheHit)
hit = reduceOrientationFrame(hit, { kind: 'card', reqId: 'ori-hit', card: card('src/a.ts') })
hit = reduceOrientationFrame(hit, { kind: 'done', reqId: 'ori-hit' })
check('cache-hit card follows the same ready gate', hit.mode === 'ready' && hit.cacheHit)

let missing = startOrientationRequest('ori-missing', 'src/a.ts')
missing = reduceOrientationFrame(missing, { kind: 'done', reqId: 'ori-missing' })
check('done without card becomes a visible error', missing.mode === 'error' && !!missing.errorMessage)

let failed = startOrientationRequest('ori-fail', 'src/a.ts')
failed = reduceOrientationFrame(failed, { kind: 'error', reqId: 'ori-fail', message: '供应商失败' })
check('wire error is terminal and visible', failed.mode === 'error' && failed.errorMessage === '供应商失败')
const retried = startOrientationRequest('ori-retry', failed.filePath)
check('retry starts clean with a new reqId', retried.mode === 'loading' && retried.reqId === 'ori-retry' && !retried.card)

console.log('\n=== rapid file switch stale-frame isolation ===')
const switched = startOrientationRequest('ori-b', 'src/b.ts')
const afterStale = reduceOrientationFrame(switched, {
  kind: 'card',
  reqId: 'ori-1',
  card: card('src/a.ts'),
})
check('old reqId is discarded by identity', afterStale === switched)
const wrongCard = reduceOrientationFrame(switched, {
  kind: 'card',
  reqId: 'ori-b',
  card: card('src/a.ts'),
})
check('matching reqId cannot write a different file card', wrongCard.mode === 'error')

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll orientation checks passed.')
