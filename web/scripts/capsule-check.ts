// S-CAP-1 deterministic presentation checks. Run with:
//   node scripts/capsule-check.ts

import { capsulePresentation, roleDirection } from '../src/capsulePresentation.ts'
import type { Capsule } from '../src/ghostTypes.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

const capsule: Capsule = {
  fnId: 'dispatch#10',
  signature: 'fn dispatch(request: Request)',
  summary: '把请求交给工作器。',
  complexity: 'moderate',
  io: 'Caller 的 Request -> Worker 的 Work',
  orientationId: 'orientation-1',
  role: {
    fnId: 'dispatch#10',
    lane: 'core',
    flowIds: ['request-flow'],
    stage: '分派请求',
    receivesFromActorIds: ['caller', 'caller'],
    consumes: ['Request'],
    sendsToActorIds: ['worker'],
    produces: ['Work'],
    why: '缺少分派就没有工作可执行。',
    evidenceIds: ['E1', 'E1', 'E2'],
  },
}

console.log('=== bound capsule presentation ===')
const view = capsulePresentation(capsule)
check('core lane is visible', view.lane === '核心')
check('stage is visible', view.stage === '分派请求')
check('named actor direction is stable and deduplicated', view.direction === 'caller → worker')
check('why stays expandable', view.why === capsule.role.why)
check('evidence IDs stay stable and deduplicated', view.evidence === 'E1 · E2')

console.log('\n=== direction edge cases ===')
check(
  'receive-only role points into the function',
  roleDirection({ ...capsule.role, sendsToActorIds: [] }) === 'caller → 本函数',
)
check(
  'send-only role points out of the function',
  roleDirection({ ...capsule.role, receivesFromActorIds: [] }) === '本函数 → worker',
)
check(
  'local role is explicit about no cross-actor flow',
  roleDirection({ ...capsule.role, receivesFromActorIds: [], sendsToActorIds: [] }) ===
    '无跨参与者流',
)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll capsule checks passed.')
