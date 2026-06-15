# 追问答案渲染 Markdown + LaTeX,流式纯文本、完成后一次性渲染

追问答案是自由 Markdown 文本,可含 LaTeX 数学公式(行内 `$...$`、块级 `$$...$$`)。答案逐 token 流入:流式期间逐字显**纯文本**(保留"生成可见状态"),收到 `done` 帧后才用 markdown-it 把整段渲为 HTML、KaTeX 渲染其中的数学、DOMPurify 消毒后注入答案面板。渲染只在答案面板的内存/DOM,源码零字节不变(核心律 1)。

## Considered Options

- **流式实时渲染(每 delta 重渲)**:贴合流式体验,但 KaTeX 遇未闭合 `$x^2`(闭合 `$` 尚未到达)会抛错,需逐帧做分隔符配对检测 + 容错;长答案每 token 全量重渲还会轻微重排/闪烁。复杂度与收益不匹配——纯文本流式已提供可见进度。
- **MathJax 替 KaTeX**:功能更全(更多 LaTeX 宏),但体积显著更大、渲染异步;"完成后一次性渲染"场景不需要其增量能力,KaTeX 同步、轻量更合适。
- **不消毒直接 v-html**:LLM 输出不可信,Markdown 可夹带 `<script>`/`onerror` → XSS。即便只读环境也不接受。

## Consequences

- 新增前端依赖:`markdown-it`(+ `@types/markdown-it`)、`katex`(含 `contrib/auto-render`)、`dompurify`。npm registry 可达,`npm run build` 可验证。
- **DOMPurify 是硬约束**:md→HTML 后、注入 DOM 前必须消毒;KaTeX auto-render 在消毒后对容器跑(KaTeX 生成的标记可信)。
- 后端 `build_query_prompt` 的 system 提示补一句:允许用 `$...$`/`$$...$$` 写公式——否则模型很少自发产出 LaTeX,渲染能力空转。
- 代码块语法高亮本刀不做(markdown-it 默认出 `<pre><code>` 无高亮),留后续(highlight.js 又一依赖)。
- 错误态(error 帧)仍按纯文本显示,不走 Markdown 渲染。
