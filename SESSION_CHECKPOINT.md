# SESSION_CHECKPOINT — 2026-06-15 (U3-R 完成 · 待提交 + 待眼验)

## 新鲜度自检
- git 仓库,远程 `https://github.com/adaelon/Fluid.git`,分支 `main`(逐刀直提主干,无 PR)。
- 最新 commit:`4ec9142` S8(已 push);其下 `7800f3e` U3、`f441fbf` U2、`d1f3499` U1。
- **U3-R 代码已完成但尚未 commit**(本会话新改动)。读入时对比 `git log --oneline -3`,若已见 U3-R commit 说明已提交,以 git 为准。

## 当前在做什么
**U3-R · Open Folder 改系统文件夹选择器**(修订 U3 / 用户反馈"要点击弹文件管理器而非输绝对路径")。**已完成**,`cargo test` **33/33** + `npm run build` 绿(CSS 5.92kB),**待 commit + 待眼验**。
机制:后端加 `rfd` + `POST /api/project/pick` 弹原生对话框→回服务端绝对路径→前端喂现成 `/api/project/open` 换根;文本框降为兜底。绕过"浏览器拿不到绝对路径"(让后端而非浏览器开对话框,ADR-0010 本地拓扑)。

## 下一步(可直接接手)
1. **提交 U3-R**(若用户同意):`git add Cargo.lock crates/fluid-server/Cargo.toml crates/fluid-server/src/routes.rs web/src/{App.vue,api.ts,styles.css} docs/adr/0015-*.md docs/代码链路.md SESSION_CHECKPOINT.md` → commit「U3-R: Open Folder 改系统文件夹选择器 — 后端 rfd 原生对话框 (修订 ADR-0015)」→ push。**不要 add `defaults`**(0 字节疑误产物)。
2. **眼验**(留用户,沙箱无 GUI):`start.bat` → 侧栏头点「打开文件夹…」→ 系统对话框弹出→选目录→树切换+tab 清空;取消无变化;文本框兜底仍可用。连带眼验 S8(`[sched] dispatch` 顺序=视口邻近、在途 ≤4)。
3. **下一刀(择一)**:
   - **S9 gutter + 手动单行**(=反馈#2):重点行脉动点 + 非重点行 hover「解释这一行」→ `/api/explain-line`。
   - **U4 命令面板(可选)**:`Ctrl+P` 模糊找文件 / `Ctrl+Shift+P`(收口 U 轨)。
4. **U 轨收口后**:起 `docs/架构.md`(C3,U1 起 PENDING 至今);S10 追问器 dock QueryPanel 到 rail。

## 未提交 / 未完成
- **U3-R 全部改动未 commit**:`Cargo.lock`、`crates/fluid-server/Cargo.toml`(+rfd)、`routes.rs`(pick_folder)、`web/src/{App.vue,api.ts,styles.css}`、`docs/adr/0015`(修订段)、`docs/代码链路.md`(U3-R 条)、本 checkpoint。
- `cargo test` 33/33;`npm run build` 绿。
- **杂项**:仓库根 0 字节空文件 `defaults`(未跟踪)——可删,待用户确认。
- **磁盘**:C: 曾满至 0(用户已清,现 ~3.5GB);rfd fetch 须 `--target x86_64-pc-windows-msvc` 才精简(否则拉 mac/linux 全树撑爆)。
- PENDING:① rfd 同步对话框 mac 须主线程事件循环(Windows 现可用);② 追问器仍浮层(dock 留 S10);③ `docs/架构.md` 待 U 轨收口起;④ reqwest 无 timeout;⑤ S9 手动单行(=反馈#2);⑥ S8 worker 间无工作窃取(够用)。
- **A2 未自动验证项**:浏览器/GUI 内 U-R2/U1/U2/U3/U3-R/S8 全部交互 —— 沙箱无 GUI+禁出网,靠 `start.bat` 由用户眼验。换根核心 + S8 排序/并发逻辑 + 后端编译已确定性覆盖。

## 冷启动读序
按顺序读这些文件能还原全局上下文:
1. `docs/adr/0015-前端壳-类VSCode界面.md`(含 U3-R 修订)+ `0016-代码阅读区右栏对齐注释.md` — UI 主线决策
2. `docs/代码链路.md` 末「U1/U2/U3/S8/U3-R」条 — 触达账本 + 机制注
3. `docs/切片计划.md` U 轨 + S8/S9/S10 — 已完/待做(下一刀=S9 或 U4)
4. `web/src/scheduler.ts` — S8 调度核;`web/src/Editor.vue` — 激活链 + scheduler 接线 + 字号/进度
5. `web/src/App.vue` + `web/src/shell/{ActivityBar,StatusBar,Tabs}.vue` — IDE 壳(U1/U2/U3/U3-R)
6. `crates/fluid-server/src/routes.rs`(`AppState`/`open_folder`/`pick_folder`/`run_generation`)+ `project_reader.rs` — 后端换根 + 穿越防护 + 选择器 + WS 串行
7. `需求文档.md §7` + `CONTEXT.md` — 视觉规范 + 术语(激活单元 _Avoid_「视口激活」= S8 不做门控的依据)

## 本会话决策摘要
- **S8 并发载体 = 前端 N 条并行 WS**(`4ec9142`,已落档 代码链路 S8);**S8 放弃大文件视口门控**(避免重引入已废弃「视口激活」)。
- **U3-R 选择器 = 本地后端弹 rfd 原生对话框**(非浏览器选择器):绕过"浏览器拿不到服务端绝对路径";后端=用户本机(ADR-0010)。已落档 代码链路 U3-R + ADR-0015 修订段。
