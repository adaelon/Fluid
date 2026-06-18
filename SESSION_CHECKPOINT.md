# SESSION_CHECKPOINT — 2026-06-18 (v0.1.3 已发:TS 支持 + start.bat 单端口)

## 新鲜度自检
- 写入时最新 commit:`65d48a3`(start.bat 单端口);其前 `b78875f`(TS 支持)。
- 已 push main + tag **v0.1.3**(触发 release.yml);读入时对比 `git log --oneline -3`。
- 提交不加 Co-Authored-By 尾注(用户要求,见 memory)。

## 当前在做什么
本会话已闭环并发版 **v0.1.3**。两件事:
- **TS 支持(两刀)**:S-TS-1 `.ts/.tsx/.js/.jsx` 语法高亮;S-TS-2 `.ts` 幽灵注释生成(tree-sitter-typescript + typescript.scm)。
- **start.bat 单端口化**:对齐单二进制发版形态(npm build + 单进程 cargo run 起 7878),cargo run 后加 pause 防"窗口一闪而关"。

## 下一步(可直接接手)
1. **眼验 v0.1.3 CI**:GitHub→Actions 看 release.yml 是否出全平台二进制(沙箱看不到)。若因权限失败→ Settings→Actions→Workflow permissions 改 Read and write 后 Re-run。
2. **眼验 TS**:`start.bat` 起 → 开 `.ts` 看高亮 + 函数胶囊 + 重点行注释;`.tsx/.js/.jsx` 应只高亮不生成。
3. **或收尾 PENDING**:tsx/jsx **生成**(JSX 重点行 query,另一刀);`docs/架构.md`(C3 长期欠账)。

## 未提交 / 未完成
- 无(本 checkpoint 即最后一项,随附 commit)。
- `defaults`(仓库根)0 字节未跟踪垃圾,勿提交。
- TS v1 已知局限:箭头/函数表达式 const 定义行有冗余 key line(tree-sitter 无 not 谓词);tsx/jsx 仅高亮不生成;对象方法/IIFE 未纳入 roster。
- 运维坑(已修):旧发版 `clamp\fluid-windows-x86_64.exe` 会占 7878 致开发实例起不来——开发时别启动它。

## 冷启动读序
1. `README.md` + `CONTEXT.md`(术语:幽灵注释/重点行/真空态)
2. `docs/切片计划.md` 末 S-TS-1/S-TS-2 + `docs/代码链路.md` 末 S-TS-1/S-TS-2
3. TS 解析链:`web/src/parser/parse.ts`(ROSTER_QUERY['ts'] + extractRoster/extractKeyLines/innermostHost)+ `keyline-queries/typescript.scm` + `browser.ts`(三语言)+ `types.ts`
4. 接线点:`web/src/Editor.vue`(langExtension 高亮 / isParserLang 生成门)+ `crates/fluid-server/src/project_reader.rs:lang_of`
5. 验证:`web/scripts/parse-check.ts`(B2 门禁,`node` 直跑);发布:`.github/workflows/release.yml`(v* tag 触发,版本取 tag 名,不依赖 Cargo.toml version)

## 本会话决策摘要
- **TS 拆两刀**:高亮(装包)与生成(S4 级 AST 适配器)两量级,先高亮后生成。已落 `代码链路.md` S-TS-1/2 + ADR-0005 后续记。
- **后端生成语言无关** → 加语言是纯前端切片;lang_of 标签是后端唯一相关改动。
- **start.bat 单端口**:对齐发版形态(用户选),消除前后端版本割裂 + 端口占用一闪而关。
