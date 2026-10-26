## 1. 只读数据和总览

- [x] 1.1 实现不创建文件/锁/schema 的 session/group/slot 读取.  
- [x] 1.2 实现显式 profile socket 的批量 pane 观测和精确关联.  
- [x] 1.3 增加 overview JSON 入口与多 profile、空层级、partial 错误处理.  

## 2. TUI 实例和 xats

- [x] 2.1 增强新 TUI 登记并实现只读 profile instances, 校验启动时间和旧记录.  
- [x] 2.2 接入可选 xats 精确 pane lookup, 验证响应 schema 并白名单输出, 失败显式降级.  

## 3. 文档和验收

- [x] 3.1 补充用户文档、生成 CLI reference 和针对性的自动化覆盖.  
- [x] 3.2 完成 fmt、编译、clippy 和独立规格核验 (第二轮 PASS, 无 CRITICAL/WARNING).  
- [x] 3.3 aoe-tester 在隔离环境完成 E2E, 包括真实 TUI/pane、身份错误分支和无副作用证据.  
- [x] 3.4 验收通过后同步主规格、归档并 commit.  
