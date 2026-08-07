<script setup lang="ts">
import { computed, nextTick, onMounted, onBeforeUnmount, ref, watch } from 'vue'
import { fetchFile, fetchTree, openFolder, pickFolder, getLlmSettings, type FileNode } from './api'
import FileTree from './FileTree.vue'
import Editor from './Editor.vue'
import MarkdownView from './MarkdownView.vue'
import InFileFindBar from './InFileFindBar.vue'
import QueryPanel from './QueryPanel.vue'
import ActivityBar from './shell/ActivityBar.vue'
import StatusBar from './shell/StatusBar.vue'
import Tabs from './shell/Tabs.vue'
import SettingsModal from './shell/SettingsModal.vue'
import CommandPalette, { type PaletteItem } from './shell/CommandPalette.vue'
import { EMPTY_QUERY_CONTEXT, type QueryContext } from './queryContext'
import type { CodeEvidenceRef, GenerationProgress } from './ghostTypes'
import type {
  InFileFindDirection,
  InFileFindQuery,
  InFileFindSnapshot,
  InFileFindSurfaceHandle,
} from './inFileFind.ts'
import {
  QUERY_DOCK_STORAGE_KEY,
  clampQueryDockHeight,
  loadQueryDockHeight,
  queryDockHeightBounds,
  queryDockHeightFromPointer,
  type QueryPresentation,
} from './queryLayout'

type OpenFile = { path: string; lang: string; source: string }

const files = ref<FileNode[]>([])
// Multi-tab model (U2): an ordered list of open files + the active one.
const openFiles = ref<OpenFile[]>([])
const activePath = ref<string | null>(null)
const current = computed<OpenFile | null>(
  () => openFiles.value.find((f) => f.path === activePath.value) ?? null,
)
// Breadcrumb segments of the active file path (U2).
const crumbs = computed<string[]>(() => current.value?.path.split('/') ?? [])
const loadError = ref<string | null>(null)

const DEFAULT_FIND_QUERY: InFileFindQuery = {
  text: '',
  mode: 'literal',
  caseSensitive: false,
}

interface InFileFindBarHandle {
  focusInput(): void
}

const editorStage = ref<HTMLElement | null>(null)
interface ActiveContentSurfaceHandle extends InFileFindSurfaceHandle {
  toggleGeneration?: () => void
}
const activeFindSurface = ref<ActiveContentSurfaceHandle | null>(null)
const findBarComponent = ref<InFileFindBarHandle | null>(null)
const findOpen = ref(false)
const findQuery = ref<InFileFindQuery>({ ...DEFAULT_FIND_QUERY })
const lastFindQuery = ref<InFileFindQuery>({ ...DEFAULT_FIND_QUERY })
const findSnapshot = ref<InFileFindSnapshot>(emptyFindSnapshot())
const activeFindQuery = computed(() => findOpen.value ? findQuery.value : null)

function emptyFindSnapshot(): InFileFindSnapshot {
  return { current: 0, total: 0, error: null }
}

function focusFindInput(): void {
  void nextTick(() => findBarComponent.value?.focusInput())
}

function openFind(): void {
  if (!current.value) return
  if (!findOpen.value) {
    findQuery.value = { ...lastFindQuery.value }
    findSnapshot.value = emptyFindSnapshot()
    findOpen.value = true
  }
  focusFindInput()
}

function closeFind(restoreFocus = true): void {
  if (!findOpen.value) return
  lastFindQuery.value = { ...findQuery.value }
  findOpen.value = false
  findSnapshot.value = emptyFindSnapshot()
  if (restoreFocus) {
    void nextTick(() => activeFindSurface.value?.focusContent())
  }
}

function updateFindQuery(query: InFileFindQuery): void {
  findQuery.value = { ...query }
  findSnapshot.value = emptyFindSnapshot()
}

function updateFindSnapshot(snapshot: InFileFindSnapshot): void {
  if (findOpen.value) findSnapshot.value = { ...snapshot }
}

function moveFind(direction: InFileFindDirection): void {
  if (
    !findOpen.value
    || findSnapshot.value.error !== null
    || findSnapshot.value.total === 0
  ) return
  activeFindSurface.value?.moveFind(direction)
}

function isFindKeyboardContext(event: KeyboardEvent): boolean {
  const stage = editorStage.value
  if (!stage || !current.value) return false
  const target = event.target instanceof Node ? event.target : document.activeElement
  return target instanceof Node && stage.contains(target)
}

function consumeFindKey(event: KeyboardEvent): void {
  event.preventDefault()
  event.stopImmediatePropagation()
}

watch(activePath, (path, previousPath) => {
  if (path !== previousPath) closeFind(false)
})

// Generation progress lifted from Editor (U1) → rendered in the status bar.
const genProgress = ref<GenerationProgress>({
  phase: 'idle',
  completed: 0,
  total: 0,
})

function toggleGeneration(): void {
  activeFindSurface.value?.toggleGeneration?.()
}

// Current-file query context lifted from Editor (S10b-cap) → handed to QueryPanel
// so follow-ups carry the roster + generated capsule summaries. Editor emits a
// fresh snapshot on switch/capsule arrival; we still null it out when no file is
// open (Editor is v-if'd away then and can't emit).
const queryCtx = ref<QueryContext>(EMPTY_QUERY_CONTEXT)

interface EvidenceReveal extends CodeEvidenceRef {
  revealKey: number
}

let evidenceRevealSequence = 0
const evidenceReveal = ref<EvidenceReveal | null>(null)
const activeEvidenceReveal = computed(() =>
  evidenceReveal.value?.filePath === current.value?.path ? evidenceReveal.value : null,
)

// S-FSQ-1: selected file set is an explicit query scope, independent from open
// tabs. Hiding checkbox mode must not clear this set; switching project root does.
const fileSelectionMode = ref(false)
const selectedFilePaths = ref<Set<string>>(new Set())
const selectedFilePathList = computed(() => Array.from(selectedFilePaths.value))
const selectedFileCount = computed(() => selectedFilePaths.value.size)

function toggleSelectedFile(path: string) {
  const next = new Set(selectedFilePaths.value)
  if (next.has(path)) next.delete(path)
  else next.add(path)
  selectedFilePaths.value = next
}

function clearSelectedFiles() {
  selectedFilePaths.value = new Set()
}

// Markdown files render as a document (MarkdownView), not in the CM6 Editor, so
// they never emit a query context. Vacuum the stale one when the active file is a
// doc (or none), so opening the query panel can't carry the last code file's
// roster. Code files are left alone — Editor emits its own fresh context on mount.
watch(
  () => current.value,
  (c) => {
    if (!c || c.lang === 'md' || queryCtx.value.filePath !== c.path) {
      queryCtx.value = EMPTY_QUERY_CONTEXT
    }
    if (!c || c.lang === 'md') {
      genProgress.value = { phase: 'idle', completed: 0, total: 0 }
    }
  },
)

// The follow-up query panel is hidden by default — the bottom space goes back to
// the code area until the user opens it from the status-bar 「💬 追问」 toggle
// (S10b dock revision). Sticky across file switches; auto-hidden when no file.
const queryPanelOpen = ref(false)
const queryPresentation = ref<QueryPresentation>('dock')
const queryFocusActive = computed(
  () => queryPanelOpen.value && Boolean(current.value) && queryPresentation.value === 'focus',
)

interface QueryPanelHandle {
  handleEscape(): boolean
}

const queryPanelComponent = ref<QueryPanelHandle | null>(null)

// S-QDOCK-1: App owns the dock geometry so opening/closing the panel does not
// discard the last user-selected size. The preferred value survives temporary
// viewport shrink; only the rendered height is re-clamped until space returns.
const queryDockViewportHeight = ref(window.innerHeight)
let queryDockPreferredHeight = loadQueryDockHeight(
  localStorage.getItem(QUERY_DOCK_STORAGE_KEY),
  queryDockViewportHeight.value,
)
const queryDockHeight = ref(queryDockPreferredHeight)
const queryDockBounds = computed(() => queryDockHeightBounds(queryDockViewportHeight.value))
const queryDockDragging = ref(false)

interface QueryDockResize {
  pointerId: number
  startY: number
  startHeight: number
  startPreferredHeight: number
}

let queryDockResize: QueryDockResize | null = null

function startQueryDockResize(e: PointerEvent): void {
  if (e.button !== 0 || queryDockResize) return
  e.preventDefault()
  queryDockResize = {
    pointerId: e.pointerId,
    startY: e.clientY,
    startHeight: queryDockHeight.value,
    startPreferredHeight: queryDockPreferredHeight,
  }
  queryDockDragging.value = true
  const handle = e.currentTarget as HTMLElement
  handle.setPointerCapture(e.pointerId)
}

function moveQueryDockResize(e: PointerEvent): void {
  const resize = queryDockResize
  if (!resize || resize.pointerId !== e.pointerId) return
  const height = queryDockHeightFromPointer(
    resize.startHeight,
    resize.startY,
    e.clientY,
    queryDockViewportHeight.value,
  )
  queryDockPreferredHeight = height
  queryDockHeight.value = height
}

function finishQueryDockResize(e: PointerEvent, persist: boolean): void {
  const resize = queryDockResize
  if (!resize || resize.pointerId !== e.pointerId) return
  queryDockResize = null
  queryDockDragging.value = false
  if (persist) {
    queryDockPreferredHeight = queryDockHeight.value
    localStorage.setItem(QUERY_DOCK_STORAGE_KEY, String(queryDockHeight.value))
  } else {
    queryDockPreferredHeight = resize.startPreferredHeight
    queryDockHeight.value = clampQueryDockHeight(
      resize.startPreferredHeight,
      queryDockViewportHeight.value,
    )
  }
  const handle = e.currentTarget as HTMLElement
  if (handle.hasPointerCapture(e.pointerId)) handle.releasePointerCapture(e.pointerId)
}

function endQueryDockResize(e: PointerEvent): void {
  finishQueryDockResize(e, true)
}

function cancelQueryDockResize(e: PointerEvent): void {
  finishQueryDockResize(e, false)
}

function loseQueryDockPointer(e: PointerEvent): void {
  const resize = queryDockResize
  if (!resize || resize.pointerId !== e.pointerId) return
  queryDockResize = null
  queryDockDragging.value = false
  queryDockPreferredHeight = resize.startPreferredHeight
  queryDockHeight.value = clampQueryDockHeight(
    resize.startPreferredHeight,
    queryDockViewportHeight.value,
  )
}

function resizeQueryDockForViewport(): void {
  queryDockViewportHeight.value = window.innerHeight
  queryDockHeight.value = clampQueryDockHeight(
    queryDockPreferredHeight,
    queryDockViewportHeight.value,
  )
}

function maximizeQueryPanel(): void {
  if (!queryPanelOpen.value || !current.value) return
  queryPresentation.value = 'focus'
}

function restoreQueryDock(): void {
  queryPresentation.value = 'dock'
}

function closeQueryPanel(): void {
  queryPresentation.value = 'dock'
  queryPanelOpen.value = false
}

function toggleQueryPanel(): void {
  if (queryPanelOpen.value) closeQueryPanel()
  else {
    queryPresentation.value = 'dock'
    queryPanelOpen.value = true
  }
}

watch(
  () => current.value,
  (file) => {
    if (!file) closeQueryPanel()
  },
)

// LLM backend settings modal (U5b, ADR-0018), opened from the activity-bar gear.
const settingsOpen = ref(false)

// S-SEL-2: user-level pre-authorization for supplier-hosted Web Search. It is
// local UI state (not an LLM credential), defaults on, and is sent with each
// selection and follow-up-query request, so both paths share one policy switch.
const ALLOW_WEB_KEY = 'fluid:allowWeb'
const allowWeb = ref(localStorage.getItem(ALLOW_WEB_KEY) !== 'false')

function setAllowWeb(value: boolean) {
  allowWeb.value = value
  localStorage.setItem(ALLOW_WEB_KEY, String(value))
}

// Command palette (U4): Ctrl/Cmd+P → fuzzy file open, Ctrl/Cmd+Shift+P → app
// commands. Null = closed. Items are rebuilt per mode from current app state.
const paletteMode = ref<'files' | 'commands' | null>(null)
const palettePlaceholder = computed(() =>
  paletteMode.value === 'commands' ? '输入命令…' : '输入文件名…',
)
const paletteItems = computed<PaletteItem[]>(() => {
  if (paletteMode.value === 'files') {
    return files.value.map((f) => ({
      id: f.path,
      label: f.path,
      hint: f.lang,
      run: () => open(f),
    }))
  }
  if (paletteMode.value === 'commands') {
    const cmds: PaletteItem[] = [
      { id: 'settings', label: '设置 · LLM 后端', run: () => (settingsOpen.value = true) },
      { id: 'open-folder', label: '打开文件夹…', run: () => void chooseFolder() },
    ]
    // Commands that need an open file are only offered when one is active.
    if (current.value) {
      cmds.push({
        id: 'toggle-query',
        label: '切换追问器',
        run: toggleQueryPanel,
      })
    }
    if (activePath.value) {
      const path = activePath.value
      cmds.push({ id: 'close-tab', label: '关闭当前标签页', run: () => closeTab(path) })
    }
    return cmds
  }
  return []
})

// Global shortcut: Ctrl/Cmd+P opens quick-open, +Shift opens the command palette.
// preventDefault stops the browser's native print/quick-find on those chords.
function onGlobalKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && queryFocusActive.value) {
    // Settings and command palette own Escape while their higher layers are open.
    if (settingsOpen.value || paletteMode.value) return
    e.preventDefault()
    e.stopImmediatePropagation()
    if (queryPanelComponent.value?.handleEscape()) return
    restoreQueryDock()
    return
  }

  const findLayerBlocked = settingsOpen.value || Boolean(paletteMode.value) || queryFocusActive.value
  const findKeyboardContext = !findLayerBlocked && !e.isComposing && isFindKeyboardContext(e)
  const findChord = (e.ctrlKey || e.metaKey)
    && !e.altKey
    && !e.shiftKey
    && e.key.toLowerCase() === 'f'
  if (findKeyboardContext && findChord) {
    consumeFindKey(e)
    openFind()
    return
  }
  if (
    findKeyboardContext
    && findOpen.value
    && e.key === 'F3'
    && !e.ctrlKey
    && !e.metaKey
    && !e.altKey
  ) {
    consumeFindKey(e)
    moveFind(e.shiftKey ? 'previous' : 'next')
    return
  }
  if (
    findKeyboardContext
    && findOpen.value
    && e.key === 'Escape'
    && !e.ctrlKey
    && !e.metaKey
    && !e.altKey
  ) {
    consumeFindKey(e)
    closeFind()
    return
  }
  if (!(e.ctrlKey || e.metaKey)) return
  if (e.key.toLowerCase() === 'p') {
    e.preventDefault()
    paletteMode.value = e.shiftKey ? 'commands' : 'files'
  }
}

// Resizable explorer sidebar (U1). Width persisted to localStorage.
const SIDEBAR_KEY = 'fluid:sidebarPx'
const SIDEBAR_MIN = 160
const SIDEBAR_MAX = 480
const sidebarWidth = ref(loadSidebarWidth())
let dragging = false

function clampSidebar(px: number): number {
  return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, Math.round(px)))
}
function loadSidebarWidth(): number {
  const raw = Number(localStorage.getItem(SIDEBAR_KEY))
  return Number.isFinite(raw) && raw > 0 ? clampSidebar(raw) : 240
}
function startResize(e: PointerEvent): void {
  dragging = true
  ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
}
function onResize(e: PointerEvent): void {
  if (!dragging) return
  // Sidebar starts after the fixed-width activity bar (48px).
  sidebarWidth.value = clampSidebar(e.clientX - 48)
}
function endResize(e: PointerEvent): void {
  if (!dragging) return
  dragging = false
  ;(e.target as HTMLElement).releasePointerCapture(e.pointerId)
  localStorage.setItem(SIDEBAR_KEY, String(sidebarWidth.value))
}

onMounted(async () => {
  window.addEventListener('keydown', onGlobalKey, true)
  window.addEventListener('resize', resizeQueryDockForViewport)
  try {
    files.value = await fetchTree()
  } catch (e) {
    loadError.value = String(e)
  }
  // First-launch nudge: if no LLM backend is configured yet, pop the settings
  // panel so generation/queries don't silently fail later (best-effort probe).
  try {
    const s = await getLlmSettings()
    if (s.keyStatus === 'unset') settingsOpen.value = true
  } catch {
    /* settings probe is best-effort — ignore */
  }
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onGlobalKey, true)
  window.removeEventListener('resize', resizeQueryDockForViewport)
})

// Open a file from the tree: if already open just activate its tab; otherwise
// fetch the source once, append a tab, and activate it (U2).
async function open(node: FileNode) {
  evidenceReveal.value = null
  if (openFiles.value.some((f) => f.path === node.path)) {
    activePath.value = node.path
    return
  }
  try {
    const source = await fetchFile(node.path)
    openFiles.value.push({ path: node.path, lang: node.lang, source })
    activePath.value = node.path
  } catch (e) {
    loadError.value = String(e)
  }
}

async function openCodeEvidence(reference: CodeEvidenceRef) {
  const node = files.value.find((file) => file.path === reference.filePath)
  if (!node) {
    loadError.value = `代码证据文件不在当前项目中：${reference.filePath}`
    return
  }
  await open(node)
  if (activePath.value !== reference.filePath) return
  evidenceReveal.value = {
    ...reference,
    revealKey: ++evidenceRevealSequence,
  }
}

function activate(path: string) {
  evidenceReveal.value = null
  activePath.value = path
}

// Open Folder (U3): switch the backend project root, then reload the tree and
// drop all open tabs (the old root's files no longer belong to this session).
const folderInput = ref('')
const switching = ref(false)

async function doSwitch(path: string) {
  if (!path || switching.value) return
  switching.value = true
  loadError.value = null
  try {
    await openFolder(path)
    openFiles.value = []
    activePath.value = null
    evidenceReveal.value = null
    fileSelectionMode.value = false
    clearSelectedFiles()
    files.value = await fetchTree()
    folderInput.value = ''
  } catch (e) {
    loadError.value = String(e)
  } finally {
    switching.value = false
  }
}

// Primary affordance (U3 revision): the local backend pops a native OS folder
// picker; the chosen absolute path then drives the root switch.
async function chooseFolder() {
  if (switching.value) return
  try {
    const path = await pickFolder()
    if (path) await doSwitch(path)
  } catch (e) {
    loadError.value = String(e)
  }
}

// Fallback: type an absolute path directly (when the native dialog is unavailable).
function switchFolder() {
  void doSwitch(folderInput.value.trim())
}

// Close a tab; if it was active, fall to the right neighbor, else the left,
// else vacuum (U2).
function closeTab(path: string) {
  const i = openFiles.value.findIndex((f) => f.path === path)
  if (i < 0) return
  openFiles.value.splice(i, 1)
  if (activePath.value !== path) return
  const next = openFiles.value[i] ?? openFiles.value[i - 1] ?? null
  activePath.value = next?.path ?? null
}
</script>

<template>
  <div class="ide-shell">
    <div
      class="ide-body"
      :aria-hidden="queryFocusActive ? 'true' : undefined"
      :inert="queryFocusActive"
    >
      <ActivityBar @open-settings="settingsOpen = true" />
      <aside class="sidebar" :style="{ width: sidebarWidth + 'px' }">
        <div class="sidebar-title">资源管理器</div>
        <button class="open-folder-pick" :disabled="switching" @click="chooseFolder">
          {{ switching ? '打开中…' : '打开文件夹…' }}
        </button>
        <form class="open-folder" @submit.prevent="switchFolder">
          <input
            v-model="folderInput"
            class="open-folder-input"
            placeholder="或输入绝对路径"
            :disabled="switching"
          />
          <button class="open-folder-btn" type="submit" :disabled="switching || !folderInput.trim()">
            {{ switching ? '…' : '打开' }}
          </button>
        </form>
        <p v-if="loadError" class="error">{{ loadError }}</p>
        <FileTree
          :files="files"
          :active="current?.path ?? null"
          :selection-mode="fileSelectionMode"
          :selected-paths="selectedFilePathList"
          @select="open"
          @toggle-selected="toggleSelectedFile"
        />
      </aside>
      <div
        class="resizer"
        @pointerdown="startResize"
        @pointermove="onResize"
        @pointerup="endResize"
      ></div>
      <main class="editor-pane">
        <Tabs
          v-if="openFiles.length"
          :tabs="openFiles"
          :active="activePath"
          @activate="activate"
          @close="closeTab"
        />
        <div v-if="current" class="path-bar">
          <span v-for="(c, i) in crumbs" :key="i" class="crumb">
            <span class="crumb-seg">{{ c }}</span>
            <span v-if="i < crumbs.length - 1" class="crumb-sep">›</span>
          </span>
        </div>
        <div ref="editorStage" class="editor-stage">
          <MarkdownView
            v-if="current && current.lang === 'md'"
            ref="activeFindSurface"
            :source="current.source"
            :path="current.path"
            :find-query="activeFindQuery"
            @find-state="updateFindSnapshot"
          />
          <Editor
            v-else-if="current"
            ref="activeFindSurface"
            :source="current.source"
            :lang="current.lang"
            :path="current.path"
            :allow-web="allowWeb"
            :reveal-evidence="activeEvidenceReveal"
            :find-query="activeFindQuery"
            @progress="genProgress = $event"
            @context="queryCtx = $event"
            @find-state="updateFindSnapshot"
          />
          <div v-else class="empty">从左侧选择一个文件以只读查看源码</div>
          <InFileFindBar
            v-if="findOpen && current"
            ref="findBarComponent"
            :query="findQuery"
            :snapshot="findSnapshot"
            @update:query="updateFindQuery"
            @move="moveFind"
            @close="closeFind()"
          />
        </div>
      </main>
    </div>
    <div
      v-if="queryPanelOpen && current"
      class="query-surface"
      :class="{
        'query-dock': queryPresentation === 'dock',
        'query-focus-shell': queryPresentation === 'focus',
        dragging: queryPresentation === 'dock' && queryDockDragging,
      }"
      :style="queryPresentation === 'dock' ? { height: queryDockHeight + 'px' } : undefined"
    >
      <div
        v-show="queryPresentation === 'dock'"
        class="query-dock-resizer"
        role="separator"
        aria-label="调整追问器高度"
        aria-orientation="horizontal"
        :aria-valuemin="queryDockBounds.min"
        :aria-valuemax="queryDockBounds.max"
        :aria-valuenow="queryDockHeight"
        @pointerdown="startQueryDockResize"
        @pointermove="moveQueryDockResize"
        @pointerup="endQueryDockResize"
        @pointercancel="cancelQueryDockResize"
        @lostpointercapture="loseQueryDockPointer"
      ></div>
      <QueryPanel
        ref="queryPanelComponent"
        :path="current.path"
        :ctx="queryCtx"
        :selection-mode="fileSelectionMode"
        :selected-count="selectedFileCount"
        :selected-paths="selectedFilePathList"
        :allow-web="allowWeb"
        :presentation="queryPresentation"
        @close="closeQueryPanel"
        @maximize="maximizeQueryPanel"
        @restore="restoreQueryDock"
        @toggle-selection-mode="fileSelectionMode = !fileSelectionMode"
        @clear-selected="clearSelectedFiles"
        @open-evidence="openCodeEvidence"
      />
    </div>
    <StatusBar
      :path="current?.path ?? null"
      :lang="current?.lang ?? null"
      :progress="genProgress"
      :query-open="queryPanelOpen"
      :aria-hidden="queryFocusActive ? 'true' : undefined"
      :inert="queryFocusActive"
      @toggle-query="toggleQueryPanel"
      @toggle-generation="toggleGeneration"
    />
    <SettingsModal
      v-if="settingsOpen"
      :allow-web="allowWeb"
      @allow-web-change="setAllowWeb"
      @close="settingsOpen = false"
    />
    <CommandPalette
      v-if="paletteMode"
      :items="paletteItems"
      :placeholder="palettePlaceholder"
      @close="paletteMode = null"
    />
  </div>
</template>
