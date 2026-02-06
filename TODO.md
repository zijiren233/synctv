# SyncTV Rust 重构 TODO 跟踪

**最后更新**: 2026-02-06
**当前状态**: 全部核心功能完成，生产可用
**架构**: 9-crate workspace (synctv, synctv-api, synctv-core, synctv-proxy, synctv-stream, synctv-sfu, synctv-cluster, synctv-proto, synctv-providers)
**编译状态**: zero warnings, zero errors

---

## ✅ P0 - 安全和核心功能（全部完成）

- [x] WebSocket JWT认证
- [x] RTMP推流认证（PublishKeyService）
- [x] 房间成员实时计数（ConnectionManager）
- [x] 播放列表详细信息
- [x] Direct类型PlaybackResult（多模式支持）
- [x] 动态文件夹支持（DynamicFolder trait）
- [x] Provider完整实现（Bilibili/Alist/Emby parse/login/list）
- [x] 动态播放列表API（HTTP + gRPC）

---

## ✅ P1 - 重要功能（全部完成）

### Provider & API
- [x] Provider实例管理API（CRUD + reconnect/enable/disable）
- [x] 弹幕完整流程（统一ChatMessage，position字段区分弹幕/聊天）

### WebRTC（Phase 1-5 全部完成）
- [x] Phase 1-5: P2P信令、STUN、TURN、SFU、网络质量监控

### 系统完善
- [x] 系统设置热重载（PostgreSQL NOTIFY/LISTEN）
- [x] 审计日志分区自动化（AuditPartitionManager）
- [x] 邮件模板系统（Handlebars + lettre SMTP）
- [x] 分布式锁（Redis SET NX EX + Lua脚本）
- [x] 聊天消息保留策略（窗口函数批量清理）

### Proxy解耦
- [x] `synctv-proxy` crate + 每个provider独立proxy路由
- [x] 移除SPA静态文件服务（native app only）
- [x] `PublicSettings`集中化 — `SettingsRegistry::to_public_settings()`

### Admin API（全部完成）
- [x] Admin HTTP路由 — `/api/admin/*`（用户/房间/设置/邮件/Vendor管理）
  - 用户管理: list/get/create/delete/ban/unban/approve/role/password/username
  - 房间管理: list/get/delete/ban/unban/approve/password/members/settings
  - 设置管理: get/set/group
  - 邮件: send test email
  - Provider instances: list/add/set/delete/reconnect/enable/disable
  - Admin管理（root only）: list/add/remove admins
  - 系统统计: get_system_stats
- [x] Room member管理HTTP路由 — kick/ban/unban/permissions
  - `POST /api/rooms/:room_id/members/:user_id/kick`
  - `POST /api/rooms/:room_id/members/:user_id/ban`
  - `POST /api/rooms/:room_id/members/:user_id/unban`
  - `POST /api/rooms/:room_id/members/:user_id/permissions`
- [x] BanMember/UnbanMember — proto定义 + impls + HTTP + gRPC
- [x] `enable_guest` setting — SettingsRegistry + PublicSettings + proto
- [x] Vendor backend discovery — `GET /api/vendor/backends/:vendor`

### 代码TODO全部修复
- [x] RTMP player settings检查 — SettingsRegistry.rtmp_player
- [x] 播放列表信息 — get_root_playlist()获取实际数据
- [x] Emby缩略图 — 从host/Items/{id}/Images/Primary构建URL

---

## 🟡 待完成 - 可延后

- [ ] **Danmu SSE实际实现** — `synctv-api/src/http/providers/bilibili.rs`
  - 当前是keep-alive stub，需要接入Bilibili弹幕获取API

---

## 🟢 P2 - 优化和完善（可延后）

### 监控和文档
- [x] Prometheus监控集成 — `/metrics`端点已实现
- [x] Swagger UI — `/swagger-ui`已实现
- [ ] 为所有HTTP端点添加完整的`#[utoipa::path]`注解

### 流媒体优化
- [ ] GOP缓存验证和测试（RTMP推流 → HLS/FLV拉流首帧延迟）
- [ ] OSS存储集成（S3/阿里云OSS）

### 测试覆盖
- [ ] Bench tests完善（当前是stub）
- [ ] 单元测试扩展（PermissionService, RoomService等）
- [ ] 集成测试扩展（完整用户流程、Provider集成）
- [ ] 端到端WebRTC测试（需要客户端）

---

## 📋 设计偏离记录

- [x] Role/Status已改为SMALLINT（users, rooms, room_members）
- [x] ID字段已使用CHAR(12)（nanoid）
- [x] JWT过期时间符合设计（Access 1h, Refresh 30d）
- Go有captcha验证码 — Rust使用更简单的email token流程（设计决策，非遗漏）
- Room TTL不需要后台清理任务 — 数据持久化在PostgreSQL+Redis中

---

## 快速链接

**核心文件**:
- Admin HTTP: `synctv-api/src/http/admin.rs`
- Admin impls: `synctv-api/src/impls/admin.rs`
- WebSocket: `synctv-api/src/http/websocket.rs`
- RTMP: `synctv/src/rtmp/mod.rs`
- Provider proxy: `synctv-proxy/src/lib.rs`
- Provider routes: `synctv-api/src/http/providers/{bilibili,alist,emby,direct_url}.rs`
- Settings: `synctv-core/src/service/global_settings.rs`
- SFU: `synctv-sfu/src/`
- Media model: `synctv-core/src/models/media.rs`
- Member management: `synctv-api/src/http/room_extra.rs`
