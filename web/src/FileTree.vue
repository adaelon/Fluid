<script setup lang="ts">
import { computed } from 'vue'
import type { FileNode } from './api'
import TreeNode from './TreeNode.vue'
import { buildTree, type TreeEntry } from './tree'

const props = defineProps<{
  files: FileNode[]
  active: string | null
  selectionMode: boolean
  selectedPaths: string[]
}>()
const emit = defineEmits<{
  select: [node: FileNode]
  toggleSelected: [path: string]
}>()

const tree = computed<TreeEntry[]>(() => buildTree(props.files))
</script>

<template>
  <ul class="tree-root">
    <TreeNode
      v-for="entry in tree"
      :key="entry.kind === 'dir' ? 'd:' + entry.name : 'f:' + entry.path"
      :entry="entry"
      :active="active"
      :selection-mode="selectionMode"
      :selected-paths="selectedPaths"
      @select="(n: FileNode) => emit('select', n)"
      @toggle-selected="(path: string) => emit('toggleSelected', path)"
    />
  </ul>
</template>
