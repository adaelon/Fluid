<script setup lang="ts">
import { ref } from 'vue'
import type { FileNode, Lang } from './api'
import type { TreeEntry } from './tree'

defineProps<{ entry: TreeEntry; active: string | null }>()
const emit = defineEmits<{ select: [node: FileNode] }>()

const open = ref(true)

function selectFile(path: string, name: string, lang: string) {
  emit('select', { path, name, lang: lang as Lang })
}
</script>

<template>
  <li v-if="entry.kind === 'dir'" class="node dir">
    <div class="row" @click="open = !open">
      <span class="caret">{{ open ? '▾' : '▸' }}</span>
      <span class="label">{{ entry.name }}</span>
    </div>
    <ul v-show="open" class="children">
      <TreeNode
        v-for="child in entry.children"
        :key="child.kind === 'dir' ? 'd:' + child.name : 'f:' + child.path"
        :entry="child"
        :active="active"
        @select="(n: FileNode) => emit('select', n)"
      />
    </ul>
  </li>
  <li
    v-else
    class="node file"
    :class="{ active: entry.path === active }"
    @click="selectFile(entry.path, entry.name, entry.lang)"
  >
    <span class="label">{{ entry.name }}</span>
  </li>
</template>
