# 博客漫游 · Blog Waterfall

从博客圈子（目前是 [开往 Travellings](https://www.travellings.cn)）的成员列表出发，读取各博客的 RSS，把最新文章做成小红书式的瀑布流。也可以在配置里加入任意 RSS 来源（资讯、周刊、播客……）。支持桌面和手机网页。

- 后端：Rust（axum + sqlx/SQLite + reqwest + feed-rs），负责同步成员列表、发现并抓取 RSS、提供 API、托管前端
- 前端：TypeScript + Vue 3 + Vite，瀑布流 / 无限滚动 / 详情弹层（手机为全屏页）

- 「推荐 / 最新 / 关注」三个页签 × 领域频道（编程开发、人工智能、旅行……）× 来源类型 × 「只看有图」
- 点文章的 `#标签` 看同标签文章；文章详情旁边有博主简介（来自关于页）和「相似文章」
- 搜索同时找博主（名称、网址、简介、关于页、领域）和文章，输入时下拉提示博主
- 「关注」存在本机浏览器的 cookie 里（无需账号）；推荐会参考关注的博主和最近读过的文章
- `/blogs` 是全部来源的目录，可按领域、来源类型、「我的关注」筛选，搜索，按更新时间 / 文章数 / 名称排序

```
backend/    Rust 服务端（feed-server）
  src/sources/     来源：博客目录（开往……，扩展点）和配置里的 RSS / OPML
  src/feed.rs      RSS 解析；src/clean.rs 修正常见的 RSS 毛病
  src/analyze.rs   关键词、标签、领域、相似文章（jieba 分词 + TF-IDF）
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
cargo run -- analyze               # 重算全部关键词、标签、领域和相似度（serve 每 6 小时自动做）
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

## RSS 里常见的问题，以及怎么处理

对已收录的约 8000 篇文章做过一次体检，下面这些都由解析器自动修正（`src/feed.rs`、`src/clean.rs`）：

- **日期**：`2026-10-05 10:11:00` 这类没有 `T`、没有时区的写法按 UTC+8 读取；所有条目都是同一个带时分秒的时间（生成器把构建时间写进每一条）视为没有日期；远在未来的日期（如 2077 年）丢弃；没有日期时从网址里取（`/2026/10/05/`、`/202610051011`）。
- **没有日期的文章**：第一次抓取到的视为「年代不明」，排在所有有日期的文章之后，界面显示「发布日期未知」；之后新出现的按发现时间算。
- **摘要**：HTML 被转义两次、`<description>` 里直接放 Markdown、「阅读全文 →」「The post … appeared first on …」「…最先出现在…」之类的尾巴、图片文件名和裸链接，都会清理掉。
- **链接**：Hexo 没配置 `url` 时文章地址是 `http://example.com/...`，改到 RSS 所在的域名。
- **作者**：`(名字)`、`@用户名 名字`、邮箱都会整理或丢弃。
- **分类**：「未分类」「默认分类」「Uncategorized」这类不保留。
- **重复**：Discourse 论坛把每条回复都列成一项（`#post_3`），合并为一个主题；同一篇文章用两个网址发布（标题和摘要都一样）只留一个。
- **垃圾站**：域名过期被博彩站接手、RSS 大半是博彩标题的来源会被自动隐藏（`flagged = 'spam'`），已存内容删除，之后恢复正常会自动解除。
- **日志**：网络错误记录具体原因（DNS、超时、证书不匹配……）而不是笼统的 "error sending request"；html5ever 和证书校验库的逐站噪音默认不输出（站点问题仍记在该博客的 `last_error`）。

## 标签、领域与推荐

全部在本地计算，只用 RSS 里给的内容（标题、作者自己的分类、摘要、RSS 正文），不请求文章页，也不调用外部服务（`src/analyze.rs`）：

- **关键词**：jieba 分词 + TF-IDF，标题和分类加权，每篇最多 12 个。
- **标签**：作者自己的分类（统一大小写和同义写法，如「人工智能」→ AI、docker → Docker），加上关键词里属于「常用标签」的词（至少 3 个博客用过的分类，或领域词表里的词；太宽泛的词如「记录」「工具」不自动添加）。
- **领域**：16 个固定领域（编程开发、人工智能、运维网络、网络安全、建站博客、数码工具、学习学术、生活随笔、旅行、摄影、阅读影视、游戏动漫、美食、职场财经、设计创作、运动健康），按标签和关键词匹配；博客的领域是其文章中占比 ≥ 25% 的领域。
- **来源类型**：博客 / 资讯 / 周刊 / 播客 / 视频 / 论坛 / 其他。配置里可以指定；没指定时从 RSS 判断（音频附件 → 播客，Discourse 等论坛网址 → 论坛，标题含「周刊 / Weekly」→ 周刊）。
- **相似文章**：标签和关键词按全站稀有程度加权成单位向量（`post_terms` 表），余弦相似度排序；只共享一个词的匹配降权，领域一致加分、不一致减分，每个博客最多 2 篇。
- **推荐**：在原有「新鲜度 + 随机扰动 + 同博客降权」的基础上，加上与「关注的博主近一年写的内容 + 最近读过的文章」的相似度。
- 新文章入库时就打好标签；全站统计每 6 小时、`export` 之前和 `feed-server analyze` 时重算。

## 自选订阅：纯 RSS 来源

除了博客目录，可以在 `config.toml` 里直接加 RSS（启动时读取，改完重启即可）：

```toml
[[sources.feeds]]
url = "https://sspai.com/feed"
kind = "news"            # blog | news | newsletter | podcast | video | forum | other
tags = ["科技"]           # 可选
# name = "少数派"         # 可选；不写就用 RSS 自己的标题
# homepage = "https://sspai.com/"  # 可选；写了会和同一网站的开往成员合并

[[sources.opml]]          # 从 RSS 阅读器导出的订阅列表
path = "feeds.opml"       # 相对 config.toml 所在目录
kind = "blog"
```

这些来源和开往成员一样抓取、一样受限速规则约束，在 `/blogs` 里显示为「自选订阅」。

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
| …`&follow=1,2&read=7,8` | 推荐向关注的博主、最近读过的文章的内容倾斜 |
| `GET /api/posts?mode=latest&cursor=&blog=&q=` | 最新 / 某博客 / 搜索，游标分页 |
| …`&blogs=1,2,3` | 只看这些博客（「关注」页签），按时间排序 |
| …`&topic=dev&kind=podcast&tag=tech&directory=travellings&cover=1` | 两种模式通用的筛选：领域、来源类型、目录标签、来源目录、只看有图 |
| …`&category=Docker` | 按标签筛选（作者分类和自动标签，同义写法合并；按时间排序） |
| `GET /api/posts/{id}` | 详情；`content_html` 仅对授权全文的博客返回 |
| `GET /api/posts/{id}/related?limit=8` | 相似文章 |
| `GET /api/tags` | 领域、来源类型、各目录的标签（含博客数、文章数、中文名）和近半年热门标签 |
| `GET /api/blogs?q=&topic=&kind=&tag=&ids=&sort=recent\|posts\|name&offset=` | 博客目录；有 `q` 时按匹配程度排序（名称 > 网址 > 简介 > 关于页 / 领域） |
| `GET /api/blogs/{id}` | 博客主页（含标签、领域、类型、常用标签） |
| `GET /api/meta` | 站点名、数据来源、统计 |
