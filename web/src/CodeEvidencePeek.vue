<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, shallowRef, watch } from 'vue'
import { EditorState, type Extension, type Range } from '@codemirror/state'
import { Decoration, EditorView } from '@codemirror/view'
import { fetchFile } from './api'
import { codeLanguageTagFromPath, readOnlyCodeViewExtensions } from './codeView'
import {
  codePeekCenterLine,
  completeCodePeekRequest,
  disposeCodePeekState,
  failCodePeekRequest,
  idleCodePeekState,
  selectCodePeekTarget,
  type CodePeekLoadState,
} from './codePeekState'
import type { CodeEvidenceRef } from './ghostTypes'

const props = defineProps<{ target: CodeEvidenceRef }>()
const emit = defineEmits<{ close: [] }>()

const host = shallowRef<HTMLDivElement | null>(null)
const view = shallowRef<EditorView | null>(null)
const loadState = shallowRef<CodePeekLoadState>(idleCodePeekState())
const visibleTarget = computed(() =>
  loadState.value.mode === 'idle' ? props.target : loadState.value.target,
)

let requestSequence = 0
let disposed = false
let scrollFrame: number | null = null

const evidenceLine = Decoration.line({ attributes: { class: 'cm-code-peek-evidence-line' } })
const evidenceTheme = EditorView.theme({
  '.cm-code-peek-evidence-line': {
    backgroundColor: 'var(--accent-soft)',
    boxShadow: 'inset 3px 0 0 var(--accent)',
  },
})

function targetSnapshot(target: CodeEvidenceRef): CodeEvidenceRef {
  return { ...target }
}

function evidenceRangeExtension(target: CodeEvidenceRef): Extension {
  return EditorView.decorations.compute([], (state) => {
    const ranges: Range<Decoration>[] = []
    const startLine = Math.max(1, target.startLine)
    const endLine = Math.min(target.endLine, state.doc.lines)
    for (let lineNumber = startLine; lineNumber <= endLine; lineNumber++) {
      ranges.push(evidenceLine.range(state.doc.line(lineNumber).from))
    }
    return Decoration.set(ranges)
  })
}

function buildPeekEditorState(
  state: Extract<CodePeekLoadState, { mode: 'ready' }>,
): EditorState {
  return EditorState.create({
    doc: state.source,
    extensions: [
      ...readOnlyCodeViewExtensions(state.lang),
      evidenceRangeExtension(state.target),
      evidenceTheme,
      EditorView.contentAttributes.of({ 'aria-label': '只读代码证据' }),
    ],
  })
}

async function renderReady(state: Extract<CodePeekLoadState, { mode: 'ready' }>): Promise<void> {
  await nextTick()
  if (disposed || loadState.value !== state || !host.value) return

  const editorState = buildPeekEditorState(state)
  if (view.value) view.value.setState(editorState)
  else view.value = new EditorView({ state: editorState, parent: host.value })

  if (scrollFrame !== null) window.cancelAnimationFrame(scrollFrame)
  scrollFrame = window.requestAnimationFrame(() => {
    scrollFrame = null
    const editor = view.value
    if (disposed || loadState.value !== state || !editor) return
    const centerLine = codePeekCenterLine(state.target)
    const centerPosition = editor.state.doc.line(centerLine).from
    editor.dispatch({
      effects: EditorView.scrollIntoView(centerPosition, { y: 'center' }),
    })
  })
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

async function loadTarget(target: CodeEvidenceRef): Promise<void> {
  const requestId = ++requestSequence
  const next = selectCodePeekTarget(loadState.value, target, requestId)
  loadState.value = next

  if (next.mode === 'ready') {
    await renderReady(next)
    return
  }
  if (next.mode !== 'loading' || next.requestId !== requestId) return

  try {
    const source = await fetchFile(target.filePath)
    if (disposed) return
    const current = loadState.value
    const accepted = completeCodePeekRequest(
      current,
      requestId,
      source,
      codeLanguageTagFromPath(target.filePath),
    )
    if (accepted === current) return
    loadState.value = accepted
    if (accepted.mode === 'ready') await renderReady(accepted)
  } catch (error) {
    if (disposed) return
    const current = loadState.value
    const accepted = failCodePeekRequest(current, requestId, errorMessage(error))
    if (accepted !== current) loadState.value = accepted
  }
}

function retry(): void {
  const state = loadState.value
  if (state.mode === 'error') void loadTarget(targetSnapshot(state.target))
}

watch(
  () => [
    props.target.id,
    props.target.filePath,
    props.target.startLine,
    props.target.endLine,
    props.target.symbol,
  ] as const,
  () => void loadTarget(targetSnapshot(props.target)),
  { immediate: true },
)

onBeforeUnmount(() => {
  disposed = true
  requestSequence++
  loadState.value = disposeCodePeekState(loadState.value)
  if (scrollFrame !== null) window.cancelAnimationFrame(scrollFrame)
  scrollFrame = null
  view.value?.destroy()
  view.value = null
})
</script>

<template>
  <aside
    class="code-evidence-peek"
    data-testid="code-evidence-peek"
    :data-state="loadState.mode"
    :aria-busy="loadState.mode === 'loading'"
    aria-label="代码证据预览"
  >
    <header class="code-peek-head">
      <div class="code-peek-title">
        <strong :title="visibleTarget.filePath">{{ visibleTarget.filePath }}</strong>
        <span>
          {{ visibleTarget.id }} · L{{ visibleTarget.startLine }}–L{{ visibleTarget.endLine }}
          <template v-if="visibleTarget.symbol"> · {{ visibleTarget.symbol }}</template>
        </span>
      </div>
      <button type="button" aria-label="关闭代码证据预览" @click="emit('close')">×</button>
    </header>

    <div class="code-peek-body">
      <div
        ref="host"
        v-show="loadState.mode === 'ready'"
        class="code-peek-host"
      ></div>
      <p v-if="loadState.mode === 'loading'" class="code-peek-status" role="status">
        正在读取完整源码…
      </p>
      <div v-else-if="loadState.mode === 'error'" class="code-peek-error" role="alert">
        <p>{{ loadState.message }}</p>
        <button type="button" @click="retry">重试</button>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.code-evidence-peek {
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  color: var(--text);
  background: var(--bg);
  border-left: 1px solid var(--border);
}

.code-peek-head {
  flex: none;
  min-height: 48px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 7px 10px 7px 12px;
  background: var(--panel);
  border-bottom: 1px solid var(--border);
}

.code-peek-title {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.code-peek-title strong,
.code-peek-title span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.code-peek-title strong {
  font-family: var(--mono);
  font-size: 12px;
  font-weight: 600;
}

.code-peek-title span {
  color: var(--muted);
  font-size: 11px;
}

.code-peek-head button,
.code-peek-error button {
  border: 1px solid var(--border);
  border-radius: 4px;
  color: var(--text);
  background: var(--btn);
  cursor: pointer;
}

.code-peek-head button {
  flex: none;
  width: 26px;
  height: 26px;
  padding: 0;
  font-size: 18px;
  line-height: 1;
}

.code-peek-head button:hover,
.code-peek-error button:hover {
  background: var(--btn-hover);
}

.code-peek-body,
.code-peek-host {
  min-height: 0;
  flex: 1;
}

.code-peek-body {
  position: relative;
  display: flex;
  overflow: hidden;
}

.code-peek-host {
  width: 100%;
  overflow: hidden;
}

.code-peek-host :deep(.cm-editor),
.code-peek-host :deep(.cm-scroller) {
  height: 100%;
}

.code-peek-status,
.code-peek-error {
  margin: auto;
  max-width: min(420px, calc(100% - 32px));
  color: var(--muted);
  font-size: 12px;
  text-align: center;
}

.code-peek-error p {
  margin: 0 0 10px;
  color: var(--danger);
  overflow-wrap: anywhere;
}

.code-peek-error button {
  padding: 4px 12px;
  font-size: 12px;
}
</style>
