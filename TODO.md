# SyncTV 重构 TODO 跟踪

**最后更新**: 2026-02-04
**当前评分**: 92/100
**目标评分**: 97/100

---

## ✅ P0 - 安全和核心功能缺失

### 已完成部分

### 安全问题（最高优先级）

- [x] **WebSocket JWT认证修复** - ✅ 已完成
  - 修改: `synctv-api/src/http/websocket.rs`
  - 实现: 从查询参数提取JWT token并验证，拒绝无效token的连接
  - 安全: 任何WebSocket连接现在都需要有效JWT

- [x] **RTMP推流认证** - ✅ 已完成
  - 修改: `synctv/src/rtmp/mod.rs`, `synctv/src/main.rs`
  - 实现: 使用PublishKeyService验证推流密钥，从JWT提取room_id并验证匹配
  - 安全: 推流现在需要专门的发布密钥，且验证room_id匹配

### 数据准确性

- [x] **房间成员计数** - ✅ 已完成
  - 修改: `synctv-api/src/impls/admin.rs`, `client.rs`
  - 实现: 使用ConnectionManager.room_connection_count()获取实时在线成员数
  - 集群: 支持集群架构下的成员计数（通过ConnectionManager）

- [x] **播放列表详细信息** - ✅ 已完成
  - 修改: `synctv-api/src/http/media.rs`, `synctv-core/src/service/room.rs`
  - 实现: 获取root playlist对象并转换为proto格式返回
  - 功能: API现在返回完整的Playlist信息（id, name, media_count等）

### 核心功能

- [x] **Direct类型Playback数据结构（多模式支持）** - ✅ 已完成
  - 位置: `synctv-core/src/models/media.rs`
  - 核心数据结构:
    - [x] `PlaybackResult` - 播放结果容器（支持多模式）
      - `playback_infos: HashMap<String, PlaybackInfo>` - 多个播放模式
      - `default_mode: String` - 默认模式名
      - `metadata: HashMap<String, JsonValue>` - 媒体级元数据（仅API返回，不存数据库）
      - `id`, `playlist_id`, `room_id`, `name`, `position` - 媒体上下文字段
    - [x] `PlaybackInfo` - 单个模式的播放信息
      - `urls: Vec<PlaybackUrl>` - 多清晰度URL列表
      - `subtitles: Vec<Subtitle>` - 字幕列表
      - `danmakus: Vec<Danmaku>` - 弹幕列表
    - [x] `PlaybackUrl` - 播放URL（含元数据）
    - [x] `PlaybackUrlMetadata` - URL元数据（分辨率、编码、比特率、FPS）
    - [x] `Subtitle`/`SubtitleUrl` - 字幕支持
    - [x] `Danmaku` - 弹幕支持
  - 数据库存储策略:
    - [x] Media表**不存储**metadata字段（已从migration和model中移除）
    - [x] source_config仅存储playback_infos, default_mode, metadata（JSONB）
    - [x] id, playlist_id, room_id, name, position存储在Media表字段
    - [x] get_playback_result()动态重组完整PlaybackResult（从表字段+source_config）
  - 实现特性:
    - [x] Media辅助方法：`is_direct()`, `get_playback_result()`, `from_direct_playback()`
    - [x] PlaybackResult Builder模式（支持多模式构建）
    - [x] PlaybackInfo Builder模式（支持单模式构建）
    - [x] 向后兼容：自动转换旧的单模式PlaybackInfo为PlaybackResult
  - 设计亮点:
    - **多模式支持**：一个媒体可同时包含direct、proxied等多种播放模式
    - **完全符合设计文档**：按照08-视频内容管理.md中"API返回示例（播放时）"实现
    - **零Provider依赖**：direct类型媒体在source_config中存储完整PlaybackResult
    - **灵活扩展**：支持自定义模式名、媒体元数据、URL元数据
    - **存储优化**：metadata仅用于API响应，不占用数据库空间
  - 优势:
    - direct类型媒体不需要Provider，播放时直接返回source_config
    - 一个媒体可提供多种播放模式（如原始链接+代理链接）
    - 每个模式支持多清晰度、多字幕、多弹幕源
    - 支持URL过期时间和自定义请求头
    - 支持媒体级元数据（duration、thumbnail、title等）
    - metadata动态生成，不浪费数据库存储空间

- [ ] **动态文件夹支持** - 1.5-2天（基础设施已完成80%）
  - **设计理念**: Playlist作为文件夹容器，Media作为文件，无需修改Media表结构
  - **架构说明**:
    - **不使用通用browse接口**：每个provider注册自己的特定API
    - **客户端生成source_config**：用户调用provider特定接口 → 返回视频信息 → 客户端生成source_config → 调用添加media API
    - **实现层级**：synctv-api/src/impls/providers（业务逻辑） → HTTP/gRPC（薄包装层）
    - **Proto定义**：synctv-proto/proto/providers/{provider}.proto

  - **现状分析**:
    - ✅ **数据模型完善** (100%):
      - Playlist模型已有动态文件夹字段：`source_provider`, `source_config`, `provider_instance_name`
      - Playlist.is_dynamic()和is_static()方法已实现
      - Media模型无需修改（作为具体文件）

    - ✅ **Provider trait架构** (100%):
      - MediaProvider trait（核心，generate_playback必须实现）
      - DynamicFolder trait（可选，list_playlist + next方法）
      - PlaybackResult, DirectoryItem, NextPlayItem等结构体已定义

    - ✅ **Proto接口定义** (100%):
      - `synctv-proto/proto/providers/bilibili.proto`: Parse, LoginQR, CheckQR, GetCaptcha, SendSMS, LoginSMS, GetUserInfo, Logout
      - `synctv-proto/proto/providers/alist.proto`: Login, **List**, GetMe, Logout, GetBinds
      - `synctv-proto/proto/providers/emby.proto`: Login, **List**, GetMe, Logout, GetBinds

    - ✅ **API Implementation骨架** (80%):
      - `synctv-api/src/impls/providers/bilibili.rs`: 已实现parse, login_qr, check_qr等方法
      - `synctv-api/src/impls/providers/alist.rs`: 已实现login, **list**, get_me等方法
      - `synctv-api/src/impls/providers/emby.rs`: 已实现login, **list**, get_me等方法

    - ✅ **HTTP路由骨架** (80%):
      - `synctv-api/src/http/providers/bilibili.rs`: HTTP handler已存在
      - `synctv-api/src/http/providers/alist.rs`: HTTP handler已存在
      - `synctv-api/src/http/providers/emby.rs`: HTTP handler已存在

  - ❌ **待实现部分** (预计1.5-2天):

    - [ ] **1. Provider特定接口完善** (1天)
      - [ ] **Bilibili** (0.3天):
        - ✅ Parse接口已实现（返回VideoInfo列表，包含bvid/cid/epid）
        - ✅ 登录相关已实现
        - [ ] 验证parse返回的数据格式符合客户端生成source_config的需求
        - [ ] 确认parse接口是否需要返回更多metadata（duration, thumbnail等）

      - [ ] **Alist** (0.3天):
        - ✅ List接口已实现（返回FileItem列表，包含name/size/is_dir）
        - ✅ Login已实现
        - [ ] 验证List接口是否支持relative_path参数进行子目录导航
        - [ ] 实现DynamicFolder trait的list_playlist()方法（内部调用List接口）
        - [ ] 实现DynamicFolder trait的next()方法（用于自动连播）

      - [ ] **Emby** (0.4天):
        - ✅ List接口已实现（返回MediaItem列表，包含id/name/type）
        - ✅ Login已实现
        - [ ] 验证List接口是否支持parent_id参数进行层级导航
        - [ ] 实现DynamicFolder trait的list_playlist()方法（内部调用List接口）
        - [ ] 实现DynamicFolder trait的next()方法（用于自动连播）

    - [ ] **2. 动态播放列表API** (0.5天)
      - [ ] `GET /api/rooms/{room_id}/playlists/{playlist_id}/items?relative_path=xxx`
        - 检查playlist是否为动态类型（source_provider != null）
        - 调用DynamicFolder.list_playlist()获取内容
        - 返回DirectoryItem列表
        - 客户端根据返回数据决定：继续导航（is_dir=true）或播放（is_dir=false）
      - [ ] 集成到现有的playlist API中

    - [ ] **3. 播放session支持动态媒体** (不需要，设计变更)
      - ❌ ~~room_playback_session添加relative_path字段~~（不需要）
      - ✅ **新设计**：动态文件夹播放时，直接创建临时Media记录
        - 用户选择动态文件夹中的视频 → 客户端调用 `/api/rooms/{room_id}/media/add`
        - Media.source_config = 完整配置（playlist base_path + relative_path合并后）
        - 播放session正常记录media_id即可
        - 优势：简化设计，避免media_id和relative_path互斥的复杂逻辑

  - **核心流程**:

    **流程A - Bilibili添加视频**:
    ```
    1. 用户输入URL → POST /api/providers/bilibili/parse
    2. 返回 ParseResponse: { title, videos: [{bvid, cid, epid, name, cover}] }
    3. 客户端生成 source_config: {"bvid": "xxx", "cid": 123, "epid": 0}
    4. 客户端调用 POST /api/rooms/{room_id}/media/add {source_provider: "bilibili", source_config: {...}}
    ```

    **流程B - Alist浏览文件夹**:
    ```
    1. 用户登录Alist → POST /api/providers/alist/login
    2. 浏览根目录 → POST /api/providers/alist/list {path: "/"}
    3. 返回 ListResponse: { content: [{name, is_dir, ...}] }
    4. 用户点击子目录 → POST /api/providers/alist/list {path: "/movies"}
    5. 用户选择视频 → 客户端生成 source_config: {"path": "/movies/video.mp4"}
    6. 客户端调用 POST /api/rooms/{room_id}/media/add
    ```

    **流程C - 动态播放列表（Alist文件夹）**:
    ```
    1. 用户添加动态播放列表 → POST /api/rooms/{room_id}/playlists/add {source_provider: "alist", source_config: {"path": "/movies"}}
    2. 用户浏览动态播放列表 → GET /api/rooms/{room_id}/playlists/{id}/items?relative_path=/action
    3. 返回该文件夹下的视频列表（调用DynamicFolder.list_playlist()）
    4. 用户点击播放 → 客户端创建临时Media → 播放（source_config = 合并后的完整路径）
    ```

  - **技术要点**:
    - 每个provider提供不同的能力接口（Parse/List/Search等）
    - 客户端根据provider返回数据生成source_config
    - 添加media时统一使用 `/api/rooms/{room_id}/media/add` 接口
    - 动态播放列表通过DynamicFolder trait支持

---

## 🟡 P1 - 重要功能（2-3周）

### API接入

- [x] **Provider实例管理API** - ✅ 已完成
  - 位置: `synctv-api/src/impls/admin.rs:294-477`
  - 状态: 已完整实现并集成到HTTP路由
  - 已完成:
    - [x] 注入ProviderInstanceManager到AdminApiImpl
    - [x] 实现`list_provider_instances`
    - [x] 实现`add_provider_instance`
    - [x] 实现`set_provider_instance`
    - [x] 实现`delete_provider_instance`
    - [x] 实现`reconnect_provider_instance`
    - [x] 实现`enable_provider_instance`
    - [x] 实现`disable_provider_instance`
    - [x] 添加`provider_instance_to_proto`辅助函数

### 功能完善

- [x] **弹幕完整流程** - ✅ 已完成
  - 位置: `proto/client.proto`, `synctv-cluster/src/sync/events.rs`, `synctv-api/src/impls/messaging.rs`
  - 已完成:
    - [x] 统一消息类型 - Chat消息支持可选的position和color字段
    - [x] 删除单独的Danmaku消息类型 - 客户端根据position决定显示方式
    - [x] 弹幕实时广播 - StreamMessageHandler + ClusterEvent
    - [x] 弹幕过滤和限流 - ContentFilter.filter_danmaku() + RateLimiter
    - [x] 权限控制 - SEND_CHAT权限检查（统一聊天和弹幕权限）
  - 设计原则:
    - **统一消息系统**: 不区分chat和danmaku，都是ChatMessage
    - **客户端决定展示**: position字段存在→显示为弹幕，否则→显示为聊天
    - **历史消息不回放**: 获取历史消息时position=None，只显示在聊天框
    - **媒体弹幕来自Provider**: PlaybackResult.danmakus包含媒体弹幕（从Bilibili等获取）
  - 功能:
    - 用户发送带position的消息 → 实时广播 → 客户端显示为弹幕
    - 用户发送普通消息 → 存储到数据库 → 历史记录查询
    - Provider返回媒体弹幕 → 客户端渲染在视频上

- [ ] **WebRTC端到端测试** - 5-7天
  - 状态: WebRTCSignalingService存在，但未充分测试
  - 任务:
    - [ ] 编写WebRTC集成测试
    - [ ] 添加STUN/TURN配置
    - [ ] 测试多人通话
    - [ ] 验证音视频权限控制

### 系统完善

- [x] **系统设置热重载验证** - ✅ 已完成
  - 位置: `migrations/20240201120002_add_settings_notify_trigger.sql`, `synctv-core/src/service/settings.rs`
  - 已完成:
    - [x] PostgreSQL NOTIFY/LISTEN已实现
    - [x] 数据库触发器自动发送通知
    - [x] SettingsService监听'settings_changed'频道
    - [x] 后台任务自动重载变更的设置
    - [x] 支持多节点配置同步（通过PostgreSQL LISTEN/NOTIFY）
  - 功能:
    - settings表INSERT/UPDATE/DELETE时自动触发pg_notify
    - SettingsService.start_listen_task()启动后台监听任务
    - reload_setting()自动刷新缓存和通知本地监听器
    - 连接断开自动重连（5秒延迟）
    - 零停机配置热更新

- [x] **审计日志分区自动化** - ✅ 已完成
  - 位置: `synctv-core/src/service/audit_partition_manager.rs`, `synctv/src/main.rs:64-77`
  - 已完成:
    - [x] AuditPartitionManager在启动时运行（ensure_audit_partitions_on_startup）
    - [x] 定时任务已启动（start_auto_management，每24小时检查一次）
    - [x] 自动创建未来6个月的分区
    - [x] 自动确保现有分区的索引
    - [x] 健康检查和统计功能
  - 功能:
    - 启动时自动创建未来6个月的分区
    - 每24小时自动检查并创建缺失的分区
    - 支持手动删除旧分区（keep_months参数）
    - 完整的健康检查和统计信息API

- [x] **邮件模板系统** - ✅ 已完成
  - 位置: `synctv-core/src/service/email_templates.rs`, `synctv-core/src/service/email.rs`
  - 已完成:
    - [x] HTML邮件模板（验证邮件、密码重置、测试邮件、通知）
    - [x] 集成Handlebars模板引擎
    - [x] 模板变量替换系统
    - [x] 实际SMTP发送（使用lettre）
    - [x] HTML + 纯文本备用（MultiPart）
  - 功能:
    - EmailTemplateManager管理所有邮件模板
    - 支持4种邮件类型：EmailVerification、PasswordReset、TestEmail、Notification
    - 响应式HTML设计，适配移动设备
    - 使用Handlebars进行变量替换
    - 通过lettre实现SMTP发送（支持TLS）
    - HTML邮件自动包含纯文本备用内容
    - 精美的邮件样式（SyncTV品牌色、圆角、阴影、图标）

- [x] **分布式锁实现** - ✅ 已完成
  - 位置: `synctv-core/src/service/distributed_lock.rs`
  - 已完成:
    - [x] Redis分布式锁实现（SET NX EX原子操作）
    - [x] acquire/release方法（Lua脚本保证原子性）
    - [x] with_lock RAII模式（自动获取和释放）
    - [x] try_with_lock（非阻塞尝试）
    - [x] LockGuard（自动释放守卫）
    - [x] extend方法（延长锁TTL）
  - 功能:
    - 使用Redis SET NX EX原子操作获取锁
    - Lua脚本确保只有锁持有者能释放锁
    - 支持RAII模式自动释放
    - 支持锁TTL延长（用于长时间操作）
  - 使用场景:
    - 创建房间（防止重复创建）
    - 更新房间设置（防止并发冲突）
    - Publisher注册（已使用HSETNX实现）
    - 其他需要跨副本互斥的操作

- [x] **测试邮件功能（Admin）** - ✅ 已完成
  - 位置: `synctv-core/src/service/email.rs`, `synctv-api/src/impls/admin.rs:272`
  - 实现了EmailService::send_test_email方法
  - AdminService::send_test_email调用EmailService发送测试邮件

---

## 🟢 P2 - 优化和完善（可延后）

### 监控和文档

- [ ] **Prometheus监控集成** - 3-5天
  - 添加prometheus和metrics依赖
  - 创建PrometheusService
  - 添加`/metrics`端点
  - 埋点关键路径（http/grpc/websocket/streaming/cache/database）

- [ ] **API文档自动生成** - 4-5天
  - 为所有HTTP端点添加`#[utoipa::path]`
  - 为所有DTO添加`#[derive(ToSchema)]`
  - 添加`/swagger-ui`端点

### 流媒体优化

- [ ] **GOP缓存验证和测试** - 2天
  - 代码有，需要测试
  - RTMP推流测试
  - HLS/FLV拉流首帧延迟测试

- [ ] **OSS存储集成** - 3-4天
  - 实现S3Storage（AWS S3）
  - 实现AliyunOssStorage（阿里云OSS）
  - 配置系统集成

### 测试覆盖（持续）

- [ ] **单元测试扩展** - 5天
  - [ ] PermissionService权限检查逻辑
  - [ ] RoomService房间操作
  - [ ] PlaybackService播放同步
  - [ ] AuthService JWT生成验证
  - [ ] ProviderClient解析逻辑

- [ ] **集成测试扩展** - 5天
  - [ ] 用户注册登录流程
  - [ ] 房间创建加入流程
  - [ ] 媒体添加播放流程
  - [ ] WebSocket实时消息
  - [ ] OAuth2授权回调
  - [ ] Provider集成（Bilibili/Alist/Emby）

- [ ] **端到端测试** - 5天
  - [ ] 浏览器测试（Selenium/Playwright）
  - [ ] RTMP推流到HLS/FLV完整链路
  - [ ] 多用户房间同步
  - [ ] 集群多节点测试

---

## 📋 已知偏离设计文档（非阻塞）

- [x] **Role和Status改为数字类型** - ✅ 已完成
  - 问题: 当前使用VARCHAR(20)存储字符串（"root", "admin", "user"等）
  - 影响: 浪费存储空间，查询性能差，索引效率低
  - 已完成:
    - [x] users表: `role`, `status` (VARCHAR → SMALLINT)
    - [x] rooms表: `status` (VARCHAR → SMALLINT)
    - [x] room_members表: `role`, `status` (VARCHAR → SMALLINT)
  - 数字映射:
    ```
    UserRole: root=1, admin=2, user=3
    UserStatus: active=1, pending=2, banned=3
    RoomStatus: active=1, pending=2, banned=3
    MemberRole: creator=1, admin=2, member=3, guest=4
    MemberStatus: active=1, pending=2, banned=3
    ```
  - 已修改:
    - [x] 修改迁移脚本（直接修改原始文件，使用SMALLINT和CHECK约束）
    - [x] 添加Rust枚举的sqlx::Type实现（i16 ↔ Enum）
      - UserRole, UserStatus in `synctv-core/src/models/user.rs`
      - RoomStatus in `synctv-core/src/models/room.rs`
      - Role (RoomRole) in `synctv-core/src/models/permission.rs`
      - MemberStatus in `synctv-core/src/models/room_member.rs`
    - [x] 数据库迁移文件已更新，使用SMALLINT类型和数字约束

- [x] **ID字段类型优化** - ✅ 已完成
  - 检查结果: 所有nanoid ID字段已使用CHAR(12)
  - 已验证:
    - 主键ID: CHAR(12) ✅ (users, rooms, media, playlists, chat_messages等)
    - 外键ID: CHAR(12) ✅ (user_id, room_id, media_id, playlist_id, creator_id等)
    - 特殊ID: 保持VARCHAR的正确情况
      - `provider_user_id VARCHAR(255)` - OAuth2 provider的用户ID（非nanoid）
      - `email VARCHAR(255)` - 邮箱地址
      - `token VARCHAR(255)` - 各种token
  - 结论: 数据库设计完全符合规范，无需修改

- [x] **聊天消息保留策略** - ✅ 已完成
  - 位置: `synctv-core/src/repository/chat.rs`, `synctv-core/src/service/chat.rs`, `synctv-core/src/service/global_settings.rs`
  - 已完成:
    - [x] 移除数据库触发器实现（改为应用层控制）
    - [x] 移除 `deleted_at` 字段（聊天消息不需要软删除，直接物理删除）
    - [x] 添加全局设置 `max_chat_messages` (默认500, 0=无限制)
    - [x] 实现 `ChatRepository::cleanup_all_rooms()` - 单SQL批量清理活跃房间
    - [x] 实现 `ChatService::cleanup_room_messages()` - 清理单个房间
    - [x] 实现 `ChatService::cleanup_all_rooms()` - 调用批量清理
    - [x] 实现 `ChatService::start_cleanup_task()` - 后台定期清理任务
  - 实现方式:
    - **核心优化**: 使用PostgreSQL窗口函数（ROW_NUMBER() OVER PARTITION BY）在单条SQL中清理所有房间
    - **活动窗口过滤**: 只处理3分钟内有新消息的房间，避免扫描不活跃房间
    - **近实时清理**: 每1分钟运行一次，及时保持消息数量在限制内
    - **物理删除**: 移除软删除机制，简化查询逻辑，减少存储成本
    - 全局设置：`server.max_chat_messages` (可通过Admin API动态修改)
    - 从SettingsRegistry读取最新配置，支持热更新
  - SQL实现:
    ```sql
    DELETE FROM chat_messages WHERE id IN (
        SELECT id FROM (
            SELECT id, room_id,
                   ROW_NUMBER() OVER (PARTITION BY room_id ORDER BY created_at DESC) as rn
            FROM chat_messages
            WHERE room_id IN (
                SELECT DISTINCT room_id FROM chat_messages
                WHERE created_at >= NOW() - INTERVAL '3 minutes'
            )
        ) ranked_messages WHERE rn > $1
    );
    ```
  - 优势:
    - **生产级性能**: 单条SQL处理所有房间，无内存压力，支持百万级房间
    - **高效扫描**: 只处理活跃房间（3分钟内有消息），大幅减少扫描成本
    - **近实时**: 1分钟清理间隔，消息数量始终在限制附近
    - **简化查询**: 移除软删除后，所有查询不再需要 `WHERE deleted_at IS NULL` 条件
    - **减少存储**: 不保留软删除的数据，节省磁盘空间
    - 灵活配置：管理员可以随时调整保留数量
    - 利用索引：使用 `idx_chat_messages_room_pagination` 索引优化查询
    - 集群友好：每个节点独立运行清理任务

- [x] **JWT过期时间检查** - ✅ 已完成
  - 位置: `synctv-core/src/service/auth/jwt.rs:132-135`
  - 设计要求: Access Token 1小时，Refresh Token 30天
  - 已验证:
    - Access Token: `Duration::hours(1)` ✅
    - Refresh Token: `Duration::days(30)` ✅
  - 结论: 完全符合设计文档要求

---

## 工作量总结

| 阶段 | 内容 | 工作量 | 人员 |
|------|------|--------|------|
| **Phase 1** | P0安全和核心功能 | 5天 | 1后端 |
| **Phase 2** | P0+P1功能完善 | 11天 | 2后端 |
| **Phase 3** | P1高级功能 | 15天 | 2后端 |
| **Phase 4** | P2测试优化 | 12天 | 1后端+1测试 |
| **Phase 5** | 数据库优化（可选） | 3天 | 1后端 |
| **总计** | | **46天** | **2-3人** |

**时间线**:
- 单人全职: 9周（2个月）
- 双人并行: 5-6周（1.5个月）
- 三人并行: 4周（1个月）

---

## 上线评估

### ⚠️ 当前状态: 不建议立即上线

**阻塞因素**:
1. 2个严重安全问题（WebSocket/RTMP认证）
2. 3个核心功能缺失（多源、文件夹、成员计数）
3. 测试覆盖不足（60%）

**上线时间建议**:
- 最快: 1个月（完成Phase 1-2）
- 推荐: 2个月（完成Phase 1-3）
- 理想: 3个月（完成所有Phase）

---

## 快速链接

**关键文件**:
- WebSocket: `synctv-api/src/http/websocket.rs`
- RTMP: `synctv/src/rtmp/mod.rs`
- Admin API: `synctv-api/src/impls/admin.rs`
- Media模型: `synctv-core/src/models/media.rs`
- Provider管理: `synctv-core/src/service/provider_instance_manager.rs`

**数据库迁移**:
- 多源: `migrations/XXX_add_more_sources_to_media.sql`
- 文件夹: `migrations/XXX_add_parent_id_to_media.sql`
- 弹幕: `migrations/XXX_add_danmu_fields_to_media.sql`
- Role/Status数字化: `migrations/XXX_convert_roles_status_to_int.sql`

**当前数据库问题**:
- users: `role VARCHAR(20)`, `status VARCHAR(20)` → 应改为 `SMALLINT`
- rooms: `status VARCHAR(20)` → 应改为 `SMALLINT`
- room_members: `role VARCHAR(20)`, `status VARCHAR(20)` → 应改为 `SMALLINT`
