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
import {
  captureMarkdownReadingAnchor,
  correctedMarkdownAnchorScrollTop,
  indexMarkdownContentBlocks,
  normalizeMarkdownReadingAnchor,
  resolveMarkdownReadingAnchor,
  type MarkdownBlockIdentity,
  type MarkdownReadingAnchor,
} from './readingAnchor.ts'

const props = defineProps<{
  source: string
  path: string
  findQuery?: InFileFindQuery | null
}>()
const emit = defineEmits<{
  'find-state': [InFileFindSnapshot]
  'reading-anchor': [path: string, anchor: MarkdownReadingAnchor]
  'reading-interaction': [path: string]
  'reading-restore-settled': [path: string]
}>()

const html = ref('')
const scroll = ref<HTMLElement | null>(null)
const head = ref<HTMLElement | null>(null)
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

const MARKDOWN_CONTENT_BLOCK_SELECTOR = 'h1,h2,h3,h4,h5,h6,p,li,pre,blockquote,table'
const READING_ANCHOR_SCROLL_EPSILON_PX = 0.5
const READING_ANCHOR_RATIO_EPSILON = 0.000_001
const READING_ANCHOR_NAVIGATION_KEYS = new Set([
  'ArrowUp',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'PageUp',
  'PageDown',
  'Home',
  'End',
  ' ',
  'Spacebar',
])

interface IndexedMarkdownBlock {
  element: HTMLElement
  identity: MarkdownBlockIdentity
}

interface MeasuredMarkdownBlock extends IndexedMarkdownBlock {
  rect: DOMRect
}

let renderedAnchorBlocks: IndexedMarkdownBlock[] = []
let readingAnchorRestoreSequence = 0
let restoredReadingAnchor: MarkdownReadingAnchor | null = null
let lastEmittedReadingAnchor: MarkdownReadingAnchor | null = null
let settledReadingAnchorRestoreSequence = -1
let readingAnchorCorrectionFrame = 0
let readingAnchorEmitFrame = 0

function zhSource(): string {
  return zhChunks.value.join('')
}

function rebuildMarkdownAnchorBlocks(): void {
  const root = article.value
  if (!root) {
    renderedAnchorBlocks = []
    return
  }

  const elements = Array.from(
    root.querySelectorAll<HTMLElement>(MARKDOWN_CONTENT_BLOCK_SELECTOR),
  ).filter((element) => element.getClientRects().length > 0)
  const identities = indexMarkdownContentBlocks(elements.map((element) => element.innerText))
  renderedAnchorBlocks = elements.flatMap((element, index) => {
    const identity = identities[index]
    return identity ? [{ element, identity }] : []
  })
}

function measuredMarkdownBlocks(): MeasuredMarkdownBlock[] {
  return renderedAnchorBlocks.flatMap((block) => {
    if (!block.element.isConnected || block.element.getClientRects().length === 0) return []
    const rect = block.element.getBoundingClientRect()
    return rect.width > 0 && rect.height > 0 ? [{ ...block, rect }] : []
  })
}

function markdownContentViewportTop(scroller: HTMLElement): number {
  const viewport = scroller.getBoundingClientRect()
  const stickyBottom = head.value?.getBoundingClientRect().bottom ?? viewport.top
  return Math.min(viewport.bottom, Math.max(viewport.top, stickyBottom))
}

function topVisibleMarkdownBlock(
  blocks: readonly MeasuredMarkdownBlock[],
  contentViewportTop: number,
  viewportBottom: number,
): MeasuredMarkdownBlock | null {
  const visible = blocks.filter((block) => (
    block.rect.bottom > contentViewportTop && block.rect.top < viewportBottom
  ))
  visible.sort((left, right) => {
    const leftIntersectsTop = left.rect.top <= contentViewportTop
    const rightIntersectsTop = right.rect.top <= contentViewportTop
    if (leftIntersectsTop !== rightIntersectsTop) return leftIntersectsTop ? -1 : 1
    if (leftIntersectsTop) {
      return (right.rect.top - left.rect.top) || (left.rect.height - right.rect.height)
    }
    return (left.rect.top - right.rect.top) || (left.rect.height - right.rect.height)
  })
  return visible[0] ?? null
}

function captureReadingAnchor(): MarkdownReadingAnchor | null {
  const scroller = scroll.value
  if (!scroller || !renderedText) return null
  const viewport = scroller.getBoundingClientRect()
  const contentViewportTop = markdownContentViewportTop(scroller)
  const block = topVisibleMarkdownBlock(
    measuredMarkdownBlocks(),
    contentViewportTop,
    viewport.bottom,
  )
  if (!block) return null

  return captureMarkdownReadingAnchor({
    ...block.identity,
    offsetPx: block.rect.top - contentViewportTop,
    scrollTop: scroller.scrollTop,
    scrollHeight: scroller.scrollHeight,
    clientHeight: scroller.clientHeight,
  })
}

function sameReadingAnchor(
  left: MarkdownReadingAnchor | null,
  right: MarkdownReadingAnchor,
): boolean {
  return left !== null
    && left.blockDigest === right.blockDigest
    && left.occurrence === right.occurrence
    && Math.abs(left.offsetPx - right.offsetPx) < READING_ANCHOR_SCROLL_EPSILON_PX
    && Math.abs(left.scrollRatio - right.scrollRatio) < READING_ANCHOR_RATIO_EPSILON
}

function scheduleReadingAnchorEmit(): void {
  if (restoredReadingAnchor) return
  const fileToken = token
  const request = renderRequest
  const filePath = props.path
  window.cancelAnimationFrame(readingAnchorEmitFrame)
  readingAnchorEmitFrame = window.requestAnimationFrame(() => {
    readingAnchorEmitFrame = 0
    if (fileToken !== token || request !== renderRequest || filePath !== props.path) return
    const anchor = captureReadingAnchor()
    if (!anchor || sameReadingAnchor(lastEmittedReadingAnchor, anchor)) return
    lastEmittedReadingAnchor = anchor
    emit('reading-anchor', filePath, anchor)
  })
}

function cancelReadingAnchorRestore(): void {
  readingAnchorRestoreSequence++
  restoredReadingAnchor = null
  window.cancelAnimationFrame(readingAnchorCorrectionFrame)
  readingAnchorCorrectionFrame = 0
}

function settleReadingAnchorRestore(sequence: number, filePath: string): void {
  if (
    sequence !== readingAnchorRestoreSequence
    || settledReadingAnchorRestoreSequence === sequence
    || filePath !== props.path
  ) return
  settledReadingAnchorRestoreSequence = sequence
  emit('reading-restore-settled', filePath)
}

function onReadingAnchorUserScroll(): void {
  cancelReadingAnchorRestore()
  emit('reading-interaction', props.path)
}

function scheduleReadingAnchorCorrection(): void {
  const anchor = restoredReadingAnchor
  if (!renderedText || !anchor || mode.value !== 'en') return
  const sequence = readingAnchorRestoreSequence
  const fileToken = token
  const request = renderRequest
  const filePath = props.path
  window.cancelAnimationFrame(readingAnchorCorrectionFrame)
  readingAnchorCorrectionFrame = window.requestAnimationFrame(() => {
    readingAnchorCorrectionFrame = 0
    const scroller = scroll.value
    if (
      !scroller
      || !renderedText
      || anchor !== restoredReadingAnchor
      || sequence !== readingAnchorRestoreSequence
      || fileToken !== token
      || request !== renderRequest
      || filePath !== props.path
      || mode.value !== 'en'
    ) return

    const blocks = measuredMarkdownBlocks()
    const resolved = resolveMarkdownReadingAnchor(
      anchor,
      blocks.map((block) => block.identity),
    )
    if (!resolved) return
    const contentViewportTop = markdownContentViewportTop(scroller)
    const target = resolved.mode === 'block' ? blocks[resolved.blockIndex] : null
    const currentOffsetPx = target ? target.rect.top - contentViewportTop : null
    const nextScrollTop = correctedMarkdownAnchorScrollTop(resolved, {
      scrollTop: scroller.scrollTop,
      currentOffsetPx,
      maxScrollTop: scroller.scrollHeight - scroller.clientHeight,
    })
    if (nextScrollTop === null) return
    if (Math.abs(scroller.scrollTop - nextScrollTop) >= READING_ANCHOR_SCROLL_EPSILON_PX) {
      scroller.scrollTop = nextScrollTop
    }
    settleReadingAnchorRestore(sequence, filePath)
  })
}

function restoreReadingAnchor(anchor: MarkdownReadingAnchor): boolean {
  const normalized = normalizeMarkdownReadingAnchor(anchor)
  if (!normalized || !scroll.value || mode.value !== 'en') {
    cancelReadingAnchorRestore()
    return false
  }

  readingAnchorRestoreSequence++
  restoredReadingAnchor = normalized
  settledReadingAnchorRestoreSequence = -1
  scheduleReadingAnchorCorrection()
  return true
}

function onReadingAnchorPointerDown(event: PointerEvent): void {
  const scroller = scroll.value
  if (!scroller || (event.button !== 1 && event.target !== scroller)) return
  onReadingAnchorUserScroll()
}

function onReadingAnchorKeyDown(event: KeyboardEvent): void {
  if (READING_ANCHOR_NAVIGATION_KEYS.has(event.key)) onReadingAnchorUserScroll()
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
  if (scrollCurrent) cancelReadingAnchorRestore()
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

function onArticleResize(): void {
  scheduleFindRepaint()
  scheduleReadingAnchorCorrection()
  scheduleReadingAnchorEmit()
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
  cancelReadingAnchorRestore()
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

interface MarkdownSurfaceHandle extends InFileFindSurfaceHandle {
  captureReadingAnchor(): MarkdownReadingAnchor | null
  restoreReadingAnchor(anchor: MarkdownReadingAnchor): boolean
  cancelReadingAnchorRestore(): void
}

defineExpose({
  moveFind,
  focusContent,
  captureReadingAnchor,
  restoreReadingAnchor,
  cancelReadingAnchorRestore,
} satisfies MarkdownSurfaceHandle)

// Render whichever source the current mode selects (en original / zh-so-far).
async function renderActive(preserveCurrent = true): Promise<void> {
  const t = token
  const request = ++renderRequest
  const preservedQuery = appliedFindQuery
  const preservedRange = preserveCurrent ? activeFindRange : null
  findRevision = findHighlights.beginRevision()
  findOverlayRects.value = []
  renderedText = null
  renderedAnchorBlocks = []
  const src = mode.value === 'zh' ? zhSource() : props.source
  const out = await renderDoc(src)
  if (t !== token || request !== renderRequest) return
  html.value = out
  await nextTick()
  if (t !== token || request !== renderRequest || !article.value) return
  await typesetMath(article.value)
  if (t !== token || request !== renderRequest || !article.value) return
  rebuildMarkdownAnchorBlocks()
  renderedText = buildRenderedTextIndex(article.value)
  applyFindQuery(props.findQuery, {
    preserveQuery: preservedQuery,
    preserveRange: preservedRange,
  })
  scheduleReadingAnchorCorrection()
}

function teardownStream(): void {
  stream?.cancel()
  stream = null
}

// Reset to the English original on every file switch (vacuum the translation state).
function reset(): void {
  cancelReadingAnchorRestore()
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
  lastEmittedReadingAnchor = null
  renderedAnchorBlocks = []
  void renderActive(false)
}

async function showOriginal(): Promise<void> {
  cancelReadingAnchorRestore()
  if (mode.value === 'en') return
  mode.value = 'en'
  await renderActive() // the stream (if any) keeps running in the background
}

function showChinese(): void {
  cancelReadingAnchorRestore()
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
    findResizeObserver = new ResizeObserver(onArticleResize)
    findResizeObserver.observe(article.value)
  }
  scroll.value?.addEventListener('scroll', scheduleReadingAnchorEmit, { passive: true })
  scroll.value?.addEventListener('wheel', onReadingAnchorUserScroll, { passive: true })
  scroll.value?.addEventListener('touchstart', onReadingAnchorUserScroll, { passive: true })
  scroll.value?.addEventListener('pointerdown', onReadingAnchorPointerDown)
  scroll.value?.addEventListener('keydown', onReadingAnchorKeyDown)
  void renderActive(false)
})
watch(() => [props.source, props.path], reset)
watch(
  () => props.findQuery,
  (query) => {
    cancelReadingAnchorRestore()
    applyFindQuery(query, { scrollCurrent: true })
  },
  { deep: true },
)
onBeforeUnmount(() => {
  token++
  renderRequest++
  window.cancelAnimationFrame(findResizeFrame)
  window.cancelAnimationFrame(readingAnchorEmitFrame)
  scroll.value?.removeEventListener('scroll', scheduleReadingAnchorEmit)
  scroll.value?.removeEventListener('wheel', onReadingAnchorUserScroll)
  scroll.value?.removeEventListener('touchstart', onReadingAnchorUserScroll)
  scroll.value?.removeEventListener('pointerdown', onReadingAnchorPointerDown)
  scroll.value?.removeEventListener('keydown', onReadingAnchorKeyDown)
  cancelReadingAnchorRestore()
  findResizeObserver?.disconnect()
  findResizeObserver = null
  teardownStream()
  renderedText = null
  renderedAnchorBlocks = []
  findMatches = []
  activeFindRange = null
  findOverlayRects.value = []
  findHighlights.dispose()
})
</script>

<template>
  <div ref="scroll" class="fluid-doc-scroll">
    <div ref="head" class="fluid-doc-head">
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
