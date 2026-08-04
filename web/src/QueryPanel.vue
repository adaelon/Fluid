<script setup lang="ts">
// S10b: the follow-up query terminal, docked as a bottom panel (ADR-0015/0016
// PENDING resolved — out of the right edge so it never fights trailing line
// notes). Asks the current file or selected file set a free-form question and
// projects status/evidence plus token deltas from the matching query WebSocket.
// Switching files or scope vacuums the in-flight Q&A.
import { computed, ref, watch, nextTick, onBeforeUnmount } from 'vue'
import { streamQuery, streamQueryFiles, type QueryStream } from './api'
import type { QueryFrame } from './ghostTypes'
import { EMPTY_QUERY_CONTEXT, type QueryContext } from './queryContext'
import { idleQueryState, reduceQueryFrame, startQueryRequest } from './queryState'
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
  }>(),
  {
    ctx: () => EMPTY_QUERY_CONTEXT,
    selectionMode: false,
    selectedCount: 0,
    selectedPaths: () => [],
    allowWeb: true,
  },
)
// Visibility is owned by the parent (App) via the status-bar toggle — default
// hidden so the bottom space goes back to the code area; the panel only asks to
// close itself, it never decides whether it is mounted.
const emit = defineEmits<{
  close: []
  toggleSelectionMode: []
  clearSelected: []
}>()

type QueryScope = 'current' | 'selected'

const question = ref('')
const answerHtml = ref('') // sanitized Markdown HTML, set once on `done`
const viewState = ref(idleQueryState())
const renderedEl = ref<HTMLElement | null>(null)
const scope = ref<QueryScope>('current')
let stream: QueryStream | null = null
let requestGeneration = 0

const answer = computed(() => viewState.value.answer)
const streaming = computed(() => viewState.value.mode === 'streaming')
const errorMsg = computed(() => viewState.value.errorMessage)
const evidenceState = computed(() => viewState.value.evidence)
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
const selectedLabel = computed(() => `已选文件(${props.selectedCount})`)
const selectedScopeHint = computed(() => {
  if (props.selectedCount === 0) return '选择至少 2 个文件后可切换到文件集追问'
  if (props.selectedCount === 1) return '再选择 1 个文件后可进行文件集追问'
  return `已选择 ${props.selectedCount} 个文件`
})
const canAsk = computed(() => {
  if (!question.value.trim() || streaming.value) return false
  if (scope.value === 'current') return Boolean(props.path)
  return selectedReady.value
})

function resetAnswer(clearQuestion = true) {
  teardown()
  viewState.value = idleQueryState()
  answerHtml.value = ''
  if (clearQuestion) question.value = ''
}

function teardown() {
  requestGeneration++
  stream?.cancel()
  stream = null
}

// Switching/closing files resets the panel (vacuum semantics, §7).
watch(
  () => props.path,
  () => {
    resetAnswer()
  },
)

watch(scope, () => resetAnswer(false))

// On `done`, render the full Markdown answer (ADR-0008): markdown-it escapes raw
// HTML, DOMPurify is defense-in-depth, then KaTeX transforms $…$/$$…$$ in the DOM.
async function renderAnswer(generation: number, text: string) {
  // Pull the render libs on demand (S11-lazy). The CSS import is a side effect
  // (injects KaTeX styles) — its module value is unused.
  const [{ renderMarkdown }, { default: DOMPurify }, { default: renderMathInElement }] =
    await Promise.all([
      import('./render/markdown'),
      import('dompurify'),
      import('katex/contrib/auto-render'),
      import('katex/dist/katex.min.css'),
    ])
  if (generation !== requestGeneration) return
  answerHtml.value = DOMPurify.sanitize(renderMarkdown(text))
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

function acceptFrame(generation: number, frame: QueryFrame) {
  if (generation !== requestGeneration) return
  const next = reduceQueryFrame(viewState.value, frame)
  viewState.value = next
  if (frame.kind === 'done') {
    stream = null
    void renderAnswer(generation, next.answer)
  } else if (frame.kind === 'error') {
    stream = null
  }
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
    const generation = ++requestGeneration
    viewState.value = startQueryRequest()
    answerHtml.value = ''
    stream = streamQueryFiles(
      {
        filePaths: props.selectedPaths,
        question: q,
        allowWeb: props.allowWeb,
      },
      {
        onFrame: (frame) => acceptFrame(generation, frame),
      },
    )
    return
  }
  if (!props.path) return
  const generation = ++requestGeneration
  viewState.value = startQueryRequest()
  answerHtml.value = ''
  stream = streamQuery(
    {
      filePath: props.path,
      question: q,
      roster: props.ctx.roster,
      rosterSpans: props.ctx.rosterSpans,
      capsules: props.ctx.capsules,
      allowWeb: props.allowWeb,
    },
    {
      onFrame: (frame) => acceptFrame(generation, frame),
    },
  )
}

onBeforeUnmount(teardown)
</script>

<template>
  <section class="query-panel" :class="{ disabled: !path }">
    <header class="query-head">
      <div class="query-head-left">
        <span class="query-title">追问器{{ path ? '' : ' · 未激活' }}</span>
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
      <button class="query-collapse" type="button" title="收起追问器" aria-label="收起追问器" @click="emit('close')">
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
        <div class="query-answer">
          <div
            v-if="streaming && statusText"
            class="query-status"
            :class="{ fallback: viewState.phase === 'fallback' }"
            role="status"
          >
            <span v-if="viewState.phase !== 'fallback'" class="query-status-spinner" aria-hidden="true"></span>
            <span>{{ statusText }}</span>
          </div>

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

          <div v-if="answerHtml" ref="renderedEl" class="query-answer-md" v-html="answerHtml"></div>
          <div v-else-if="answer" class="query-answer-plain">{{ answer }}</div>
          <span v-else-if="!streaming && !errorMsg && scope === 'selected'" class="query-hint">
            {{ selectedScopeHint }}
          </span>
          <span v-else-if="!streaming && !errorMsg" class="query-hint">
            就当前文件提问，例如「这个文件做什么？」
          </span>
          <p v-if="errorMsg" class="query-error">
            {{ answer ? `回答中断：${errorMsg}` : errorMsg }}
          </p>
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
