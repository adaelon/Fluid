<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { fetchFile, fetchTree, type FileNode } from './api'
import FileTree from './FileTree.vue'
import Editor from './Editor.vue'
import QueryPanel from './QueryPanel.vue'
import ActivityBar from './shell/ActivityBar.vue'
import StatusBar from './shell/StatusBar.vue'
import Tabs from './shell/Tabs.vue'

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

// Generation progress lifted from Editor (U1) → rendered in the status bar.
const genProgress = ref<{ phase: 'idle' | 'running' | 'done'; completed: number; total: number }>({
  phase: 'idle',
  completed: 0,
  total: 0,
})

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
  try {
    files.value = await fetchTree()
  } catch (e) {
    loadError.value = String(e)
  }
})

// Open a file from the tree: if already open just activate its tab; otherwise
// fetch the source once, append a tab, and activate it (U2).
async function open(node: FileNode) {
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

function activate(path: string) {
  activePath.value = path
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
    <div class="ide-body">
      <ActivityBar />
      <aside class="sidebar" :style="{ width: sidebarWidth + 'px' }">
        <div class="sidebar-title">资源管理器</div>
        <p v-if="loadError" class="error">{{ loadError }}</p>
        <FileTree :files="files" :active="current?.path ?? null" @select="open" />
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
        <Editor
          v-if="current"
          :source="current.source"
          :lang="current.lang"
          :path="current.path"
          @progress="genProgress = $event"
        />
        <div v-else class="empty">从左侧选择一个文件以只读查看源码</div>
      </main>
      <QueryPanel />
    </div>
    <StatusBar :path="current?.path ?? null" :lang="current?.lang ?? null" :progress="genProgress" />
  </div>
</template>
