<script setup lang="ts">
// Tabs — the open-file tab strip of the VSCode-like shell (U2, ADR-0015).
// Shows one tab per open file; clicking activates, the × closes. Switching the
// active tab swaps Editor props in App, reusing the existing activation chain.
// Standard IDE chrome (§7.4 governs only in-editor ghost notes).
defineProps<{ tabs: { path: string; lang: string }[]; active: string | null }>()
const emit = defineEmits<{ activate: [path: string]; close: [path: string] }>()

function basename(path: string): string {
  return path.split('/').pop() || path
}
</script>

<template>
  <div class="tabs">
    <div
      v-for="t in tabs"
      :key="t.path"
      class="tab"
      :class="{ active: t.path === active }"
      :title="t.path"
      @mousedown="emit('activate', t.path)"
    >
      <span class="tab-name">{{ basename(t.path) }}</span>
      <button
        class="tab-close"
        title="关闭"
        aria-label="关闭"
        @mousedown.stop="emit('close', t.path)"
      >
        ×
      </button>
    </div>
  </div>
</template>
