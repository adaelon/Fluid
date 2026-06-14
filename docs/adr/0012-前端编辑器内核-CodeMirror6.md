# 前端编辑器内核定为 CodeMirror 6

ADR-0001 留待实现期的 Monaco vs CodeMirror,现定为 **CodeMirror 6**。Fluid 不是编辑器,而是一张"只读 + 重度自定义注释"的画布——Monaco 的核心价值(编辑、IntelliSense、VS Code 既视感)对我们全是负载,而 Fluid 最吃重的两个动作(行间插玻璃态卡片、gutter 脉动点)恰是 CM6 的一等公民(block widget 装饰 + 可渲染任意 DOM 的 gutter API),Monaco 的 view zones/glyph margin 则偏重且受限。CM6 还更轻、可 tree-shake。

## Considered Options

- **Monaco**:全功能、巨文件虚拟化最稳、生态大;但重,且行间富卡片/自定义 gutter 别扭。其优势(编辑/IntelliSense)Fluid 只读用不到;巨文件优势被 ADR-0008「大文件视口渐进」抵消。

## Consequences

- 无现成"IDE 皮",外观全自定义(对 §7 的玻璃态叙事反而是优势)。
- 解析仍独立于编辑器(tree-sitter,ADR-0009);CM6 仅作渲染器,其内建 Lezer 不使用。
- 巨文件滚动丝滑略逊 Monaco,结合视口渐进生成可接受。
