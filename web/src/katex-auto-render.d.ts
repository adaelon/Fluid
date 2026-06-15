// Minimal ambient types for KaTeX's auto-render contrib entry (S11-md). The
// katex package ships JS for this submodule but no .d.ts for the path; we only
// use the default export, so declare just that.
declare module 'katex/contrib/auto-render' {
  interface AutoRenderDelimiter {
    left: string
    right: string
    display: boolean
  }
  interface AutoRenderOptions {
    delimiters?: AutoRenderDelimiter[]
    throwOnError?: boolean
    [key: string]: unknown
  }
  export default function renderMathInElement(
    elem: HTMLElement,
    options?: AutoRenderOptions,
  ): void
}
