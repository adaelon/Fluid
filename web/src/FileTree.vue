<script setup lang="ts">
import { computed } from 'vue'
import type { FileNode } from './api'
import TreeNode from './TreeNode.vue'
import {
  buildTree,
  type DirectoryExpansionChange,
  type TreeEntry,
} from './tree'

const props = defineProps<{
  files: FileNode[]
  active: string | null
  expandedDirectories: ReadonlySet<string>
  selectionMode: boolean
  selectedPaths: string[]
}>()
const emit = defineEmits<{
  select: [node: FileNode]
  toggleSelected: [path: string]
  'set-directory-expanded': [change: DirectoryExpansionChange]
}>()

const tree = computed<TreeEntry[]>(() => buildTree(props.files))
</script>

<template>
  <ul class="tree-root">
    <TreeNode
      v-for="entry in tree"
      :key="entry.kind === 'dir' ? 'd:' + entry.path : 'f:' + entry.path"
      :entry="entry"
      :active="active"
      :expanded-directories="expandedDirectories"
      :selection-mode="selectionMode"
      :selected-paths="selectedPaths"
      @select="(n: FileNode) => emit('select', n)"
      @toggle-selected="(path: string) => emit('toggleSelected', path)"
      @set-directory-expanded="
        (change: DirectoryExpansionChange) => emit('set-directory-expanded', change)
      "
    />
  </ul>
</template>
