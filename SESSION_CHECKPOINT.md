# SESSION_CHECKPOINT — 2026-06-15 00:30

## 新鲜度自检
- git 仓库,远程 `https://github.com/adaelon/Fluid.git`,分支 `main`(逐刀直提主干,无 PR)。
- 最新 commit:`df373f0` S5 旁路缓存(已 push)。**S6 已完成、判据全达、尚未 commit**(等用户点头即提)。
- 读入时对比 `git log --oneline -3`,不一致以 git log 为准。

## 当前在做什么
S6 = 后端单函数生成(非流式)**已完成并验证通过**。`/api/generate` → 装配上下文 → 调 opencode zen 网关 glm-5.1(OpenAI 兼容,配置走 `.env`)→ 整段返回 `{cacheHit,capsule,lines}`,接 S5 缓存。`cargo test` **25/25 绿**、build 无警告。判据两半全达(见下)。

## S6 判据验证结果(B2,全达)
- 对 alphaGPT `execution/config.py:get_payer_keypair`[25,34],经 `.env` 配置 glm-5.1:
- 首调 cache MISS → 真实 LLM:`cacheHit:false`,合理中文 Capsule + 7 重点行注释(语义色温),~20s,落盘 `5b9dd253f14d3afd.json`。
- 二次同请求 cache HIT:`cacheHit:true`,0.14s,逐字段相同;日志恰一次 `calling LLM` 后 `cache HIT … zero token`(零 token 证毕)。
- 响应 UTF-8 正确(控制台 mojibake 仅 cp936 显示假象)。

## 下一步(可直接接手)
1. **commit S6 到 main**(用户点头后):`git add` 全部 S6 改动(见"未提交"清单,**勿 add `.env`**——已 gitignore),提交信息形如 `S6: per-function generation — /api/generate + LLM proxy (glm-5.1 via .env) + cache wiring`。
2. 然后进 **S7**(`docs/切片计划.md`):`/api/generate` 改 WS 流式(capsule→lines 逐个推)+ 前端 GhostStore + CM6 block widget 渲染显影(§7.2/7.3),折叠/展开纯 UI 零重算。
3. 复跑验证模板:从 **Fluid 根**起服务(让 dotenvy 读 `.env`)→ `./target/debug/fluid.exe E:/allwork/download/agent/alphaGPT --port <p>`;请求体 = `/tmp/genreq.json`(get_payer_keypair#25);核对中文用 `python` 显式 utf-8 解码,**勿用 `python -m json.tool` 管道**(Windows 按 GBK 解 stdin 出假 mojibake)。

## 未提交 / 未完成
- **S6 全部改动未 commit**:`Cargo.toml`(reqwest+dotenvy)、新增 `llm_proxy.rs`/`context_assembler.rs`、改 `routes.rs`/`main.rs`;`.gitignore`(+`.env`)、新增 `.env.example`(提交);`docs/代码链路.md`(+S6 条,判据已达)、`docs/切片计划.md`(去 Claude 措辞)、本 checkpoint。
- **不提交**:`.env`(gitignore,含 key)、`/tmp/genreq.json`(非仓库,可重建)。
- PENDING(后续刀):S8 大文件阈值;缓存 file-watch 失效;FNV→SHA-256 仅碰撞为患时;§5 的"现 LLM 补 fileSummary/calleeSummaries"(S6 暂省)。

## 冷启动读序
1. `docs/技术方案.md` §5 生成管道 / §6 缓存键 / §7 状态机 — S7 的靶
2. `docs/代码链路.md` — 改动账本(S1~S6 已记)
3. `docs/切片计划.md` — S7(WS 流式 + 前端渲染)
4. `crates/fluid-server/src/{llm_proxy,context_assembler,routes,main}.rs` — S6 实现
5. `web/src/{App,Editor}.vue`、`web/src/parser/` — S3/S4 前端,S7 接线点
6. `CONTEXT.md` / `需求文档.md` — 术语 / 四核律

## 本会话决策摘要
- **模型/接入 = opencode zen 网关 glm-5.1(OpenAI 兼容);配置走 `.env`(Fluid 根,gitignore),`dotenvy` 启动加载,真实 env 优先**:推翻早期"默认最新 Claude"(用户指定)。未开 ADR(沿用 S5 FNV 判例,reverse=改 .env+缓存重算)。已记 `docs/代码链路.md` S6 + 同步切片计划措辞;模板 `.env.example` 已建。
