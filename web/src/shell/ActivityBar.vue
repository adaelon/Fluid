<script setup lang="ts">
// ActivityBar — the left vertical icon strip of the VSCode-like shell (U1,
// ADR-0015). Explorer and Query now mutually own the resizable left column;
// Settings remains anchored at the bottom. Standard IDE chrome — not glass
// material (§7.4 only governs the in-editor ghost notes).
import type { SidebarView } from '../queryLayout'

defineProps<{
  sidebarView: SidebarView
  queryEnabled: boolean
}>()

const emit = defineEmits<{
  toggleExplorer: []
  toggleQuery: []
  openSettings: []
}>()
</script>

<template>
  <nav class="activity-bar">
    <button
      class="activity-item"
      :class="{ active: sidebarView === 'explorer' }"
      title="资源管理器"
      aria-label="资源管理器"
      :aria-pressed="sidebarView === 'explorer'"
      @click="emit('toggleExplorer')"
    >
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
        <path d="M3 5h6l2 2h10v12H3z" />
      </svg>
    </button>
    <button
      class="activity-item"
      :class="{ active: sidebarView === 'query' }"
      title="追问器"
      aria-label="追问器"
      :aria-pressed="sidebarView === 'query'"
      :disabled="!queryEnabled"
      @click="emit('toggleQuery')"
    >
      <svg
        width="22"
        height="22"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M21 11.5a8.38 8.38 0 0 1-8.5 8.5 9.5 9.5 0 0 1-4-.9L3 20l1.4-4.5A8.5 8.5 0 1 1 21 11.5z" />
      </svg>
    </button>
    <button
      class="activity-item activity-settings"
      title="设置"
      aria-label="设置"
      @click="emit('openSettings')"
    >
      <svg
        width="22"
        height="22"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="12" cy="12" r="3.2" />
        <path
          d="M12 2.4v2.6M12 19v2.6M21.6 12H19M5 12H2.4M18.8 5.2l-1.8 1.8M7 17l-1.8 1.8M18.8 18.8 17 17M7 7 5.2 5.2"
        />
      </svg>
    </button>
  </nav>
</template>
