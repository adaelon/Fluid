<script setup lang="ts">
import { computed } from 'vue'
import type { FileNode, Lang } from './api'
import type { DirectoryExpansionChange, TreeEntry } from './tree'

const props = defineProps<{
  entry: TreeEntry
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

const expanded = computed(
  () => props.entry.kind === 'dir'
    && props.expandedDirectories.has(props.entry.path),
)
const selected = computed(
  () => props.entry.kind === 'file' && props.selectedPaths.includes(props.entry.path),
)

function toggleDirectory() {
  if (props.entry.kind !== 'dir') return
  emit('set-directory-expanded', {
    path: props.entry.path,
    expanded: !props.expandedDirectories.has(props.entry.path),
  })
}

function selectFile(path: string, name: string, lang: string) {
  emit('select', { path, name, lang: lang as Lang })
}

function toggleSelectedFile(path: string) {
  emit('toggleSelected', path)
}
</script>

<template>
  <li v-if="entry.kind === 'dir'" class="node dir">
    <div class="row" @click="toggleDirectory">
      <span class="caret">{{ expanded ? '▾' : '▸' }}</span>
      <span class="label">{{ entry.name }}</span>
    </div>
    <ul v-show="expanded" class="children">
      <TreeNode
        v-for="child in entry.children"
        :key="child.kind === 'dir' ? 'd:' + child.path : 'f:' + child.path"
        :entry="child"
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
  </li>
  <li
    v-else
    class="node file"
    :class="{ active: entry.path === active, selectable: selectionMode, selected }"
    @click="selectFile(entry.path, entry.name, entry.lang)"
  >
    <input
      v-if="selectionMode"
      class="file-select"
      type="checkbox"
      :checked="selected"
      :aria-label="`选择 ${entry.name}`"
      @click.stop
      @change="toggleSelectedFile(entry.path)"
    />
    <span class="label">{{ entry.name }}</span>
  </li>
</template>
