import { ref, toRaw, type Ref } from 'vue'
import {
  QueryHistoryApiError,
  createQueryThread,
  deleteQueryThread,
  forkQueryThreadCurrent,
  getQueryThread,
  listQueryThreads,
  type QueryScopeSpec,
  type QueryThread,
  type QueryThreadListResponse,
  type QueryThreadSummary,
  type QueryThreadWarning,
} from './api'
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
  selectedQueryScope,
  startQueryRequest,
  startQueryTrace,
  type QueryScopeIdentity,
  type QueryViewState,
} from './queryState'

export type QueryScope = 'current' | 'selected'

export interface QueryWorkspaceStream {
  cancel(): void
}

export interface QueryHistoryClient {
  list(): Promise<QueryThreadListResponse>
  get(threadId: string): Promise<QueryThread>
  create(req: { scope: QueryScopeSpec; originalQuestion: string }): Promise<QueryThread>
  delete(threadId: string): Promise<void>
  forkCurrent(threadId: string): Promise<QueryThread>
}

export type QueryHistorySelectionResult =
  | {
      kind: 'selected'
      turns: QueryTrace['turns']
      snapshots: QueryTurnPresentationSnapshot[]
    }
  | { kind: 'missing' | 'ignored' | 'error' }

const DEFAULT_QUERY_HISTORY_CLIENT: QueryHistoryClient = {
  list: listQueryThreads,
  get: getQueryThread,
  create: createQueryThread,
  delete: deleteQueryThread,
  forkCurrent: forkQueryThreadCurrent,
}

/** Keep server RFC-3339 ordering deterministic even if a proxy/client returns
 * summaries out of order. Equal timestamps use the opaque id as a stable key. */
export function sortQueryThreadSummaries(
  summaries: readonly QueryThreadSummary[],
): QueryThreadSummary[] {
  return [...summaries].sort((left, right) =>
    right.updatedAt.localeCompare(left.updatedAt) || left.id.localeCompare(right.id),
  )
}

function summaryForThread(thread: QueryThread): QueryThreadSummary {
  return {
    id: thread.id,
    title: thread.title,
    updatedAt: thread.updatedAt,
    scope: thread.scope,
    turnCount: thread.turns.length,
    freshness: thread.freshness,
    ...(thread.staleReason ? { staleReason: thread.staleReason } : {}),
  }
}

function scopeKeyForThread(thread: QueryThread): string {
  if (thread.scope.kind === 'current') return `current:${thread.scope.paths[0]}`
  return selectedQueryScope(thread.scope.paths).scopeKey
}

function scopesEqual(left: QueryScopeSpec, right: QueryScopeSpec): boolean {
  return left.kind === right.kind
    && left.paths.length === right.paths.length
    && left.paths.every((path, index) => path === right.paths[index])
}

function validCurrentSourceFork(source: QueryThread, forked: QueryThread): boolean {
  return source.freshness === 'stale'
    && source.staleReason === 'source-changed'
    && forked.id !== source.id
    && forked.freshness === 'fresh'
    && forked.staleReason === undefined
    && forked.turns.length === 0
    && forked.sourceRevision !== source.sourceRevision
    && forked.originalQuestion === source.originalQuestion
    && scopesEqual(forked.scope, source.scope)
}

function traceForThread(thread: QueryThread): QueryTrace {
  return {
    scopeKey: scopeKeyForThread(thread),
    scopeRevision: thread.sourceRevision,
    originalQuestion: thread.originalQuestion,
    turns: thread.turns.map((turn) => ({
      question: turn.question,
      answer: turn.answer,
      codeEvidenceIds: [...turn.codeEvidenceIds],
    })),
  }
}

function snapshotsForThread(thread: QueryThread): QueryTurnPresentationSnapshot[] {
  return thread.turns.map((turn) => ({
    map: turn.map,
    evidence: turn.evidence
      ? { ...turn.evidence, sources: turn.evidence.sources.map((source) => ({ ...source })) }
      : null,
    answerHtml: '',
  }))
}

export interface QueryWorkspaceRequest {
  generation: number
  requestId: string
  threadId: string | null
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
  threadId: Ref<string | null>
  threadUpdatedAt: Ref<string | null>
  historySummaries: Ref<QueryThreadSummary[]>
  historyWarnings: Ref<QueryThreadWarning[]>
  historyLoading: Ref<boolean>
  historySelectingId: Ref<string | null>
  historyForkingId: Ref<string | null>
  historyError: Ref<string>
  selectedThread: Ref<QueryThread | null>
  loadProjectHistory(): Promise<boolean>
  replaceProject(): Promise<boolean>
  resetForProjectChange(): void
  selectHistoryThread(threadId: string): Promise<QueryHistorySelectionResult>
  deleteHistoryThread(threadId: string): Promise<boolean>
  forkSelectedThreadCurrent(): Promise<boolean>
  ensureRequestThread(
    request: QueryWorkspaceRequest,
    scope: QueryScopeSpec,
  ): Promise<string | null>
  canContinueSelectedThread(identity: QueryScopeIdentity): boolean
  handleScopeIdentityChange(clearQuestion?: boolean): void
  beginRequest(identity: QueryScopeIdentity): QueryWorkspaceRequest | null
  bindThread(
    request: QueryWorkspaceRequest,
    nextThreadId: string,
    updatedAt: string,
  ): boolean
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
 * Explicit close clears only the active projection; project history survives
 * panel moves while project replacement invalidates every older async result.
 */
export function createQueryWorkspace(
  historyClient: QueryHistoryClient = DEFAULT_QUERY_HISTORY_CLIENT,
): QueryWorkspaceController {
  const question = ref('')
  const viewState = ref(idleQueryState())
  const trace = ref<QueryTrace | null>(null)
  const traceSnapshots = ref<QueryTurnPresentationSnapshot[]>([])
  const selectedTurn = ref<QueryTurnSelection>(null)
  const activeQuestion = ref('')
  const scope = ref<QueryScope>('current')
  const threadId = ref<string | null>(null)
  const threadUpdatedAt = ref<string | null>(null)
  const historySummaries = ref<QueryThreadSummary[]>([])
  const historyWarnings = ref<QueryThreadWarning[]>([])
  const historyLoading = ref(false)
  const historySelectingId = ref<string | null>(null)
  const historyForkingId = ref<string | null>(null)
  const historyError = ref('')
  const selectedThread = ref<QueryThread | null>(null)

  let stream: QueryWorkspaceStream | null = null
  let requestGeneration = 0
  let requestSequence = 0
  let projectGeneration = 0
  let historyRequestSequence = 0
  let historySelectionSequence = 0

  function detachStream(cancel: boolean): void {
    const active = stream
    stream = null
    if (cancel) active?.cancel()
  }

  function teardown(): void {
    requestGeneration++
    detachStream(true)
  }

  function upsertHistorySummary(next: QueryThreadSummary): void {
    historySummaries.value = sortQueryThreadSummaries([
      next,
      ...historySummaries.value.filter((item) => item.id !== next.id),
    ])
  }

  function restoreThreadProjection(thread: QueryThread, preserveRendered = false): {
    turns: QueryTrace['turns']
    snapshots: QueryTurnPresentationSnapshot[]
  } {
    const restoredTrace = traceForThread(thread)
    const previousSnapshots = preserveRendered ? traceSnapshots.value : []
    const snapshots = snapshotsForThread(thread).map((snapshot, index) => ({
      ...snapshot,
      answerHtml: previousSnapshots[index]?.answerHtml ?? '',
    }))
    selectedThread.value = thread
    threadId.value = thread.id
    threadUpdatedAt.value = thread.updatedAt
    trace.value = restoredTrace
    traceSnapshots.value = snapshots
    selectedTurn.value = defaultQueryTurnSelection(restoredTrace.turns.length, false)
    activeQuestion.value = ''
    viewState.value = idleQueryState()
    return { turns: restoredTrace.turns, snapshots }
  }

  async function loadProjectHistory(): Promise<boolean> {
    const generation = projectGeneration
    const sequence = ++historyRequestSequence
    historyLoading.value = true
    historyError.value = ''
    try {
      const response = await historyClient.list()
      if (generation !== projectGeneration || sequence !== historyRequestSequence) {
        return false
      }
      historySummaries.value = sortQueryThreadSummaries(response.threads)
      historyWarnings.value = response.warnings.map((warning) => ({ ...warning }))
      const selectedId = selectedThread.value?.id
      if (selectedId && !historySummaries.value.some((item) => item.id === selectedId)) {
        resetTrace(false)
        historyError.value = '当前追问线程已删除，已返回项目历史列表'
      }
      return true
    } catch (error) {
      if (generation !== projectGeneration || sequence !== historyRequestSequence) {
        return false
      }
      historyError.value = error instanceof Error ? error.message : '加载项目追问历史失败'
      return false
    } finally {
      if (generation === projectGeneration && sequence === historyRequestSequence) {
        historyLoading.value = false
      }
    }
  }

  function resetForProjectChange(): void {
    projectGeneration++
    historyRequestSequence++
    historySelectionSequence++
    historyLoading.value = false
    historySelectingId.value = null
    historyForkingId.value = null
    historyError.value = ''
    historyWarnings.value = []
    historySummaries.value = []
    resetTrace()
    scope.value = 'current'
  }

  async function replaceProject(): Promise<boolean> {
    resetForProjectChange()
    return loadProjectHistory()
  }

  async function selectHistoryThread(
    nextThreadId: string,
  ): Promise<QueryHistorySelectionResult> {
    if (!nextThreadId) return { kind: 'ignored' }
    const generation = projectGeneration
    const sequence = ++historySelectionSequence
    historyForkingId.value = null
    historySelectingId.value = nextThreadId
    historyError.value = ''
    teardown()
    viewState.value = idleQueryState()
    activeQuestion.value = ''
    try {
      const thread = await historyClient.get(nextThreadId)
      if (generation !== projectGeneration || sequence !== historySelectionSequence) {
        return { kind: 'ignored' }
      }
      scope.value = thread.scope.kind
      const restored = restoreThreadProjection(thread)
      upsertHistorySummary(summaryForThread(thread))
      question.value = ''
      return { kind: 'selected', ...restored }
    } catch (error) {
      if (generation !== projectGeneration || sequence !== historySelectionSequence) {
        return { kind: 'ignored' }
      }
      if (error instanceof QueryHistoryApiError && error.status === 404) {
        historySummaries.value = historySummaries.value.filter(
          (item) => item.id !== nextThreadId,
        )
        if (selectedThread.value?.id === nextThreadId) resetTrace(false)
        historyError.value = '该追问线程已删除，已从项目历史移除'
        return { kind: 'missing' }
      }
      historyError.value = error instanceof Error ? error.message : '读取追问线程失败'
      return { kind: 'error' }
    } finally {
      if (generation === projectGeneration && sequence === historySelectionSequence) {
        historySelectingId.value = null
      }
    }
  }

  async function deleteHistoryThread(deletedThreadId: string): Promise<boolean> {
    if (!deletedThreadId) return false
    const generation = projectGeneration
    historySelectionSequence++
    historySelectingId.value = null
    historyForkingId.value = null
    const deletingSelected = selectedThread.value?.id === deletedThreadId
    if (deletingSelected) {
      teardown()
      const thread = selectedThread.value
      if (thread) restoreThreadProjection(thread, true)
    }
    historyError.value = ''
    try {
      await historyClient.delete(deletedThreadId)
    } catch (error) {
      if (generation !== projectGeneration) return false
      if (!(error instanceof QueryHistoryApiError) || error.status !== 404) {
        historyError.value = error instanceof Error ? error.message : '删除追问线程失败'
        return false
      }
    }
    if (generation !== projectGeneration) return false
    historySummaries.value = historySummaries.value.filter(
      (item) => item.id !== deletedThreadId,
    )
    if (deletingSelected) resetTrace(false)
    return true
  }

  async function forkSelectedThreadCurrent(): Promise<boolean> {
    const source = selectedThread.value
    if (
      !source
      || source.freshness !== 'stale'
      || source.staleReason !== 'source-changed'
    ) {
      return false
    }
    const generation = projectGeneration
    const sequence = ++historySelectionSequence
    historySelectingId.value = null
    historyForkingId.value = source.id
    historyError.value = ''
    teardown()
    try {
      const forked = await historyClient.forkCurrent(source.id)
      if (generation !== projectGeneration || sequence !== historySelectionSequence) {
        return false
      }
      if (!validCurrentSourceFork(source, forked)) {
        historyError.value = '服务端返回的当前源码新线程不满足版本隔离契约'
        return false
      }
      scope.value = forked.scope.kind
      restoreThreadProjection(forked)
      upsertHistorySummary(summaryForThread(forked))
      question.value = forked.originalQuestion
      return true
    } catch (error) {
      if (generation !== projectGeneration || sequence !== historySelectionSequence) {
        return false
      }
      historyError.value = error instanceof Error
        ? error.message
        : '基于当前源码新建追问失败'
      return false
    } finally {
      if (generation === projectGeneration && sequence === historySelectionSequence) {
        historyForkingId.value = null
      }
    }
  }

  function canContinueSelectedThread(identity: QueryScopeIdentity): boolean {
    const thread = selectedThread.value
    return Boolean(
      thread
      && thread.freshness === 'fresh'
      && identity.scopeRevision
      && scopeKeyForThread(thread) === identity.scopeKey,
    )
  }

  function handleScopeIdentityChange(clearQuestion = true): void {
    const hadActiveProjection = Boolean(activeQuestion.value)
      || viewState.value.mode === 'streaming'
      || viewState.value.mode === 'error'
    teardown()
    const thread = selectedThread.value
    if (thread && hadActiveProjection) restoreThreadProjection(thread, true)
    else if (thread) {
      activeQuestion.value = ''
      viewState.value = idleQueryState()
      selectedTurn.value = defaultQueryTurnSelection(thread.turns.length, false)
    }
    else {
      trace.value = null
      const presentation = resetQueryTurnPresentation()
      selectedTurn.value = presentation.selection
      traceSnapshots.value = presentation.snapshots
      activeQuestion.value = ''
      viewState.value = idleQueryState()
      threadId.value = null
      threadUpdatedAt.value = null
    }
    if (clearQuestion) question.value = ''
  }

  function resetTrace(clearQuestion = true): void {
    historySelectionSequence++
    historySelectingId.value = null
    historyForkingId.value = null
    teardown()
    trace.value = null
    const presentation = resetQueryTurnPresentation()
    selectedTurn.value = presentation.selection
    traceSnapshots.value = presentation.snapshots
    activeQuestion.value = ''
    viewState.value = idleQueryState()
    threadId.value = null
    threadUpdatedAt.value = null
    selectedThread.value = null
    if (clearQuestion) question.value = ''
  }

  function resetForClose(): void {
    resetTrace()
    scope.value = 'current'
  }

  function beginRequest(identity: QueryScopeIdentity): QueryWorkspaceRequest | null {
    const askedQuestion = question.value.trim()
    if (!askedQuestion) return null

    if (selectedThread.value) {
      if (!canContinueSelectedThread(identity) || !trace.value) return null
      trace.value = {
        ...trace.value,
        scopeKey: identity.scopeKey,
        scopeRevision: identity.scopeRevision,
      }
    }

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
      threadId: threadId.value,
      identity: requestIdentity,
      question: askedQuestion,
      trace: requestTrace,
    }
    viewState.value = startQueryRequest(requestId)
    activeQuestion.value = askedQuestion
    question.value = ''
    return request
  }

  function bindThread(
    request: QueryWorkspaceRequest,
    nextThreadId: string,
    updatedAt: string,
  ): boolean {
    if (
      request.generation !== requestGeneration ||
      viewState.value.mode !== 'streaming' ||
      !nextThreadId ||
      (threadId.value !== null && threadId.value !== nextThreadId) ||
      (request.threadId !== null && request.threadId !== nextThreadId)
    ) {
      return false
    }
    request.threadId = nextThreadId
    threadId.value = nextThreadId
    threadUpdatedAt.value = updatedAt
    return true
  }

  async function ensureRequestThread(
    request: QueryWorkspaceRequest,
    threadScope: QueryScopeSpec,
  ): Promise<string | null> {
    if (request.threadId) return request.threadId
    const generation = projectGeneration
    const created = await historyClient.create({
      scope: threadScope,
      originalQuestion: request.question,
    })
    if (generation !== projectGeneration) return null
    if (!bindThread(request, created.id, created.updatedAt)) return null
    selectedThread.value = created
    upsertHistorySummary(summaryForThread(created))
    return created.id
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
    if (frame.reqId !== previous.requestId) return { kind: 'ignored' }
    if (
      frame.kind === 'done' &&
      (!request.threadId || frame.threadId !== request.threadId)
    ) {
      viewState.value = reduceQueryFrame(previous, {
        kind: 'error',
        reqId: request.requestId,
        message: '追问线程身份不匹配，完整轮次未接收',
      })
      detachStream(true)
      return { kind: 'error' }
    }
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
      threadUpdatedAt.value = frame.updatedAt
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
      const durable = selectedThread.value
      if (durable?.id === frame.threadId) {
        const updatedThread: QueryThread = {
          ...durable,
          updatedAt: frame.updatedAt,
          turns: [
            ...durable.turns,
            {
              question: request.question,
              answer: next.answer,
              map: next.map,
              evidence: next.evidence
                ? {
                    ...next.evidence,
                    sources: next.evidence.sources.map((source) => ({ ...source })),
                  }
                : null,
              codeEvidenceIds: [...citations.knownIds],
              completedAt: frame.updatedAt,
            },
          ],
        }
        selectedThread.value = updatedThread
        upsertHistorySummary(summaryForThread(updatedThread))
      } else {
        const existing = historySummaries.value.find((item) => item.id === frame.threadId)
        if (existing) {
          upsertHistorySummary({
            ...existing,
            updatedAt: frame.updatedAt,
            turnCount: completed.turns.length,
          })
        }
      }
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
    // Vue deep-wraps arrays assigned to a ref. Compare their raw identities so
    // the generation guard accepts the exact array returned by the controller.
    if (toRaw(traceSnapshots.value) !== toRaw(expectedSnapshots)) return false
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
    threadId,
    threadUpdatedAt,
    historySummaries,
    historyWarnings,
    historyLoading,
    historySelectingId,
    historyForkingId,
    historyError,
    selectedThread,
    loadProjectHistory,
    replaceProject,
    resetForProjectChange,
    selectHistoryThread,
    deleteHistoryThread,
    forkSelectedThreadCurrent,
    ensureRequestThread,
    canContinueSelectedThread,
    handleScopeIdentityChange,
    beginRequest,
    bindThread,
    attachStream,
    acceptFrame,
    showValidationError,
    applyRenderedAnswers,
    resetTrace,
    resetForClose,
    teardown,
  }
}
