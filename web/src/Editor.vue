<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { Compartment, EditorSelection, EditorState, type Extension } from '@codemirror/state'
import { EditorView, type Panel } from '@codemirror/view'
import {
  findNext,
  findPrevious,
  getSearchQuery,
  openSearchPanel,
  search,
  searchPanelOpen,
  setSearchQuery,
} from '@codemirror/search'
import { GhostStore } from './ghostStore'
import { ghostField, foldClickHandler, retryClickHandler, refreshGhosts } from './render/ghostField'
import { fnGutter, explainClickHandler } from './render/gutter'
import {
  explainLine as fetchExplainLine,
  streamOrientation,
  streamSelectionExplanation,
  type OrientationStream,
  type SelectionStream,
} from './api'
import { getParser } from './parser/browser'
import { readOnlyCodeViewExtensions } from './codeView'
import { GenScheduler, viewportDistance } from './scheduler'
import { buildQueryContext, type QueryContext } from './queryContext'
import OrientationCard from './OrientationCard.vue'
import SelectionPopover from './SelectionPopover.vue'
import {
  idleOrientationState,
  orientationCanActivate,
  reduceOrientationFrame,
  startOrientationRequest,
  type OrientationViewState,
} from './orientationState'
import {
  reduceSelectionFrame,
  selectionToUtf8ByteRange,
  startSelectionRequest,
  type SelectionByteRange,
  type SelectionViewState,
} from './selectionState'
import type { DeclSpan, FunctionSpan, ParserLang } from './parser/types.ts'
import type { CodeEvidenceRef, GenerationProgress, GenFrame } from './ghostTypes'
import {
  collectInFileMatches,
  createInFileSearchQuery,
  moveInFileFindCurrent,
  snapshotInFileFindState,
  type FindRange,
  type InFileFindDirection,
  type InFileFindQuery,
  type InFileFindSnapshot,
  type InFileFindSurfaceHandle,
} from './inFileFind.ts'
import {
  captureCodeReadingAnchor,
  correctedCodeAnchorScrollTop,
  normalizeCodeReadingAnchor,
  resolveCodeReadingAnchor,
  type CodeReadingAnchor,
} from './readingAnchor.ts'

interface EvidenceReveal extends CodeEvidenceRef {
  revealKey: number
}

const props = defineProps<{
  source: string
  lang: string
  path: string
  allowWeb: boolean
  revealEvidence?: EvidenceReveal | null
  findQuery?: InFileFindQuery | null
}>()
// Generation progress surfaces to the status bar (U1) via @progress; the
// per-file query context (roster + generated capsules) surfaces to QueryPanel
// via @context (S10b-cap), lifted through App as a sibling-component bridge.
const emit = defineEmits<{
  progress: [GenerationProgress]
  context: [QueryContext]
  'find-state': [InFileFindSnapshot]
  'reading-anchor': [path: string, anchor: CodeReadingAnchor]
  'reading-interaction': [path: string]
  'reading-restore-settled': [path: string]
}>()

// Push the current-file query snapshot up to QueryPanel (S10b-cap). Called on
// reset (→ empty), once the roster is parsed, and after each capsule arrives, so
// follow-ups always carry whatever has been generated so far.
function emitContext(): void {
  const orientation = orientationState.value
  const orientationId = orientationCanActivate(orientation, currentPath)
    ? orientation.card.orientationId
    : ''
  emit(
    'context',
    buildQueryContext(
      currentRoster,
      (id) => store.capsule(id)?.summary,
      orientationId,
      currentPath,
    ),
  )
}

const wrap = shallowRef<HTMLDivElement | null>(null)
const host = shallowRef<HTMLDivElement | null>(null)
// ADR-0014: the CM6 EditorView is an imperative object. Hold it in a
// shallowRef so Vue never deep-proxies its internal state. NEVER a plain ref().
const view = shallowRef<EditorView | null>(null)

const READING_ANCHOR_SCROLL_EPSILON_PX = 0.5
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
const readingAnchorCorrectionMeasureKey = {}
const readingAnchorEmitMeasureKey = {}
let readingAnchorRestoreSequence = 0
let restoredReadingAnchor: CodeReadingAnchor | null = null
let lastEmittedReadingAnchor: CodeReadingAnchor | null = null
let settledReadingAnchorRestoreSequence = -1

function captureReadingAnchorFromView(editor: EditorView): CodeReadingAnchor | null {
  const scrollerTop = editor.scrollDOM.getBoundingClientRect().top
  const topBlock = editor.lineBlockAtHeight(scrollerTop - editor.documentTop)
  const line = editor.state.doc.lineAt(topBlock.from)
  const offsetPx = editor.documentTop + editor.lineBlockAt(line.from).top - scrollerTop
  return captureCodeReadingAnchor({
    topLine: line.number,
    offsetPx,
    totalLines: editor.state.doc.lines,
  })
}

function captureReadingAnchor(): CodeReadingAnchor | null {
  const editor = view.value
  return editor ? captureReadingAnchorFromView(editor) : null
}

function sameReadingAnchor(
  left: CodeReadingAnchor | null,
  right: CodeReadingAnchor,
): boolean {
  return left !== null
    && left.topLine === right.topLine
    && left.totalLines === right.totalLines
    && Math.abs(left.offsetPx - right.offsetPx) < READING_ANCHOR_SCROLL_EPSILON_PX
}

function scheduleReadingAnchorEmit(): void {
  const editor = view.value
  if (!editor || !currentPath || restoredReadingAnchor) return
  const token = activationToken
  const filePath = currentPath
  editor.requestMeasure({
    key: readingAnchorEmitMeasureKey,
    read: (measuredView) => captureReadingAnchorFromView(measuredView),
    write: (anchor, measuredView) => {
      if (
        !anchor
        || measuredView !== view.value
        || token !== activationToken
        || filePath !== currentPath
        || sameReadingAnchor(lastEmittedReadingAnchor, anchor)
      ) return
      lastEmittedReadingAnchor = anchor
      emit('reading-anchor', filePath, anchor)
    },
  })
}

function cancelReadingAnchorRestore(): void {
  readingAnchorRestoreSequence++
  restoredReadingAnchor = null
}

function settleReadingAnchorRestore(sequence: number, filePath: string): void {
  if (
    sequence !== readingAnchorRestoreSequence
    || settledReadingAnchorRestoreSequence === sequence
    || filePath !== currentPath
  ) return
  settledReadingAnchorRestoreSequence = sequence
  emit('reading-restore-settled', filePath)
}

function onReadingAnchorUserScroll(): void {
  cancelReadingAnchorRestore()
  if (currentPath) emit('reading-interaction', currentPath)
}

function scheduleReadingAnchorCorrection(): void {
  const editor = view.value
  const anchor = restoredReadingAnchor
  if (!editor || !anchor) return

  const sequence = readingAnchorRestoreSequence
  const token = activationToken
  const filePath = currentPath
  editor.requestMeasure({
    key: readingAnchorCorrectionMeasureKey,
    read: (measuredView) => {
      if (
        measuredView !== view.value
        || sequence !== readingAnchorRestoreSequence
        || anchor !== restoredReadingAnchor
        || token !== activationToken
        || filePath !== currentPath
      ) return null

      const target = resolveCodeReadingAnchor(anchor, measuredView.state.doc.lines)
      if (!target) return null
      const line = measuredView.state.doc.line(target.lineNumber)
      const scrollerTop = measuredView.scrollDOM.getBoundingClientRect().top
      const currentOffsetPx = measuredView.documentTop
        + measuredView.lineBlockAt(line.from).top
        - scrollerTop
      return correctedCodeAnchorScrollTop({
        scrollTop: measuredView.scrollDOM.scrollTop,
        currentOffsetPx,
        savedOffsetPx: target.offsetPx,
        maxScrollTop: measuredView.scrollDOM.scrollHeight - measuredView.scrollDOM.clientHeight,
      })
    },
    write: (nextScrollTop, measuredView) => {
      if (
        nextScrollTop === null
        || measuredView !== view.value
        || sequence !== readingAnchorRestoreSequence
        || anchor !== restoredReadingAnchor
        || token !== activationToken
        || filePath !== currentPath
      ) return
      if (
        Math.abs(measuredView.scrollDOM.scrollTop - nextScrollTop)
        >= READING_ANCHOR_SCROLL_EPSILON_PX
      ) {
        measuredView.scrollDOM.scrollTop = nextScrollTop
      }
      settleReadingAnchorRestore(sequence, filePath)
    },
  })
}

function restoreReadingAnchor(anchor: CodeReadingAnchor): boolean {
  const editor = view.value
  const normalized = normalizeCodeReadingAnchor(anchor)
  if (!editor || !normalized || !resolveCodeReadingAnchor(normalized, editor.state.doc.lines)) {
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
  const editor = view.value
  if (!editor || (event.button !== 1 && event.target !== editor.scrollDOM)) return
  onReadingAnchorUserScroll()
}

function onReadingAnchorKeyDown(event: KeyboardEvent): void {
  if (READING_ANCHOR_NAVIGATION_KEYS.has(event.key)) onReadingAnchorUserScroll()
}

let suppressFindStateEmit = false
// GhostStore + scheduler are imperative too — plain (non-reactive) component state.
const store = new GhostStore()
// Viewport-aware generation scheduler (S8): orders requests by viewport
// proximity, runs a small pool of parallel sockets, re-orders on scroll. Created
// on mount with closures reading the live current-file state below.
let scheduler: GenScheduler | null = null
// File orientation is a separate, single-request activation gate. Its socket is
// cancelled on every file switch; reducer reqId + activationToken guard writes.
const orientationState = ref<OrientationViewState>(idleOrientationState())
let orientationStream: OrientationStream | null = null
let orientationRequestSeq = 0
let capsulesStartedForToken: number | null = null
// Guards async parser load against rapid file switches: each activation bumps the
// token; a stale callback (parser resolved after a switch) sees a mismatch and bails.
let activationToken = 0
// Current file's roster + path — needed to resend a single function on retry (S7.6).
let currentRoster: FunctionSpan[] = []
// Top-level declarations (S-TS-3): targets for the manual "解释这个声明" hotspot.
let currentDecls: DeclSpan[] = []
let currentPath = ''

interface SelectionTarget extends SelectionByteRange {
  from: number
  to: number
}

// S-SEL-2 transient selection UI. It never enters GhostStore and is discarded
// on selection/file changes, Esc, outside click, or unmount. The request sequence
// is a stale-write guard independent of the backend's echoed reqId.
const selectionTarget = ref<SelectionTarget | null>(null)
const selectionState = ref<SelectionViewState>({ mode: 'idle' })
const selectionAnchor = ref({ left: 8, top: 8 })
let selectionStream: SelectionStream | null = null
let selectionRequestSeq = 0

// Generation progress (S7.5) — reactive; emitted up to the status bar (U1).
const phase = ref<GenerationProgress['phase']>('idle')
const total = ref(0)
const completed = ref(0)
watch([phase, total, completed], () => {
  emit('progress', { phase: phase.value, completed: completed.value, total: total.value })
})

// Adjustable code font size (U-R2, 需求 §7.6). The .cm-scroller font-size lives in
// a Compartment so it can be reconfigured live (Ctrl+= / Ctrl+- / Ctrl+0) without
// rebuilding the editor state. Ghost notes are sized in `em` (styles.css), so they
// scale with this proportionally. Persisted to localStorage, restored on mount.
const FONT_KEY = 'fluid:fontPx'
const FONT_MIN = 9
const FONT_MAX = 28
const FONT_DEFAULT = 13
const fontCompartment = new Compartment()
const fontPx = ref(loadFontPx())

function loadFontPx(): number {
  const raw = Number(localStorage.getItem(FONT_KEY))
  return Number.isFinite(raw) && raw > 0 ? clampFont(raw) : FONT_DEFAULT
}

function clampFont(px: number): number {
  return Math.min(FONT_MAX, Math.max(FONT_MIN, Math.round(px)))
}

function fontTheme(px: number): Extension {
  return EditorView.theme({ '.cm-scroller': { fontSize: `${px}px` } })
}

// Apply a new code font size: clamp, persist, and reconfigure live.
function setFont(px: number): void {
  const next = clampFont(px)
  if (next === fontPx.value) return
  fontPx.value = next
  localStorage.setItem(FONT_KEY, String(next))
  view.value?.dispatch({ effects: fontCompartment.reconfigure(fontTheme(next)) })
}

// Ctrl+= zoom in / Ctrl+- zoom out / Ctrl+0 reset (need + handles shifted '=').
function onFontKey(e: KeyboardEvent): void {
  if (!e.ctrlKey || e.altKey || e.metaKey) return
  if (e.key === '=' || e.key === '+') {
    e.preventDefault()
    setFont(fontPx.value + 1)
  } else if (e.key === '-' || e.key === '_') {
    e.preventDefault()
    setFont(fontPx.value - 1)
  } else if (e.key === '0') {
    e.preventDefault()
    setFont(FONT_DEFAULT)
  }
}

const EMPTY_FIND_QUERY: InFileFindQuery = {
  text: '',
  mode: 'literal',
  caseSensitive: false,
}

// CodeMirror only enables its search decorations while a panel is active. Keep
// that state active through an inert hidden panel; the real controlled surface
// is owned by App in S-FIND-UI1, and the default .cm-panel.cm-search is never made.
function createControlledFindPanel(): Panel {
  const dom = document.createElement('div')
  dom.className = 'cm-fluid-find-panel'
  dom.hidden = true
  dom.setAttribute('aria-hidden', 'true')
  return { dom }
}

function ensureFindSearchState(editor: EditorView): void {
  if (!searchPanelOpen(editor.state)) openSearchPanel(editor)
}

function emitFindState(state: EditorState): void {
  emit('find-state', snapshotInFileFindState(state))
}

function selectFindRange(editor: EditorView, range: FindRange): void {
  cancelReadingAnchorRestore()
  const selection = EditorSelection.single(range.from, range.to)
  suppressFindStateEmit = true
  try {
    editor.dispatch({
      selection,
      effects: EditorView.scrollIntoView(selection.main),
      userEvent: 'select.search',
    })
  } finally {
    suppressFindStateEmit = false
  }
  scheduleReadingAnchorEmit()
  emitFindState(editor.state)
}

function applyFindQuery(query: InFileFindQuery | null | undefined): void {
  const editor = view.value
  if (!editor) return

  ensureFindSearchState(editor)
  const nextQuery = createInFileSearchQuery(query ?? EMPTY_FIND_QUERY)
  const previousQuery = getSearchQuery(editor.state)
  if (previousQuery.eq(nextQuery)) {
    emitFindState(editor.state)
    return
  }

  editor.dispatch({ effects: setSearchQuery.of(nextQuery) })
  const matches = collectInFileMatches(editor.state, nextQuery)
  if (!nextQuery.valid || matches.length === 0) {
    emitFindState(editor.state)
    return
  }
  selectFindRange(editor, matches[0])
}

function moveFind(direction: InFileFindDirection): void {
  const editor = view.value
  if (!editor) return

  const controlledQuery = createInFileSearchQuery(props.findQuery ?? EMPTY_FIND_QUERY)
  if (!getSearchQuery(editor.state).eq(controlledQuery)) applyFindQuery(props.findQuery)
  ensureFindSearchState(editor)

  const query = getSearchQuery(editor.state)
  const matches = collectInFileMatches(editor.state, query)
  const before = snapshotInFileFindState(editor.state)
  if (!query.valid || matches.length === 0) {
    emitFindState(editor.state)
    return
  }

  const expectedCurrent = moveInFileFindCurrent(before.current, matches.length, direction)
  cancelReadingAnchorRestore()
  suppressFindStateEmit = true
  let moved = false
  try {
    moved = (direction === 'next' ? findNext : findPrevious)(editor)
  } finally {
    suppressFindStateEmit = false
  }

  // CodeMirror's regexp command can remain on the same zero-width match, and
  // overlapping string matches can differ from the shared cursor contract.
  // Retain the command's normal selection/scroll behavior, then correct only a
  // semantic mismatch to the stable shared range sequence.
  const after = snapshotInFileFindState(editor.state)
  if (!moved || after.current !== expectedCurrent) {
    selectFindRange(editor, matches[expectedCurrent - 1])
    return
  }
  scheduleReadingAnchorEmit()
  emitFindState(editor.state)
}

function focusContent(): void {
  view.value?.focus()
}

interface EditorSurfaceHandle extends InFileFindSurfaceHandle {
  toggleGeneration(): void
  captureReadingAnchor(): CodeReadingAnchor | null
  restoreReadingAnchor(anchor: CodeReadingAnchor): boolean
  cancelReadingAnchorRestore(): void
}

defineExpose({
  moveFind,
  focusContent,
  toggleGeneration,
  captureReadingAnchor,
  restoreReadingAnchor,
  cancelReadingAnchorRestore,
} satisfies EditorSurfaceHandle)

function buildState(source: string, lang: string): EditorState {
  return EditorState.create({
    doc: source,
    extensions: [
      ...readOnlyCodeViewExtensions(lang, [fontCompartment.of(fontTheme(fontPx.value))]),
      EditorView.contentAttributes.of({ tabindex: '0' }),
      search({ createPanel: createControlledFindPanel }),
      ghostField(store),
      fnGutter(store),
      foldClickHandler(store),
      retryClickHandler(retry),
      explainClickHandler(explainLine),
      // Scroll → re-order the pending generation queue by the new viewport (S8).
      EditorView.updateListener.of((u) => {
        if (u.viewportChanged) scheduler?.reprioritize(viewportDist())
        if (u.viewportChanged) scheduleReadingAnchorEmit()
        if (u.geometryChanged) scheduleReadingAnchorCorrection()
        if (u.selectionSet) {
          const findSelection = u.transactions.some((transaction) => (
            transaction.isUserEvent('select.search')
          ))
          if (findSelection) {
            closeSelectionUi()
            if (!suppressFindStateEmit) emitFindState(u.state)
          } else {
            syncSelection(u.view)
          }
        }
        if (u.viewportChanged || u.geometryChanged) updateSelectionAnchor()
      }),
    ],
  })
}

function isParserLang(l: string): l is ParserLang {
  // 'ts' generates ghost annotations; 'tsx'/'js'/'jsx' are highlight-only for now
  // (JSX key-line rules are a separate slice), so they stay out of generation.
  return l === 'py' || l === 'rs' || l === 'ts'
}

function wsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  return `${proto}://${location.host}/api/generate`
}

// Build the generation request payload for one function (reqId = fn.id, the
// scheduler routes terminal frames by it). Reads the live current-file state.
function buildRequest(fnId: string): unknown {
  const fn = currentRoster.find((r) => r.id === fnId)
  const orientation = orientationState.value
  if (!orientationCanActivate(orientation, currentPath)) {
    throw new Error('capsule generation dispatched before file orientation completed')
  }
  return {
    reqId: fnId,
    filePath: currentPath,
    orientationId: orientation.card.orientationId,
    fn,
    roster: currentRoster.map((r) => r.name),
    rosterSpans: currentRoster,
    keyLines: store.keyLinesOf(fnId),
    shared: {},
  }
}

// Route one inbound generation frame to the store / progress (S7.5/S7.6).
function onFrame(frame: GenFrame): void {
  switch (frame.kind) {
    case 'capsule':
      store.putCapsule(frame.capsule)
      refresh()
      emitContext() // new summary available → refresh QueryPanel's snapshot
      break
    case 'line':
      store.putLine(frame.line)
      refresh()
      break
    case 'done':
      settle(frame.reqId, true)
      break
    case 'error':
      console.warn('[generate]', frame.reqId, frame.message)
      settle(frame.reqId, false, frame.message)
      break
    // 'cache-hit': no rendering effect (capsule/line/done frames follow).
  }
}

// Current viewport distance per function (S8 scheduling priority). Functions
// whose definition line is on screen sort first; falls back to start line when
// the view isn't ready yet.
function viewportDist(): Map<string, number> {
  const m = new Map<string, number>()
  const v = view.value
  if (!v) {
    for (const fn of currentRoster) m.set(fn.id, fn.lineRange[0])
    return m
  }
  const { from, to } = v.viewport
  const fromLine = v.state.doc.lineAt(from).number
  const toLine = v.state.doc.lineAt(to).number
  for (const fn of currentRoster) m.set(fn.id, viewportDistance(fn.lineRange[0], { fromLine, toLine }))
  return m
}

function refresh(): void {
  view.value?.dispatch({ effects: refreshGhosts.of() })
  scheduleReadingAnchorCorrection()
}

function cancelOrientationRequest(): void {
  orientationRequestSeq++
  orientationStream?.cancel()
  orientationStream = null
}

/** Render the accepted card first, then open the per-function scheduler. The
 * nextTick is part of the gate: users see the file-level coordinate system
 * before any child capsule socket can dispatch. */
async function startCapsulesAfterOrientation(token: number, filePath: string): Promise<void> {
  await nextTick()
  if (
    token !== activationToken ||
    filePath !== currentPath ||
    capsulesStartedForToken === token ||
    !orientationCanActivate(orientationState.value, filePath) ||
    !scheduler
  ) {
    return
  }

  capsulesStartedForToken = token
  for (const fn of currentRoster) store.markPending(fn.id)
  total.value = currentRoster.length
  completed.value = 0
  phase.value = currentRoster.length > 0 ? 'running' : 'idle'
  refresh()

  const ids = currentRoster.map((fn) => fn.id)
  if (ids.length > 0) {
    scheduleReadingAnchorCorrection()
    scheduler.start(ids, viewportDist())
  }
}

function requestOrientation(token: number, filePath: string): void {
  cancelOrientationRequest()
  const guard = orientationRequestSeq
  const reqId = `orientation-${token}-${guard}`
  orientationState.value = startOrientationRequest(reqId, filePath)
  orientationStream = streamOrientation(
    { reqId, filePath, rosterSpans: currentRoster },
    (frame) => {
      if (
        guard !== orientationRequestSeq ||
        token !== activationToken ||
        filePath !== currentPath
      ) {
        return
      }
      const previous = orientationState.value
      const next = reduceOrientationFrame(previous, frame)
      orientationState.value = next
      if (frame.kind === 'done' || frame.kind === 'error') orientationStream = null
      if (!orientationCanActivate(previous, filePath) && orientationCanActivate(next, filePath)) {
        emitContext() // publish the revision that binds the current-file trace
        void startCapsulesAfterOrientation(token, filePath)
      }
    },
  )
}

function retryOrientation(): void {
  if (orientationState.value.mode !== 'error' || !currentPath) return
  requestOrientation(activationToken, currentPath)
}

function cancelSelectionRequest(): void {
  selectionRequestSeq++
  selectionStream?.cancel()
  selectionStream = null
}

function closeSelectionUi(): void {
  cancelSelectionRequest()
  selectionTarget.value = null
  selectionState.value = { mode: 'idle' }
}

function syncSelection(editor: EditorView): void {
  const ranges = editor.state.selection.ranges
  if (ranges.length !== 1) {
    closeSelectionUi()
    return
  }
  const { from, to } = ranges[0]
  const range = selectionToUtf8ByteRange(editor.state.doc.toString(), from, to)
  if (!range) {
    closeSelectionUi()
    return
  }

  const previous = selectionTarget.value
  if (!previous || previous.from !== from || previous.to !== to) {
    cancelSelectionRequest()
    selectionState.value = { mode: 'idle' }
    selectionTarget.value = { ...range, from, to }
  }
  updateSelectionAnchor()
}

function updateSelectionAnchor(): void {
  const editor = view.value
  const container = wrap.value
  const target = selectionTarget.value
  if (!editor || !container || !target) return

  const start = editor.coordsAtPos(target.from)
  const end = editor.coordsAtPos(target.to)
  if (!start && !end) return
  const rect = container.getBoundingClientRect()
  const right = Math.max(start?.right ?? 0, end?.right ?? 0)
  const top = Math.min(start?.top ?? Number.POSITIVE_INFINITY, end?.top ?? Number.POSITIVE_INFINITY)
  const bottom = Math.max(start?.bottom ?? 0, end?.bottom ?? 0)
  const overlayWidth = selectionState.value.mode === 'idle' ? 70 : 420
  const overlayHeight = selectionState.value.mode === 'idle' ? 34 : 340
  const maxLeft = Math.max(8, rect.width - overlayWidth - 8)
  const left = Math.max(8, Math.min(right - rect.left + 8, maxLeft))
  let anchoredTop = bottom - rect.top + 6
  if (anchoredTop + overlayHeight > rect.height - 8) {
    anchoredTop = Math.max(8, top - rect.top - overlayHeight - 6)
  }
  selectionAnchor.value = { left, top: anchoredTop }
}

function explainSelection(forceRefresh = false): void {
  const target = selectionTarget.value
  if (!target || !currentPath) return

  cancelSelectionRequest()
  const guard = selectionRequestSeq
  const reqId = `selection-${guard}`
  const filePath = currentPath
  selectionState.value = startSelectionRequest()
  updateSelectionAnchor()
  selectionStream = streamSelectionExplanation(
    {
      reqId,
      filePath,
      startByte: target.startByte,
      endByte: target.endByte,
      rosterSpans: currentRoster,
      allowWeb: props.allowWeb,
      forceRefresh,
    },
    (frame) => {
      if (guard !== selectionRequestSeq || filePath !== currentPath) return
      selectionState.value = reduceSelectionFrame(selectionState.value, frame)
      if (frame.kind === 'done' || frame.kind === 'error') selectionStream = null
      updateSelectionAnchor()
    },
  )
}

function onSelectionKey(event: KeyboardEvent): void {
  if (event.key !== 'Escape' || !selectionTarget.value) return
  event.preventDefault()
  closeSelectionUi()
}

function onDocumentPointerDown(event: PointerEvent): void {
  if (!selectionTarget.value) return
  const target = event.target
  if (target instanceof Element && target.closest('.selection-action, .selection-popover')) return
  closeSelectionUi()
}

// Mark one function's generation finished (S7.5): advance progress once, and
// when all functions are settled, flash the done chip then fade it out. On
// failure the message is kept for the 生成失败 chip (S7.6).
function settle(fnId: string, ok: boolean, message = ''): void {
  if (!fnId) return
  if (store.statusOf(fnId) === 'pending') completed.value++
  store.settle(fnId, ok, message)
  refresh()
  if (total.value > 0 && completed.value >= total.value) {
    phase.value = 'done'
    const tk = activationToken
    window.setTimeout(() => {
      if (tk === activationToken) phase.value = 'idle'
    }, 2800)
  }
}

/** Stop queue dispatch and close every in-flight generation socket. Settled
 * capsules remain in GhostStore; only unfinished functions enter paused state. */
function pauseGeneration(): void {
  if (phase.value !== 'running') return
  scheduler?.pause()
  store.pausePending()
  phase.value = 'paused'
  refresh()
}

/** Re-arm paused functions and let current viewport distance establish the new
 * order. `extraIds` supports retrying a failed function while globally paused. */
function resumeGeneration(extraIds: string[] = []): void {
  if (phase.value !== 'paused') return
  const ids = [...new Set([...extraIds, ...store.pausedIds()])]
  if (ids.length === 0) return
  for (const id of ids) store.markPending(id)
  phase.value = 'running'
  refresh()
  scheduler?.start(ids, viewportDist())
}

function toggleGeneration(): void {
  if (phase.value === 'running') pauseGeneration()
  else if (phase.value === 'paused') resumeGeneration()
}

// Retry one failed function (S7.6): re-arm it to pending, rewind progress one
// step, and hand it back to the scheduler (jumps the queue, S8).
function retry(fnId: string): void {
  const fn = currentRoster.find((r) => r.id === fnId)
  if (!fn) return
  if (store.statusOf(fnId) === 'error' && completed.value > 0) completed.value--
  store.markPending(fnId)
  if (phase.value === 'paused') {
    resumeGeneration([fnId])
    return
  }
  phase.value = 'running'
  refresh()
  scheduler?.retry(fnId)
}

// Manual single-line fill (S9): explain one non-key line on demand via
// POST /api/explain-line, then drop the returned annotation into the store. A
// "解释中…" hotspot shows while in flight. Guarded by activationToken so a file
// switch mid-request can't apply the result to the wrong file.
async function explainLine(id: string, lineNumber: number): Promise<void> {
  // `id` is a function id (non-key line) or a decl id (top-level declaration,
  // S-TS-3). For a decl, pass it as a degenerate fn (name+span) + its kind so the
  // backend uses the decl-flavored prompt; the returned note anchors at its line.
  const fn = currentRoster.find((r) => r.id === id)
  const decl = fn ? undefined : currentDecls.find((d) => d.id === id)
  const target = fn ?? (decl && { id: decl.id, name: decl.name, lineRange: decl.lineRange })
  if (!target || store.isExplaining(id, lineNumber)) return
  const token = activationToken
  store.markExplaining(id, lineNumber)
  refresh()
  try {
    // Decl FIRST line → whole-declaration prompt (declKind); an inner line of a
    // multi-line decl → ordinary line prompt (S-TS-4), so no declKind there.
    const declKind = decl && lineNumber === decl.lineRange[0] ? decl.kind : undefined
    const line = await fetchExplainLine({
      filePath: currentPath,
      fn: target,
      lineNumber,
      declKind,
    })
    if (token !== activationToken) return // switched files mid-request
    store.putLine(line)
  } catch (e) {
    console.warn('[explain-line]', id, lineNumber, e)
  } finally {
    store.clearExplaining(id, lineNumber)
    refresh()
  }
}

// Activate a file: parse → orient → show card → schedule per-function generation.
async function activate(source: string, lang: string, path: string): Promise<void> {
  cancelReadingAnchorRestore()
  lastEmittedReadingAnchor = null
  const token = ++activationToken
  scheduler?.stop()
  cancelOrientationRequest()
  closeSelectionUi()
  store.reset()
  orientationState.value = idleOrientationState()
  capsulesStartedForToken = null
  currentRoster = []
  currentDecls = []
  currentPath = path
  phase.value = 'idle'
  total.value = 0
  completed.value = 0
  refresh()
  emitContext() // vacuum the QueryPanel snapshot on every file switch (§7)

  if (!isParserLang(lang)) return // non py/rs: read-only source only (§7 VACUUM stays bare)

  let parser
  try {
    parser = await getParser()
  } catch (e) {
    console.error('Fluid parser failed to load', e)
    return
  }
  if (token !== activationToken) return // switched files while loading

  let parsed
  try {
    parsed = parser.parse(lang, source)
  } catch (e) {
    console.error('Fluid parse failed', e)
    return
  }
  store.setRoster(parsed.roster, parsed.keyLines)
  store.setDecls(parsed.decls) // top-level decls → manual explain hotspots (S-TS-3)
  currentRoster = parsed.roster
  currentDecls = parsed.decls
  emitContext() // roster known (capsules still streaming in)
  // The roster is visible to the orientation request, but capsule placeholders
  // stay absent until a matching card+done opens the activation gate.
  total.value = parsed.roster.length
  completed.value = 0
  phase.value = 'idle'
  refresh()
  requestOrientation(token, path)
}

onMounted(() => {
  // One scheduler for the Editor's lifetime; its closures read the live
  // current-file state, and stop()/start() re-arm it on every file switch (S8).
  scheduler = new GenScheduler({ wsUrl: wsUrl(), buildRequest, onFrame })
  view.value = new EditorView({
    state: buildState(props.source, props.lang),
    parent: host.value!,
  })
  view.value.scrollDOM.addEventListener('wheel', onReadingAnchorUserScroll, { passive: true })
  view.value.scrollDOM.addEventListener('touchstart', onReadingAnchorUserScroll, { passive: true })
  view.value.scrollDOM.addEventListener('pointerdown', onReadingAnchorPointerDown)
  view.value.contentDOM.addEventListener('keydown', onReadingAnchorKeyDown)
  applyFindQuery(props.findQuery)
  window.addEventListener('keydown', onFontKey)
  window.addEventListener('keydown', onSelectionKey)
  window.addEventListener('resize', updateSelectionAnchor)
  document.addEventListener('pointerdown', onDocumentPointerDown, true)
  void activate(props.source, props.lang, props.path)
})

watch(
  () => [props.source, props.lang, props.path] as const,
  () => {
    cancelReadingAnchorRestore()
    view.value?.setState(buildState(props.source, props.lang))
    applyFindQuery(props.findQuery)
    void activate(props.source, props.lang, props.path)
  },
)

watch(
  () => props.findQuery,
  (query) => applyFindQuery(query),
  { deep: true },
)

watch(
  () => props.revealEvidence,
  async (reference) => {
    if (!reference || reference.filePath !== props.path) return
    await nextTick()
    window.requestAnimationFrame(() => {
      const editor = view.value
      if (!editor || reference.filePath !== currentPath) return
      const lineNumber = Math.min(
        Math.max(1, reference.startLine),
        editor.state.doc.lines,
      )
      const position = editor.state.doc.line(lineNumber).from
      closeSelectionUi()
      cancelReadingAnchorRestore()
      editor.dispatch({
        selection: { anchor: position },
        effects: EditorView.scrollIntoView(position, { y: 'center' }),
      })
      scheduleReadingAnchorEmit()
      editor.focus()
    })
  },
)

onBeforeUnmount(() => {
  activationToken++
  view.value?.scrollDOM.removeEventListener('wheel', onReadingAnchorUserScroll)
  view.value?.scrollDOM.removeEventListener('touchstart', onReadingAnchorUserScroll)
  view.value?.scrollDOM.removeEventListener('pointerdown', onReadingAnchorPointerDown)
  view.value?.contentDOM.removeEventListener('keydown', onReadingAnchorKeyDown)
  window.removeEventListener('keydown', onFontKey)
  window.removeEventListener('keydown', onSelectionKey)
  window.removeEventListener('resize', updateSelectionAnchor)
  document.removeEventListener('pointerdown', onDocumentPointerDown, true)
  cancelReadingAnchorRestore()
  cancelOrientationRequest()
  cancelSelectionRequest()
  scheduler?.stop()
  scheduler = null
  view.value?.destroy()
  view.value = null
})
</script>

<template>
  <div ref="wrap" class="cm-wrap">
    <OrientationCard :state="orientationState" @retry="retryOrientation" />
    <div ref="host" class="cm-host"></div>
    <button
      v-if="selectionTarget && selectionState.mode === 'idle'"
      class="selection-action"
      type="button"
      :style="{ left: `${selectionAnchor.left}px`, top: `${selectionAnchor.top}px` }"
      @pointerdown.stop
      @click="explainSelection(false)"
    >
      解释
    </button>
    <SelectionPopover
      v-else-if="selectionTarget && selectionState.mode !== 'idle'"
      :state="selectionState"
      :selected-text="selectionTarget.selectedText"
      :style="{ left: `${selectionAnchor.left}px`, top: `${selectionAnchor.top}px` }"
      @pointerdown.stop
      @close="closeSelectionUi"
      @regenerate="explainSelection(true)"
    />
  </div>
</template>
