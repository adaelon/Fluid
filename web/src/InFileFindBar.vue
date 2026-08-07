<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type {
  InFileFindDirection,
  InFileFindQuery,
  InFileFindSnapshot,
} from './inFileFind.ts'

const props = defineProps<{
  query: InFileFindQuery
  snapshot: InFileFindSnapshot
}>()

const emit = defineEmits<{
  'update:query': [InFileFindQuery]
  move: [InFileFindDirection]
  close: []
}>()

const input = ref<HTMLInputElement | null>(null)
const resultLabel = computed(() => {
  if (props.snapshot.error === 'invalid-regexp') return '正则无效'
  if (props.snapshot.total === 0) return '无结果'
  return `${props.snapshot.current}/${props.snapshot.total}`
})
const canMove = computed(() => (
  props.snapshot.error === null && props.snapshot.total > 0
))

function updateQuery(patch: Partial<InFileFindQuery>): void {
  emit('update:query', { ...props.query, ...patch })
}

function updateText(event: Event): void {
  updateQuery({ text: (event.target as HTMLInputElement).value })
}

function moveFromInput(event: KeyboardEvent): void {
  emit('move', event.shiftKey ? 'previous' : 'next')
}

function focusInput(): void {
  input.value?.focus()
  input.value?.select()
}

defineExpose({ focusInput })
onMounted(focusInput)
</script>

<template>
  <div class="in-file-find-bar" role="search" aria-label="文件内查找">
    <input
      ref="input"
      class="in-file-find-input"
      type="text"
      :value="query.text"
      autocomplete="off"
      spellcheck="false"
      aria-label="在当前文件中查找"
      aria-describedby="in-file-find-status"
      :aria-invalid="snapshot.error ? 'true' : undefined"
      @input="updateText"
      @keydown.enter.prevent.stop="moveFromInput"
    />
    <span
      id="in-file-find-status"
      class="in-file-find-status"
      :class="{ error: snapshot.error }"
      role="status"
      aria-live="polite"
    >
      {{ resultLabel }}
    </span>
    <button
      class="in-file-find-button"
      type="button"
      :disabled="!canMove"
      aria-label="上一个匹配项"
      title="上一个匹配项 (Shift+Enter)"
      @click="$emit('move', 'previous')"
    >
      ↑
    </button>
    <button
      class="in-file-find-button"
      type="button"
      :disabled="!canMove"
      aria-label="下一个匹配项"
      title="下一个匹配项 (Enter)"
      @click="$emit('move', 'next')"
    >
      ↓
    </button>
    <button
      class="in-file-find-button in-file-find-mode"
      :class="{ active: query.caseSensitive }"
      type="button"
      :aria-pressed="query.caseSensitive"
      aria-label="区分大小写"
      title="区分大小写"
      @click="updateQuery({ caseSensitive: !query.caseSensitive })"
    >
      Aa
    </button>
    <button
      class="in-file-find-button in-file-find-mode"
      :class="{ active: query.mode === 'regexp' }"
      type="button"
      :aria-pressed="query.mode === 'regexp'"
      aria-label="使用正则表达式"
      title="使用正则表达式"
      @click="updateQuery({ mode: query.mode === 'regexp' ? 'literal' : 'regexp' })"
    >
      .*
    </button>
    <button
      class="in-file-find-button"
      type="button"
      aria-label="关闭文件内查找"
      title="关闭文件内查找 (Esc)"
      @click="$emit('close')"
    >
      ×
    </button>
  </div>
</template>
