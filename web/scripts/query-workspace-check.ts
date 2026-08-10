// S-QSTATE-R0 deterministic characterization checks. Run with:
//   npx tsx scripts/query-workspace-check.ts
// No DOM, browser, real socket or provider is needed.

import { readFileSync } from 'node:fs'
import {
  createQueryWorkspace,
  type QueryWorkspaceStream,
} from '../src/queryWorkspace.ts'
import type { QueryMap } from '../src/ghostTypes.ts'
import { currentQueryScope } from '../src/queryState.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function mapFor(filePath: string): QueryMap {
  return {
    actors: [{ id: 'file', name: filePath, role: '本轮文件', boundary: 'inside-file' }],
    direction: [],
    coreFunctionIds: [],
    supportingFunctionIds: [],
    walkthrough: {
      title: '直接作用',
      input: filePath,
      steps: [{ text: '核对本轮源码。', evidenceIds: ['E1'] }],
    },
    evidence: [{ id: 'E1', filePath, startLine: 2, endLine: 4, symbol: 'answer' }],
  }
}

function fakeStream(): QueryWorkspaceStream & { cancelCount: number } {
  return {
    cancelCount: 0,
    cancel() {
      this.cancelCount++
    },
  }
}

const identity = currentQueryScope('src/a.ts', 'orientation-a1')
const workspace = createQueryWorkspace()

console.log('=== first question and complete turn ownership ===')
workspace.question.value = '  为什么这里要排队？  '
const first = workspace.beginRequest(identity)
check('a non-empty draft starts a request', first !== null)
if (!first) process.exit(1)
check('the controller trims and clears the submitted draft', first.question === '为什么这里要排队？' && workspace.question.value === '')
check('the first request creates an empty trace with the original question', workspace.trace.value?.originalQuestion === first.question && workspace.trace.value.turns.length === 0)
check('the active question and turn selection live in the controller', workspace.activeQuestion.value === first.question && workspace.selectedTurn.value?.kind === 'active')
check('a first request starts without a durable thread binding', first.threadId === null)
check('the matching REST-created thread binds to the active request generation', workspace.bindThread(first, 'thread-1', '2026-08-10T10:00:00.000Z') && workspace.threadId.value === 'thread-1')

const firstStream = fakeStream()
check('the matching stream attaches to the active generation', workspace.attachStream(first.generation, firstStream))
workspace.acceptFrame(first, { kind: 'map', reqId: first.requestId, map: mapFor('src/a.ts') })
workspace.acceptFrame(first, { kind: 'delta', reqId: first.requestId, text: '为了限制并发。[E1]' })
const completed = workspace.acceptFrame(first, {
  kind: 'done',
  reqId: first.requestId,
  threadId: 'thread-1',
  updatedAt: '2026-08-10T10:01:00.000Z',
})
check('done appends exactly one complete question-answer turn', completed.kind === 'completed' && workspace.trace.value?.turns.length === 1 && workspace.trace.value.turns[0]?.answer === '为了限制并发。[E1]')
check('done advances the durable thread update marker', workspace.threadUpdatedAt.value === '2026-08-10T10:01:00.000Z')
check('done records only known code-evidence citations', workspace.trace.value?.turns[0]?.codeEvidenceIds.join(',') === 'E1')
check('done appends an index-aligned presentation snapshot', workspace.traceSnapshots.value.length === 1 && workspace.traceSnapshots.value[0]?.map.evidence[0]?.filePath === 'src/a.ts')
check('done selects the latest completed turn and clears the active question', workspace.selectedTurn.value?.kind === 'completed' && workspace.selectedTurn.value.index === 0 && workspace.activeQuestion.value === '')
check('normal completion detaches without cancelling the completed stream', firstStream.cancelCount === 0)
const completedSnapshots = workspace.traceSnapshots.value
check('rendered answer HTML updates only the matching snapshot generation', workspace.applyRenderedAnswers(completedSnapshots, ['<p>为了限制并发。</p>']) && workspace.traceSnapshots.value[0]?.answerHtml.includes('为了限制并发') === true)

const mismatched = createQueryWorkspace()
mismatched.question.value = '线程身份会串吗？'
const mismatchedRequest = mismatched.beginRequest(identity)
if (!mismatchedRequest) process.exit(1)
mismatched.bindThread(mismatchedRequest, 'thread-a', '2026-08-10T10:00:00.000Z')
mismatched.acceptFrame(mismatchedRequest, { kind: 'map', reqId: mismatchedRequest.requestId, map: mapFor('src/a.ts') })
mismatched.acceptFrame(mismatchedRequest, { kind: 'delta', reqId: mismatchedRequest.requestId, text: '不能串写' })
const mismatchedDone = mismatched.acceptFrame(mismatchedRequest, {
  kind: 'done',
  reqId: mismatchedRequest.requestId,
  threadId: 'thread-b',
  updatedAt: '2026-08-10T10:01:00.000Z',
})
check('a done frame for another durable thread is rejected without appending history', mismatchedDone.kind === 'error' && mismatched.viewState.value.mode === 'error' && mismatched.trace.value?.turns.length === 0)

console.log('\n=== errors retain partial output but not history ===')
workspace.question.value = '如果不限制会怎样？'
const errored = workspace.beginRequest(identity)
if (!errored) process.exit(1)
check('follow-up requests reuse the same durable thread id', errored.threadId === 'thread-1')
const errorStream = fakeStream()
workspace.attachStream(errored.generation, errorStream)
workspace.acceptFrame(errored, { kind: 'map', reqId: errored.requestId, map: mapFor('src/a.ts') })
workspace.acceptFrame(errored, { kind: 'delta', reqId: errored.requestId, text: '部分回答' })
workspace.acceptFrame(errored, { kind: 'error', reqId: errored.requestId, message: '供应商断开' })
check('stream error preserves accumulated answer and visible error', workspace.viewState.value.mode === 'error' && workspace.viewState.value.answer === '部分回答' && workspace.viewState.value.errorMessage === '供应商断开')
check('an errored partial answer never enters the complete trace', workspace.trace.value?.turns.length === 1)
check('an errored turn stays active for presentation', workspace.activeQuestion.value === errored.question && workspace.selectedTurn.value?.kind === 'active')

console.log('\n=== teardown, reset and request-generation isolation ===')
workspace.question.value = '旧请求'
const stale = workspace.beginRequest(identity)
if (!stale) process.exit(1)
const staleStream = fakeStream()
workspace.attachStream(stale.generation, staleStream)
workspace.teardown()
const beforeStaleFrame = workspace.viewState.value
const staleResult = workspace.acceptFrame(stale, { kind: 'map', reqId: stale.requestId, map: mapFor('src/old.ts') })
check('teardown cancels the live stream exactly once', staleStream.cancelCount === 1)
check('a torn-down request generation cannot write frames', staleResult.kind === 'ignored' && workspace.viewState.value === beforeStaleFrame)

workspace.question.value = '保留草稿'
workspace.resetTrace(false)
check('scope reset can preserve the draft while clearing trace and presentation', workspace.question.value === '保留草稿' && workspace.trace.value === null && workspace.traceSnapshots.value.length === 0 && workspace.selectedTurn.value === null)
check('scope reset clears the durable thread binding', workspace.threadId.value === null && workspace.threadUpdatedAt.value === null)
check('scope reset returns the runtime projection to idle', workspace.viewState.value.mode === 'idle' && workspace.activeQuestion.value === '')
workspace.scope.value = 'selected'
workspace.resetForClose()
check('close reset clears the draft and restores the initial current scope', workspace.question.value === '' && workspace.scope.value === 'current')

console.log('\n=== dock/focus consume one project-scoped controller ===')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
const panelSource = readFileSync(new URL('../src/QueryPanel.vue', import.meta.url), 'utf8')
const apiSource = readFileSync(new URL('../src/api.ts', import.meta.url), 'utf8')
const queryPanelTags = appSource.match(/<QueryPanel\b[\s\S]*?\/>/g) ?? []
check('App creates one project-scoped query workspace', appSource.includes('const queryWorkspace = createQueryWorkspace()'))
check('dock and focus share one unkeyed QueryPanel consumer', queryPanelTags.length === 1 && queryPanelTags[0]?.includes(':workspace="queryWorkspace"') === true && !queryPanelTags[0]?.includes(':key='))
check('project lifetime, not the view shell, owns final teardown', appSource.includes('queryWorkspace.teardown()') && !panelSource.includes('workspace.teardown()'))
check('QueryPanel creates or reuses a durable thread before opening a socket', panelSource.includes('createQueryThread') && panelSource.includes('ensureRequestThread'))
check('query socket wire sends threadId and no longer sends the client trace', apiSource.includes('threadId: req.threadId') && !apiSource.includes('trace: req.trace'))

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query-workspace checks passed.')
