## ADDED Requirements

### Requirement: New Session 不提供 Sandbox 创建入口

TUI New Session flow SHALL NOT 暴露 Sandbox controls, 并且 SHALL 始终提交 non-sandbox session, 不受 `sandbox.enabled_by_default` 影响。  该限制 SHALL 只适用于 New Session。  CLI sandbox flags、Settings、配置文件、已有 sandbox sessions 和 container lifecycle SHALL 保持支持。

#### Scenario: Sandbox default does not silently affect New Session
- **WHEN** `sandbox.enabled_by_default` 为 true
- **AND** 用户通过 New Session 创建 session
- **THEN** 新 session SHALL 不包含 enabled `SandboxInfo`

#### Scenario: CLI Sandbox remains available
- **WHEN** 用户通过受支持的 CLI flag 显式创建 sandbox session
- **THEN** 现有 Sandbox 创建和 container lifecycle SHALL 保持不变

#### Scenario: Existing sandbox session remains usable
- **WHEN** AoE 加载或重启本变更前创建的 sandbox session
- **THEN** 该 session SHALL 继续使用已记录的 container configuration
