## 1. 数据模型与持久化

- [x] 1.1 在 `PaneConfig` (`src/session/pane.rs`) 增加 `xats_team` 与 `xats_agent_name` 两个字段, serde 默认空字符串, 空值表示未声明
- [x] 1.2 在写入路径做边界校验: 拒绝含控制字符或换行的值, 限定长度上限; 校验失败不得写入持久层
- [x] 1.3 扩展 `agent_slot` schema, 用既有幂等自愈补列方式新增两列 (`DEFAULT ''`), 重复运行不得报错
- [x] 1.4 让 `upsert_agent_slot_config` 与 slot 读取路径带上这两个字段, 旧行读出为空
- [x] 1.5 补单元测试: 补列幂等、旧行读作未声明、含引号与空格的值往返不失真、非法值被拒

## 2. 声明值随 pane 生命周期稳定

- [x] 2.1 让 restart / resume / fresh restart / cold recovery 各路径在重写 slot 时保留既有声明值, 不得写空覆盖
- [x] 2.2 确认 sibling pane 的声明值互不影响 (与 identity key 同样按 slot 隔离)
- [x] 2.3 补单元测试: 各重启路径后声明值不变; 两个 pane 各自声明互不覆盖

## 3. 环境变量注入 (通用通道)

- [x] 3.1 在 pane 启动命令的环境注入前缀中加入 `XATS_TEAM` / `XATS_AGENT_NAME`, 仅注入非空部分
- [x] 3.2 值经既有 `shell_escape` 转义后拼入, 不手工拼引号
- [x] 3.3 未声明的 pane 的启动命令必须与变更前逐字节一致
- [x] 3.4 补单元测试: 只声明一半时只注入一半; 未声明时命令不变; 含空格与引号的值转义正确

## 4. Codex pre-registration 兼容重试 (先于新 flag 落地)

- [x] 4.1 把 codex xats bootstrap 的 pre-registration 改为"声明身份调用失败后, 去掉声明 flag 但**保留 identity key 与 TTL** 重试一次; 未声明的 pane 不生成重试分支"
- [x] 4.2 判定只依据退出码, 不解析 CLI 错误文本; 继续使用 `|| failed=1` 写法, 不引入 `set -e`
- [x] 4.3 两次调用都失败时保持既有的显式失败行为与错误文案
- [x] 4.4 补单元测试: 已声明时脚本含重试分支且重试仍带 key 与 TTL; 未声明时只有一次调用且失败即致命

## 5. Codex pre-registration 携带声明身份

- [x] 5.1 在增强调用上追加声明身份参数, 仅追加非空部分, 值经 `shell_escape` 转义
- [x] 5.2 未声明的 codex pane 的 pre-registration 调用与变更前一致
- [x] 5.3 补单元测试: 已声明时参数出现在首次调用且不出现在重试调用; 未声明时调用不含该参数

## 6. TUI 配置入口

- [x] 6.1 在 New Session 与 pane 配置对话框中, 按 pane 增加 xats team 与 agent name 两个输入项
- [x] 6.2 仅在该 pane 的 Cross Agent Team 开启时可编辑, 关闭时不可编辑
- [x] 6.3 提交前清空字段即表示未声明 (AoE 无重配已建 pane 的流程, 见 design)
- [x] 6.4 补 TUI 单元测试: 开关联动的可编辑性、回显、清空语义

## 7. 收尾

- [x] 7.1 运行 `cargo fmt`、`cargo clippy`、`cargo build` (有实时 session 时不跑全量 `cargo test`)
- [x] 7.2 更新 `openspec/specs/cross-agent-team/spec.md` 之外的受影响文档 (如涉及)
- [x] 7.3 把最终的 flag 名与环境变量名同步给 xats-main 确认
