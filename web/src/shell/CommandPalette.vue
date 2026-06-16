<script lang="ts">
// One palette entry. `run` performs the action; the palette closes after it.
export interface PaletteItem {
  id: string
  label: string
  hint?: string
  run: () => void
}
</script>

<script setup lang="ts">
// CommandPalette — quick-open / command palette (U4). Driven entirely by the
// `items` the parent supplies: file-open entries (Ctrl+P) or app commands
// (Ctrl+Shift+P). Fuzzy filter + ↑↓ navigation + Enter/click to run, Esc to
// close. Standard IDE chrome (rounded, shadowed — §7.4 only governs code-area glass).
import { ref, computed, watch, onMounted } from 'vue'
import { fuzzyFilter } from './fuzzy'

const props = defineProps<{ items: PaletteItem[]; placeholder: string }>()
const emit = defineEmits<{ close: [] }>()

const query = ref('')
const selected = ref(0)
const inputEl = ref<HTMLInputElement | null>(null)

const results = computed(() => fuzzyFilter(query.value, props.items, (i) => i.label))

// Keep the highlight in range as the filtered list shrinks/grows.
watch(results, () => {
  selected.value = 0
})

onMounted(() => inputEl.value?.focus())

function move(delta: number) {
  const n = results.value.length
  if (n === 0) return
  selected.value = (selected.value + delta + n) % n
}

function runAt(i: number) {
  const item = results.value[i]
  if (!item) return
  item.run()
  emit('close')
}

function onKey(e: KeyboardEvent) {
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    move(1)
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    move(-1)
  } else if (e.key === 'Enter') {
    e.preventDefault()
    runAt(selected.value)
  } else if (e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}
</script>

<template>
  <div class="palette-overlay" @click.self="emit('close')">
    <div class="palette" role="dialog" aria-label="命令面板">
      <input
        ref="inputEl"
        v-model="query"
        class="palette-input"
        :placeholder="placeholder"
        autocomplete="off"
        spellcheck="false"
        @keydown="onKey"
      />
      <ul v-if="results.length" class="palette-list">
        <li
          v-for="(item, i) in results"
          :key="item.id"
          class="palette-item"
          :class="{ active: i === selected }"
          @mouseenter="selected = i"
          @click="runAt(i)"
        >
          <span class="palette-label">{{ item.label }}</span>
          <span v-if="item.hint" class="palette-hint">{{ item.hint }}</span>
        </li>
      </ul>
      <p v-else class="palette-empty">无匹配项</p>
    </div>
  </div>
</template>
