# SESSION_CHECKPOINT — 2026-06-17 (Markdown 渲染 + 翻译三刀完成,准备发 v0.1.1)

## 新鲜度自检
- git 仓库,远程 `https://github.com/adaelon/Fluid.git`,分支 `main`(逐刀直提主干,无 PR)。
- 写入时最新 commit:`fe55fcb`(Markdown 文档翻译:分块并发 + 流式进度/增量渲染);前序 `f033517`(Markdown 文档渲染视图,已 push)。
- **提交不加 Co-Authored-By 尾注**(用户要求,见 memory)。
- 读入时对比 `git log --oneline -3`;不一致以 git 为准。唯一应未提交项 = 本 checkpoint。

## 当前在做什么
**Markdown 文档渲染 + 翻译**全部完成,准备打 `v0.1.1` tag 发版(带全部修复 + 新功能的二进制)。
MVP 早已闭环(见 `docs/切片计划.md` 全 ✅);本轮新增 S-MD(渲染视图)+ S-MD-T/T2/T3(翻译)。

## 下一步(可直接接手)
1. **打 tag 发版**:`git tag v0.1.1 && git push origin v0.1.1` → 触发 `.github/workflows/release.yml` 出全平台二进制。CI 结果须在 GitHub 端眼验(沙箱看不到);若 Actions 因权限失败→ Settings→Actions→Workflow permissions 改 Read and write 后 Re-run。
2. **眼验翻译**(留用户,沙箱禁网跑不了真 LLM):打开长 `.md` → 点「译中文」→ 看进度「翻译中 N/total 段」+ 逐段增量显影 + 每块耗时日志;重开未改零 token。
3. **清理**:仓库根 0 字节 `defaults`(未跟踪垃圾)可删,勿入 commit。
4. **或收尾 PENDING**:`docs/架构.md`(C3 一直欠)、翻译换更快模型(单次延迟是模型瓶颈,非代码)。

## 未提交 / 未完成
- 仅本 checkpoint 待提交;`defaults` 0 字节未跟踪(勿提交)。
- 后端 `cargo test` **101/101**、clippy 净;前端 `npm run build` 绿。
- 翻译 PENDING(非阻塞):单次延迟=glm-5.1/网关物理下限(治本换模型);无 token 级进度;整篇单缓存键(非分块缓存);单超长无空行段仍可能慢;行内 code 靠 LLM 指令保留;相对路径图片/内嵌 HTML 不渲染;`buffered` 保序(后块快于前块仍等)。

## 冷启动读序
1. `README.md` — 总览 + 安装/运行
2. `docs/切片计划.md`(全 ✅,末 S-MD/T/T2/T3)+ `docs/代码链路.md` 末四条(S-MD、S-MD-T、S-MD-T2、S-MD-T3)
3. 翻译链:`crates/fluid-server/src/translate.rs`(protect/restore/split_chunks)+ `routes.rs`(`run_translate_stream`/`TranslateFrame`/`translate_one_chunk` + 常量 chunk3500/并发4/timeout240)+ `cache_store.rs`(translate 键)
4. 渲染/前端:`web/src/MarkdownView.vue`(原文/译中文切换 + 流式进度 + 增量渲染)+ `render/markdownDoc.ts` + `api.ts`(`streamTranslate`)+ `CONTEXT.md`(术语「文档渲染视图」「文档翻译」)
5. 后端核(如需):`routes.rs` AppState/router、`llm_proxy.rs`、`project_reader.rs:lang_of`(md 标签)

## 本会话决策摘要
- **S-MD**:`.md` 直接渲染取代源码(用户选),复用 markdown-it→DOMPurify→KaTeX 链,不进生成管线。已落 `代码链路.md` S-MD。
- **S-MD-T/T2/T3 翻译**:英译中、旁路 `.fluid/`、按钮原地切换、代码块占位符保护不译;长文档分块并发(修整篇一次 500);据实跑调参(并发4/chunk3500)+ 流式 WS 进度/增量渲染。失败策略=坏块保留原文。已落 `代码链路.md` S-MD-T/T2/T3。
