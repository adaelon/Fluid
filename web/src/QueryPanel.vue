<script setup lang="ts">
// S10b: the follow-up query terminal, docked as a bottom panel (ADR-0015/0016
// PENDING resolved — out of the right edge so it never fights trailing line
// notes). Asks the current file or selected file set a free-form question and
// projects the backend QueryMap before evidence/token deltas from the matching
// query WebSocket, and turns only known E# citations into source navigation.
// Switching files or scope cancels an in-flight request but keeps a selected
// durable thread readable; only a matching fresh scope can continue it.
import { computed, ref, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'
import {
  streamQuery,
  streamQueryFiles,
  type QueryScopeSpec,
} from './api'
import type { CodeEvidenceRef, QueryFrame, QueryTrace } from './ghostTypes'
import { EMPTY_QUERY_CONTEXT, type QueryContext } from './queryContext'
import CodeEvidencePeek from './CodeEvidencePeek.vue'
import QueryMapView from './QueryMapView.vue'
import {
  queryAnswerEvidenceCitations,
  queryCodeEvidenceNavigationEnabled,
  queryStaleReasonMessage,
} from './queryEvidence'
import {
  QUERY_PEEK_STORAGE_KEY,
  clampCodePeekWidth,
  codePeekWidthBounds,
  codePeekWidthFromPointer,
  loadCodePeekWidth,
  type QueryPresentation,
} from './queryLayout'
import {
  activeQueryTurnSelection,
  completedQueryTurnSelection,
  defaultQueryTurnSelection,
  normalizeQueryTurnSelection,
  queryTurnEvidenceById,
  queryTurnSelectionFromKey,
  queryTurnSelectionKey,
  type QueryTurnPresentationSnapshot,
  type QueryTurnSelection,
} from './queryPresentation'
import {
  currentQueryScope,
  selectedQueryScope,
  type QueryEvidenceState,
  type QueryScopeIdentity,
} from './queryState'
import type {
  QueryWorkspaceController,
  QueryWorkspaceRequest,
} from './queryWorkspace'
// S11-lazy: markdown-it / DOMPurify / KaTeX (+ its CSS) are heavy and only needed
// once an answer finishes streaming, so they are dynamically import()ed inside
// renderAnswer() rather than at module top — Rollup splits them into async chunks
// kept out of the first-paint bundle. Behavior is unchanged from ADR-0008.

const props = withDefaults(
  defineProps<{
    workspace: QueryWorkspaceController
    path: string | null
    ctx?: QueryContext
    selectionMode?: boolean
    selectedCount?: number
    selectedPaths?: string[]
    allowWeb?: boolean
    presentation?: QueryPresentation
  }>(),
  {
    ctx: () => EMPTY_QUERY_CONTEXT,
    selectionMode: false,
    selectedCount: 0,
    selectedPaths: () => [],
    allowWeb: true,
    presentation: 'dock',
  },
)
// Visibility is owned by the parent (App) via the status-bar toggle — default
// hidden so the bottom space goes back to the code area; the panel only asks to
// close itself, it never decides whether it is mounted.
const emit = defineEmits<{
  close: []
  toggleSelectionMode: []
  clearSelected: []
  openEvidence: [CodeEvidenceRef]
  restoreScope: [QueryScopeSpec]
  maximize: []
  restore: []
  moveSidebar: []
  moveDock: []
}>()

const workspace = props.workspace
const {
  question,
  viewState,
  trace,
  traceSnapshots,
  selectedTurn,
  activeQuestion,
  scope,
  historySummaries,
  historyWarnings,
  historyLoading,
  historySelectingId,
  historyForkingId,
  historyError,
  selectedThread,
} = workspace
const renderedEl = ref<HTMLElement | null>(null)
const codePeekTarget = ref<CodeEvidenceRef | null>(null)
type QuerySidebarPane = 'history' | 'thread'
const querySidebarPane = ref<QuerySidebarPane>('history')
const codePeekViewportWidth = ref(window.innerWidth)
let codePeekPreferredWidth = loadCodePeekWidth(
  localStorage.getItem(QUERY_PEEK_STORAGE_KEY),
  codePeekViewportWidth.value,
)
const codePeekWidth = ref(codePeekPreferredWidth)
const codePeekBounds = computed(() => codePeekWidthBounds(codePeekViewportWidth.value))
const codePeekDragging = ref(false)
let answerRenderGeneration = 0
let mathRenderGeneration = 0

interface CodePeekResize {
  pointerId: number
  startX: number
  startWidth: number
  startPreferredWidth: number
}

let codePeekResize: CodePeekResize | null = null

const answer = computed(() => viewState.value.answer)
const completedTurns = computed(() => trace.value?.turns ?? [])
const streaming = computed(() => viewState.value.mode === 'streaming')
const errorMsg = computed(() => viewState.value.errorMessage)
const evidenceState = computed(() => viewState.value.evidence)
const hasActiveTurn = computed(
  () => Boolean(activeQuestion.value) && (streaming.value || Boolean(errorMsg.value)),
)
const activeUnknownEvidenceIds = computed(() => {
  const map = viewState.value.map
  return map ? queryAnswerEvidenceCitations(answer.value, map.evidence).unknownIds : []
})
const traceUnknownEvidenceIds = computed(() =>
  completedTurns.value.map((turn, index) => {
    const map = traceSnapshots.value[index]?.map
    return map ? queryAnswerEvidenceCitations(turn.answer, map.evidence).unknownIds : []
  }),
)

function evidenceDisplayFor(evidence: QueryEvidenceState | null) {
  switch (evidence?.status) {
    case 'project-source':
      return { label: '项目源码', tone: 'project' }
    case 'web-cited':
      return { label: '网页有来源', tone: 'cited' }
    case 'web-uncited':
      return { label: '联网无来源', tone: 'uncited' }
    default:
      return { label: '未核验', tone: 'unverified' }
  }
}

const evidenceDisplay = computed(() => evidenceDisplayFor(evidenceState.value))
const statusText = computed(() => {
  switch (viewState.value.phase) {
    case 'connecting':
      return '正在连接追问服务…'
    case 'planning-source':
      return '正在规划相关源码证据…'
    case 'planning-web':
      return '正在规划联网检索…'
    case 'searching-web':
      return '正在检索网页…'
    case 'answering':
      return '正在生成回答…'
    case 'fallback':
      return viewState.value.statusMessage || '联网失败，正在改用本地上下文…'
    default:
      return ''
  }
})

const selectedReady = computed(() => props.selectedCount >= 2)
const currentOrientationId = computed(() =>
  props.path && props.ctx.filePath === props.path ? props.ctx.orientationId : '',
)
const scopeIdentity = computed<QueryScopeIdentity>(() =>
  scope.value === 'current'
    ? currentQueryScope(props.path ?? '', currentOrientationId.value)
    : selectedQueryScope(props.selectedPaths),
)
const selectedLabel = computed(() => `已选文件(${props.selectedCount})`)
const selectedScopeHint = computed(() => {
  if (props.selectedCount === 0) return '选择至少 2 个文件后可切换到文件集追问'
  if (props.selectedCount === 1) return '再选择 1 个文件后可进行文件集追问'
  return `已选择 ${props.selectedCount} 个文件`
})
const selectedThreadScopeMatches = computed(() => {
  const thread = selectedThread.value
  if (!thread) return true
  if (thread.scope.kind === 'current') return thread.scope.paths[0] === props.path
  const expected = Array.from(new Set(thread.scope.paths)).sort()
  const actual = Array.from(new Set(props.selectedPaths)).sort()
  return JSON.stringify(expected) === JSON.stringify(actual)
})
const selectedThreadCanContinue = computed(() =>
  workspace.canContinueSelectedThread(scopeIdentity.value),
)
const historyNeedsScopeRestore = computed(() =>
  selectedThread.value?.freshness === 'fresh' && !selectedThreadScopeMatches.value,
)
const codeEvidenceEnabled = computed(() =>
  queryCodeEvidenceNavigationEnabled(selectedThread.value?.freshness),
)
const historyStaleMessage = computed(() =>
  queryStaleReasonMessage(selectedThread.value?.staleReason),
)
const historyReadOnly = computed(() =>
  Boolean(selectedThread.value && !selectedThreadCanContinue.value),
)
const canAsk = computed(() => {
  if (!question.value.trim() || streaming.value) return false
  if (selectedThread.value && !selectedThreadCanContinue.value) return false
  if (scope.value === 'current') return Boolean(props.path && currentOrientationId.value)
  return selectedReady.value
})
const focusedSelection = computed(() =>
  normalizeQueryTurnSelection(
    selectedTurn.value,
    completedTurns.value.length,
    hasActiveTurn.value,
  ),
)
const focusedSelectionKey = computed(() => queryTurnSelectionKey(focusedSelection.value))
const focusedIsActive = computed(() => focusedSelection.value?.kind === 'active')
const focusedCompletedIndex = computed(() =>
  focusedSelection.value?.kind === 'completed' ? focusedSelection.value.index : null,
)
const focusedCompletedTurn = computed(() => {
  const index = focusedCompletedIndex.value
  return index === null ? null : completedTurns.value[index] ?? null
})
const focusedSnapshot = computed(() => {
  const index = focusedCompletedIndex.value
  return index === null ? null : traceSnapshots.value[index] ?? null
})
const focusedQuestion = computed(() =>
  focusedIsActive.value ? activeQuestion.value : focusedCompletedTurn.value?.question ?? '',
)
const focusedAnswer = computed(() =>
  focusedIsActive.value ? answer.value : focusedCompletedTurn.value?.answer ?? '',
)
const focusedAnswerHtml = computed(() =>
  focusedIsActive.value ? '' : focusedSnapshot.value?.answerHtml ?? '',
)
const focusedMap = computed(() =>
  focusedIsActive.value ? viewState.value.map : focusedSnapshot.value?.map ?? null,
)
const focusedEvidence = computed(() =>
  focusedIsActive.value ? evidenceState.value : focusedSnapshot.value?.evidence ?? null,
)
const focusedEvidenceDisplay = computed(() => evidenceDisplayFor(focusedEvidence.value))
const focusedUnknownEvidenceIds = computed(() => {
  if (focusedIsActive.value) return activeUnknownEvidenceIds.value
  const index = focusedCompletedIndex.value
  return index === null ? [] : traceUnknownEvidenceIds.value[index] ?? []
})
const hasFocusHistory = computed(() => completedTurns.value.length > 0 || hasActiveTurn.value)

function historyScopeLabel(threadScope: QueryScopeSpec): string {
  return threadScope.kind === 'current'
    ? threadScope.paths[0]
    : `${threadScope.paths.length} 个文件`
}

function historyOptionLabel(index: number): string {
  const item = historySummaries.value[index]
  if (!item) return ''
  const freshness = item.freshness === 'fresh'
    ? '源码一致'
    : item.staleReason === 'source-missing'
      ? '范围文件缺失 · 只读'
      : '源码已变更 · 只读'
  return `${item.title} · ${item.turnCount} 轮 · ${historyScopeLabel(item.scope)} · ${freshness}`
}

function resetTrace(clearQuestion = true) {
  workspace.resetTrace(clearQuestion)
  answerRenderGeneration++
  mathRenderGeneration++
  closeCodePeek()
}

function closeCodePeek(): void {
  codePeekTarget.value = null
}

function startCodePeekResize(e: PointerEvent): void {
  if (e.button !== 0 || codePeekResize) return
  e.preventDefault()
  codePeekResize = {
    pointerId: e.pointerId,
    startX: e.clientX,
    startWidth: codePeekWidth.value,
    startPreferredWidth: codePeekPreferredWidth,
  }
  codePeekDragging.value = true
  const handle = e.currentTarget as HTMLElement
  handle.setPointerCapture(e.pointerId)
}

function moveCodePeekResize(e: PointerEvent): void {
  const resize = codePeekResize
  if (!resize || resize.pointerId !== e.pointerId) return
  const width = codePeekWidthFromPointer(
    resize.startWidth,
    resize.startX,
    e.clientX,
    codePeekViewportWidth.value,
  )
  codePeekPreferredWidth = width
  codePeekWidth.value = width
}

function finishCodePeekResize(e: PointerEvent, persist: boolean): void {
  const resize = codePeekResize
  if (!resize || resize.pointerId !== e.pointerId) return
  codePeekResize = null
  codePeekDragging.value = false
  if (persist) {
    codePeekPreferredWidth = codePeekWidth.value
    localStorage.setItem(QUERY_PEEK_STORAGE_KEY, String(codePeekWidth.value))
  } else {
    codePeekPreferredWidth = resize.startPreferredWidth
    codePeekWidth.value = clampCodePeekWidth(
      resize.startPreferredWidth,
      codePeekViewportWidth.value,
    )
  }
  const handle = e.currentTarget as HTMLElement
  if (handle.hasPointerCapture(e.pointerId)) handle.releasePointerCapture(e.pointerId)
}

function endCodePeekResize(e: PointerEvent): void {
  finishCodePeekResize(e, true)
}

function cancelCodePeekResize(e: PointerEvent): void {
  finishCodePeekResize(e, false)
}

function loseCodePeekPointer(e: PointerEvent): void {
  const resize = codePeekResize
  if (!resize || resize.pointerId !== e.pointerId) return
  codePeekResize = null
  codePeekDragging.value = false
  codePeekPreferredWidth = resize.startPreferredWidth
  codePeekWidth.value = clampCodePeekWidth(
    resize.startPreferredWidth,
    codePeekViewportWidth.value,
  )
}

function resizeCodePeekForViewport(): void {
  codePeekViewportWidth.value = window.innerWidth
  codePeekWidth.value = clampCodePeekWidth(
    codePeekPreferredWidth,
    codePeekViewportWidth.value,
  )
}

// File, scope, selected-set or source-revision changes all end the current trace.
watch(
  () => [
    props.path ?? '',
    scope.value,
    scopeIdentity.value.scopeKey,
    scopeIdentity.value.scopeRevision,
  ] as const,
  (next, previous) => {
    workspace.handleScopeIdentityChange(next[0] !== previous[0])
    answerRenderGeneration++
    mathRenderGeneration++
    closeCodePeek()
  },
)

async function renderVisibleMath(): Promise<void> {
  const generation = ++mathRenderGeneration
  await nextTick()
  const element = renderedEl.value
  if (!element?.querySelector('.query-answer-md')) return
  const [{ default: renderMathInElement }] = await Promise.all([
    import('katex/contrib/auto-render'),
    import('katex/dist/katex.min.css'),
  ])
  if (generation !== mathRenderGeneration || renderedEl.value !== element) return
  renderMathInElement(element, {
    delimiters: [
      { left: '$$', right: '$$', display: true },
      { left: '$', right: '$', display: false },
      { left: '\\[', right: '\\]', display: true },
      { left: '\\(', right: '\\)', display: false },
    ],
    throwOnError: false,
  })
}

// On `done`, render every completed Markdown answer once into its presentation
// snapshot. KaTeX is applied only to the currently mounted dock/focus view.
async function renderTraceAnswers(
  turns: QueryTrace['turns'],
  snapshots: QueryTurnPresentationSnapshot[],
): Promise<void> {
  const generation = ++answerRenderGeneration
  // Pull the render libs on demand (S11-lazy). The CSS import is a side effect
  // performed by renderVisibleMath when a rendered answer is actually mounted.
  const [{ renderQueryMarkdown }, { default: DOMPurify }] = await Promise.all([
    import('./render/markdown'),
    import('dompurify'),
  ])
  if (generation !== answerRenderGeneration) return
  const htmlByTurn = turns.map((turn, index) => {
    const evidence = codeEvidenceEnabled.value
      ? snapshots[index]?.map.evidence ?? []
      : []
    return DOMPurify.sanitize(renderQueryMarkdown(turn.answer, evidence))
  })
  if (generation !== answerRenderGeneration) return
  if (!workspace.applyRenderedAnswers(snapshots, htmlByTurn)) return
  await renderVisibleMath()
}

function acceptFrame(request: QueryWorkspaceRequest, frame: QueryFrame) {
  const result = workspace.acceptFrame(request, frame)
  if (result.kind === 'completed') {
    void renderTraceAnswers(result.turns, result.snapshots)
  }
}

function openEvidence(reference: CodeEvidenceRef) {
  if (!codeEvidenceEnabled.value) return
  if (props.presentation === 'focus') {
    codePeekTarget.value = { ...reference }
    return
  }
  emit('openEvidence', reference)
}

function handleEscape(): boolean {
  if (props.presentation !== 'focus' || !codePeekTarget.value) return false
  closeCodePeek()
  return true
}

defineExpose({ handleEscape })

function onAnswerClick(event: MouseEvent) {
  const target = event.target
  if (!(target instanceof Element)) return
  const anchor = target.closest<HTMLAnchorElement>('a.query-code-evidence-link')
  const href = anchor?.getAttribute('href') ?? ''
  const prefix = '#fluid-evidence-'
  if (!anchor || !href.startsWith(prefix)) return
  if (!codeEvidenceEnabled.value) {
    event.preventDefault()
    return
  }

  const turn = anchor.closest<HTMLElement>('[data-query-turn-index]')
  const index = Number(turn?.dataset.queryTurnIndex)
  if (!Number.isInteger(index)) return
  const reference = queryTurnEvidenceById(
    traceSnapshots.value,
    index,
    href.slice(prefix.length),
  )
  if (!reference) return
  event.preventDefault()
  openEvidence(reference)
}

function selectFocusedTurn(selection: QueryTurnSelection): void {
  if (!selection) return
  if (queryTurnSelectionKey(selection) !== focusedSelectionKey.value) closeCodePeek()
  selectedTurn.value = selection
}

function selectCompletedTurn(index: number): void {
  selectFocusedTurn(completedQueryTurnSelection(index, completedTurns.value.length))
}

function selectActiveTurn(): void {
  if (hasActiveTurn.value) selectFocusedTurn(activeQueryTurnSelection())
}

function selectTurnFromPicker(event: Event): void {
  const value = (event.currentTarget as HTMLSelectElement).value
  selectFocusedTurn(
    queryTurnSelectionFromKey(
      value,
      completedTurns.value.length,
      hasActiveTurn.value,
    ),
  )
}

function newTrace() {
  resetTrace()
  querySidebarPane.value = 'thread'
}

async function selectHistory(nextThreadId: string): Promise<void> {
  if (!nextThreadId) {
    newTrace()
    return
  }
  if (selectedThread.value?.id === nextThreadId) {
    querySidebarPane.value = 'thread'
    return
  }
  closeCodePeek()
  const result = await workspace.selectHistoryThread(nextThreadId)
  if (result.kind === 'selected') {
    await renderTraceAnswers(result.turns, result.snapshots)
    querySidebarPane.value = 'thread'
  }
}

async function selectHistoryFromPicker(event: Event): Promise<void> {
  await selectHistory((event.currentTarget as HTMLSelectElement).value)
}

async function selectSidebarHistory(threadId: string): Promise<void> {
  await selectHistory(threadId)
}

async function deleteSelectedHistory(): Promise<void> {
  const selectedId = selectedThread.value?.id
  if (!selectedId) return
  closeCodePeek()
  if (await workspace.deleteHistoryThread(selectedId)) {
    querySidebarPane.value = 'history'
  }
}

async function forkSelectedHistory(): Promise<void> {
  closeCodePeek()
  await workspace.forkSelectedThreadCurrent()
}

async function ask() {
  const q = question.value.trim()
  if (!q || streaming.value) return
  if (scope.value === 'selected') {
    if (!selectedReady.value) {
      workspace.showValidationError(selectedScopeHint.value)
      return
    }
    const [firstPath, secondPath, ...remainingPaths] = props.selectedPaths
    if (!firstPath || !secondPath) return
    const filePaths = [firstPath, secondPath, ...remainingPaths] as [
      string,
      string,
      ...string[],
    ]
    closeCodePeek()
    const request = workspace.beginRequest(scopeIdentity.value)
    if (!request) return
    let threadId: string | null
    try {
      threadId = await workspace.ensureRequestThread(request, {
        kind: 'selected',
        paths: filePaths,
      })
    }
    catch (error) {
      acceptFrame(request, {
        kind: 'error',
        reqId: request.requestId,
        message: error instanceof Error ? error.message : '创建追问线程失败',
      })
      return
    }
    if (!threadId) return
    const nextStream = streamQueryFiles(
      {
        reqId: request.requestId,
        threadId,
        filePaths,
        question: request.question,
        allowWeb: props.allowWeb,
      },
      {
        onFrame: (frame) => acceptFrame(request, frame),
      },
    )
    workspace.attachStream(request.generation, nextStream)
    return
  }
  const filePath = props.path
  const orientationId = currentOrientationId.value
  if (!filePath || !orientationId) return
  closeCodePeek()
  const request = workspace.beginRequest(scopeIdentity.value)
  if (!request) return
  let threadId: string | null
  try {
    threadId = await workspace.ensureRequestThread(request, {
      kind: 'current',
      paths: [filePath],
    })
  }
  catch (error) {
    acceptFrame(request, {
      kind: 'error',
      reqId: request.requestId,
      message: error instanceof Error ? error.message : '创建追问线程失败',
    })
    return
  }
  if (!threadId) return
  const nextStream = streamQuery(
    {
      reqId: request.requestId,
      threadId,
      filePath,
      orientationId,
      question: request.question,
      roster: props.ctx.roster,
      rosterSpans: props.ctx.rosterSpans,
      capsules: props.ctx.capsules,
      allowWeb: props.allowWeb,
    },
    {
      onFrame: (frame) => acceptFrame(request, frame),
    },
  )
  workspace.attachStream(request.generation, nextStream)
}

watch(
  () => props.presentation,
  (presentation) => {
    if (presentation === 'focus') {
      selectedTurn.value = defaultQueryTurnSelection(
        completedTurns.value.length,
        hasActiveTurn.value,
      )
      resizeCodePeekForViewport()
    }
    else {
      codePeekResize = null
      codePeekDragging.value = false
      closeCodePeek()
      if (
        presentation === 'sidebar'
        && (selectedThread.value || trace.value || activeQuestion.value || question.value)
      ) {
        querySidebarPane.value = 'thread'
      }
    }
  },
)

watch(codeEvidenceEnabled, (enabled) => {
  if (!enabled) closeCodePeek()
})

watch(
  () => [props.presentation, focusedSelectionKey.value] as const,
  () => void renderVisibleMath(),
  { flush: 'post' },
)

onMounted(() => window.addEventListener('resize', resizeCodePeekForViewport))
onBeforeUnmount(() => {
  window.removeEventListener('resize', resizeCodePeekForViewport)
  answerRenderGeneration++
  mathRenderGeneration++
})
</script>

<template>
  <section
    class="query-panel"
    :class="{
      disabled: !path,
      sidebar: presentation === 'sidebar',
      focus: presentation === 'focus',
      'peek-open': presentation === 'focus' && codePeekTarget,
      'peek-dragging': codePeekDragging,
    }"
    :data-presentation="presentation"
    data-testid="query-panel"
  >
    <header class="query-head">
      <div v-if="presentation === 'sidebar'" class="query-head-left query-sidebar-head-left">
        <button
          v-if="querySidebarPane === 'thread'"
          class="query-sidebar-back"
          type="button"
          aria-label="返回项目追问历史"
          @click="querySidebarPane = 'history'"
        >
          ←
        </button>
        <span class="query-title">
          {{ querySidebarPane === 'history' ? '追问器 · 项目历史' : '追问器 · 线程详情' }}
        </span>
      </div>
      <div v-else class="query-head-left">
        <span class="query-title">
          追问器{{ path ? '' : ' · 未激活' }}{{ presentation === 'focus' ? ' · 专注' : '' }}
        </span>
        <div class="query-scope" role="tablist" aria-label="追问范围">
          <button
            class="query-scope-btn"
            :class="{ active: scope === 'current' }"
            type="button"
            role="tab"
            :aria-selected="scope === 'current'"
            @click="scope = 'current'"
          >
            当前文件
          </button>
          <button
            class="query-scope-btn"
            :class="{ active: scope === 'selected' }"
            type="button"
            role="tab"
            :aria-selected="scope === 'selected'"
            @click="scope = 'selected'"
          >
            {{ selectedLabel }}
          </button>
        </div>
        <label class="query-history-picker">
          <span class="query-history-picker-label">项目历史</span>
          <select
            data-testid="query-history-picker"
            :value="selectedThread?.id ?? ''"
            :disabled="historyLoading || Boolean(historySelectingId) || Boolean(historyForkingId)"
            @change="selectHistoryFromPicker"
          >
            <option value="">
              {{ historyLoading ? '加载中…' : `项目历史 (${historySummaries.length})` }}
            </option>
            <option
              v-for="(item, index) in historySummaries"
              :key="item.id"
              :value="item.id"
            >
              {{ historyOptionLabel(index) }}
            </option>
          </select>
        </label>
      </div>
      <div class="query-head-actions">
        <template v-if="presentation === 'sidebar'">
          <button
            class="query-presentation-toggle"
            type="button"
            title="移到底栏"
            aria-label="将追问器移到底栏"
            @click="emit('moveDock')"
          >
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              aria-hidden="true"
            >
              <path d="M4 5h16v14H4zM4 14h16" />
            </svg>
          </button>
          <button
            class="query-presentation-toggle"
            type="button"
            title="进入追问专注模式"
            aria-label="进入追问专注模式"
            @click="emit('maximize')"
          >
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M8 3H3v5M16 3h5v5M8 21H3v-5M16 21h5v-5" />
            </svg>
          </button>
        </template>
        <template v-else>
          <button
            class="query-tool"
            type="button"
            :disabled="!trace && !streaming && !errorMsg"
            @click="newTrace"
          >
            新追问
          </button>
          <button
            class="query-tool query-history-delete"
            type="button"
            :disabled="!selectedThread || Boolean(historySelectingId) || Boolean(historyForkingId)"
            @click="deleteSelectedHistory"
          >
            删除
          </button>
          <button
            class="query-tool"
            :class="{ active: selectionMode }"
            type="button"
            :aria-pressed="selectionMode"
            @click="emit('toggleSelectionMode')"
          >
            选择文件
          </button>
          <button
            class="query-tool"
            type="button"
            :disabled="selectedCount === 0"
            @click="emit('clearSelected')"
          >
            清空
          </button>
          <button
            v-if="presentation === 'dock'"
            class="query-presentation-toggle"
            type="button"
            title="移到左栏"
            aria-label="将追问器移到左栏"
            @click="emit('moveSidebar')"
          >
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              aria-hidden="true"
            >
              <path d="M4 5h16v14H4zM10 5v14" />
            </svg>
          </button>
          <button
            v-if="presentation === 'dock'"
            class="query-presentation-toggle"
            type="button"
            title="进入追问专注模式"
            aria-label="进入追问专注模式"
            @click="emit('maximize')"
          >
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M8 3H3v5M16 3h5v5M8 21H3v-5M16 21h5v-5" />
            </svg>
          </button>
          <button
            v-else
            class="query-presentation-toggle"
            type="button"
            title="还原追问器"
            aria-label="还原追问器"
            @click="emit('restore')"
          >
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" />
            </svg>
          </button>
        </template>
        <button
          class="query-collapse"
          type="button"
          title="关闭追问器"
          aria-label="关闭追问器"
          @click="emit('close')"
        >
          <svg
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            aria-hidden="true"
          >
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      </div>
    </header>
    <div
      v-if="historyWarnings.length || historyError"
      class="query-history-notice"
      :class="{ error: Boolean(historyError) }"
      role="status"
    >
      <span v-if="historyError">{{ historyError }}</span>
      <span v-else>
        {{ historyWarnings.length }} 条历史记录无法读取；其余线程仍可使用。
      </span>
    </div>
    <section
      v-if="presentation === 'sidebar' && querySidebarPane === 'history'"
      class="query-sidebar-history"
      data-testid="query-sidebar-history"
      aria-label="项目追问历史"
    >
      <div class="query-sidebar-history-head">
        <div>
          <strong>项目历史</strong>
          <span>{{ historySummaries.length }} 个线程</span>
        </div>
        <button class="query-tool" type="button" @click="newTrace">新追问</button>
      </div>
      <div class="query-sidebar-history-list">
        <p v-if="historyLoading" class="query-hint">正在加载项目历史…</p>
        <p v-else-if="historySummaries.length === 0" class="query-hint">
          还没有追问线程。新追问会保存在当前项目中。
        </p>
        <template v-else>
          <button
            v-for="item in historySummaries"
            :key="item.id"
            class="query-sidebar-history-item"
            :class="{ selected: selectedThread?.id === item.id }"
            type="button"
            :disabled="Boolean(historySelectingId) || Boolean(historyForkingId)"
            @click="selectSidebarHistory(item.id)"
          >
            <span class="query-sidebar-history-title">{{ item.title }}</span>
            <span class="query-sidebar-history-meta">
              {{ item.turnCount }} 轮 · {{ historyScopeLabel(item.scope) }}
            </span>
            <span
              class="query-sidebar-history-freshness"
              :class="{ stale: item.freshness === 'stale' }"
            >
              {{
                historySelectingId === item.id
                  ? '读取中…'
                  : item.freshness === 'fresh'
                    ? '源码一致'
                    : item.staleReason === 'source-missing'
                      ? '范围文件缺失 · 只读'
                      : '源码已变更 · 只读'
              }}
            </span>
          </button>
        </template>
      </div>
    </section>
    <template v-if="presentation !== 'sidebar' || querySidebarPane === 'thread'">
      <div v-if="presentation === 'sidebar'" class="query-sidebar-controls">
        <div class="query-scope" role="tablist" aria-label="追问范围">
          <button
            class="query-scope-btn"
            :class="{ active: scope === 'current' }"
            type="button"
            role="tab"
            :aria-selected="scope === 'current'"
            @click="scope = 'current'"
          >
            当前文件
          </button>
          <button
            class="query-scope-btn"
            :class="{ active: scope === 'selected' }"
            type="button"
            role="tab"
            :aria-selected="scope === 'selected'"
            @click="scope = 'selected'"
          >
            {{ selectedLabel }}
          </button>
        </div>
        <div class="query-sidebar-thread-actions">
          <button class="query-tool" type="button" @click="newTrace">新追问</button>
          <button
            class="query-tool query-history-delete"
            type="button"
            :disabled="!selectedThread || Boolean(historySelectingId) || Boolean(historyForkingId)"
            @click="deleteSelectedHistory"
          >
            删除
          </button>
          <button
            class="query-tool"
            :class="{ active: selectionMode }"
            type="button"
            :aria-pressed="selectionMode"
            @click="emit('toggleSelectionMode')"
          >
            选文件
          </button>
          <button
            class="query-tool"
            type="button"
            :disabled="selectedCount === 0"
            @click="emit('clearSelected')"
          >
            清空
          </button>
        </div>
      </div>
    <div v-if="selectedThread" class="query-history-current">
      <span class="query-history-current-title">{{ selectedThread.title }}</span>
      <span class="query-history-current-meta">
        {{ selectedThread.turns.length }} 轮 · {{ historyScopeLabel(selectedThread.scope) }} ·
        {{ selectedThread.freshness === 'fresh' ? '源码一致' : '陈旧' }}
      </span>
      <template v-if="selectedThread.freshness === 'stale'">
        <span class="query-history-stale">历史只读 · {{ historyStaleMessage }}</span>
        <button
          v-if="selectedThread.staleReason === 'source-changed'"
          class="query-history-fork"
          type="button"
          :disabled="Boolean(historyForkingId)"
          @click="forkSelectedHistory"
        >
          {{ historyForkingId ? '新建中…' : '基于当前源码新建追问' }}
        </button>
      </template>
      <button
        v-else-if="historyNeedsScopeRestore"
        class="query-history-restore"
        type="button"
        @click="emit('restoreScope', selectedThread.scope)"
      >
        回到追问范围并继续
      </button>
      <span v-else-if="historyReadOnly" class="query-history-waiting">
        等待当前范围就绪后可继续
      </span>
    </div>
    <div v-if="!path && scope === 'current'" class="query-vacuum">打开文件以启用追问</div>
    <template v-else>
      <div class="query-content">
        <template v-if="presentation === 'focus'">
          <nav v-if="hasFocusHistory" class="query-turn-history" aria-label="问题历史">
            <div class="query-turn-history-head">
              <strong>问题历史</strong>
              <span>{{ completedTurns.length + (hasActiveTurn ? 1 : 0) }} 轮</span>
            </div>
            <div class="query-turn-history-list">
              <button
                v-for="(turn, index) in completedTurns"
                :key="index"
                class="query-turn-history-item"
                :class="{ selected: focusedCompletedIndex === index }"
                type="button"
                :aria-current="focusedCompletedIndex === index ? 'true' : undefined"
                @click="selectCompletedTurn(index)"
              >
                <span class="query-turn-history-label">问 {{ index + 1 }}</span>
                <span class="query-turn-history-question">{{ turn.question }}</span>
              </button>
              <button
                v-if="hasActiveTurn"
                class="query-turn-history-item active-turn"
                :class="{ selected: focusedIsActive }"
                type="button"
                :aria-current="focusedIsActive ? 'true' : undefined"
                @click="selectActiveTurn"
              >
                <span class="query-turn-history-label">
                  {{ streaming ? '回答中' : '已中断' }}
                </span>
                <span class="query-turn-history-question">{{ activeQuestion }}</span>
              </button>
            </div>
          </nav>

          <div class="query-focus-reader">
            <label v-if="hasFocusHistory" class="query-turn-picker">
              <span>问题历史</span>
              <select :value="focusedSelectionKey" @change="selectTurnFromPicker">
                <option
                  v-for="(turn, index) in completedTurns"
                  :key="index"
                  :value="`completed:${index}`"
                >
                  问 {{ index + 1 }} · {{ turn.question }}
                </option>
                <option v-if="hasActiveTurn" value="active">
                  {{ streaming ? '回答中' : '已中断' }} · {{ activeQuestion }}
                </option>
              </select>
            </label>

            <div
              ref="renderedEl"
              class="query-answer"
              :class="{ 'code-evidence-disabled': !codeEvidenceEnabled }"
              @click="onAnswerClick"
            >
              <div class="query-answer-content">
                <article
                  v-if="focusedSelection"
                  class="query-focus-turn"
                  :class="{ active: focusedIsActive }"
                  :data-query-turn-index="focusedCompletedIndex ?? undefined"
                >
                  <header class="query-focus-question">
                    <span class="query-focus-eyebrow">
                      {{
                        focusedIsActive
                          ? streaming
                            ? '正在回答'
                            : '回答中断'
                          : `问题 ${(focusedCompletedIndex ?? 0) + 1}`
                      }}
                    </span>
                    <h2>{{ focusedQuestion }}</h2>
                  </header>

                  <div
                    v-if="focusedIsActive && streaming && statusText"
                    class="query-status"
                    :class="{ fallback: viewState.phase === 'fallback' }"
                    role="status"
                  >
                    <span
                      v-if="viewState.phase !== 'fallback'"
                      class="query-status-spinner"
                      aria-hidden="true"
                    ></span>
                    <span>{{ statusText }}</span>
                  </div>

                  <template v-if="focusedIsActive">
                    <QueryMapView
                      v-if="focusedMap"
                      :map="focusedMap"
                      :code-evidence-enabled="codeEvidenceEnabled"
                      @open-evidence="openEvidence"
                    />
                    <div v-if="focusedEvidence" class="query-evidence-block">
                      <div class="query-evidence-summary">
                        <span
                          class="query-evidence-badge"
                          :class="`tone-${focusedEvidenceDisplay.tone}`"
                        >
                          {{ focusedEvidenceDisplay.label }}
                        </span>
                        <span v-if="focusedEvidence.sources.length" class="query-source-count">
                          {{ focusedEvidence.sources.length }} 个来源
                        </span>
                      </div>
                      <ul v-if="focusedEvidence.sources.length" class="query-sources">
                        <li v-for="source in focusedEvidence.sources" :key="source.url">
                          <a :href="source.url" target="_blank" rel="noopener noreferrer">
                            {{ source.title }}
                          </a>
                        </li>
                      </ul>
                      <p
                        v-else-if="focusedEvidence.status === 'web-uncited'"
                        class="query-warning"
                      >
                        供应商返回了联网整理内容，但没有可追溯 URL。
                      </p>
                      <p
                        v-else-if="
                          focusedEvidence.status === 'unverified' && !focusedEvidence.warning
                        "
                        class="query-warning"
                      >
                        本次回答仅依据本地上下文，未由外部来源核验。
                      </p>
                      <p v-if="focusedEvidence.warning" class="query-warning">
                        {{ focusedEvidence.warning }}
                      </p>
                    </div>
                  </template>

                  <div v-if="focusedAnswer" class="query-focus-answer">
                    <div
                      v-if="focusedAnswerHtml"
                      class="query-answer-md"
                      v-html="focusedAnswerHtml"
                    ></div>
                    <div v-else class="query-answer-plain">{{ focusedAnswer }}</div>
                  </div>

                  <details
                    v-if="!focusedIsActive && (focusedMap || focusedEvidence)"
                    :key="`query-support-${focusedCompletedIndex}`"
                    class="query-focus-support"
                  >
                    <summary>
                      <span>方向图与证据</span>
                      <small>展开核验</small>
                    </summary>
                    <div class="query-focus-support-body">
                      <QueryMapView
                        v-if="focusedMap"
                        :map="focusedMap"
                        :code-evidence-enabled="codeEvidenceEnabled"
                        @open-evidence="openEvidence"
                      />
                      <div v-if="focusedEvidence" class="query-evidence-block">
                        <div class="query-evidence-summary">
                          <span
                            class="query-evidence-badge"
                            :class="`tone-${focusedEvidenceDisplay.tone}`"
                          >
                            {{ focusedEvidenceDisplay.label }}
                          </span>
                          <span v-if="focusedEvidence.sources.length" class="query-source-count">
                            {{ focusedEvidence.sources.length }} 个来源
                          </span>
                        </div>
                        <ul v-if="focusedEvidence.sources.length" class="query-sources">
                          <li v-for="source in focusedEvidence.sources" :key="source.url">
                            <a :href="source.url" target="_blank" rel="noopener noreferrer">
                              {{ source.title }}
                            </a>
                          </li>
                        </ul>
                        <p
                          v-else-if="focusedEvidence.status === 'web-uncited'"
                          class="query-warning"
                        >
                          供应商返回了联网整理内容，但没有可追溯 URL。
                        </p>
                        <p
                          v-else-if="
                            focusedEvidence.status === 'unverified' && !focusedEvidence.warning
                          "
                          class="query-warning"
                        >
                          本次回答仅依据本地上下文，未由外部来源核验。
                        </p>
                        <p v-if="focusedEvidence.warning" class="query-warning">
                          {{ focusedEvidence.warning }}
                        </p>
                      </div>
                    </div>
                  </details>

                  <p v-if="focusedUnknownEvidenceIds.length" class="query-warning" role="alert">
                    回答引用了未知代码证据：{{ focusedUnknownEvidenceIds.join('、') }}。
                    这些编号不可跳转。
                  </p>
                  <p v-if="focusedIsActive && errorMsg" class="query-error">
                    {{ focusedAnswer ? `回答中断：${errorMsg}` : errorMsg }}
                  </p>
                </article>

                <div v-else class="query-focus-empty">
                  <span v-if="scope === 'selected'" class="query-hint">
                    {{ selectedScopeHint }}
                  </span>
                  <span v-else-if="!currentOrientationId" class="query-hint">
                    文件定向完成后可追问当前文件
                  </span>
                  <span v-else class="query-hint">
                    就当前文件提问，例如「这个文件做什么？」
                  </span>
                  <p v-if="errorMsg" class="query-error">{{ errorMsg }}</p>
                </div>
              </div>
            </div>
          </div>
        </template>

        <div
          v-else
          ref="renderedEl"
          class="query-answer"
          :class="{ 'code-evidence-disabled': !codeEvidenceEnabled }"
          @click="onAnswerClick"
        >
          <div class="query-answer-content">
          <div
            v-if="streaming && statusText"
            class="query-status"
            :class="{ fallback: viewState.phase === 'fallback' }"
            role="status"
          >
            <span v-if="viewState.phase !== 'fallback'" class="query-status-spinner" aria-hidden="true"></span>
            <span>{{ statusText }}</span>
          </div>

          <QueryMapView
            v-if="viewState.map"
            :map="viewState.map"
            :code-evidence-enabled="codeEvidenceEnabled"
            @open-evidence="openEvidence"
          />

          <div v-if="evidenceState" class="query-evidence-block">
            <div class="query-evidence-summary">
              <span class="query-evidence-badge" :class="`tone-${evidenceDisplay.tone}`">
                {{ evidenceDisplay.label }}
              </span>
              <span v-if="evidenceState.sources.length" class="query-source-count">
                {{ evidenceState.sources.length }} 个来源
              </span>
            </div>
            <ul v-if="evidenceState.sources.length" class="query-sources">
              <li v-for="source in evidenceState.sources" :key="source.url">
                <a :href="source.url" target="_blank" rel="noopener noreferrer">
                  {{ source.title }}
                </a>
              </li>
            </ul>
            <p v-else-if="evidenceState.status === 'web-uncited'" class="query-warning">
              供应商返回了联网整理内容，但没有可追溯 URL。
            </p>
            <p
              v-else-if="evidenceState.status === 'unverified' && !evidenceState.warning"
              class="query-warning"
            >
              本次回答仅依据本地上下文，未由外部来源核验。
            </p>
            <p v-if="evidenceState.warning" class="query-warning">
              {{ evidenceState.warning }}
            </p>
          </div>

          <div v-if="completedTurns.length" class="query-turn-list">
            <article
              v-for="(turn, index) in completedTurns"
              :key="index"
              class="query-turn"
              :data-query-turn-index="index"
            >
              <div class="query-turn-question">
                <span class="query-turn-label">问 {{ index + 1 }}</span>
                <span>{{ turn.question }}</span>
              </div>
              <QueryMapView
                v-if="
                  traceSnapshots[index]?.map &&
                  !(viewState.mode === 'done' && index === completedTurns.length - 1)
                "
                :map="traceSnapshots[index].map"
                :code-evidence-enabled="codeEvidenceEnabled"
                @open-evidence="openEvidence"
              />
              <div class="query-turn-answer">
                <span class="query-turn-label">答</span>
                <div
                  v-if="traceSnapshots[index]?.answerHtml"
                  class="query-answer-md"
                  v-html="traceSnapshots[index].answerHtml"
                ></div>
                  <div v-else class="query-answer-plain">{{ turn.answer }}</div>
              </div>
              <p v-if="traceUnknownEvidenceIds[index]?.length" class="query-warning" role="alert">
                回答引用了未知代码证据：{{ traceUnknownEvidenceIds[index].join('、') }}。
                这些编号不可跳转。
              </p>
            </article>
          </div>

          <article v-if="activeQuestion && (streaming || errorMsg)" class="query-turn active">
            <div class="query-turn-question">
              <span class="query-turn-label">问</span>
              <span>{{ activeQuestion }}</span>
            </div>
            <div v-if="answer" class="query-turn-answer">
              <span class="query-turn-label">答</span>
              <div class="query-answer-plain">{{ answer }}</div>
            </div>
            <p v-if="activeUnknownEvidenceIds.length" class="query-warning" role="alert">
              回答引用了未知代码证据：{{ activeUnknownEvidenceIds.join('、') }}。
              这些编号不可跳转。
            </p>
          </article>

          <span
            v-if="!completedTurns.length && !activeQuestion && !streaming && !errorMsg && scope === 'selected'"
            class="query-hint"
          >
            {{ selectedScopeHint }}
          </span>
          <span
            v-else-if="!completedTurns.length && !activeQuestion && !streaming && !errorMsg && !currentOrientationId"
            class="query-hint"
          >
            文件定向完成后可追问当前文件
          </span>
          <span
            v-else-if="!completedTurns.length && !activeQuestion && !streaming && !errorMsg"
            class="query-hint"
          >
            就当前文件提问，例如「这个文件做什么？」
          </span>
          <p v-if="errorMsg" class="query-error">
            {{ answer ? `回答中断：${errorMsg}` : errorMsg }}
          </p>
          </div>
        </div>
        <div
          v-if="presentation === 'focus' && codePeekTarget"
          class="query-peek-resizer"
          role="separator"
          aria-label="调整代码证据预览宽度"
          aria-orientation="vertical"
          :aria-valuemin="codePeekBounds.min"
          :aria-valuemax="codePeekBounds.max"
          :aria-valuenow="codePeekWidth"
          @pointerdown="startCodePeekResize"
          @pointermove="moveCodePeekResize"
          @pointerup="endCodePeekResize"
          @pointercancel="cancelCodePeekResize"
          @lostpointercapture="loseCodePeekPointer"
        ></div>
        <div
          v-if="presentation === 'focus' && codePeekTarget"
          class="query-peek-pane"
          :style="{ width: codePeekWidth + 'px' }"
        >
          <CodeEvidencePeek :target="codePeekTarget" @close="closeCodePeek" />
        </div>
      </div>
      <form class="query-form" @submit.prevent="ask">
        <input
          v-model="question"
          class="query-input"
          :placeholder="scope === 'selected' ? '追问已选文件…' : '追问当前文件…'"
          :disabled="streaming || historyReadOnly"
        />
        <button class="query-send" type="submit" :disabled="!canAsk">
          {{ streaming ? '…' : '追问' }}
        </button>
      </form>
    </template>
    </template>
  </section>
</template>
