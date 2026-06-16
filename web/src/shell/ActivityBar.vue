<script setup lang="ts">
// ActivityBar — the left vertical icon strip of the VSCode-like shell (U1,
// ADR-0015). U1 ships the Explorer item (always active); the Settings gear at the
// bottom opens the LLM backend settings modal (U5b, ADR-0018). Standard IDE
// chrome — not glass material (§7.4 only governs the in-editor ghost notes).
const emit = defineEmits<{ openSettings: [] }>()
</script>

<template>
  <nav class="activity-bar">
    <button class="activity-item active" title="资源管理器" aria-label="资源管理器">
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
        <path d="M3 5h6l2 2h10v12H3z" />
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
