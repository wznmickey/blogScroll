# 博客漫游 · Blog Waterfall

从博客圈子（目前是 [开往 Travellings](https://www.travellings.cn)）的成员列表出发，读取各博客的 RSS，把最新文章做成小红书式的瀑布流。支持桌面和手机网页。

- 后端：Rust（axum + sqlx/SQLite + reqwest + feed-rs），负责同步成员列表、发现并抓取 RSS、提供 API、托管前端
- 前端：TypeScript + Vue 3 + Vite，瀑布流 / 无限滚动 / 详情弹层（手机为全屏页）

```
backend/    Rust 服务端（feed-server）
  src/sources/     博客目录（开往……），扩展点
  src/fulltext.rs  全文抓取，扩展点（默认关闭）
  src/http.rs      所有对外请求都经过这里（限速、条件请求）
frontend/   Vue 前端
shell.nix   开发环境（cargo、gcc、node）
```

## 运行

```sh
nix-shell                          # NixOS 上提供 cc / cmake / node；其他系统自备即可

cd frontend && npm install && npm run build && cd ..

cd backend
cp config.example.toml config.toml # 务必把 user_agent 改成能联系到你的信息
cargo run --release -- serve       # http://127.0.0.1:8080
```

`serve` 会在后台：每天同步一次成员列表 → 为每个博客发现 RSS → 按各自的节奏刷新。首次启动后几分钟就能看到内容，全部 1300+ 个博客跑完一轮大约需要一两个小时（这是有意放慢的）。

其他命令：

```sh
cargo run -- sync                  # 只同步成员列表
cargo run -- crawl --limit 20      # 只抓 20 个到期的博客，然后退出
cargo run -- crawl --blog 42       # 立刻抓某一个博客
cargo run -- serve --no-crawl      # 只提供网页和 API
cargo test
```

前端开发：`cd frontend && npm run dev`（Vite 会把 `/api` 代理到 8080 端口）。

## 抓取策略：只读 RSS，不给别人添麻烦

- **只用 RSS 里的内容**。默认只保存标题、约 240 字摘要、头图链接和标签；即使 RSS 是全文输出，也不保存全文。详情页引导读者去原站。
- **图片不落地**：直接从原站加载（`referrerpolicy="no-referrer"`），本站不缓存、不代理。
- **限速**：全局最多 4 个并发请求；同一主机两次请求间隔 ≥ 10 秒。
- **低频**：每个 RSS 初始 4 小时刷新一次，没更新就逐步放慢到 24 小时，有更新再恢复；遵守 RSS `<ttl>`。
- **条件请求**：带 `ETag` / `Last-Modified`，没变化时只花一个 304。
- **退避**：失败按指数退避（最长 7 天），遵守 `429` / `503` 的 `Retry-After`。
- **RSS 发现很克制**：先读一次首页的 `<link rel="alternate">`，没有再试几个常见路径（`atom.xml`、`feed`、`index.xml`……），遇到 403 / 5xx / 超时立即停止；都没有就 7 天后再看。
- 响应体上限 4 MB，超时 20 秒。

以上都可在 `config.toml` 的 `[crawler]` 中调整。

## 博主的选择：`[policy]`

```toml
[policy]
exclude = ["https://不想被收录的博客.com"]   # 不请求、不展示，已存内容会被删除

[[policy.full_text]]                          # 作者授权后才开启全文
site = "https://example.com"
note = "2026-11-01 邮件授权"

[[policy.feed_overrides]]                     # 手动指定 RSS 地址
site = "https://example.com"
feed = "https://example.com/custom.xml"
```

站点按「去掉 www 的域名 + 路径」匹配，`https://www.a.com/` 和 `a.com` 视为同一站。撤销授权（从列表移除）后，已存的全文会在下次启动或同步时清除。

## 扩展

### 增加博客目录（如十年之约）

在 `backend/src/sources/` 新建模块，实现 `Directory` trait，然后在 `sources::enabled` 里注册：

```rust
pub trait Directory: Send + Sync {
    fn id(&self) -> &'static str;          // 存进数据库，如 "foreverblog"
    fn name(&self) -> &'static str;        // 界面显示，如 "十年之约"
    fn homepage(&self) -> &'static str;
    fn fetch<'a>(&'a self, http: &'a PoliteClient)
        -> BoxFuture<'a, anyhow::Result<Vec<DirectoryEntry>>>;
}
```

目录只需要回答「有哪些博客」（可选地带上 RSS 地址）；去重（同一博客出现在多个目录只算一个）、RSS 发现、抓取、存储都是共用的。

> 关于十年之约：foreverblog.cn 目前没有公开的成员 / RSS 列表接口——`blogs.html` 是分页 HTML，只有站内详情页链接，加载更多的接口需要会话和 CSRF，且有人机验证与限流；`feeds.html` 是文章流而不是订阅源列表。建议先联系站长询问是否能提供导出（JSON / OPML），拿到授权的数据源后再写对应的 `Directory`。

### 全文抓取

`backend/src/fulltext.rs` 中的 `FullTextExtractor` trait 是全文接口，自带一个按常见主题选择器取正文的 `ArticleExtractor`。它**只会**对 `[policy.full_text]` 中列出的博客生效：

1. RSS 本身是全文（≥ 500 字）→ 直接使用 RSS 内容，不额外请求；
2. 否则每次抓取最多为 3 篇文章请求原文页，提取正文，经 ammonia 清洗后保存。

可以替换成更好的实现（Readability 算法、按站点定制规则、作者提供的 API 等），在 `main.rs` 里传给 `Crawler::new` 即可。

## API

| 路径 | 说明 |
|---|---|
| `GET /api/posts?mode=discover&seed=&cursor=` | 推荐：近期文章，按种子轻度打散，同一博客连续出现会降权 |
| `GET /api/posts?mode=latest&cursor=&blog=&q=` | 最新 / 某博客 / 搜索，游标分页 |
| `GET /api/posts/{id}` | 详情；`content_html` 仅对授权全文的博客返回 |
| `GET /api/blogs?q=&offset=` · `GET /api/blogs/{id}` | 博客列表 / 博客主页 |
| `GET /api/meta` | 站点名、数据来源、统计 |
