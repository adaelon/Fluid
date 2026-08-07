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
import {
  collectInFileMatches,
  createInFileSearchQuery,
  moveInFileFindCurrent,
  type FindRange,
  type InFileFindDirection,
  type InFileFindQuery,
  type InFileFindSnapshot,
  type InFileFindSurfaceHandle,
} from './inFileFind.ts'
import {
  buildRenderedTextIndex,
  MarkdownFindHighlightLayer,
  renderedHighlightRects,
  type RenderedHighlightRect,
  type RenderedTextIndex,
} from './markdownFind.ts'

const props = defineProps<{
  source: string
  path: string
  findQuery?: InFileFindQuery | null
}>()
const emit = defineEmits<{
  'find-state': [InFileFindSnapshot]
}>()

const html = ref('')
const scroll = ref<HTMLElement | null>(null)
const article = ref<HTMLElement | null>(null)
const findOverlayRects = ref<RenderedHighlightRect[]>([])
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
// Separately guards overlapping renders within one file (translation chunks can
// arrive faster than Markdown/KaTeX finishes rendering).
let renderRequest = 0

const EMPTY_FIND_QUERY: InFileFindQuery = {
  text: '',
  mode: 'literal',
  caseSensitive: false,
}
const findHighlights = new MarkdownFindHighlightLayer()
let findRevision = findHighlights.currentRevision()
let renderedText: RenderedTextIndex | null = null
let findMatches: FindRange[] = []
let findCurrent = 0
let activeFindRange: FindRange | null = null
let appliedFindQuery = createInFileSearchQuery(EMPTY_FIND_QUERY)
let findResizeObserver: ResizeObserver | null = null
let findResizeFrame = 0

function zhSource(): string {
  return zhChunks.value.join('')
}

function sameRange(left: FindRange, right: FindRange): boolean {
  return left.from === right.from && left.to === right.to
}

function emitFindState(): void {
  emit('find-state', {
    current: findMatches.length > 0 ? findCurrent : 0,
    total: findMatches.length,
    error: appliedFindQuery.regexp
      && appliedFindQuery.search.length > 0
      && !appliedFindQuery.valid
      ? 'invalid-regexp'
      : null,
  })
}

function scrollRangeIntoView(range: Range | null): void {
  const scroller = scroll.value
  if (!range || !scroller) return

  const rectangles = Array.from(range.getClientRects())
  const target = rectangles.find((rect) => rect.width > 0 || rect.height > 0)
    ?? range.getBoundingClientRect()
  const viewport = scroller.getBoundingClientRect()
  if (target.top >= viewport.top && target.bottom <= viewport.bottom) return

  const viewportHeight = scroller.clientHeight || viewport.height
  const centeredOffset = target.top - viewport.top - Math.max(0, (viewportHeight - target.height) / 2)
  scroller.scrollTop = Math.max(0, scroller.scrollTop + centeredOffset)
}

function paintFindHighlights(scrollCurrent: boolean): void {
  if (!renderedText) return
  const mapped = findHighlights.apply(
    findRevision,
    renderedText,
    findMatches,
    findCurrent,
  )
  const scroller = scroll.value
  if (mapped && scroller && !findHighlights.usesNativeHighlights()) {
    const origin = scroller.getBoundingClientRect()
    findOverlayRects.value = renderedHighlightRects(
      mapped.all,
      mapped.current,
      origin,
      scroller.scrollLeft,
      scroller.scrollTop,
    )
  } else {
    findOverlayRects.value = []
  }
  if (scrollCurrent) scrollRangeIntoView(mapped?.current ?? null)
}

function clearFindHighlights(): void {
  findHighlights.clear()
  findOverlayRects.value = []
}

function scheduleFindRepaint(): void {
  window.cancelAnimationFrame(findResizeFrame)
  findResizeFrame = window.requestAnimationFrame(() => {
    findResizeFrame = 0
    if (renderedText && findMatches.length > 0) paintFindHighlights(false)
  })
}

function applyFindQuery(
  query: InFileFindQuery | null | undefined,
  options: {
    preserveQuery?: typeof appliedFindQuery
    preserveRange?: FindRange | null
    scrollCurrent?: boolean
  } = {},
): void {
  const nextQuery = createInFileSearchQuery(query ?? EMPTY_FIND_QUERY)
  const preserveQuery = options.preserveQuery ?? appliedFindQuery
  const preserveRange = options.preserveRange === undefined
    ? activeFindRange
    : options.preserveRange
  const canPreserve = preserveQuery.eq(nextQuery) && preserveRange !== null
  appliedFindQuery = nextQuery

  if (!renderedText) {
    findMatches = []
    findCurrent = 0
    activeFindRange = null
    clearFindHighlights()
    emitFindState()
    return
  }

  findMatches = collectInFileMatches(renderedText.text, nextQuery)
  if (!nextQuery.valid || findMatches.length === 0) {
    findCurrent = 0
    activeFindRange = null
    clearFindHighlights()
    emitFindState()
    return
  }

  const preservedIndex = canPreserve
    ? findMatches.findIndex((match) => sameRange(match, preserveRange))
    : -1
  findCurrent = preservedIndex >= 0 ? preservedIndex + 1 : 1
  activeFindRange = findMatches[findCurrent - 1]
  paintFindHighlights(options.scrollCurrent ?? false)
  emitFindState()
}

function moveFind(direction: InFileFindDirection): void {
  if (!renderedText || !appliedFindQuery.valid || findMatches.length === 0) {
    emitFindState()
    return
  }
  findCurrent = moveInFileFindCurrent(findCurrent, findMatches.length, direction)
  activeFindRange = findMatches[findCurrent - 1] ?? null
  paintFindHighlights(true)
  emitFindState()
}

function focusContent(): void {
  article.value?.focus({ preventScroll: true })
}

defineExpose({ moveFind, focusContent } satisfies InFileFindSurfaceHandle)

// Render whichever source the current mode selects (en original / zh-so-far).
async function renderActive(preserveCurrent = true): Promise<void> {
  const t = token
  const request = ++renderRequest
  const preservedQuery = appliedFindQuery
  const preservedRange = preserveCurrent ? activeFindRange : null
  findRevision = findHighlights.beginRevision()
  findOverlayRects.value = []
  renderedText = null
  const src = mode.value === 'zh' ? zhSource() : props.source
  const out = await renderDoc(src)
  if (t !== token || request !== renderRequest) return
  html.value = out
  await nextTick()
  if (t !== token || request !== renderRequest || !article.value) return
  await typesetMath(article.value)
  if (t !== token || request !== renderRequest || !article.value) return
  renderedText = buildRenderedTextIndex(article.value)
  applyFindQuery(props.findQuery, {
    preserveQuery: preservedQuery,
    preserveRange: preservedRange,
  })
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
  findMatches = []
  findCurrent = 0
  activeFindRange = null
  void renderActive(false)
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

onMounted(() => {
  if (typeof ResizeObserver !== 'undefined' && article.value) {
    findResizeObserver = new ResizeObserver(scheduleFindRepaint)
    findResizeObserver.observe(article.value)
  }
  void renderActive(false)
})
watch(() => [props.source, props.path], reset)
watch(
  () => props.findQuery,
  (query) => applyFindQuery(query, { scrollCurrent: true }),
  { deep: true },
)
onBeforeUnmount(() => {
  token++
  renderRequest++
  window.cancelAnimationFrame(findResizeFrame)
  findResizeObserver?.disconnect()
  findResizeObserver = null
  teardownStream()
  renderedText = null
  findMatches = []
  activeFindRange = null
  findOverlayRects.value = []
  findHighlights.dispose()
})
</script>

<template>
  <div ref="scroll" class="fluid-doc-scroll">
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
    <article ref="article" class="fluid-doc" tabindex="0" v-html="html"></article>
    <div
      v-if="findOverlayRects.length > 0"
      class="fluid-doc-find-overlay"
      aria-hidden="true"
    >
      <span
        v-for="(rect, index) in findOverlayRects"
        :key="index"
        class="fluid-doc-find-hit"
        :class="{ current: rect.current }"
        :style="{
          left: `${rect.left}px`,
          top: `${rect.top}px`,
          width: `${rect.width}px`,
          height: `${rect.height}px`,
        }"
      ></span>
    </div>
  </div>
</template>
