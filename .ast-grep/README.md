# 结构检查与测试布局门禁

采用[官方安装方式](https://ast-grep.github.io/guide/quick-start)，固定 ast-grep **0.51.0**：`npm install --global @ast-grep/cli@0.51.0`。

```sh
python3 scripts/check-test-layout.py
python3 -m unittest discover -s scripts -p test_test_layout.py
ast-grep test -c sgconfig.yml
ast-grep scan -c sgconfig.yml
```

测试布局已接入 `scripts/check.sh`、Go/TS SDK 的 `scripts/test-sdks.sh`、TS SDK/Web 的 test/lint，以及既有 Linux/Web CI。工具缺失、版本不符、规则加载或解析失败均非零退出。检查不会自动修复文件。

## YAML 能判断的结构

6 个新增 `test-layout-*` 规则文件：

- TS/TSX：生产文件中直接导入或 require `node:test`、`vitest`、`@jest/globals`；测试文件和登记集成目录除外。注释、路径字符串和业务同名函数不匹配。
- Go：生产文件中具有测试签名的 Test/Benchmark/Fuzz 或无参数 Example 函数；合法 `_test.go` 与生成协议排除，不把所有 `_test.go` 判为错误。
- Rust：源目录中的外置测试模块、`mod tests` 缺少明确 `#[cfg(test)]`、测试函数位于该容器之外。容器内可用子模块组织场景。

## 跨文件检查

`scripts/check-test-layout.py` 才能检查源码是否存在、文件名/扩展名对应、同一源码仅一个单测文件，以及目录登记。Go `foo.go → foo_test.go`；TS `foo.ts → foo.test.ts`、`foo.tsx → foo.test.tsx`，项目可在 `test-layout.json` 统一选择 `.spec`；Rust 测试集中在源码内唯一 `mod tests`，其数量也通过 AST 记录核对。没有测试的源码不会因此被判为失败。

真正集成测试使用 Go/TS 包的 `test/`、Cargo crate 的 `tests/`。Go/TS 集成入口和辅助 fixture 在 `test-layout.json` 逐文件记录用途；缺少、空理由、过期或目录错误的登记会失败。现有生成协议目录显式排除，不用“generated”注释任意跳过文件。

静态工具不能证明测试是否真的跨组件、被测源码归属是否合理或覆盖充分；登记理由需要人工审查。自定义测试框架、别名导入、Go testing 包别名、宏生成的 Rust 测试或 cfg_attr 等写法不保证识别，需要扩展规则并补样例，不能据零命中宣称全部语义已验证。端到端脚本、Python 等语言不套用三语言单元测试命名。

## 原有手动提示

原有5个语言规则、38个样例继续保留：TS 双断言 warning；SDK Promise.all 数组内直接 await warning（Web 已有 Oxlint 同类规则）；Rust `anyhow::anyhow!(e.to_string())` hint。它们没有接入强制扫描门禁。根检查运行全部规则样例以防退化，但只对测试布局规则执行强制扫描。

原提示继续排除依赖、构建、生成绑定与测试/fixture；不解析类型、别名或证明并发独立性。结构命中需要人工核实，不能直接称为 bug。
