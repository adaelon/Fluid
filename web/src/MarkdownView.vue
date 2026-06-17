<script setup lang="ts">
// Document Render View (CONTEXT「文档渲染视图」): when a .md/.markdown file is
// opened, render it as a formatted document instead of plain source. It deliberately
// bypasses the whole generation/ghost pipeline — no parser, no scheduler, no
// GhostStore — so the md file stays in the vacuum state; this is purely a different
// *way to look at* the file's own content. Zero byte contamination holds (render
// only, never write back).
import { ref, watch, onMounted, nextTick } from 'vue'
import { renderDoc, typesetMath } from './render/markdownDoc'

const props = defineProps<{ source: string; path: string }>()

const html = ref('')
const article = ref<HTMLElement | null>(null)
// Guards async render against rapid file switches: each render bumps the token; a
// stale callback (libs/typeset resolved after a switch) sees a mismatch and bails.
let token = 0

async function render(src: string): Promise<void> {
  const t = ++token
  const out = await renderDoc(src)
  if (t !== token) return // switched files mid-render
  html.value = out
  await nextTick()
  if (t !== token || !article.value) return
  await typesetMath(article.value)
}

onMounted(() => void render(props.source))
watch(
  () => [props.source, props.path] as const,
  () => void render(props.source),
)
</script>

<template>
  <div class="fluid-doc-scroll">
    <article ref="article" class="fluid-doc" v-html="html"></article>
  </div>
</template>
