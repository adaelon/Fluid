// PlaceholderWidget — function-level generation feedback (S7.5/S7.6). Shown at a
// function's definition line while no capsule has arrived: a pulsing "生成中"
// skeleton (mode 'pending'), or a "生成失败" chip carrying the failure reason +
// a 重试 button (mode 'error', S7.6). Replaced by the real CapsuleWidget once the
// `capsule` frame lands. Shares the .fluid-ghost glass pipeline (§7.4).

import { WidgetType } from '@codemirror/view'

export type PlaceholderMode = 'pending' | 'error'

export class PlaceholderWidget extends WidgetType {
  constructor(
    readonly fnId: string,
    readonly mode: PlaceholderMode,
    readonly message = '',
  ) {
    super()
  }

  eq(other: PlaceholderWidget): boolean {
    return other.fnId === this.fnId && other.mode === this.mode && other.message === this.message
  }

  toDOM(): HTMLElement {
    const root = document.createElement('div')
    root.className = 'fluid-ghost fluid-placeholder ' + this.mode

    const bar = document.createElement('span')
    bar.className = 'fluid-bar'
    bar.style.setProperty('--c', this.mode === 'error' ? '#ff7b72' : '#484f58')

    const body = document.createElement('div')
    body.className = 'fluid-body'

    if (this.mode === 'error') {
      const label = document.createElement('span')
      label.className = 'fluid-err-label'
      label.textContent = '· 生成失败'
      if (this.message) label.title = this.message // hover to see the reason

      const retry = document.createElement('button')
      retry.className = 'fluid-retry'
      retry.setAttribute('data-retry', this.fnId)
      retry.textContent = '重试'

      body.append(label, retry)
    } else {
      body.textContent = '生成中…'
    }

    root.append(bar, body)
    return root
  }

  // Let clicks (the retry button) reach the editor's retry handler.
  ignoreEvent(): boolean {
    return false
  }
}
