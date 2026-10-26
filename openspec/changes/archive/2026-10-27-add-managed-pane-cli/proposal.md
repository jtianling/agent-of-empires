## Why

管理 agent 已能安全操作自己创建的 session, 但不能准确管理同一 session 中的多个工作 pane.  需要独立创建、发送输入、读取输出和关闭 pane, 同时防止旧 slot 引用或布局变化误伤其他 pane.

## What Changes

- 新增 `aoe managed pane list/create/remove/send/capture`, 沿用 context、profile 和 JSON 契约.
- 为 pane 提供持久 ID 和明确 owner, 左右位置只作展示, 不作为操作身份.
- 创建 pane 支持独立 xats name/team, 每个 pane 使用独立 key、claim 和运行时代际.
- 人工或其他 owner 的 pane 不因位于受管 session 中而自动获得可修改权限; 整个 session 的操作不能绕过此保护.
- 精确验证 session、slot、tmux pane 与当前占用关系, 删除和失败回收只处理目标 pane 的资源.

## Capabilities

### New Capabilities

- `managed-pane-cli`: pane 级命令、稳定身份、权限、独立 xats 与安全回收.

### Modified Capabilities

- `managed-session-cli`: 混合所有权 pane 的父 session 操作保护及人工所有权转移语义.

## Impact

涉及 managed CLI、session/slot 元数据、既有 tmux 精确目标工具、三种 managed xats runtime 适配及文档.  复用既有四 pane 上限和布局机制, 不增加远程 API.  真实生命周期测试继续在与宿主实时 session 隔离的容器中执行.
