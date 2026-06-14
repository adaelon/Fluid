<script setup lang="ts">
import { onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import { EditorState, type Extension } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { basicSetup } from 'codemirror'
import { python } from '@codemirror/lang-python'
import { rust } from '@codemirror/lang-rust'

const props = defineProps<{ source: string; lang: string }>()

const host = shallowRef<HTMLDivElement | null>(null)
// ADR-0014: the CM6 EditorView is an imperative object. Hold it in a
// shallowRef so Vue never deep-proxies its internal state. NEVER a plain ref().
const view = shallowRef<EditorView | null>(null)

function langExtension(lang: string): Extension {
  if (lang === 'py') return python()
  if (lang === 'rs') return rust()
  return []
}

// Read-only render: no Fluid gutter indicator dots yet (vacuum state, 需求 §7.1).
function buildState(source: string, lang: string): EditorState {
  return EditorState.create({
    doc: source,
    extensions: [
      basicSetup,
      langExtension(lang),
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
    ],
  })
}

onMounted(() => {
  view.value = new EditorView({
    state: buildState(props.source, props.lang),
    parent: host.value!,
  })
})

watch(
  () => [props.source, props.lang] as const,
  () => view.value?.setState(buildState(props.source, props.lang)),
)

onBeforeUnmount(() => {
  view.value?.destroy()
  view.value = null
})
</script>

<template>
  <div ref="host" class="cm-host"></div>
</template>
