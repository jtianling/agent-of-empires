## Why

steward 实测旧 xats 端点 404 在一个 profile 中重复返回 46 条相同错误, 且提示 invalid error 无法说明缺少接口.  需要减少重复输出并准确说明不可用原因.  

## What Changes

- 同 profile 的相同来源和原因错误聚合, 返回 affected_count.  
- pane 保留自己的原因, partial 与 unknown 语义不变.  
- lookup HTTP 404 明确提示服务不支持该端点, 不依赖旧服务响应体格式.  

## Capabilities

### New Capabilities

无.  

### Modified Capabilities

- `readonly-agent-overview`: 增加重复错误聚合和缺失接口诊断要求.  

## Impact

仅总览错误输出和 xats lookup 的 404 诊断, 不改变查询或控制权限.  
