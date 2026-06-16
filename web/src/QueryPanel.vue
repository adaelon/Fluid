<script setup lang="ts">
// S10b: the follow-up query terminal, docked as a bottom panel (ADR-0015/0016
// PENDING resolved — out of the right edge so it never fights trailing line
// notes). Asks the current file a free-form question and streams the answer back
// token by token over WS /api/query (S10a). Context is the whole current file
// (CONTEXT 追问器); switching files vacuums the in-flight Q&A.
import { ref, watch, nextTick, onBeforeUnmount } from 'vue'
import { streamQuery, type QueryStream } from './api'
import { EMPTY_QUERY_CONTEXT, type QueryContext } from './queryContext'
// S11-lazy: markdown-it / DOMPurify / KaTeX (+ its CSS) are heavy and only needed
// once an answer finishes streaming, so they are dynamically import()ed inside
// renderAnswer() rather than at module top — Rollup splits them into async chunks
// kept out of the first-paint bundle. Behavior is unchanged from ADR-0008.

const props = withDefaults(
  defineProps<{ path: string | null; ctx?: QueryContext }>(),
  { ctx: () => EMPTY_QUERY_CONTEXT },
)

const question = ref('')
const answer = ref('') // plain token-by-token text shown while streaming
const answerHtml = ref('') // sanitized Markdown HTML, set once on `done`
const streaming = ref(false)
const errorMsg = ref('')
const collapsed = ref(false)
const renderedEl = ref<HTMLElement | null>(null)
let stream: QueryStream | null = null

function teardown() {
  stream?.cancel()
  stream = null
  streaming.value = false
}

// Switching/closing files resets the panel (vacuum semantics, §7).
watch(
  () => props.path,
  () => {
    teardown()
    answer.value = ''
    answerHtml.value = ''
    errorMsg.value = ''
    question.value = ''
  },
)

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
  if (!q || !props.path || streaming.value) return
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
  <section class="query-panel" :class="{ disabled: !path, collapsed }">
    <header class="query-head">
      <span class="query-title">追问器{{ path ? '' : ' · 未激活' }}</span>
      <button v-if="path" class="query-collapse" type="button" @click="collapsed = !collapsed">
        {{ collapsed ? '▴' : '▾' }}
      </button>
    </header>
    <template v-if="!collapsed">
      <div v-if="!path" class="query-vacuum">打开文件以启用追问</div>
      <template v-else>
        <div class="query-answer">
          <span v-if="errorMsg" class="query-error">{{ errorMsg }}</span>
          <div v-else-if="answerHtml" ref="renderedEl" class="query-answer-md" v-html="answerHtml"></div>
          <template v-else-if="answer">{{ answer }}</template>
          <span v-else-if="streaming" class="query-thinking">思考中…</span>
          <span v-else class="query-hint">就当前文件提问，例如「这个文件做什么？」</span>
        </div>
        <form class="query-form" @submit.prevent="ask">
          <input
            v-model="question"
            class="query-input"
            placeholder="追问当前文件…"
            :disabled="streaming"
          />
          <button class="query-send" type="submit" :disabled="streaming || !question.trim()">
            {{ streaming ? '…' : '追问' }}
          </button>
        </form>
      </template>
    </template>
  </section>
</template>
