## Context

总览目前逐 pane 向 profile.errors 追加错误, 旧 daemon 的 404 响应格式还会被误报为 invalid error.  

## Goals / Non-Goals

聚合重复诊断, 保留 pane 级原因和局部失败语义.  不新增缓存、重试或服务升级.  

## Decisions

- 在既有 record_error 中按 source 和完整 message 相等合并, 新条目 affected_count 为 1, 重复时递增, observed_at 保留首次观测时间.  不跨 profile 合并, 不按模糊文本匹配.  
- HTTP 404 在解析响应体前识别为 lookup 端点不受支持, 因此兼容旧服务 HTML/纯文本/JSON.  其它 HTTP 状态保留原具体诊断路径.  
- 每 pane 的 identity unavailable、reason 和 runtime unknown 保持不变, profile partial 不变.  

## Risks / Trade-offs

相同来源但不同原因不会合并, 避免隐藏真正不同的错误.  affected_count 表示该原因影响的观测条数, lookup 场景为 pane 数; 不能当作整个 profile 的 pane 总数.  
