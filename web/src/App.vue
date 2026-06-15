<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { fetchFile, fetchTree, type FileNode } from './api'
import FileTree from './FileTree.vue'
import Editor from './Editor.vue'
import QueryPanel from './QueryPanel.vue'
import ActivityBar from './shell/ActivityBar.vue'
import StatusBar from './shell/StatusBar.vue'

const files = ref<FileNode[]>([])
const current = ref<{ path: string; lang: string; source: string } | null>(null)
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

async function open(node: FileNode) {
  try {
    const source = await fetchFile(node.path)
    current.value = { path: node.path, lang: node.lang, source }
  } catch (e) {
    loadError.value = String(e)
  }
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
        <div v-if="current" class="path-bar">{{ current.path }}</div>
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
