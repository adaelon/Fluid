<script setup lang="ts">
import { computed } from 'vue'
import type { SelectionExplanation } from './ghostTypes'
import type { SelectionViewState } from './selectionState'

const props = defineProps<{
  state: SelectionViewState
  selectedText: string
}>()

const emit = defineEmits<{
  close: []
  regenerate: []
}>()

const explanation = computed<SelectionExplanation | null>(() =>
  props.state.mode === 'result' ? props.state.explanation : null,
)

const evidence = computed(() => {
  switch (explanation.value?.evidenceStatus) {
    case 'project-source':
      return { label: '项目源码', tone: 'project' }
    case 'web-cited':
      return { label: '网页有来源', tone: 'cited' }
    case 'web-uncited':
      return { label: '联网无来源', tone: 'uncited' }
    default:
      return { label: '未核验', tone: 'unverified' }
  }
})
</script>

<template>
  <section class="selection-popover" role="dialog" aria-label="选区解释">
    <header class="selection-popover-head">
      <div class="selection-popover-title">
        <span>选区解释</span>
        <code>{{ selectedText }}</code>
      </div>
      <button
        class="selection-popover-close"
        type="button"
        title="关闭"
        aria-label="关闭选区解释"
        @click="emit('close')"
      >
        ×
      </button>
    </header>

    <div v-if="state.mode === 'loading'" class="selection-popover-body selection-loading">
      <span class="selection-spinner" aria-hidden="true"></span>
      <div>
        <div>{{ state.message }}</div>
        <div v-if="state.cacheHit" class="selection-cache-note">已命中旁路缓存</div>
      </div>
    </div>

    <div v-else-if="state.mode === 'error'" class="selection-popover-body">
      <p class="selection-error">{{ state.message }}</p>
      <button class="selection-secondary-btn" type="button" @click="emit('regenerate')">
        重试
      </button>
    </div>

    <div v-else-if="explanation" class="selection-popover-body selection-result">
      <div class="selection-result-meta">
        <span class="selection-kind">{{ explanation.kind }}</span>
        <span v-if="state.mode === 'result' && state.cacheHit" class="selection-cache-note">
          缓存秒显
        </span>
      </div>

      <div class="selection-section">
        <div class="selection-section-label">它是什么</div>
        <p>{{ explanation.meaning }}</p>
      </div>
      <div class="selection-section">
        <div class="selection-section-label">这里做什么</div>
        <p>{{ explanation.roleHere }}</p>
      </div>
      <div class="selection-section">
        <div class="selection-section-label">来源状态</div>
        <div class="selection-evidence-row">
          <span class="selection-evidence" :class="`tone-${evidence.tone}`">
            {{ evidence.label }}
          </span>
          <span v-if="explanation.origin" class="selection-origin">{{ explanation.origin }}</span>
        </div>
        <ul v-if="explanation.sources?.length" class="selection-sources">
          <li v-for="source in explanation.sources" :key="source.url">
            <a :href="source.url" target="_blank" rel="noopener noreferrer">{{ source.title }}</a>
          </li>
        </ul>
        <p v-else-if="explanation.evidenceStatus === 'web-uncited'" class="selection-warning">
          供应商返回了联网整理内容，但没有可追溯 URL。
        </p>
        <p v-else-if="explanation.evidenceStatus === 'unverified'" class="selection-warning">
          当前解释只依据本地上下文，未由外部来源核验。
        </p>
        <p v-if="explanation.warning" class="selection-warning">{{ explanation.warning }}</p>
      </div>

      <div class="selection-actions">
        <button class="selection-secondary-btn" type="button" @click="emit('regenerate')">
          重新生成
        </button>
      </div>
    </div>
  </section>
</template>
