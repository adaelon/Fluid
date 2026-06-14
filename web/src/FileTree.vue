<script setup lang="ts">
import { computed } from 'vue'
import type { FileNode } from './api'
import TreeNode from './TreeNode.vue'
import { buildTree, type TreeEntry } from './tree'

const props = defineProps<{ files: FileNode[]; active: string | null }>()
const emit = defineEmits<{ select: [node: FileNode] }>()

const tree = computed<TreeEntry[]>(() => buildTree(props.files))
</script>

<template>
  <ul class="tree-root">
    <TreeNode
      v-for="entry in tree"
      :key="entry.kind === 'dir' ? 'd:' + entry.name : 'f:' + entry.path"
      :entry="entry"
      :active="active"
      @select="(n: FileNode) => emit('select', n)"
    />
  </ul>
</template>
