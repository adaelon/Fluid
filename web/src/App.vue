<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { fetchFile, fetchTree, type FileNode } from './api'
import FileTree from './FileTree.vue'
import Editor from './Editor.vue'
import QueryPanel from './QueryPanel.vue'

const files = ref<FileNode[]>([])
const current = ref<{ path: string; lang: string; source: string } | null>(null)
const loadError = ref<string | null>(null)

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
  <div class="layout">
    <aside class="sidebar">
      <div class="sidebar-title">Fluid</div>
      <p v-if="loadError" class="error">{{ loadError }}</p>
      <FileTree :files="files" :active="current?.path ?? null" @select="open" />
    </aside>
    <main class="editor-pane">
      <div v-if="current" class="path-bar">{{ current.path }}</div>
      <Editor v-if="current" :source="current.source" :lang="current.lang" :path="current.path" />
      <div v-else class="empty">从左侧选择一个文件以只读查看源码</div>
    </main>
    <QueryPanel />
  </div>
</template>
