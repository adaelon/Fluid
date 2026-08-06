<script setup lang="ts">
// S10b: the follow-up query terminal, docked as a bottom panel (ADR-0015/0016
// PENDING resolved — out of the right edge so it never fights trailing line
// notes). Asks the current file or selected file set a free-form question and
// projects the backend QueryMap before evidence/token deltas from the matching
// query WebSocket, and turns only known E# citations into source navigation.
// Switching files or scope vacuums the in-flight Q&A.
import { computed, ref, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { streamQuery, streamQueryFiles, type QueryStream } from './api'
import type { CodeEvidenceRef, QueryFrame, QueryMap, QueryTrace } from './ghostTypes'
import { EMPTY_QUERY_CONTEXT, type QueryContext } from './queryContext'
import CodeEvidencePeek from './CodeEvidencePeek.vue'
import QueryMapView from './QueryMapView.vue'
import { queryAnswerEvidenceCitations, queryEvidenceById } from './queryEvidence'
import {
  QUERY_PEEK_STORAGE_KEY,
  clampCodePeekWidth,
  codePeekWidthBounds,
  codePeekWidthFromPointer,
  loadCodePeekWidth,
  type QueryPresentation,
} from './queryLayout'
import {
  alignQueryTrace,
  appendCompletedQueryTurn,
  currentQueryScope,
  idleQueryState,
  reduceQueryFrame,
  selectedQueryScope,
  startQueryRequest,
  startQueryTrace,
  type QueryScopeIdentity,
} from './queryState'
// S11-lazy: markdown-it / DOMPurify / KaTeX (+ its CSS) are heavy and only needed
// once an answer finishes streaming, so they are dynamically import()ed inside
// renderAnswer() rather than at module top — Rollup splits them into async chunks
// kept out of the first-paint bundle. Behavior is unchanged from ADR-0008.

const props = withDefaults(
  defineProps<{
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
  maximize: []
  restore: []
}>()

type QueryScope = 'current' | 'selected'

const question = ref('')
const viewState = ref(idleQueryState())
const trace = ref<QueryTrace | null>(null)
const traceMaps = ref<QueryMap[]>([])
const traceAnswerHtml = ref<string[]>([])
const activeQuestion = ref('')
const renderedEl = ref<HTMLElement | null>(null)
const scope = ref<QueryScope>('current')
const codePeekTarget = ref<CodeEvidenceRef | null>(null)
const codePeekViewportWidth = ref(window.innerWidth)
let codePeekPreferredWidth = loadCodePeekWidth(
  localStorage.getItem(QUERY_PEEK_STORAGE_KEY),
  codePeekViewportWidth.value,
)
const codePeekWidth = ref(codePeekPreferredWidth)
const codePeekBounds = computed(() => codePeekWidthBounds(codePeekViewportWidth.value))
const codePeekDragging = ref(false)
let stream: QueryStream | null = null
let requestGeneration = 0
let requestSequence = 0

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
const activeUnknownEvidenceIds = computed(() => {
  const map = viewState.value.map
  return map ? queryAnswerEvidenceCitations(answer.value, map.evidence).unknownIds : []
})
const traceUnknownEvidenceIds = computed(() =>
  completedTurns.value.map((turn, index) => {
    const map = traceMaps.value[index]
    return map ? queryAnswerEvidenceCitations(turn.answer, map.evidence).unknownIds : []
  }),
)
const evidenceDisplay = computed(() => {
  switch (evidenceState.value?.status) {
    case 'project-source':
      return { label: '项目源码', tone: 'project' }
    case 'web-cited':
      return { label: '网页有来源', tone: 'cited' }
    case 'web-uncited':
      return { label: '联网无来源', tone: 'uncited' }
    default:
      return { label: '未核验', tone: 'unverified' }
  }
})
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
const canAsk = computed(() => {
  if (!question.value.trim() || streaming.value) return false
  if (scope.value === 'current') return Boolean(props.path && currentOrientationId.value)
  return selectedReady.value
})

function resetTrace(clearQuestion = true) {
  teardown()
  trace.value = null
  traceMaps.value = []
  traceAnswerHtml.value = []
  activeQuestion.value = ''
  closeCodePeek()
  viewState.value = idleQueryState()
  if (clearQuestion) question.value = ''
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

function teardown() {
  requestGeneration++
  stream?.cancel()
  stream = null
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
    resetTrace(next[0] !== previous[0])
  },
)

// On `done`, render every completed Markdown answer (ADR-0008): markdown-it
// escapes raw HTML, DOMPurify is defense-in-depth, then KaTeX transforms math.
async function renderTraceAnswers(
  generation: number,
  turns: QueryTrace['turns'],
  maps: QueryMap[],
) {
  // Pull the render libs on demand (S11-lazy). The CSS import is a side effect
  // (injects KaTeX styles) — its module value is unused.
  const [{ renderQueryMarkdown }, { default: DOMPurify }, { default: renderMathInElement }] =
    await Promise.all([
      import('./render/markdown'),
      import('dompurify'),
      import('katex/contrib/auto-render'),
      import('katex/dist/katex.min.css'),
  ])
  if (generation !== requestGeneration) return
  traceAnswerHtml.value = turns.map((turn, index) =>
    DOMPurify.sanitize(renderQueryMarkdown(turn.answer, maps[index]?.evidence ?? [])),
  )
  await nextTick()
  if (generation !== requestGeneration || !renderedEl.value) return
  renderMathInElement(renderedEl.value, {
    delimiters: [
      { left: '$$', right: '$$', display: true },
      { left: '$', right: '$', display: false },
      { left: '\\[', right: '\\]', display: true },
      { left: '\\(', right: '\\)', display: false },
    ],
    throwOnError: false,
  })
}

function acceptFrame(
  generation: number,
  identity: QueryScopeIdentity,
  askedQuestion: string,
  frame: QueryFrame,
) {
  if (generation !== requestGeneration) return
  const previous = viewState.value
  const next = reduceQueryFrame(previous, frame)
  if (next === previous && frame.reqId !== previous.requestId) return
  viewState.value = next
  if (previous.mode !== 'error' && next.mode === 'error' && frame.kind !== 'error') {
    stream?.cancel()
    stream = null
    return
  }
  if (frame.kind === 'done' && next.mode === 'done' && next.map) {
    stream = null
    const citations = queryAnswerEvidenceCitations(next.answer, next.map.evidence)
    const completed = appendCompletedQueryTurn(
      trace.value,
      identity,
      askedQuestion,
      next.answer,
      citations.knownIds,
    )
    trace.value = completed
    const maps = [...traceMaps.value, next.map]
    traceMaps.value = maps
    activeQuestion.value = ''
    void renderTraceAnswers(generation, completed.turns, maps)
  } else if (frame.kind === 'error') {
    stream = null
  }
}

function openEvidence(reference: CodeEvidenceRef) {
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

  const turn = anchor.closest<HTMLElement>('[data-query-turn-index]')
  const index = Number(turn?.dataset.queryTurnIndex)
  if (!Number.isInteger(index)) return
  const map = traceMaps.value[index]
  const reference = map && queryEvidenceById(map.evidence, href.slice(prefix.length))
  if (!reference) return
  event.preventDefault()
  openEvidence(reference)
}

function newTrace() {
  resetTrace()
}

function ask() {
  const q = question.value.trim()
  if (!q || streaming.value) return
  if (scope.value === 'selected') {
    if (!selectedReady.value) {
      viewState.value = {
        ...idleQueryState(),
        mode: 'error',
        errorMessage: selectedScopeHint.value,
      }
      return
    }
    const identity = { ...scopeIdentity.value }
    const requestTrace =
      alignQueryTrace(trace.value, identity) ?? startQueryTrace(identity, q)
    trace.value = requestTrace
    const reqId = `qf-${++requestSequence}`
    const generation = ++requestGeneration
    viewState.value = startQueryRequest(reqId)
    activeQuestion.value = q
    question.value = ''
    stream = streamQueryFiles(
      {
        reqId,
        filePaths: props.selectedPaths,
        question: q,
        trace: requestTrace,
        allowWeb: props.allowWeb,
      },
      {
        onFrame: (frame) => acceptFrame(generation, identity, q, frame),
      },
    )
    return
  }
  if (!props.path || !currentOrientationId.value) return
  const identity = { ...scopeIdentity.value }
  const requestTrace = alignQueryTrace(trace.value, identity) ?? startQueryTrace(identity, q)
  trace.value = requestTrace
  const reqId = `q-${++requestSequence}`
  const generation = ++requestGeneration
  viewState.value = startQueryRequest(reqId)
  activeQuestion.value = q
  question.value = ''
  stream = streamQuery(
    {
      reqId,
      filePath: props.path,
      orientationId: currentOrientationId.value,
      question: q,
      trace: requestTrace,
      roster: props.ctx.roster,
      rosterSpans: props.ctx.rosterSpans,
      capsules: props.ctx.capsules,
      allowWeb: props.allowWeb,
    },
    {
      onFrame: (frame) => acceptFrame(generation, identity, q, frame),
    },
  )
}

watch(
  () => props.presentation,
  (presentation) => {
    if (presentation === 'focus') resizeCodePeekForViewport()
    else {
      codePeekResize = null
      codePeekDragging.value = false
      closeCodePeek()
    }
  },
)

onMounted(() => window.addEventListener('resize', resizeCodePeekForViewport))
onBeforeUnmount(() => {
  window.removeEventListener('resize', resizeCodePeekForViewport)
  teardown()
})
</script>

<template>
  <section
    class="query-panel"
    :class="{
      disabled: !path,
      focus: presentation === 'focus',
      'peek-open': presentation === 'focus' && codePeekTarget,
      'peek-dragging': codePeekDragging,
    }"
    :data-presentation="presentation"
    data-testid="query-panel"
  >
    <header class="query-head">
      <div class="query-head-left">
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
      </div>
      <div class="query-head-actions">
        <button
          class="query-tool"
          type="button"
          :disabled="!trace && !streaming && !errorMsg"
          @click="newTrace"
        >
          新追问
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
          title="还原追问 dock"
          aria-label="还原追问 dock"
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
    <div v-if="!path && scope === 'current'" class="query-vacuum">打开文件以启用追问</div>
    <template v-else>
      <div class="query-content">
        <div class="query-answer" @click="onAnswerClick">
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

          <div v-if="completedTurns.length" ref="renderedEl" class="query-turn-list">
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
                  traceMaps[index] &&
                  !(viewState.mode === 'done' && index === completedTurns.length - 1)
                "
                :map="traceMaps[index]"
                @open-evidence="openEvidence"
              />
              <div class="query-turn-answer">
                <span class="query-turn-label">答</span>
                <div
                  v-if="traceAnswerHtml[index]"
                  class="query-answer-md"
                  v-html="traceAnswerHtml[index]"
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
          :disabled="streaming"
        />
        <button class="query-send" type="submit" :disabled="!canAsk">
          {{ streaming ? '…' : '追问' }}
        </button>
      </form>
    </template>
  </section>
</template>
