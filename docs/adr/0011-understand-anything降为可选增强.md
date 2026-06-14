# understand-anything 降为可选增强,非硬前置

Fluid 不强制要求项目先跑过 understand-anything。有 `.understand-anything/` 则用其 summary 与 edges 加速;无则降级自给:tree-sitter 出 roster 并解析 import/call 关系替代 edges,文件/函数 summary 由 LLM 生成,跨文件被调对象现生成一句话摘要兜底(可缓存)。理由:understand-anything 函数节点本就稀疏(实测 14/?),Fluid 的 AST + LLM 管线无论如何都得能独立补全所有粒度——既然这套兜底必须存在,就让 understand-anything 退为"有则加速、无则自给"的增强层。

## Considered Options

- **硬前置(无 `.understand-anything/` 不让用)**:实现简单、上下文有保证,但多一道门槛,且与"AST+LLM 必须能独立兜底"重复。

## Consequences

- Fluid 对任意项目自包含可用。
- 无 edges 时跨文件摘要由 LLM 现生成(小调用,可缓存),首次成本略高;质量较 understand-anything 节点 summary 略降。
- tree-sitter 需具备从语法树抽取 import/call 关系的能力,作为 edges 的本地替代源。
