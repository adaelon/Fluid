<script setup lang="ts">
// StatusBar — the bottom bar of the VSCode-like shell (U1, ADR-0015). Left shows
// the active file path + language; right shows generation progress, lifted from
// Editor via App (the S7.5 progress chip moved here from its temporary editor
// overlay). Standard IDE chrome.
import { computed } from 'vue'

const props = defineProps<{
  path: string | null
  lang: string | null
  progress: { phase: 'idle' | 'running' | 'done'; completed: number; total: number }
}>()

const progressText = computed(() => {
  const p = props.progress
  if (p.phase === 'running') return `⟳ 生成中 ${p.completed}/${p.total}`
  if (p.phase === 'done') return `✓ ${p.total} 个函数已显影`
  return ''
})
</script>

<template>
  <footer class="status-bar">
    <span class="status-left">
      <template v-if="path">{{ path }}<span v-if="lang" class="status-lang"> · {{ lang }}</span></template>
      <template v-else>就绪</template>
    </span>
    <span class="status-right" :class="{ done: progress.phase === 'done' }">{{ progressText }}</span>
  </footer>
</template>
