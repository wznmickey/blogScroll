# 博客漫游 · Blog Waterfall

从博客圈子（目前是 [开往 Travellings](https://www.travellings.cn)）的成员列表出发，读取各博客的 RSS，把最新文章做成小红书式的瀑布流。支持桌面和手机网页。

- 后端：Rust（axum + sqlx/SQLite + reqwest + feed-rs），负责同步成员列表、发现并抓取 RSS、提供 API、托管前端
- 前端：TypeScript + Vue 3 + Vite，瀑布流 / 无限滚动 / 详情弹层（手机为全屏页）

筛选：「推荐 / 最新」两种排序 × 开往标签频道（博客、技术、生活、综合……）× 「只看有图」；点文章的 `#标签` 看同标签文章；搜索框聚焦时给出热门标签；`/blogs` 是全部成员博客的目录，可按标签筛选、搜索、按更新时间 / 文章数 / 名称排序。

```
backend/    Rust 服务端（feed-server）
  src/sources/     博客目录（开往……），扩展点
  src/fulltext.rs  全文抓取，扩展点（默认关闭）
  src/http.rs      所有对外请求都经过这里（限速、条件请求）
  src/cache.rs     本站自己的 HTTP 缓存（API 内存缓存 + ETag，静态资源缓存头）
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
cargo run -- crawl --sync --all    # 定时任务用：同步（如到期）并抓完所有到期的博客
cargo run -- export --out DIR      # 导出静态 JSON（静态版前端读取）
cargo test
```

前端开发：`cd frontend && npm run dev`（Vite 会把 `/api` 代理到 8080 端口）。

## 部署

两种方式二选一（**不要同时开**，否则每个博客会被抓两遍）。

### A. 自己的服务器：Docker

推送到默认分支或打 `v*` 标签时，`.github/workflows/ci.yml` 跑完测试后会把镜像发布到 `ghcr.io/<owner>/<repo>`。服务器上：

```sh
# 把 compose.yaml 里的 OWNER/REPO 改成你的仓库（小写）
docker compose up -d          # http://127.0.0.1:8080，前面再挂 Caddy / nginx 做 HTTPS
```

- 数据在命名卷 `feed-data`（容器内 `/data/feed.db`）。
- 配置：镜像自带 `deploy/config.docker.toml`；要改就挂载自己的文件到 `/app/config.toml`。
- `FEED_USER_AGENT` 环境变量可直接覆盖 User-Agent，建议写上联系方式。
- 服务器放在国内或香港，抓国内博客更稳。

### B. 不要服务器：GitHub Actions 定时抓取 + GitHub Pages

`.github/workflows/pages.yml` 每 3 小时：恢复上次的数据库 → 同步成员列表（每天一次）→ 只抓到期的 RSS → 导出静态 JSON → 构建静态版前端 → 发布到 Pages。

默认关闭，开启步骤：

1. Settings → Pages → Source 选 **GitHub Actions**。
2. Settings → Secrets and variables → Actions → **Variables** 新建：
   - `ENABLE_PAGES_CRAWL` = `true`
   - `FEED_USER_AGENT` = `FeedWaterfall/0.1 (RSS reader; +https://你的联系方式)`
3. Actions 里手动运行一次 “Crawl & publish”。首次要发现 1300 多个 RSS，约一小时；之后每次几分钟。

说明：

- 数据库（ETag、刷新节奏、退避状态）存在 Actions 缓存里，每次运行存一份新的，只保留最近 3 份；万一缓存丢失，只是下一轮重新发现一遍。
- 静态版用 `#/` 路由（Pages 不能把深链接改写到 index.html）；推荐、筛选、搜索都在浏览器里完成，覆盖最新 3000 篇，博客主页覆盖该博客已存的全部文章。
- Runner 在海外机房：少数国内博客会拦截或超时，数据会比自建服务器少一些。
- 私有仓库每月只有 2000 分钟免费额度，这个频率会超；建议用公开仓库，或把 cron 改成每 6 小时。公开仓库 60 天没有提交，GitHub 会自动停用定时任务。
- 本地预览静态版：

  ```sh
  cd frontend && VITE_STATIC=1 npm run build && cd ../backend
  cargo run -- export --out ../frontend/dist/data
  cd ../frontend/dist && python3 -m http.server 8000   # http://127.0.0.1:8000
  ```

## 抓取策略：只读 RSS，不给别人添麻烦

- **只用 RSS 里的内容**。默认只保存标题、约 240 字摘要、头图链接和标签；即使 RSS 是全文输出，也不保存全文。详情页引导读者去原站。
- **图片不落地**：直接从原站加载（`referrerpolicy="no-referrer"`），本站不缓存、不代理。
- **限速**：全局最多 4 个并发请求；同一主机两次请求间隔 ≥ 10 秒。
- **低频**：每个 RSS 初始 4 小时刷新一次，没更新就逐步放慢到 24 小时，有更新再恢复。
- **尊重对方的缓存声明**：RSS `<ttl>`、HTTP `Cache-Control: max-age`、`Expires` 都作为下次请求的最早时间。
- **条件请求**：带 `ETag` / `Last-Modified`，没变化时只花一个 304；对不支持条件请求的站点，内容哈希未变就跳过解析。成员列表每天同步一次，同样走条件请求。
- **退避**：失败按指数退避（最长 7 天），遵守 `429` / `503` 的 `Retry-After`。
- **RSS 发现很克制**：先读一次首页的 `<link rel="alternate">`，没有再试几个常见路径（`atom.xml`、`feed`、`index.xml`……），遇到 403 / 5xx / 超时立即停止；都没有就 7 天后再看。
- 响应体上限 4 MB，超时 20 秒。
- **存储有上限**：每个博客最多保留最新 50 篇（`max_posts_per_blog`），只收录一年内的文章。

以上都可在 `config.toml` 的 `[crawler]` 中调整。

## 博主信息与评论

- **简介**：每月读一次首页和关于页（每个博客 2 个请求）：meta 描述、头像（主题的作者头像位，否则 apple-touch-icon）、关于页链接与约 300 字摘要、社交链接（GitHub、B站、微博、知乎……与 `rel="me"`）、博客程序（generator）。
- **评论系统识别**：只看首页和关于页里的配置值（Twikoo 的 `envId`、Waline 的 `serverURL`、Artalk 的 `server`、Giscus / Utterances 的仓库、Disqus、Halo、Valine），仅出现产品名不算；WordPress / Typecho 等按 generator 判断为内置评论。不读文章页。
- **评论数**：
  - RSS 自带的 `<slash:comments>` / `<thr:total>`，不需要额外请求；
  - Twikoo（自建地址）、Waline、Artalk、Halo 2：每天最多一次批量请求（最新 20 篇）；401/403/404 视为博主不允许，30 天内不再请求；不伪造 Referer / Origin。
  - Giscus、Utterances、Disqus 没有批量接口，只显示「去原文评论」。
- **内嵌评论区**（`[[policy.embed_comments]]` 授权后）：详情页加载博客自己的评论组件，评论读写都在原博客的评论服务。组件脚本固定版本、只从 jsDelivr / giscus.app / utteranc.es 加载，从不加载博客站点本身的脚本；博主需要把本站域名加入评论服务的允许列表。静态版（Pages）只显示评论数和链接，不内嵌。

## 本站的缓存

- API：内存缓存（条数上限 `api_cache_entries`，默认 512），列表 60 秒、标签 / 统计 / 博客主页 5 分钟、文章详情 10 分钟；响应带弱 `ETag` 和 `Cache-Control: max-age=…, stale-while-revalidate=…`，浏览器复查时返回 304。
- 静态资源：`/assets/*`（文件名带哈希）一年 `immutable`；HTML 一律 `no-cache`，发布新版本立即生效；其他文件 1 天。
- 前端：已显示过的卡片在内存中最多保留 2000 条，用于秒开详情。

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
| …`&tag=tech&directory=travellings&cover=1` | 两种模式通用的筛选：目录标签、来源目录、只看有图 |
| …`&category=旅行` | 按文章自带的分类筛选（按时间排序） |
| `GET /api/posts/{id}` | 详情；`content_html` 仅对授权全文的博客返回 |
| `GET /api/tags` | 各目录的标签（含博客数、文章数、中文名）和近半年热门文章分类 |
| `GET /api/blogs?q=&tag=&sort=recent\|posts\|name&offset=` | 博客目录 |
| `GET /api/blogs/{id}` | 博客主页（含标签） |
| `GET /api/meta` | 站点名、数据来源、统计 |
