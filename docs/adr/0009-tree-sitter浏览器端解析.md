# AST 解析器用 tree-sitter,在浏览器端(WASM)解析

Fluid 的函数清单枚举、行范围、重点行判定统一用 tree-sitter:一套 API 覆盖 Python、Rust 及未来语言(各有官方 grammar),容错 + 增量解析,重点行判定表达为对语法树的 query(每语言一份规则)。解析跑在浏览器端的 WASM 构建里。

## Considered Options

- **每语言接原生解析器(Python `ast`、Rust `syn`)**:每加一门语言重写一套;Python `ast` 无法在浏览器运行,须放服务端。
- **服务端解析**:每个文件源码都要上传解析,增加往返与服务端负载。

## Consequences

- 浏览器端解析 → 出 roster/重点行零网络往返、零 token、源码不必上传;仅 LLM 生成步骤出网(只发必要函数 span + 共享上下文)。
- 新增语言 = 加一个 tree-sitter grammar + 一份重点行 query。
