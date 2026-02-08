# RESTful API 合规性分析

## 概述

本文档分析 SyncTV HTTP API 是否符合 RESTful 设计规范，并提供改进建议。

## RESTful 核心原则

1. **资源导向**：URL 应该表示资源（名词），而非动作（动词）
2. **标准 HTTP 方法**：使用 GET/POST/PUT/PATCH/DELETE 表示操作
3. **无状态**：每个请求包含完整的信息
4. **统一接口**：一致的命名和结构
5. **可缓存性**：适当使用 HTTP 缓存头

## 当前 API 分析

### ✅ 符合 RESTful 的端点

#### 认证相关
- ✅ `POST /api/auth/register` - 创建用户资源
- ✅ `POST /api/auth/login` - 创建会话资源
- ✅ `POST /api/auth/refresh` - 刷新令牌资源

#### 房间资源
- ✅ `POST /api/rooms` - 创建房间
- ✅ `GET /api/rooms/:room_id` - 获取房间信息
- ✅ `DELETE /api/rooms/:room_id` - 删除房间
- ✅ `GET /api/rooms/:room_id/settings` - 获取房间设置
- ✅ `GET /api/rooms/:room_id/members` - 获取房间成员列表

#### 媒体资源
- ✅ `POST /api/rooms/:room_id/media` - 添加媒体到播放列表
- ✅ `GET /api/rooms/:room_id/media` - 获取播放列表
- ✅ `DELETE /api/rooms/:room_id/media/:media_id` - 删除媒体项
- ✅ `POST /api/rooms/:room_id/media/batch` - 批量添加媒体
- ✅ `DELETE /api/rooms/:room_id/media/batch` - 批量删除媒体

#### 用户资源
- ✅ `GET /api/user/me` - 获取当前用户信息
- ✅ `GET /api/user/rooms` - 获取用户加入的房间
- ✅ `DELETE /api/user/rooms/:room_id` - 删除用户创建的房间

#### OAuth2
- ✅ `GET /api/oauth2/:provider/authorize` - 获取授权 URL
- ✅ `GET /api/oauth2/:provider/callback` - OAuth 回调
- ✅ `POST /api/oauth2/:provider/callback` - OAuth 回调（POST 方式）
- ✅ `POST /api/oauth2/:provider/bind` - 绑定 OAuth 账户
- ✅ `DELETE /api/oauth2/:provider/bind` - 解绑 OAuth 账户
- ✅ `GET /api/oauth2/providers` - 获取可用的提供商列表

#### WebRTC
- ✅ `GET /api/rooms/:room_id/webrtc/ice-servers` - 获取 ICE 服务器配置
- ✅ `GET /api/rooms/:room_id/webrtc/network-quality` - 获取网络质量

#### WebSocket
- ✅ `GET /ws/rooms/:room_id` - WebSocket 连接（正确使用 ws 前缀）

### ❌ 不符合 RESTful 的端点

#### 1. URL 中包含动作动词（最严重的问题）

##### 用户操作
- ❌ `POST /api/user/logout`
  - **问题**：使用动词 "logout"
  - **建议**：`DELETE /api/auth/session` 或 `DELETE /api/auth/token`
  - **原因**：登出应该是删除会话/令牌资源

- ❌ `POST /api/user/username`
  - **问题**：应该更新用户资源的属性
  - **建议**：`PATCH /api/user` 或 `PUT /api/user/username`
  - **原因**：更新用户名是修改用户资源

- ❌ `POST /api/user/password`
  - **问题**：应该更新用户资源的属性
  - **建议**：`PATCH /api/user` 或 `PUT /api/user/password`
  - **原因**：更新密码是修改用户资源

##### 房间操作
- ❌ `POST /api/rooms/:room_id/join`
  - **问题**：使用动词 "join"
  - **建议**：`POST /api/rooms/:room_id/members` 或 `PUT /api/rooms/:room_id/members/:user_id`
  - **原因**：加入房间是创建成员资源

- ❌ `POST /api/rooms/:room_id/leave`
  - **问题**：使用动词 "leave"
  - **建议**：`DELETE /api/rooms/:room_id/members/:user_id`
  - **原因**：离开房间是删除成员资源

- ❌ `POST /api/user/rooms/:room_id/exit`
  - **问题**：与 leave 功能重复，使用动词 "exit"
  - **建议**：统一使用上面的 DELETE 端点
  - **原因**：功能重复，应该删除

- ❌ `GET /api/room/check/:room_id`
  - **问题**：使用动词 "check"，与 `GET /api/rooms/:room_id` 功能重复
  - **建议**：直接使用 `GET /api/rooms/:room_id`，返回 404 表示不存在
  - **原因**：检查资源是否存在应该直接 GET 资源

- ❌ `POST /api/rooms/:room_id/pwd/check`
  - **问题**：使用动词 "check"
  - **建议**：`POST /api/rooms/:room_id/password/verify` 或 `POST /api/rooms/:room_id/auth`
  - **原因**：虽然是验证操作，但应该使用名词表示验证资源

##### 播放控制
- ❌ `POST /api/rooms/:room_id/playback/play`
  - **问题**：使用动词 "play"
  - **建议**：`PATCH /api/rooms/:room_id/playback` (body: `{state: "playing"}`)
  - **原因**：播放是修改播放状态

- ❌ `POST /api/rooms/:room_id/playback/pause`
  - **问题**：使用动词 "pause"
  - **建议**：`PATCH /api/rooms/:room_id/playback` (body: `{state: "paused"}`)
  - **原因**：暂停是修改播放状态

- ❌ `POST /api/rooms/:room_id/playback/seek`
  - **问题**：使用动词 "seek"
  - **建议**：`PATCH /api/rooms/:room_id/playback` (body: `{position: 123}`)
  - **原因**：跳转是修改播放位置

- ❌ `POST /api/rooms/:room_id/playback/speed`
  - **问题**：虽然是名词，但路径结构不一致
  - **建议**：`PATCH /api/rooms/:room_id/playback` (body: `{speed: 1.5}`)
  - **原因**：应该统一使用 PATCH 修改播放状态的不同属性

- ❌ `POST /api/rooms/:room_id/playback/switch`
  - **问题**：使用动词 "switch"
  - **建议**：`PATCH /api/rooms/:room_id/playback` (body: `{media_id: "xxx"}`)
  - **原因**：切换媒体是修改当前播放的媒体

##### 媒体操作
- ❌ `POST /api/rooms/:room_id/media/:media_id/edit`
  - **问题**：使用动词 "edit"
  - **建议**：`PUT /api/rooms/:room_id/media/:media_id` 或 `PATCH /api/rooms/:room_id/media/:media_id`
  - **原因**：编辑是更新资源

- ❌ `POST /api/rooms/:room_id/media/swap`
  - **问题**：使用动词 "swap"
  - **建议**：`PATCH /api/rooms/:room_id/media` (body: 包含位置变更信息)
  - **原因**：交换位置是修改媒体集合的顺序

- ❌ `POST /api/rooms/:room_id/media/clear`
  - **问题**：使用动词 "clear"
  - **建议**：`DELETE /api/rooms/:room_id/media` (删除整个集合)
  - **原因**：清空是删除所有媒体项

- ❌ `POST /api/rooms/:room_id/media/reorder`
  - **问题**：使用动词 "reorder"
  - **建议**：`PATCH /api/rooms/:room_id/media` (body: 包含新的顺序)
  - **原因**：重新排序是修改媒体集合的顺序

##### 成员管理
- ❌ `POST /api/rooms/:room_id/members/:user_id/kick`
  - **问题**：使用动词 "kick"
  - **建议**：`DELETE /api/rooms/:room_id/members/:user_id?reason=kicked`
  - **原因**：踢出是删除成员资源

- ❌ `POST /api/rooms/:room_id/members/:user_id/ban`
  - **问题**：使用动词 "ban"
  - **建议**：`POST /api/rooms/:room_id/bans` (body: `{user_id: "xxx"}`)
  - **原因**：封禁是创建一个封禁记录资源

- ❌ `POST /api/rooms/:room_id/members/:user_id/unban`
  - **问题**：使用动词 "unban"
  - **建议**：`DELETE /api/rooms/:room_id/bans/:user_id`
  - **原因**：解封是删除封禁记录

#### 2. 集合命名不一致

- ❌ `GET /api/room/list`
  - **问题**：使用单数 "room" 和动词 "list"
  - **建议**：已有 `GET /api/rooms`，应该删除此端点
  - **原因**：获取集合应该 GET 复数资源，不需要 /list 后缀

- ❌ `GET /api/room/hot`
  - **问题**：使用单数 "room" 和形容词 "hot"
  - **建议**：`GET /api/rooms?sort=hot` 或 `GET /api/rooms?filter=hot`
  - **原因**：应该使用查询参数过滤/排序，而不是额外的路径

#### 3. HTTP 方法使用不当

- ❌ `POST /api/rooms/:room_id/admin/settings`
  - **问题**：应该使用 PUT 或 PATCH
  - **建议**：`PATCH /api/rooms/:room_id/settings` (使用 PATCH 部分更新)
  - **原因**：更新资源应该用 PUT（完整替换）或 PATCH（部分更新）

- ❌ `POST /api/rooms/:room_id/admin/password`
  - **问题**：应该使用 PUT 或 PATCH
  - **建议**：`PUT /api/rooms/:room_id/password`
  - **原因**：设置密码是更新资源属性

- ❌ `POST /api/rooms/:room_id/members/:user_id/permissions`
  - **问题**：应该使用 PUT 或 PATCH
  - **建议**：`PATCH /api/rooms/:room_id/members/:user_id` (body 包含 permissions)
  - **原因**：更新权限是修改成员资源

## 改进建议总结

### 高优先级（严重违反 RESTful 原则）

1. **统一播放控制为单一端点**
   ```
   当前：5 个不同的 POST 端点
   建议：PATCH /api/rooms/:room_id/playback
   Body 示例：
   - {state: "playing"}
   - {state: "paused"}
   - {position: 123}
   - {speed: 1.5}
   - {current_media_id: "xxx"}
   ```

2. **重构成员管理**
   ```
   当前：join/leave/exit/kick 使用动词
   建议：
   - POST /api/rooms/:room_id/members (加入)
   - DELETE /api/rooms/:room_id/members/:user_id (离开/踢出)
   - POST /api/rooms/:room_id/bans (封禁)
   - DELETE /api/rooms/:room_id/bans/:user_id (解封)
   ```

3. **统一媒体操作**
   ```
   当前：edit/swap/clear/reorder 使用动词
   建议：
   - PUT/PATCH /api/rooms/:room_id/media/:media_id (编辑)
   - PATCH /api/rooms/:room_id/media (重排序/交换)
   - DELETE /api/rooms/:room_id/media (清空)
   ```

### 中优先级（改进一致性）

4. **统一用户更新操作**
   ```
   当前：POST /api/user/username, POST /api/user/password
   建议：PATCH /api/user 或 PATCH /api/user/{attribute}
   ```

5. **移除重复端点**
   ```
   删除：GET /api/room/list (使用 GET /api/rooms)
   删除：GET /api/room/check/:id (使用 GET /api/rooms/:id，404表示不存在)
   删除：POST /api/user/rooms/:room_id/exit (使用统一的 leave 端点)
   ```

6. **使用查询参数而非路径段**
   ```
   当前：GET /api/room/hot
   建议：GET /api/rooms?sort=activity 或 GET /api/rooms?filter=popular
   ```

### 低优先级（语义优化）

7. **认证端点优化**
   ```
   当前：POST /api/user/logout
   建议：DELETE /api/auth/session
   ```

8. **密码验证端点**
   ```
   当前：POST /api/rooms/:room_id/pwd/check
   建议：POST /api/rooms/:room_id/password/verify
   ```

## 向后兼容性策略

为了不破坏现有客户端，建议采用以下策略：

### 方案 A：渐进式迁移（推荐）
1. 保留旧端点，标记为 deprecated
2. 添加新的 RESTful 端点
3. 在文档中说明旧端点将在未来版本移除
4. 给客户端足够的迁移时间（如 6-12 个月）
5. 最终移除旧端点

### 方案 B：版本化 API
1. 创建 `/api/v2/` 路径下的新 RESTful API
2. 保持 `/api/` 下的旧 API 不变
3. 新功能只在 v2 中添加
4. 逐步迁移客户端到 v2

### 方案 C：别名路由（临时方案）
1. 新端点作为主要实现
2. 旧端点作为别名，内部路由到新端点
3. 添加 deprecation 警告头
4. 在文档中明确说明新旧端点的对应关系

## 实施建议

### 第一阶段：添加新端点（不破坏现有功能）
- 实现所有新的 RESTful 端点
- 与旧端点并行运行
- 在响应头中添加 `Deprecation` 和 `Link` 头指向新端点

### 第二阶段：更新文档和 OpenAPI 规范
- 标记旧端点为 deprecated
- 文档中说明新旧端点对应关系
- 提供迁移指南

### 第三阶段：客户端迁移期
- 监控旧端点的使用情况
- 联系主要客户端维护者协助迁移
- 在日志中记录旧端点的使用（不影响性能）

### 第四阶段：移除旧端点
- 在足够的迁移期后（建议 6-12 个月）
- 发布主要版本更新
- 完全移除旧端点

## RESTful 最佳实践检查清单

- [ ] URL 只包含名词（资源），不包含动词（操作）
- [ ] 使用复数形式表示集合（如 `/rooms` 而非 `/room`）
- [ ] 正确使用 HTTP 方法：
  - GET - 获取资源
  - POST - 创建资源
  - PUT - 完整替换资源
  - PATCH - 部分更新资源
  - DELETE - 删除资源
- [ ] 使用查询参数进行过滤、排序、分页
- [ ] 使用适当的 HTTP 状态码
- [ ] 提供 HATEOAS 链接（可选）
- [ ] API 版本化策略明确
- [ ] 一致的错误响应格式
- [ ] 适当的缓存头

## 结论

当前 SyncTV API 有约 **40% 的端点不符合 RESTful 规范**，主要问题是：
1. URL 中包含动作动词（最严重）
2. 使用 POST 而非 PUT/PATCH 进行更新
3. 集合命名不一致
4. 存在重复端点

建议采用**渐进式迁移策略**，在不破坏现有客户端的前提下，逐步将 API 重构为完全符合 RESTful 规范的设计。

---

**更新日期**：2026-02-08
**状态**：待审核
