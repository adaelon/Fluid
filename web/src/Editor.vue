<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { Compartment, EditorState, type Extension } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { basicSetup } from 'codemirror'
import { python } from '@codemirror/lang-python'
import { rust } from '@codemirror/lang-rust'
import { GhostStore } from './ghostStore'
import { ghostField, foldClickHandler, retryClickHandler, refreshGhosts } from './render/ghostField'
import { fnGutter, explainClickHandler } from './render/gutter'
import { explainLine as fetchExplainLine } from './api'
import { getParser } from './parser/browser'
import { fluidDarkTheme } from './theme'
import { GenScheduler, viewportDistance } from './scheduler'
import type { FunctionSpan, ParserLang } from './parser/types.ts'
import type { GenFrame } from './ghostTypes'

const props = defineProps<{ source: string; lang: string; path: string }>()
// Generation progress surfaces to the status bar (U1): App lifts it via @progress.
const emit = defineEmits<{
  progress: [{ phase: 'idle' | 'running' | 'done'; completed: number; total: number }]
}>()

const host = shallowRef<HTMLDivElement | null>(null)
// ADR-0014: the CM6 EditorView is an imperative object. Hold it in a
// shallowRef so Vue never deep-proxies its internal state. NEVER a plain ref().
const view = shallowRef<EditorView | null>(null)
// GhostStore + scheduler are imperative too — plain (non-reactive) component state.
const store = new GhostStore()
// Viewport-aware generation scheduler (S8): orders requests by viewport
// proximity, runs a small pool of parallel sockets, re-orders on scroll. Created
// on mount with closures reading the live current-file state below.
let scheduler: GenScheduler | null = null
// Guards async parser load against rapid file switches: each activation bumps the
// token; a stale callback (parser resolved after a switch) sees a mismatch and bails.
let activationToken = 0
// Current file's roster + path — needed to resend a single function on retry (S7.6).
let currentRoster: FunctionSpan[] = []
let currentPath = ''

// Generation progress (S7.5) — reactive; emitted up to the status bar (U1).
const phase = ref<'idle' | 'running' | 'done'>('idle')
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

function langExtension(lang: string): Extension {
  if (lang === 'py') return python()
  if (lang === 'rs') return rust()
  return []
}

function buildState(source: string, lang: string): EditorState {
  return EditorState.create({
    doc: source,
    extensions: [
      basicSetup,
      fluidDarkTheme,
      fontCompartment.of(fontTheme(fontPx.value)),
      langExtension(lang),
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      ghostField(store),
      fnGutter(store),
      foldClickHandler(store),
      retryClickHandler(retry),
      explainClickHandler(explainLine),
      // Scroll → re-order the pending generation queue by the new viewport (S8).
      EditorView.updateListener.of((u) => {
        if (u.viewportChanged) scheduler?.reprioritize(viewportDist())
      }),
    ],
  })
}

function isParserLang(l: string): l is ParserLang {
  return l === 'py' || l === 'rs'
}

function wsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  return `${proto}://${location.host}/api/generate`
}

// Build the generation request payload for one function (reqId = fn.id, the
// scheduler routes terminal frames by it). Reads the live current-file state.
function buildRequest(fnId: string): unknown {
  const fn = currentRoster.find((r) => r.id === fnId)
  return {
    reqId: fnId,
    filePath: currentPath,
    fn,
    roster: currentRoster.map((r) => r.name),
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

// Retry one failed function (S7.6): re-arm it to pending, rewind progress one
// step, and hand it back to the scheduler (jumps the queue, S8).
function retry(fnId: string): void {
  const fn = currentRoster.find((r) => r.id === fnId)
  if (!fn) return
  if (store.statusOf(fnId) === 'error' && completed.value > 0) completed.value--
  store.markPending(fnId)
  phase.value = 'running'
  refresh()
  scheduler?.retry(fnId)
}

// Manual single-line fill (S9): explain one non-key line on demand via
// POST /api/explain-line, then drop the returned annotation into the store. A
// "解释中…" hotspot shows while in flight. Guarded by activationToken so a file
// switch mid-request can't apply the result to the wrong file.
async function explainLine(fnId: string, lineNumber: number): Promise<void> {
  const fn = currentRoster.find((r) => r.id === fnId)
  if (!fn || store.isExplaining(fnId, lineNumber)) return
  const token = activationToken
  store.markExplaining(fnId, lineNumber)
  refresh()
  try {
    const line = await fetchExplainLine({ filePath: currentPath, fn, lineNumber })
    if (token !== activationToken) return // switched files mid-request
    store.putLine(line)
  } catch (e) {
    console.warn('[explain-line]', fnId, lineNumber, e)
  } finally {
    store.clearExplaining(fnId, lineNumber)
    refresh()
  }
}

// Activate a file: parse → open WS → stream per-function generation → render.
async function activate(source: string, lang: string, path: string): Promise<void> {
  scheduler?.stop()
  store.reset()
  currentRoster = []
  currentPath = path
  phase.value = 'idle'
  total.value = 0
  completed.value = 0
  refresh()
  const token = ++activationToken

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
  currentRoster = parsed.roster
  // Show "生成中" skeletons immediately (before the WS even opens) + arm progress.
  for (const fn of parsed.roster) store.markPending(fn.id)
  total.value = parsed.roster.length
  completed.value = 0
  phase.value = parsed.roster.length > 0 ? 'running' : 'idle'
  refresh()

  // Hand the roster to the scheduler, ordered by current viewport proximity (S8).
  const ids = parsed.roster.map((fn) => fn.id)
  scheduler?.start(ids, viewportDist())
}

onMounted(() => {
  // One scheduler for the Editor's lifetime; its closures read the live
  // current-file state, and stop()/start() re-arm it on every file switch (S8).
  scheduler = new GenScheduler({ wsUrl: wsUrl(), buildRequest, onFrame })
  view.value = new EditorView({
    state: buildState(props.source, props.lang),
    parent: host.value!,
  })
  window.addEventListener('keydown', onFontKey)
  void activate(props.source, props.lang, props.path)
})

watch(
  () => [props.source, props.lang, props.path] as const,
  () => {
    view.value?.setState(buildState(props.source, props.lang))
    void activate(props.source, props.lang, props.path)
  },
)

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onFontKey)
  scheduler?.stop()
  scheduler = null
  view.value?.destroy()
  view.value = null
})
</script>

<template>
  <div class="cm-wrap">
    <div ref="host" class="cm-host"></div>
  </div>
</template>
