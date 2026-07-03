<script setup lang="ts">
// S10b: the follow-up query terminal, docked as a bottom panel (ADR-0015/0016
// PENDING resolved — out of the right edge so it never fights trailing line
// notes). Asks the current file a free-form question and streams the answer back
// token by token over WS /api/query (S10a). Context is the whole current file
// (CONTEXT 追问器); switching files vacuums the in-flight Q&A.
import { computed, ref, watch, nextTick, onBeforeUnmount } from 'vue'
import { streamQuery, streamQueryFiles, type QueryStream } from './api'
import { EMPTY_QUERY_CONTEXT, type QueryContext } from './queryContext'
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
  }>(),
  { ctx: () => EMPTY_QUERY_CONTEXT, selectionMode: false, selectedCount: 0, selectedPaths: () => [] },
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
const answer = ref('') // plain token-by-token text shown while streaming
const answerHtml = ref('') // sanitized Markdown HTML, set once on `done`
const streaming = ref(false)
const errorMsg = ref('')
const renderedEl = ref<HTMLElement | null>(null)
const scope = ref<QueryScope>('current')
let stream: QueryStream | null = null

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
  answer.value = ''
  answerHtml.value = ''
  errorMsg.value = ''
  if (clearQuestion) question.value = ''
}

function teardown() {
  stream?.cancel()
  stream = null
  streaming.value = false
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
async function renderAnswer() {
  // Pull the render libs on demand (S11-lazy). The CSS import is a side effect
  // (injects KaTeX styles) — its module value is unused.
  const [{ renderMarkdown }, { default: DOMPurify }, { default: renderMathInElement }] =
    await Promise.all([
      import('./render/markdown'),
      import('dompurify'),
      import('katex/contrib/auto-render'),
      import('katex/dist/katex.min.css'),
    ])
  answerHtml.value = DOMPurify.sanitize(renderMarkdown(answer.value))
  await nextTick()
  if (!renderedEl.value) return
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

function ask() {
  const q = question.value.trim()
  if (!q || streaming.value) return
  if (scope.value === 'selected') {
    if (!selectedReady.value) {
      errorMsg.value = selectedScopeHint.value
      return
    }
    answer.value = ''
    answerHtml.value = ''
    errorMsg.value = ''
    streaming.value = true
    stream = streamQueryFiles(
      {
        filePaths: props.selectedPaths,
        question: q,
      },
      {
        onDelta: (t) => {
          answer.value += t
        },
        onDone: () => {
          streaming.value = false
          stream = null
          void renderAnswer()
        },
        onError: (m) => {
          errorMsg.value = m
          streaming.value = false
          stream = null
        },
      },
    )
    return
  }
  if (!props.path) return
  answer.value = ''
  answerHtml.value = ''
  errorMsg.value = ''
  streaming.value = true
  stream = streamQuery(
    {
      filePath: props.path,
      question: q,
      roster: props.ctx.roster,
      rosterSpans: props.ctx.rosterSpans,
      capsules: props.ctx.capsules,
    },
    {
      onDelta: (t) => {
        answer.value += t
      },
      onDone: () => {
        streaming.value = false
        stream = null
        void renderAnswer()
      },
      onError: (m) => {
        errorMsg.value = m
        streaming.value = false
        stream = null
      },
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
          <span v-if="errorMsg" class="query-error">{{ errorMsg }}</span>
          <div v-else-if="answerHtml" ref="renderedEl" class="query-answer-md" v-html="answerHtml"></div>
          <template v-else-if="answer">{{ answer }}</template>
          <span v-else-if="streaming" class="query-thinking">思考中…</span>
          <span v-else-if="scope === 'selected'" class="query-hint">{{ selectedScopeHint }}</span>
          <span v-else class="query-hint">就当前文件提问，例如「这个文件做什么？」</span>
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
