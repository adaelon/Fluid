# Fluid 自带 AST 解析器,第一阶段支持 Python 与 Rust

Fluid 内置自己的 AST 解析器,用于零 Token、确定性地枚举文件的完整函数清单、行范围,并判定哪些是"重点行"。understand-anything 仅作为已有摘要与 edges 的输入源,不承担解析职责。第一阶段支持 Python 与 Rust。

## Considered Options

- **复用 understand-anything 的图谱作为完整真相源**:其函数节点稀疏(样例 37 文件仅 14 个函数节点)且无任何行级数据,无法提供完整函数清单与行映射,故必须自解析。

## Consequences

- 解析(确定性、Fluid)与摘要生成(LLM)职责分离。
- 新增支持语言 = 新增一个 AST 适配器。
- "重点行"判定纯靠 AST 启发式(确定性工具判定,LLM 只负责生成内容)。
