# Fluid 自带 AST 解析器,第一阶段支持 Python 与 Rust

Fluid 内置自己的 AST 解析器,用于零 Token、确定性地枚举文件的完整函数清单、行范围,并判定哪些是"重点行"。understand-anything 仅作为已有摘要与 edges 的输入源,不承担解析职责。第一阶段支持 Python 与 Rust。

## Considered Options

- **复用 understand-anything 的图谱作为完整真相源**:其函数节点稀疏(样例 33 个 file 节点仅 14 个函数节点)且无任何行级数据,无法提供完整函数清单与行映射,故必须自解析。

## Consequences

- 解析(确定性、Fluid)与摘要生成(LLM)职责分离。
- 新增支持语言 = 新增一个 AST 适配器。
- "重点行"判定纯靠 AST 启发式(确定性工具判定,LLM 只负责生成内容)。

## 后续:扩展 TypeScript(S-TS-2,2026-06-18)

第三门语言 TypeScript 经"新增 AST 适配器"路径加入(印证上面的 consequence):`ParserLang +'ts'`、装载 `tree-sitter-typescript.wasm`、`ROSTER_QUERY['ts']`(函数声明/类方法/箭头函数·函数表达式·类字段箭头的赋值)、新 `keyline-queries/typescript.scm`。后端生成链路语言无关(按行范围切 span 发 LLM),故纯前端接线。`.tsx/.js/.jsx` 仅高亮(S-TS-1)不生成,JSX 重点行另议。
