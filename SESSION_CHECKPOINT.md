# SESSION_CHECKPOINT — 2026-06-14

## 新鲜度自检
- 非 git 仓库,无 commit hash 可比。以文件 mtime 为准。
- 本会话产出:`需求文档.md`(重写 v2)、`docs/技术方案.md`、`docs/切片计划.md`、`CONTEXT.md`、`docs/adr/0001~0013`、`docs/需求文档修订清单.md`。

## 当前在做什么
Fluid 设计对齐完成(§0 + §0.5)。需求 v2 + 技术方案 + 切片计划全部落盘,13 条地基决策落 ADR。**设计封顶,尚未写任何代码。** 栈:后端 Rust(axum/tokio)、前端 TS+CodeMirror 6、浏览器 tree-sitter WASM 解析。

## 下一步(可直接接手)
1. 进编码第一刀 = **S1 后端骨架 + L0 文件树**(`docs/切片计划.md` S1):Rust 后端,`fluid ./project` 起服务,`GET /api/project/tree` 返回文件树、`GET /api/file` 返回源码。判据:对 alphaGPT 返回 37 文件树,无 `.understand-anything/` 也能起。触达 `crates/fluid-server/src/{main,project_reader,routes}.rs` + `Cargo.toml`。
2. 按 `docs/切片计划.md` S1→S11 依序推进(依赖序见该文件顶部图)。
3. 开干前可选:用 TaskCreate 把 S1~S11 镜像成会话进度板(A4)。

## 未提交 / 未完成
- 全部为文档,未写任何代码。
- PENDING(实现期决策):重点行 tree-sitter query 规则(S4 定);大文件阈值数值(S8 定);缓存 file-watch 失效 + "上下文可能过期"提示;LLM 模型选型(默认最新 Claude)。

## 冷启动读序
1. `需求文档.md` — v2 对齐版,需求骨架(§1~§9)
2. `docs/技术方案.md` — 架构、接口签名、生成管道、状态机(§0 有 ADR 速查表)
3. `docs/切片计划.md` — S1~S11 施工刀(下一步从 S1 起)
4. `CONTEXT.md` — 术语表
5. `docs/adr/0001~0013` — 13 条决策依据
6. `docs/需求文档修订清单.md` — v1→v2 差异 + 真实数据修正

## 本会话决策摘要(详见 docs/adr/)
- 0001 交付载体=自研网页 IDE · 0002 文件为原子激活单元/弃三级链/零Token粒度=文件级/放宽延迟
- 0003 落盘旁路缓存(键=内容hash+模型版本;技术方案§6细化为函数span粒度) · 0004 按函数流式生成+roster共享上下文
- 0005 自带AST解析,首阶段Python+Rust · 0006 追问上下文分层装配+超窗降级
- 0007 跨文件默认注摘要+破例临时取源 · 0008 生成调度视口邻近+大文件视口渐进
- 0009 tree-sitter浏览器端WASM解析 · 0010 拓扑=本地后端+浏览器前端,LLM走后端代理
- 0011 understand-anything降为可选增强 · 0012 前端内核=CodeMirror 6 · 0013 后端语言=Rust
