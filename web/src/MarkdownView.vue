<script setup lang="ts">
// Document Render View (CONTEXT「文档渲染视图」) + Document Translation
// (CONTEXT「文档翻译」): a .md/.markdown file renders as a formatted document, and a
// header toggle [原文 | 译中文] flips it in place between the English original and a
// Simplified-Chinese translation. Translation is on-demand (button), streams from the
// backend (WS /api/translate) chunk by chunk so progress shows and the document
// renders incrementally as chunks arrive — a long doc is many slow LLM calls, so the
// live feedback matters (memory: generation needs visible status). Code blocks are
// preserved (backend), the result caches to .fluid/, and the source is never written.
// The whole pipeline bypasses generation / ghost annotations → md stays vacuum.
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { renderDoc, typesetMath } from './render/markdownDoc'
import { streamTranslate, type TranslateStream } from './api'

const props = defineProps<{ source: string; path: string }>()

const html = ref('')
const article = ref<HTMLElement | null>(null)
// 'en' shows props.source; 'zh' shows the translated chunks joined in order.
const mode = ref<'en' | 'zh'>('en')
const zhChunks = ref<string[]>([]) // translated chunks by index (filled in order)
const zhComplete = ref(false) // a full translation is cached in-component for this file
const translating = ref(false)
const progressDone = ref(0)
const progressTotal = ref(0)
const error = ref('')
let stream: TranslateStream | null = null
// Bumps on every file switch; async render/stream callbacks bail if it moved.
let token = 0

function zhSource(): string {
  return zhChunks.value.join('')
}

// Render whichever source the current mode selects (en original / zh-so-far).
async function renderActive(): Promise<void> {
  const t = token
  const src = mode.value === 'zh' ? zhSource() : props.source
  const out = await renderDoc(src)
  if (t !== token) return
  html.value = out
  await nextTick()
  if (t !== token || !article.value) return
  await typesetMath(article.value)
}

function teardownStream(): void {
  stream?.cancel()
  stream = null
}

// Reset to the English original on every file switch (vacuum the translation state).
function reset(): void {
  token++
  teardownStream()
  mode.value = 'en'
  zhChunks.value = []
  zhComplete.value = false
  translating.value = false
  progressDone.value = 0
  progressTotal.value = 0
  error.value = ''
  void renderActive()
}

async function showOriginal(): Promise<void> {
  if (mode.value === 'en') return
  mode.value = 'en'
  await renderActive() // the stream (if any) keeps running in the background
}

function showChinese(): void {
  error.value = ''
  mode.value = 'zh'
  // Already translated (or mid-stream) → just view what we have, no new request.
  if (zhComplete.value || translating.value) {
    void renderActive()
    return
  }
  // Start a fresh streaming translation.
  const t = token
  translating.value = true
  zhChunks.value = []
  progressDone.value = 0
  progressTotal.value = 0
  void renderActive() // show the (empty) zh view; chunks fill it in
  stream = streamTranslate(props.path, {
    onCached: (text) => {
      if (t !== token) return
      zhChunks.value = [text]
      void renderActive()
    },
    onTotal: (total) => {
      if (t === token) progressTotal.value = total
    },
    onChunk: (index, text) => {
      if (t !== token) return
      zhChunks.value[index] = text
      progressDone.value += 1
      void renderActive() // incremental: re-render the growing document
    },
    onDone: () => {
      if (t !== token) return
      translating.value = false
      zhComplete.value = true
      stream = null
      void renderActive()
    },
    onError: (message) => {
      if (t !== token) return
      translating.value = false
      stream = null
      error.value = message
      if (zhChunks.value.length === 0) {
        mode.value = 'en' // nothing usable → back to original
        void renderActive()
      }
    },
  })
}

onMounted(() => void renderActive())
watch(() => [props.source, props.path], reset)
onBeforeUnmount(teardownStream)
</script>

<template>
  <div class="fluid-doc-scroll">
    <div class="fluid-doc-head">
      <div class="fluid-doc-toggle" role="group" aria-label="原文或译文">
        <button
          type="button"
          class="fluid-doc-tab"
          :class="{ active: mode === 'en' }"
          @click="showOriginal"
        >
          原文
        </button>
        <button
          type="button"
          class="fluid-doc-tab"
          :class="{ active: mode === 'zh' }"
          @click="showChinese"
        >
          译中文
        </button>
      </div>
      <span v-if="translating" class="fluid-doc-progress">
        翻译中 {{ progressDone }}<template v-if="progressTotal">/{{ progressTotal }}</template> 段…
      </span>
      <span v-else-if="error" class="fluid-doc-err" :title="error">翻译失败</span>
    </div>
    <article ref="article" class="fluid-doc" v-html="html"></article>
  </div>
</template>
