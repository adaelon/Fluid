import { ref, type Ref } from 'vue'
import type { QueryFrame, QueryTrace } from './ghostTypes'
import { queryAnswerEvidenceCitations } from './queryEvidence'
import {
  activeQueryTurnSelection,
  appendQueryTurnPresentationSnapshot,
  defaultQueryTurnSelection,
  resetQueryTurnPresentation,
  setQueryTurnAnswerHtml,
  type QueryTurnPresentationSnapshot,
  type QueryTurnSelection,
} from './queryPresentation'
import {
  alignQueryTrace,
  appendCompletedQueryTurn,
  idleQueryState,
  reduceQueryFrame,
  startQueryRequest,
  startQueryTrace,
  type QueryScopeIdentity,
  type QueryViewState,
} from './queryState'

export type QueryScope = 'current' | 'selected'

export interface QueryWorkspaceStream {
  cancel(): void
}

export interface QueryWorkspaceRequest {
  generation: number
  requestId: string
  identity: QueryScopeIdentity
  question: string
  trace: QueryTrace
}

export type QueryWorkspaceFrameResult =
  | { kind: 'ignored' }
  | { kind: 'updated' }
  | { kind: 'error' }
  | {
      kind: 'completed'
      turns: QueryTrace['turns']
      snapshots: QueryTurnPresentationSnapshot[]
    }

export interface QueryWorkspaceController {
  question: Ref<string>
  viewState: Ref<QueryViewState>
  trace: Ref<QueryTrace | null>
  traceSnapshots: Ref<QueryTurnPresentationSnapshot[]>
  selectedTurn: Ref<QueryTurnSelection>
  activeQuestion: Ref<string>
  scope: Ref<QueryScope>
  beginRequest(identity: QueryScopeIdentity): QueryWorkspaceRequest | null
  attachStream(generation: number, nextStream: QueryWorkspaceStream): boolean
  acceptFrame(
    request: QueryWorkspaceRequest,
    frame: QueryFrame,
  ): QueryWorkspaceFrameResult
  showValidationError(message: string): void
  applyRenderedAnswers(
    expectedSnapshots: QueryTurnPresentationSnapshot[],
    htmlByTurn: readonly string[],
  ): boolean
  resetTrace(clearQuestion?: boolean): void
  resetForClose(): void
  teardown(): void
}

/**
 * Project-scoped owner for the query runtime. Vue views consume these refs, but
 * socket identity, bounded trace and turn presentation survive view-shell moves.
 * Explicit close/scope reset methods keep the pre-S-QSTATE-R0 clearing rules.
 */
export function createQueryWorkspace(): QueryWorkspaceController {
  const question = ref('')
  const viewState = ref(idleQueryState())
  const trace = ref<QueryTrace | null>(null)
  const traceSnapshots = ref<QueryTurnPresentationSnapshot[]>([])
  const selectedTurn = ref<QueryTurnSelection>(null)
  const activeQuestion = ref('')
  const scope = ref<QueryScope>('current')

  let stream: QueryWorkspaceStream | null = null
  let requestGeneration = 0
  let requestSequence = 0

  function detachStream(cancel: boolean): void {
    const active = stream
    stream = null
    if (cancel) active?.cancel()
  }

  function teardown(): void {
    requestGeneration++
    detachStream(true)
  }

  function resetTrace(clearQuestion = true): void {
    teardown()
    trace.value = null
    const presentation = resetQueryTurnPresentation()
    selectedTurn.value = presentation.selection
    traceSnapshots.value = presentation.snapshots
    activeQuestion.value = ''
    viewState.value = idleQueryState()
    if (clearQuestion) question.value = ''
  }

  function resetForClose(): void {
    resetTrace()
    scope.value = 'current'
  }

  function beginRequest(identity: QueryScopeIdentity): QueryWorkspaceRequest | null {
    const askedQuestion = question.value.trim()
    if (!askedQuestion) return null

    selectedTurn.value = activeQueryTurnSelection()
    const requestIdentity = { ...identity }
    const requestTrace =
      alignQueryTrace(trace.value, requestIdentity) ??
      startQueryTrace(requestIdentity, askedQuestion)
    trace.value = requestTrace
    const prefix = scope.value === 'selected' ? 'qf' : 'q'
    const requestId = `${prefix}-${++requestSequence}`
    const request = {
      generation: ++requestGeneration,
      requestId,
      identity: requestIdentity,
      question: askedQuestion,
      trace: requestTrace,
    }
    viewState.value = startQueryRequest(requestId)
    activeQuestion.value = askedQuestion
    question.value = ''
    return request
  }

  function attachStream(
    generation: number,
    nextStream: QueryWorkspaceStream,
  ): boolean {
    if (generation !== requestGeneration || viewState.value.mode !== 'streaming') {
      nextStream.cancel()
      return false
    }
    detachStream(true)
    stream = nextStream
    return true
  }

  function acceptFrame(
    request: QueryWorkspaceRequest,
    frame: QueryFrame,
  ): QueryWorkspaceFrameResult {
    if (request.generation !== requestGeneration) return { kind: 'ignored' }
    const previous = viewState.value
    const next = reduceQueryFrame(previous, frame)
    if (next === previous && frame.reqId !== previous.requestId) {
      return { kind: 'ignored' }
    }
    viewState.value = next

    if (previous.mode !== 'error' && next.mode === 'error' && frame.kind !== 'error') {
      detachStream(true)
      return { kind: 'error' }
    }

    if (frame.kind === 'done' && next.mode === 'done' && next.map) {
      detachStream(false)
      const citations = queryAnswerEvidenceCitations(next.answer, next.map.evidence)
      const completed = appendCompletedQueryTurn(
        trace.value,
        request.identity,
        request.question,
        next.answer,
        citations.knownIds,
      )
      trace.value = completed
      const snapshots = appendQueryTurnPresentationSnapshot(
        traceSnapshots.value,
        next.map,
        next.evidence,
      )
      traceSnapshots.value = snapshots
      activeQuestion.value = ''
      selectedTurn.value = defaultQueryTurnSelection(completed.turns.length, false)
      return { kind: 'completed', turns: completed.turns, snapshots }
    }

    if (frame.kind === 'error') {
      detachStream(false)
      return { kind: 'error' }
    }
    return { kind: 'updated' }
  }

  function showValidationError(message: string): void {
    viewState.value = {
      ...idleQueryState(),
      mode: 'error',
      errorMessage: message,
    }
  }

  function applyRenderedAnswers(
    expectedSnapshots: QueryTurnPresentationSnapshot[],
    htmlByTurn: readonly string[],
  ): boolean {
    if (traceSnapshots.value !== expectedSnapshots) return false
    traceSnapshots.value = setQueryTurnAnswerHtml(expectedSnapshots, htmlByTurn)
    return true
  }

  return {
    question,
    viewState,
    trace,
    traceSnapshots,
    selectedTurn,
    activeQuestion,
    scope,
    beginRequest,
    attachStream,
    acceptFrame,
    showValidationError,
    applyRenderedAnswers,
    resetTrace,
    resetForClose,
    teardown,
  }
}
