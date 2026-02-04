# SyncTV 重构 TODO 跟踪

**最后更新**: 2026-02-05
**当前评分**: 98/100
**目标评分**: 100/100
**P0状态**: ✅ 全部完成！
**P1进度**: WebRTC Phase 1-3已完成（信令+STUN+TURN），Phase 4-5待实施

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

- [x] **动态文件夹支持** - ✅ 已完成
  - **设计理念**: Playlist作为文件夹容器，Media作为文件，无需修改Media表结构
  - **架构说明**:
    - **不使用通用browse接口**：每个provider注册自己的特定API
    - **客户端生成source_config**：用户调用provider特定接口 → 返回视频信息 → 客户端生成source_config → 调用添加media API
    - **实现层级**：synctv-api/src/impls/providers（业务逻辑） → HTTP/gRPC（薄包装层）
    - **Proto定义**：synctv-proto/proto/providers/{provider}.proto

  - **完成情况**:
    - ✅ **数据模型完善** (100%):
      - Playlist模型已有动态文件夹字段：`source_provider`, `source_config`, `provider_instance_name`
      - Playlist.is_dynamic()和is_static()方法已实现
      - Media模型无需修改（作为具体文件）

    - ✅ **Provider trait架构** (100%):
      - MediaProvider trait（核心，generate_playback必须实现）
      - DynamicFolder trait（可选，list_playlist + next方法）
      - PlaybackResult, DirectoryItem, NextPlayItem等结构体已定义
      - MediaProvider新增as_dynamic_folder()方法用于能力检测

    - ✅ **Proto接口定义** (100%):
      - `synctv-proto/proto/client.proto`: 新增ListPlaylistItemsRequest/Response, DirectoryItem, ItemType
      - `synctv-proto/proto/providers/bilibili.proto`: Parse, LoginQR, CheckQR, GetCaptcha, SendSMS, LoginSMS, GetUserInfo, Logout
      - `synctv-proto/proto/providers/alist.proto`: Login, **List**, GetMe, Logout, GetBinds
      - `synctv-proto/proto/providers/emby.proto`: Login, **List**, GetMe, Logout, GetBinds

    - ✅ **完整实现** (100%):

      - ✅ **Bilibili**:
        - Parse接口已实现（返回VideoInfo列表，包含bvid/cid/epid）
        - 登录相关已实现
        - VideoInfo包含所有必需字段（bvid, cid, epid, name, coverImage）

      - ✅ **Alist**:
        - List接口已实现（返回FileItem列表，包含name/size/is_dir）
        - Login已实现
        - ✅ 实现DynamicFolder trait的list_playlist()方法（`synctv-core/src/provider/alist.rs:284`）
        - ✅ 实现DynamicFolder trait的next()方法（支持RepeatOne/Sequential/RepeatAll/Shuffle）
        - ✅ 实现as_dynamic_folder()方法返回DynamicFolder能力

      - ✅ **Emby**:
        - List接口已实现（返回MediaItem列表，包含id/name/type）
        - Login已实现
        - ✅ 实现DynamicFolder trait的list_playlist()方法（`synctv-core/src/provider/emby.rs:288`）
        - ✅ 实现DynamicFolder trait的next()方法（支持RepeatOne/Sequential/RepeatAll/Shuffle）
        - ✅ 实现as_dynamic_folder()方法返回DynamicFolder能力

    - ✅ **动态播放列表API** (100%):
      - ✅ 核心服务：`MediaService::list_dynamic_playlist_items()` (`synctv-core/src/service/media.rs:396`)
      - ✅ HTTP路由：`GET /api/rooms/{room_id}/playlists/{playlist_id}/items` (`synctv-api/src/http/media.rs:90`)
      - ✅ gRPC接口：`MediaService::list_playlist_items()` (`synctv-api/src/grpc/client_service.rs:1717`)
      - ✅ 权限检查：VIEW_PLAYLIST权限
      - ✅ Provider能力检测：通过as_dynamic_folder()检测
      - ✅ 支持分页：page, page_size参数
      - ✅ 支持相对路径导航：relative_path参数

    - ✅ **播放session支持** (设计变更):
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

### WebRTC实时通信

- [ ] **WebRTC完整架构（生产级）** - 预计15-20天

  **设计原则**：
  - ✅ **模块化架构**：信令转发、STUN、TURN、SFU独立可选
  - ✅ **配置驱动**：部署者可根据资源情况选择模式
  - ✅ **渐进式增强**：从零成本P2P到企业级SFU
  - ❌ **不实现录制**：录制功能暂不纳入计划

  详细实施计划见下方独立章节。

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

- [ ] **WebRTC完整架构（生产级）** - 预计15-20天

  **设计原则**：
  - ✅ **模块化架构**：信令转发、STUN、TURN、SFU独立可选
  - ✅ **配置驱动**：部署者可根据资源情况选择模式
  - ✅ **渐进式增强**：从零成本P2P到企业级SFU
  - ❌ **不实现录制**：录制功能暂不纳入计划

  **⚠️ 首先清理过度设计代码**：
  - 删除 `synctv-core/src/service/webrtc/*` 整个模块
  - 删除 `synctv-api/src/http/webrtc.rs` HTTP REST API
  - 删除 AppState中的`webrtc_service`字段
  - **原因**：当前实现试图构建SFU但不完整，重新设计更高效

---

### Phase 1: 基础信令转发（P2P模式）- 1-2天

**目标**：实现零成本的P2P WebRTC信令中继

- [ ] **清理旧代码并重构配置**
  - 删除旧的WebRTC模块
  - 重新设计`WebRTCConfig`支持多种模式

- [ ] **Proto定义** - `synctv-proto/proto/client.proto`
  ```protobuf
  message WebRTCData {
    string data = 1;        // Offer/Answer/ICE的JSON字符串（opaque）
    string to = 2;          // 目标："user_id:conn_id"
    string from = 3;        // 发送者（服务器自动设置，防止伪造）
  }

  // 添加消息类型
  ELEMENT_TYPE_WEBRTC_OFFER = 14;
  ELEMENT_TYPE_WEBRTC_ANSWER = 15;
  ELEMENT_TYPE_WEBRTC_ICE_CANDIDATE = 16;
  ELEMENT_TYPE_WEBRTC_JOIN = 17;
  ELEMENT_TYPE_WEBRTC_LEAVE = 18;
  ```

- [ ] **WebSocket Handler** - `synctv-api/src/http/websocket.rs`
  - 实现5个消息处理函数：
    - `handle_webrtc_offer()` - 转发Offer（1对1）
    - `handle_webrtc_answer()` - 转发Answer（1对1）
    - `handle_webrtc_ice_candidate()` - 转发ICE候选（1对1）
    - `handle_webrtc_join()` - 广播Join（通知房间内其他RTC用户）
    - `handle_webrtc_leave()` - 广播Leave
  - 权限检查：`USE_WEBRTC` permission
  - 防伪造：服务器强制设置`from`字段
  - 状态跟踪：`ConnectionInfo.rtc_joined: bool`

- [ ] **配置系统**
  ```rust
  pub struct WebRTCConfig {
      // 模式选择
      pub mode: WebRTCMode,

      // STUN配置
      pub enable_builtin_stun: bool,
      pub builtin_stun_port: u16,
      pub builtin_stun_host: String,
      pub external_stun_servers: Vec<String>,

      // TURN配置
      pub enable_turn: bool,
      pub turn_server_url: Option<String>,
      pub turn_static_secret: Option<String>,
      pub turn_credential_ttl: u64,

      // SFU配置
      pub sfu_threshold: Option<usize>,  // 超过N人自动切换SFU
      pub enable_simulcast: bool,
      pub max_sfu_rooms: usize,
  }

  pub enum WebRTCMode {
      // 模式1：纯P2P（零成本）
      PeerToPeer,

      // 模式2：混合模式（推荐）
      Hybrid {
          sfu_threshold: usize,  // 如5人以上用SFU
      },

      // 模式3：纯SFU（企业级）
      SFU,

      // 模式4：禁用（仅信令转发，无STUN/TURN）
      SignalingOnly,
  }
  ```

**工作量**：1-2天，约200行代码
**成本**：零（纯转发，不消耗服务器资源）
**连接成功率**：约70-75%（取决于用户NAT类型）

---

### Phase 2: 内置STUN服务器 - ✅ 已完成

**目标**：提升P2P连接成功率到85-90%

- [x] **依赖集成**
  - 自实现RFC 5389 STUN协议（无需外部依赖）
  - 手动实现字节流解析和构造

- [x] **STUN服务器实现** - 在`synctv-core/src/service/stun.rs`
  ```rust
  pub struct StunServer {
      socket: UdpSocket,
      listen_addr: SocketAddr,
  }

  impl StunServer {
      // 启动STUN服务
      pub async fn start(host: String, port: u16) -> Result<Self>;

      // 主循环：接收Binding Request，返回Binding Response
      pub async fn run(&self) -> Result<()> {
          loop {
              let (msg, addr) = self.socket.recv_from().await?;

              // 解析STUN消息
              if let Ok(binding_request) = parse_stun_message(&msg) {
                  // 构造响应：告诉客户端其公网IP和端口
                  let response = StunBindingResponse {
                      xor_mapped_address: addr,  // 客户端的公网地址
                      message_integrity: compute_hmac(...),
                  };

                  self.socket.send_to(&response.encode(), addr).await?;
              }
          }
      }
  }
  ```

- [x] **启动集成** - `synctv/src/main.rs`
  ```rust
  if config.webrtc.enable_builtin_stun {
      let stun = StunServer::start(
          config.webrtc.builtin_stun_host.clone(),
          config.webrtc.builtin_stun_port,
      ).await?;

      tokio::spawn(async move {
          if let Err(e) = stun.run().await {
              error!("STUN server error: {}", e);
          }
      });

      info!("Built-in STUN server listening on {}:{}",
          config.webrtc.builtin_stun_host,
          config.webrtc.builtin_stun_port
      );
  }
  ```

- [x] **ICE服务器配置API** (已在Phase 1实现)
  - gRPC: `GetIceServers()` → 返回STUN/TURN列表
  - HTTP: `GET /api/webrtc/ice-servers`
  ```rust
  pub async fn get_ice_servers(user_id: UserId) -> Vec<IceServer> {
      let mut servers = vec![];

      // 内置STUN
      if config.enable_builtin_stun {
          servers.push(IceServer {
              urls: vec![format!("stun:{}:{}",
                  config.server.host,
                  config.builtin_stun_port)],
              username: None,
              credential: None,
          });
      }

      // 外部STUN（如Google）
      for url in &config.external_stun_servers {
          servers.push(IceServer {
              urls: vec![url.clone()],
              username: None,
              credential: None,
          });
      }

      servers
  }
  ```

**工作量**：2-3天
**成本**：极低（UDP消息，每次请求<200字节）
**连接成功率**：85-90%

---

### Phase 3: TURN服务器集成 - 3-4天

**目标**：实现99%+连接成功率（支持Symmetric NAT）

- [ ] **方案选择**：集成coturn（推荐）
  - coturn作为独立服务运行
  - SyncTV生成临时凭证（HMAC-SHA1）
  - 避免实现完整TURN协议（工作量巨大）

- [ ] **TURN凭证服务** - `synctv-core/src/service/turn.rs`
  ```rust
  pub struct TurnCredentialService {
      static_secret: String,
      ttl: Duration,
  }

  impl TurnCredentialService {
      // 生成时间限制的临时凭证
      pub fn generate_credential(&self, username: &str) -> TurnCredential {
          let expiry = (Utc::now() + self.ttl).timestamp();
          let username = format!("{}:{}", expiry, username);

          // HMAC-SHA1签名
          let mut mac = HmacSha1::new_from_slice(self.static_secret.as_bytes())?;
          mac.update(username.as_bytes());
          let password = base64::encode(mac.finalize().into_bytes());

          TurnCredential { username, password, expiry }
      }
  }
  ```

- [ ] **配置集成**
  ```toml
  # config.toml
  [webrtc]
  mode = "hybrid"  # PeerToPeer | Hybrid | SFU | SignalingOnly

  # STUN配置
  enable_builtin_stun = true
  builtin_stun_port = 3478
  builtin_stun_host = "0.0.0.0"
  external_stun_servers = ["stun:stun.l.google.com:19302"]

  # TURN配置（可选）
  enable_turn = false  # 🔧 部署者可关闭以节省带宽
  turn_server_url = "turn:turn.example.com:3478"
  turn_static_secret = "your-secret-key"
  turn_credential_ttl = 86400  # 24小时
  ```

- [ ] **GetIceServers API增强**
  ```rust
  pub async fn get_ice_servers(user_id: UserId) -> Vec<IceServer> {
      let mut servers = vec![];

      // STUN servers...
      // (同Phase 2)

      // TURN server
      if config.enable_turn {
          let cred = turn_service.generate_credential(&user_id.to_string());
          servers.push(IceServer {
              urls: vec![config.turn_server_url.clone()],
              username: Some(cred.username),
              credential: Some(cred.password),
          });
      }

      servers
  }
  ```

- [ ] **coturn部署文档**
  ```bash
  # 安装
  apt-get install coturn

  # 配置 /etc/turnserver.conf
  listening-port=3478
  realm=synctv.example.com
  use-auth-secret
  static-auth-secret=<与SyncTV配置同步>

  # 限制带宽（可选）
  max-bps=1000000  # 每连接1Mbps
  total-quota=100  # 最多100个连接

  # 启动
  systemctl start coturn
  ```

**工作量**：3-4天
**成本**：中等（10%用户需要TURN，约占总流量10%）
**连接成功率**：99%+

**带宽成本估算**：
- 假设1000并发用户，10%需要TURN = 100人
- 每人1Mbps视频 × 2（上下行）= 200Mbps
- 月流量：200Mbps × 86400 × 30 ≈ 64TB
- 成本（阿里云）：约¥6400/月

**优化策略**：
- 配置`enable_turn = false`可完全关闭（成本为0）
- 设置`max-bps`限制单个连接带宽
- 提示企业用户自建TURN服务器

---

### Phase 4: SFU架构（大房间支持）- 8-10天 🔄 进行中 (60%完成)

**目标**：支持10人以上大房间，降低客户端带宽压力

**当前进度**：2026-02-05

#### ✅ 已完成 (60%)

- [x] **synctv-sfu Crate 创建** ✅
  - 位置: `/synctv-sfu/`
  - 依赖: `webrtc = "0.11"`, tokio, dashmap, parking_lot等
  - 完整的模块化架构

- [x] **基础类型系统** (`types.rs`) - 100% ✅
  - `PeerId`, `RoomId`, `TrackId` 类型定义
  - 完整的 Display 和 From trait 实现

- [x] **SFU配置** (`config.rs`) - 100% ✅
  - `SfuConfig` 结构体
  - sfu_threshold, max_sfu_rooms, max_peers_per_room
  - enable_simulcast, simulcast_layers配置
  - max_bitrate_per_peer, enable_bandwidth_estimation

- [x] **Track模块** (`track.rs`) - 100% 完整实现 ✅
  - ✅ `MediaTrack` 完整实现
  - ✅ `TrackKind` (Audio/Video)
  - ✅ `QualityLayer` (High/Medium/Low) with Simulcast支持
  - ✅ `ForwardablePacket` 结构用于RTP转发
  - ✅ RTP packet读取循环 (`start_reading`)
  - ✅ 完整统计收集 (packets, bytes, bitrate, packet_loss)
  - ✅ 带宽自适应质量选择 (`QualityLayer::from_bandwidth`)
  - ✅ Track生命周期管理 (activate/deactivate)
  - ✅ 与webrtc-rs完整集成 (TrackRemote, RTCRtpReceiver)

- [x] **Peer模块** (`peer.rs`) - 100% 完整实现 ✅
  - ✅ `SfuPeer` 完整实现
  - ✅ WebRTC PeerConnection集成
  - ✅ Track发布管理 (`published_tracks`)
  - ✅ Track订阅管理 (`subscribed_tracks` with quality layer)
  - ✅ **BandwidthEstimator** - 完整带宽估算算法
    - 基于最近1秒数据窗口
    - 指数平滑 (smoothing_factor = 0.8)
    - 每500ms更新一次
  - ✅ **自适应质量调整** - 根据带宽自动切换质量层
    - 带宽变化超过500kbps时触发
    - 自动为所有订阅轨道更新质量
  - ✅ 控制消息处理 (`PeerControlMessage`)
    - UpdateQuality: 更新轨道质量层
    - ForwardPacket: 转发RTP packet到peer
    - Close: 关闭peer连接
  - ✅ RTP packet转发 (`forward_packet`)
  - ✅ TrackLocalStaticRTP用于发送到peer
  - ✅ RTCP处理任务
  - ✅ 完整统计 (`PeerStats`)
    - packets/bytes received/sent
    - bitrate, available_bandwidth
    - rtt, packet_loss_rate, quality_score
  - ✅ Peer生命周期管理

#### 🔄 待完成 (40%)

- [ ] **Room模块** (`room.rs`) - 需要完整实现 (当前仅基础框架)
  - [ ] 完整的媒体转发逻辑
    - 从发布者读取RTP packets
    - 路由到所有订阅者
    - 根据订阅者的quality layer过滤
  - [ ] P2P ↔ SFU 自动模式切换
    - 完善 `check_mode_switch` 逻辑
    - 实现 `switch_to_sfu` 和 `switch_to_p2p`
    - 通知信令层模式变化
  - [ ] Track路由和订阅管理
    - 实现 `forward_track_to_subscribers`
    - 处理新peer加入时的track订阅
    - 处理peer离开时的清理
  - [ ] Simulcast处理
    - 多质量层track管理
    - 动态质量层切换
  - [ ] 完整统计收集 (`RoomStats`)

- [ ] **Manager模块** (`manager.rs`) - 需要完整实现 (当前仅基础框架)
  - [ ] 多房间管理
  - [ ] 资源限制检查
    - max_sfu_rooms限制
    - max_peers_per_room限制
  - [ ] 房间生命周期管理
  - [ ] 空房间自动清理
  - [ ] 完整的监控接口
  - [ ] `ManagerStats` 统计

- [ ] **集成到主应用**
  - [ ] 在 `synctv/src/main.rs` 中初始化 SfuManager
  - [ ] 集成到 WebRTC 信令流程
  - [ ] 在 `get_ice_servers` 中根据 mode 返回配置
  - [ ] Room加入时决定P2P还是SFU模式

- [ ] **信令层集成**
  - [ ] 扩展 ClientMessage/ServerMessage 支持SFU
  - [ ] 添加 TrackPublished/TrackSubscribed 消息
  - [ ] 处理质量层切换信令

- [ ] **测试**
  - [ ] Track模块单元测试
  - [ ] Peer模块单元测试
  - [ ] Room模式切换集成测试
  - [ ] 端到端SFU测试

- [ ] **文档**
  - [ ] SFU使用文档
  - [ ] API文档
  - [ ] 配置指南

#### 📋 当前实现亮点

**1. 完整的RTP Packet转发流程**：
```rust
// Track读取RTP packets
pub async fn start_reading(&mut self) -> Result<mpsc::UnboundedReceiver<ForwardablePacket>>

// Peer转发packets到订阅者
pub fn forward_packet(&self, track_id: TrackId, packet: ForwardablePacket) -> Result<()>
```

**2. 智能带宽估算和自适应质量**：
```rust
// 带宽估算器 - 基于最近1秒数据
struct BandwidthEstimator {
    recent_bytes: Vec<(Instant, usize)>,
    current_bandwidth_kbps: u32,
    smoothing_factor: f64, // 0.8 - 指数平滑
}

// 自动质量调整
pub async fn update_bandwidth_estimation(&self) {
    let estimated_bandwidth = self.bandwidth_estimator.write().estimate();
    if bandwidth_changed_significantly {
        let new_quality = QualityLayer::from_bandwidth(estimated_bandwidth);
        // 更新所有订阅轨道的质量层
    }
}
```

**3. Simulcast多质量层支持**：
```rust
pub enum QualityLayer {
    High,    // >= 2 Mbps - 2500 kbps expected
    Medium,  // >= 1 Mbps - 1200 kbps expected
    Low,     // < 1 Mbps - 500 kbps expected
}
```

**下一步**：完整实现 Room 和 Manager 模块

- [ ] **SFU核心实现** - 新建`synctv-sfu`模块
  ```rust
  use webrtc::peer_connection::RTCPeerConnection;
  use webrtc::track::track_remote::TrackRemote;

  pub struct SfuRoom {
      room_id: RoomId,
      peers: HashMap<UserId, SfuPeer>,
      mode: RoomMode,  // P2P或SFU
  }

  pub struct SfuPeer {
      user_id: UserId,
      peer_connection: Arc<RTCPeerConnection>,

      // 接收
      video_track: Option<Arc<TrackRemote>>,
      audio_track: Option<Arc<TrackRemote>>,

      // 发送（转发其他人的流）
      outgoing_tracks: Vec<Arc<TrackLocalStaticRTP>>,

      // 订阅管理
      subscriptions: HashSet<UserId>,
  }

  impl SfuRoom {
      // 核心：接收并转发媒体流
      pub async fn forward_media(&self) -> Result<()> {
          for sender in self.peers.values() {
              if let Some(track) = &sender.video_track {
                  let mut buf = vec![0u8; 1500];

                  // 持续读取RTP包
                  while let Ok((n, _)) = track.read(&mut buf).await {
                      let rtp_packet = &buf[..n];

                      // 转发给所有订阅者
                      for receiver in self.peers.values() {
                          if receiver.user_id == sender.user_id {
                              continue;
                          }

                          if receiver.subscriptions.contains(&sender.user_id) {
                              receiver.send_rtp(rtp_packet).await?;
                          }
                      }
                  }
              }
          }
          Ok(())
      }
  }
  ```

- [ ] **模式切换逻辑**
  ```rust
  impl SfuRoom {
      // 根据人数自动切换模式
      pub async fn check_mode_switch(&mut self) -> Result<()> {
          let peer_count = self.peers.len();
          let threshold = config.webrtc.sfu_threshold.unwrap_or(5);

          match self.mode {
              RoomMode::P2P if peer_count >= threshold => {
                  info!("Room {} switching to SFU mode ({} peers)",
                      self.room_id, peer_count);
                  self.switch_to_sfu().await?;
              }
              RoomMode::SFU if peer_count < threshold => {
                  info!("Room {} switching back to P2P mode", self.room_id);
                  self.switch_to_p2p().await?;
              }
              _ => {}
          }

          Ok(())
      }
  }
  ```

- [ ] **Simulcast支持**（多码率自适应）
  ```rust
  pub enum QualityLayer {
      High,    // 1920x1080 @ 2Mbps
      Medium,  // 1280x720 @ 1Mbps
      Low,     // 640x480 @ 500Kbps
  }

  impl SfuPeer {
      // 根据网络质量选择码率
      pub async fn select_layer(&self, sender: &SfuPeer) -> QualityLayer {
          let stats = self.get_network_stats().await;

          if stats.available_bandwidth > 2_000_000 {
              QualityLayer::High
          } else if stats.available_bandwidth > 1_000_000 {
              QualityLayer::Medium
          } else {
              QualityLayer::Low
          }
      }
  }
  ```

- [ ] **配置控制**
  ```toml
  [webrtc]
  mode = "hybrid"
  sfu_threshold = 5  # 5人以上自动切换SFU

  # SFU资源限制（防止成本失控）
  max_sfu_rooms = 10  # 🔧 最多10个房间使用SFU
  max_peers_per_sfu_room = 20  # 每个SFU房间最多20人

  # Simulcast
  enable_simulcast = true
  simulcast_layers = ["high", "medium", "low"]
  ```

**工作量**：8-10天（协议栈复杂）
**成本**：高（服务器承担所有流量转发）
**适用场景**：10人以上大房间

**成本估算**（单个10人SFU房间）：
- 接收：10人 × 1Mbps = 10Mbps
- 发送：10人 × 9Mbps = 90Mbps
- 总计：100Mbps/房间

**优化策略**：
- 配置`mode = "peer_to_peer"`完全禁用SFU
- 配置`sfu_threshold = 999`实质上禁用SFU
- 设置`max_sfu_rooms`限制并发SFU房间数量

---

### Phase 5: 网络质量监控和自适应 - 3-4天

**目标**：实时监控连接质量，自动调整码率

- [ ] **网络质量监控** - `synctv-core/src/service/network_monitor.rs`
  ```rust
  pub struct NetworkStats {
      pub rtt: Duration,              // 往返延迟
      pub packet_loss_rate: f32,      // 丢包率 0.0-1.0
      pub jitter: Duration,           // 抖动
      pub available_bandwidth: u64,   // 可用带宽（bps）
  }

  pub struct NetworkQualityMonitor {
      peer_stats: HashMap<UserId, NetworkStats>,
  }

  impl NetworkQualityMonitor {
      // 从WebRTC RTCP统计中提取数据
      pub async fn monitor_peer(&mut self, peer: &SfuPeer) -> Result<()> {
          let stats = peer.peer_connection.get_stats().await?;

          self.peer_stats.insert(peer.user_id.clone(), NetworkStats {
              rtt: stats.round_trip_time,
              packet_loss_rate: stats.packets_lost as f32
                  / stats.packets_sent as f32,
              jitter: stats.jitter,
              available_bandwidth: estimate_bandwidth(&stats),
          });

          Ok(())
      }

      // 质量评分（0-5星）
      pub fn calculate_score(&self, user_id: &UserId) -> u8 {
          let stats = &self.peer_stats[user_id];
          let mut score = 5;

          if stats.rtt > Duration::from_millis(300) { score -= 1; }
          if stats.packet_loss_rate > 0.05 { score -= 1; }
          if stats.packet_loss_rate > 0.15 { score -= 2; }

          score
      }
  }
  ```

- [ ] **自适应码率调整**
  ```rust
  impl SfuRoom {
      pub async fn adapt_quality(&self, peer: &SfuPeer) -> Result<()> {
          let stats = self.monitor.get_stats(&peer.user_id).await?;

          // 策略1：丢包严重，降低质量
          if stats.packet_loss_rate > 0.10 {
              peer.switch_to_layer(QualityLayer::Low).await?;
              log::warn!("User {} high packet loss, switching to low quality",
                  peer.user_id);
          }

          // 策略2：带宽不足，降帧率
          if stats.available_bandwidth < 500_000 {
              peer.set_max_framerate(15).await?;  // 30fps → 15fps
          }

          // 策略3：丢包>20%，切换到纯音频
          if stats.packet_loss_rate > 0.20 {
              peer.disable_video().await?;
          }

          Ok(())
      }
  }
  ```

- [ ] **质量报告API**
  - gRPC: `GetNetworkQuality()`
  - 返回当前用户和房间内所有人的网络质量

**工作量**：3-4天
**成本**：极低（仅统计数据）
**价值**：提升用户体验，减少投诉

---

## 📊 WebRTC功能总览

| 功能 | 实现阶段 | 工作量 | 服务器成本 | 可配置关闭 | 优先级 |
|------|---------|-------|-----------|----------|--------|
| **信令转发（P2P）** | Phase 1 | 1-2天 | 零 | ❌ 必需 | P0 |
| **内置STUN** | Phase 2 | 2-3天 | 极低 | ✅ | P0 |
| **TURN中继** | Phase 3 | 3-4天 | 中等 | ✅ | P1 |
| **SFU架构** | Phase 4 | 8-10天 | 高 | ✅ | P1 |
| **Simulcast** | Phase 4 | +2天 | 低 | ✅ | P1 |
| **质量监控** | Phase 5 | 3-4天 | 极低 | ✅ | P1 |

**总工作量**：17-27天（根据实施范围）

**灵活部署示例**：

```toml
# 配置示例1：个人部署（最小成本）
[webrtc]
mode = "peer_to_peer"
enable_builtin_stun = true
enable_turn = false
# 成本：几乎为0，连接成功率85%

# 配置示例2：小型服务（推荐）
[webrtc]
mode = "hybrid"
sfu_threshold = 8
enable_builtin_stun = true
enable_turn = true
max_sfu_rooms = 5
# 成本：低-中等，连接成功率99%

# 配置示例3：企业部署（完整功能）
[webrtc]
mode = "sfu"
enable_builtin_stun = true
enable_turn = true
enable_simulcast = true
max_sfu_rooms = 100
# 成本：按需扩展，连接成功率99.9%
```

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

## 📡 WebRTC完整架构实施计划（P1优先级）

### 概述

**目标**：提供生产级别的WebRTC实时音视频通信能力，支持从零成本个人部署到企业级大规模房间。

**核心特点**：
- 🎯 **灵活配置**：部署者可根据资源情况选择不同模式
- 💰 **成本可控**：从零成本P2P到按需付费的SFU
- 📈 **渐进式**：可以先实施基础功能，逐步增强
- 🔒 **生产验证**：所有技术均已在Zoom、Jitsi、Discord等产品中验证

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
