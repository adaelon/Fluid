# SESSION_CHECKPOINT — 2026-06-14 16:30

## 新鲜度自检
- 已是 git 仓库,远程 `https://github.com/adaelon/Fluid.git`。
- 读入时对比 `git log --oneline -3`;若与下方"最新 commit"不一致,以 git log 为准。
- 最新 commit(写入时):**S1 代码尚未 commit**(工作区有未提交改动,见下)。最后一次 commit = `9009f62` Merge(仅文档)。

## 当前在做什么
S1(后端骨架 + L0 文件树)**已实现并验证通过,尚未 commit**。Rust 后端 `crates/fluid-server` 可 `fluid <project>` 起服务,出文件树与单文件源码。设计文档已全部落盘并推上 GitHub。

## 下一步(可直接接手)
1. **提交 S1**:`git add -A && git commit`(代码 + docs/代码链路.md + checkpoint),用户确认后 `git push`。
2. 进 **S2 · 图谱加载**(`docs/切片计划.md` S2):`crates/fluid-server/src/graph_loader.rs` 读 `.understand-anything/knowledge-graph.json`,`encoding_rs` 解 GBK,`GET /api/project/graph`;无图谱返回 `null` 不崩。判据:alphaGPT 解出 91 节点/199 边、中文不乱码。
3. 或先修文档"37 文件"误数为 33(独立小改,见决策摘要)。

## 未提交 / 未完成
- `crates/fluid-server/`(main/routes/project_reader + 2 个 Cargo.toml)、`.gitignore`、`docs/代码链路.md`:**已实现已测,未 commit**。
- `cargo build` 绿、`cargo test` 5/5 绿、端到端 curl 全过。
- PENDING(后续刀):S4 重点行 query 规则;S8 大文件阈值;缓存 file-watch 失效;LLM 模型选型。

## 冷启动读序
1. `需求文档.md` — v2 需求骨架
2. `docs/技术方案.md` — 架构/接口签名/状态机(§0 ADR 速查)
3. `docs/切片计划.md` — S1~S11 施工刀(S1 完成,下一刀 S2)
4. `docs/代码链路.md` — 改动账本(S1 已记)
5. `CONTEXT.md` — 术语表
6. `docs/adr/0001~0013` — 决策依据

## 本会话决策摘要
- **数据现实修正**:alphaGPT 知识图谱实测 **91 节点(40 class/33 file/14 function/4 document)**,`.py` 文件 **33** 个。文档多处写的"37 文件"为早期误估,应改 33。S1 已按真实数 33 验收。
- **S1 实现决策**:CLI `fluid <project> [--port 7878]`,绑 127.0.0.1;文件树扁平 `FileNode[]` 跳噪声目录;`/api/file` 做路径穿越防护;源码 UTF-8 lossy。
