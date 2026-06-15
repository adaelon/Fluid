<script setup lang="ts">
// ActivityBar — the left vertical icon strip of the VSCode-like shell (U1,
// ADR-0015). U1 ships only the Explorer item (always active); Search / command
// palette (U4) land later. Standard IDE chrome — not glass material (§7.4 only
// governs the in-editor ghost notes).
</script>

<template>
  <nav class="activity-bar">
    <button class="activity-item active" title="资源管理器" aria-label="资源管理器">
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
        <path d="M3 5h6l2 2h10v12H3z" />
      </svg>
    </button>
  </nav>
</template>
