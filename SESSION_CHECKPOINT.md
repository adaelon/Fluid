# SESSION_CHECKPOINT — 2026-06-16 (打包:单二进制分发 · 待 commit)

## 新鲜度自检
- git 仓库,远程 `https://github.com/adaelon/Fluid.git`,分支 `main`(逐刀直提主干,无 PR)。
- 写入时最新 commit:`8e8c6ce` 文档(全部切片完成 + README,已 push);前序 `ce32a1a` U4、`9289b88` S10c。
- **本会话「打包」改动已写完 + 测试绿,尚未 commit**(见下「未提交」)。
- 读入时对比 `git log --oneline -3`;不一致以 git 为准。

## 当前在做什么
**打包为单二进制 + 一行安装运行**(§0 渠道用户定=预编译二进制 + 安装脚本)。三层全部完成:
1. **后端托管前端**(共用前提):`rust-embed` 把 `web/dist` 编进二进制;`static_assets.rs` 的 `static_handler` 真实资源→回资源、非 `/api` 未命中→`index.html`(SPA)、`/api` typo→404;`router.fallback`;`build.rs` 占位页保证 dist 路径恒存在;`main.rs` 打印 URL + `open` 自动开浏览器。→ `fluid <project>` 一条命令起整个 app(同端口 7878,运行时零 Node)。
2. **release CI**:`.github/workflows/release.yml`,tag `v*` 触发,matrix(linux x86_64 / macos arm64+x86_64 / windows x86_64)npm build → cargo build --release → 发 Release + install.sh。
3. **install.sh**:`uname` 探测 → 拉对应资产 → 入 PATH;`README` 加「安装(预编译二进制)」+「从源码运行/开发」。

## 下一步(可直接接手)
1. **commit + push 打包改动**(项目惯例逐刀直提 main):`git add -u` + 显式加新文件(`build.rs`、`crates/fluid-server/src/static_assets.rs`、`.github/workflows/release.yml`、`scripts/install.sh`),排除根 0 字节 `defaults`。建议信息「打包: 单二进制分发(后端托管前端 + release CI + install.sh)」。**注意 `Cargo.lock` 也变了(新增 rust-embed/mime_guess/open),一并提交。**
2. **发首个版本验证端到端**(留用户):`git tag v0.1.0 && git push origin v0.1.0` → 看 Actions 跑通、Release 出 4 平台二进制 + install.sh → 真机 `curl … | sh` → `fluid <proj>` 浏览器自动开。
3. **或转收尾**:起 `docs/架构.md`(C3,功能已稳定)、消化 PENDING(⑥ reqwest timeout、⑦ 代码块语法高亮等)。

## 未提交 / 未完成
- **打包改动未 commit**。新增:`crates/fluid-server/build.rs`、`src/static_assets.rs`、`.github/workflows/release.yml`、`scripts/install.sh`。改:`crates/fluid-server/Cargo.toml`、`Cargo.lock`、`src/{routes,main}.rs`、`README.md`、`docs/代码链路.md`、本 checkpoint。`cargo test` **90/90**(+4 static_assets)、clippy 净;`release.yml` YAML 通过;`install.sh` sh -n + 四平台 dry-run 通过。
- **杂项**:仓库根 0 字节 `defaults`(未跟踪)——可删,勿入 commit。
- PENDING(均非阻塞):⑥ reqwest 无 timeout;⑦ 代码块语法高亮;⑨ `docs/架构.md` 待起;⑮ 追问开关态不持久化;⑱ theme.ts 手工跟 token;⑲ Tabs `×` 未换 SVG;㉑ TOCTOU;㉒ 仅 OpenAI 兼容;㉓ 不持久化最近 provider;㉔ S10c 受图谱稀疏性限;㉕ 命令无独立快捷键/无折叠全部;㉖(新)linux/arm64 无预编译包(脚本拒绝);㉗(新)CI/install.sh 真实跑留发版眼验。

## 冷启动读序
按顺序读这些文件能还原全局上下文:
1. `README.md` — 项目总览 + 安装/运行(单二进制 + 源码)
2. `docs/切片计划.md` — 全部切片 ✅;`docs/代码链路.md` 末「打包」条
3. 打包:`crates/fluid-server/{build.rs,src/static_assets.rs,src/routes.rs(fallback),src/main.rs}` + `.github/workflows/release.yml` + `scripts/install.sh`
4. 后端核:`crates/fluid-server/src/{routes,context_assembler,settings,llm_proxy,cache_store}.rs`
5. 前端:`web/src/App.vue` + `shell/*` + `Editor.vue` + `api.ts` + `render/*`;`CONTEXT.md` 术语表
6. `docs/adr/0015/0016/0006~0008/0017/0018`

## 本会话决策摘要
- **打包 = 后端 rust-embed 托管前端(共用前提)+ 预编译二进制 + install.sh(渠道,用户定)**:单二进制运行靠把 `web/dist` 嵌入 + router fallback 托管 SPA(运行时零 Node,dev 仍用 Vite 热重载);分发选预编译二进制(终端用户零工具链),CI tag 触发跨平台编译发 Release,install.sh 按平台拉资产。build.rs 占位页保护嵌入路径。已落档 `代码链路.md`「打包」条。
