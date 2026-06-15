# SESSION_CHECKPOINT — 2026-06-15 (S8 完成 · 待提交 + 待眼验)

## 新鲜度自检
- git 仓库,远程 `https://github.com/adaelon/Fluid.git`,分支 `main`(逐刀直提主干,无 PR)。
- 写入时最新 commit:`7800f3e` U3。**S8 代码已完成但尚未 commit**(本会话新改动)。
- 读入时对比 `git log --oneline -4`;若已见 S8 commit 说明已提交,以 git 为准。

## 当前在做什么
**S8 生成调度**(切片计划 S8 / 反馈#1 速度面):视口邻近排序 + 有限并发(N 条并行 WS,默认 4)+ 滚动重排。**已完成**,`scheduler-check.ts` 13/13 + `npm run build` 绿(67 模块)+ 后端 `cargo test` 维持 33/33。**待 commit + 待用户眼验**。
§0 收窄:**放弃大文件视口门控**(会重引入 CONTEXT 已废弃「视口激活」),整文件必全生成;未改 CONTEXT、未开 ADR。

## 下一步(可直接接手)
1. **提交 S8**(若用户同意):`git add web/src/scheduler.ts web/scripts/scheduler-check.ts web/src/Editor.vue docs/代码链路.md docs/切片计划.md SESSION_CHECKPOINT.md` → commit「S8: 生成调度 — 视口排序+N路并发WS+滚动重排(切片计划S8,反馈#1,§0放弃门控)」→ push。
2. **眼验累积五刀**(留用户,沙箱无浏览器):`start.bat` 起前后端 → ① U-R2 字号缩放 ② U1 三区+拖侧栏+状态栏进度 ③ U2 多 tab ④ U3 换根 ⑤ **S8**:开 py/rs 文件看控制台 `[sched] dispatch <fnId>` 顺序=视口邻近、在途 ≤4、滚到别处未生成函数插队。
3. **下一刀(择一)**:
   - **S9 gutter + 手动单行**(=反馈#2):重点行脉动点 + 非重点行 hover「解释这一行」→ `/api/explain-line`。
   - **U4 命令面板(可选)**:`Ctrl+P` 模糊找文件 / `Ctrl+Shift+P` 命令(收口 U 轨)。
4. **U 轨收口后**:起 `docs/架构.md`(C3,U1 起 PENDING 至今);S10 追问器实做时 dock QueryPanel 到 rail。

## 未提交 / 未完成
- **S8 全部改动未 commit**:`web/src/scheduler.ts`(新)、`web/scripts/scheduler-check.ts`(新)、`web/src/Editor.vue`(改)、`docs/代码链路.md`、`docs/切片计划.md`、本 checkpoint。
- 后端未动;`cargo test` 33/33;前端 `npm run build` 绿(67 模块,CSS 5.60kB)。
- **杂项**:仓库根 0 字节空文件 `defaults`(疑误重定向产物,未跟踪)——可删,待用户确认。
- PENDING:① 追问器仍浮层(正式 dock 留 S10);② 活动栏/状态栏静态 chrome;③ `docs/架构.md` 待 U 轨收口起;④ reqwest 无 timeout;⑤ S9 手动单行(=反馈#2);⑥ S8 worker 间无工作窃取、视口距离按定义行(够用,非 bug)。
- **A2 未自动验证项**:浏览器内 U-R2/U1/U2/U3/S8 全部视觉与交互 —— 沙箱无浏览器+禁出网,靠 `start.bat` 由用户眼验。S8 纯排序/并发逻辑已由 scheduler-check 覆盖。

## 冷启动读序
按顺序读这些文件能还原全局上下文:
1. `docs/adr/0015-前端壳-类VSCode界面.md` + `0016-代码阅读区右栏对齐注释.md`(含修订)— UI 主线决策
2. `docs/代码链路.md` 末「U-R2/U1/U2/U3/S8」条 — 触达账本 + 机制注
3. `docs/切片计划.md` U 轨 + S8/S9/S10 — 已完/待做规格(下一刀=S9 或 U4)
4. `web/src/scheduler.ts` — S8 调度核(viewportDistance/PendingQueue/GenScheduler)
5. `web/src/Editor.vue` — 激活链 + scheduler 接线 + 字号/进度 emit
6. `web/src/App.vue` + `web/src/shell/{ActivityBar,StatusBar,Tabs}.vue` — IDE 壳(U1/U2/U3)
7. `crates/fluid-server/src/routes.rs`(`AppState`/`open_folder`/`run_generation`)+ `project_reader.rs` — 后端换根 + 穿越防护 + WS 串行
8. `需求文档.md §7` + `CONTEXT.md` — 视觉规范 + 术语(注:激活单元 _Avoid_「视口激活」= S8 不做门控的依据)

## 本会话决策摘要
- **S8 并发载体 = 前端 N 条并行 WS**(非后端 spawn、非单 socket 窗口):后端单 socket 串行,N 条 → axum N 个并发 task;`run_generation` 锁不跨 await 保并发读安全(已落档 代码链路 S8)。
- **S8 §0 放弃大文件视口门控**:门控 = CONTEXT 已废弃的「视口激活」,与「打开即整体生成」铁律冲突;用户拍板整文件必全生成,滚动只改顺序(已落档 代码链路 S8 + 切片计划 S8)。
