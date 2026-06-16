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
  /** Whether the follow-up query panel is currently open (toggle reflects it). */
  queryOpen: boolean
}>()

// The query terminal is hidden by default; this bar carries the only affordance
// to open it, handing the bottom space back to the code area until asked for.
const emit = defineEmits<{ toggleQuery: [] }>()

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
    <span class="status-right-group">
      <button
        class="status-query-toggle"
        :class="{ active: queryOpen }"
        type="button"
        :disabled="!path"
        :title="path ? '追问当前文件' : '打开文件以启用追问'"
        @click="emit('toggleQuery')"
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M21 11.5a8.38 8.38 0 0 1-8.5 8.5 9.5 9.5 0 0 1-4-.9L3 20l1.4-4.5A8.5 8.5 0 1 1 21 11.5z" />
        </svg>
        追问
      </button>
      <span class="status-right" :class="{ done: progress.phase === 'done' }">{{ progressText }}</span>
    </span>
  </footer>
</template>
