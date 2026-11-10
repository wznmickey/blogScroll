# 博客瀑布 · blogScroll

从博客圈子（目前是 [开往 Travellings](https://www.travellings.cn)）的成员列表出发，读取各博客的 RSS，把最新文章做成小红书式的瀑布流。也可以在配置里加入任意 RSS 来源（资讯、周刊、播客……）。支持桌面和手机网页。

- 后端：Rust（axum + sqlx/SQLite + reqwest + feed-rs），负责同步成员列表、发现并抓取 RSS、提供 API、托管前端
- 前端：TypeScript + Vue 3 + Vite，瀑布流 / 无限滚动 / 详情弹层（手机为全屏页）

- 首页是一条频道栏：「推荐」（按你的喜好）、「反推荐」（反过来，专挑你还没看过的方向）、「发现」（不做个性化，人人一样）、「最新」、「关注」，以及各分类、来源类型（播客、周刊……）和你添加的标签。手指左右滑，或用鼠标滚轮、按住拖动、两侧箭头；点右侧 ▾ 管理频道：排序、置顶（固定在左侧不随滑动，最多 3 个）、隐藏、添加。频道栏右侧有三个可以同时勾选的过滤条件：「无图」「短摘要」（摘要不到 50 字，只留长摘要）「0 评论」（没有评论，或不知道评论数）。文章详情里的 RSS 摘要保留原来的格式（段落、列表、小标题、引用、粗体、代码），无论 feed 给的是 HTML 还是 Markdown 源码。瀑布流在宽屏上最多几列由 `[server] columns` 决定（默认 6），窄屏自动减少；读者也可以在 ▾ 里的「每行几列」自己选（自动或 1–8 列，卡片不会窄于 150px），保存在本机、随备份走；没有图、摘要又很短的卡片会相应缩小
- 每篇文章有且只有一个**分类**（两级：「技术 › AI」「旅行」……），可以有多个**标签**（作者自己的加上自动提取的）；两者分开显示、分开筛选
- 「喜欢」「收藏」、浏览历史和「刷到过」（在屏幕上停留过的卡片，点没点开都算），连同「关注」，都只存在本机浏览器里（无需账号）；在「我的」页查看、搜索，并可以**备份成文件、再恢复**（合并或覆盖），用来换设备
- 按时间排的频道（最新、关注、分类按最新）记得上次看到哪里：之后的新文章标「新」，并可以「从上次看到的地方继续」
- 搜索一次覆盖文章、博主、分类和标签，以及自己的收藏、喜欢、浏览历史和刷到过的文章；输入时下拉提示
- 文章详情旁边有博主简介（来自关于页）和「相似文章」
- 窗口够宽（≥ 1100 px）时，「阅读原文」、博客首页等链接在页面右侧打开，左边仍是瀑布流，可以边翻边读；中间的分隔线可拖动。网站禁止被嵌入（`X-Frame-Options` / CSP `frame-ancestors`）时照旧开新标签页，按住 Ctrl / ⌘ 或用中键点击也是新标签页
- 抓到的数据可以定期以文本快照提交到一个 git 仓库（比如 GitHub 私有仓库），也能从仓库恢复
- `/blogs` 是全部来源的目录，可按分类、来源类型、「我的关注」筛选，搜索，按更新时间 / 文章数 / 名称排序

```
backend/    Rust 服务端（feed-server）
  src/sources/     来源：博客目录（开往……，扩展点）和配置里的 RSS / OPML
  src/feed.rs      RSS 解析；src/clean.rs 修正常见的 RSS 毛病
  src/analyze.rs   关键词、标签、分类、相似文章（jieba 分词 + TF-IDF + 朴素贝叶斯）
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
cargo run -- analyze               # 重算全部关键词、标签、分类和相似度（serve 每 6 小时自动做）
cargo run -- export --out DIR      # 导出静态 JSON（静态版前端读取）；加 --site-url 再生成给搜索引擎的页面
cargo run -- backup                # 立刻做一次 git 备份（见下文；serve 按 every_hours 自动做）
cargo run -- restore --from DIR    # 从备份仓库恢复到一个新的空数据库
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

两个工作流，抓取和发布分开：

- `.github/workflows/pages.yml`（Crawl）每 3 小时：恢复上次的数据库 → 同步成员列表（每天一次）→ 只抓到期的 RSS → 存回缓存（和备份）→ 调用发布。
- `.github/workflows/publish.yml`（Publish site）：用缓存里最新的数据库构建静态版并发布到 Pages，不抓取，几分钟完成。除了抓完以后，每次推送到 main 且 CI 测试通过也会运行，所以改代码不用等下一轮抓取；也可以手动运行。

默认关闭，开启步骤：

1. Settings → Pages → Source 选 **GitHub Actions**。
2. Settings → Secrets and variables → Actions → **Variables** 新建：
   - `ENABLE_PAGES_CRAWL` = `true`
   - `FEED_USER_AGENT` = `blogScroll/0.1 (RSS reader; low frequency; honours ETag, Cache-Control and Retry-After)`
3. Actions 里手动运行一次 “Crawl (GitHub Pages)”。首次要发现 1300 多个 RSS，约一小时；之后每次几分钟。

说明：

- 数据库（ETag、刷新节奏、退避状态）存在 Actions 缓存里，每次运行存一份新的，只保留最近 3 份。缓存并不永久（7 天没用到或总量超过 10 GB 会被清掉）；丢失时如果配了下面的 git 备份，就从备份恢复，否则下一轮从头重新抓一遍。
- 可选：每次抓取前先把一个私有仓库里的备份合并进数据库，抓完再备份回去（见「备份到 GitHub 私有仓库」）；缓存丢失时也从它恢复。在那个仓库加一个有写权限的 deploy key，把私钥存为本仓库的 Secret `BACKUP_DEPLOY_KEY`，再加 Variable `BACKUP_REPO` = `你/私有仓库名`。备份失败不影响网站发布。
- 静态版用真实路径（`/blogScroll/blog/12`）：导出时为每个要被搜索引擎看到的页面生成 HTML 文件（见「搜索引擎」），其余路径（搜索、我的）由 `404.html` 接住，照常打开。以前的 `#/blog/12` 链接会自动跳到新地址。推荐、筛选、搜索都在浏览器里完成，覆盖最新 3000 篇，博客主页覆盖该博客已存的全部文章。
- Runner 在海外机房：少数国内博客会拦截或超时，数据会比自建服务器少一些。
- 私有仓库每月只有 2000 分钟免费额度，这个频率会超；建议用公开仓库，或把 cron 改成每 6 小时。公开仓库 60 天没有提交，GitHub 会自动停用定时任务。
- GitHub 的定时触发不保证准时，繁忙时会推迟或直接跳过。抓取是增量的，少跑几次只是更新慢一些；需要准时的话，可以从外部（自己的机器、cron-job.org 等）定时调用 `POST /repos/<owner>/<repo>/actions/workflows/pages.yml/dispatches`（body `{"ref":"main"}`，用只有本仓库 Actions 写权限的 token）。
- 本地预览静态版（需要一个像 Pages 一样把 `/blog/12` 对应到 `blog/12.html`、找不到时给 `404.html` 的服务器，例如 `npx serve`）：

  ```sh
  cd frontend && VITE_STATIC=1 npm run build && cd ../backend
  cargo run -- export --out ../frontend/dist/data --site-url http://localhost:3000
  npx serve ../frontend/dist   # http://localhost:3000
  ```

### 搜索引擎

静态版导出时（`--site-url` 为网站地址，工作流会自动填）在应用的 `index.html` 基础上，为首页、全部博客、关于和每个博客的主页各生成一个页面：标题、描述、canonical、Open Graph，以及正文本身（简介、社交链接、分类标签、文章列表）的普通 HTML，不运行 JavaScript 的爬虫也能读到；浏览器打开后应用照常接管。另外生成 `sitemap.xml`、`robots.txt` 和 `404.html`（搜索、我的等页面用它打开，标记 `noindex`）。

文章不单独生成页面：列表里的文章直接链到原文，搜索结果会把人带到原博客，也给小博客带去外链，而不是和作者抢排名。

GitHub Pages 的项目页（`你.github.io/blogScroll/`）不在域名根目录，`robots.txt` 不生效；在 Google Search Console、Bing Webmaster Tools 里添加网站后，手动提交 `https://你.github.io/blogScroll/sitemap.xml`。用自定义域名时 `robots.txt` 会自动指向 sitemap。

## 抓取策略：只读 RSS，不给别人添麻烦

- **只用 RSS 里的内容**。默认只保存标题、约 240 字摘要、头图链接和标签；即使 RSS 是全文输出，也不保存全文。详情页引导读者去原站。
- **图片不落地**：直接从原站加载（`referrerpolicy="no-referrer"`），本站不缓存、不代理。
- **限速**：全局最多 4 个并发请求；同一主机两次请求间隔 ≥ 10 秒。
- **低频**：每个 RSS 初始 4 小时刷新一次，没更新就逐步放慢到 24 小时，有更新再恢复。
- **尊重对方的缓存声明**：RSS `<ttl>`、HTTP `Cache-Control: max-age`、`Expires` 都作为下次请求的最早时间。
- **条件请求**：带 `ETag` / `Last-Modified`，没变化时只花一个 304；对不支持条件请求的站点，内容哈希未变就跳过解析。成员列表每天同步一次，同样走条件请求。
- **退避**：失败按指数退避（最长 7 天），遵守 `429` / `503` 的 `Retry-After`。
- **RSS 发现**：先读一次首页的 `<link rel="alternate">`（最多试前 3 个），再按顺序试常见路径：`atom.xml`、`feed`、`rss.xml`、`index.xml`、`feed.xml`、`rss`、`rss2.xml`、`atom`、`feed.php`、`rss.php`、`posts.rss`、`blog.xml`、`rss.en.xml`，找到第一个能解析的就停。某个地址出错（404、403、5xx、超时……）也继续试下一个，但同一网站的请求间隔照旧，对方给了 `Retry-After` 会先等它（最多 1 分钟）；都没有就 7 天后再看。
- 响应体上限 4 MB，超时 20 秒。
- **默认什么都不删**：读到的文章一直保留，不限篇数、不限年代，RSS 里有几条就读几条（`max_items_per_feed`，默认 0 = 全部）。想给数据库设上限，可以设 `max_posts_per_blog`（每个博客最多保留几篇，超出删最旧的）和 `max_post_age_days`（跳过多少天以前的条目，已存的不删），0 表示不限。改动 `max_post_age_days` 或 `max_items_per_feed` 后，每个 RSS 会在下次检查时完整读一次，把之前跳过的条目补进来。

以上都可在 `config.toml` 的 `[crawler]` 中调整。

## RSS 里常见的问题，以及怎么处理

对已收录的约 8000 篇文章做过一次体检，下面这些都由解析器自动修正（`src/feed.rs`、`src/clean.rs`）：

- **日期**：`2026-10-05 10:11:00` 这类没有 `T`、没有时区的写法按 UTC+8 读取；所有条目都是同一个带时分秒的时间（生成器把构建时间写进每一条）视为没有日期；远在未来的日期（如 2077 年）丢弃；没有日期时从网址里取（`/2026/10/05/`、`/202610051011`）。
- **没有日期的文章**：第一次抓取到的视为「年代不明」，排在所有有日期的文章之后，界面显示「发布日期未知」；之后新出现的按发现时间算。
- **摘要**：HTML 被转义两次、`<description>` 里直接放 Markdown、「阅读全文 →」「The post … appeared first on …」「…最先出现在…」之类的尾巴、图片文件名和裸链接，都会清理掉。
- **链接**：Hexo 没配置 `url` 时文章地址是 `http://example.com/...`，改到 RSS 所在的域名。
- **作者**：`(名字)`、`@用户名 名字`、邮箱都会整理或丢弃。
- **分类**：「未分类」「默认分类」「Uncategorized」这类不保留；`tag:Docker`、`category:教程` 这种把类型写进名字的去掉前缀。
- **重复**：Discourse 论坛把每条回复都列成一项（`#post_3`），合并为一个主题；同一篇文章用两个网址发布（标题和摘要都一样）只留一个。
- **垃圾站**：域名过期被博彩站接手、RSS 大半是博彩标题的来源会被自动隐藏（`flagged = 'spam'`），已存内容删除，之后恢复正常会自动解除。
- **日志**：网络错误记录具体原因（DNS、超时、证书不匹配……）而不是笼统的 "error sending request"；html5ever 和证书校验库的逐站噪音默认不输出（站点问题仍记在该博客的 `last_error`）。

## 分类、标签与推荐

全部在本地计算，只用 RSS 里给的内容（标题、作者自己的分类、摘要、RSS 正文），不请求文章页，也不调用外部服务（`src/analyze.rs`）：

- **关键词**：jieba 分词 + TF-IDF，标题和分类加权，每篇最多 12 个。
- **标签**（每篇多个）：作者自己的分类和标签（统一大小写和同义写法，如「人工智能」→ AI、docker → Docker），加上关键词里属于「常用标签」的词（至少 3 个博客用过的，或分类词表里的词；太宽泛的词如「记录」「工具」不自动添加）。
- **分类**（每篇一个）：两级。大类是 技术 / 数码 / 生活 / 旅行 / 摄影 / 文娱 / 学习 / 职场财经 / 设计创作，其中技术分编程开发、AI、运维网络、网络安全、建站博客，生活分随笔、美食、运动健康，文娱分书影音、游戏动漫；都判断不了的归「其他」。做法：
  1. 词表打底：标签和关键词命中某个小类的词表、且明显强于其他小类的文章，直接定为该类（宽泛词如「生活」「笔记」只算一部分）；
  2. 本地自学习：用这些「有把握」的文章训练一个朴素贝叶斯分类器，它会学到词表里没有、但总和某类一起出现的词（软件名、相机型号、地名……），再给其余文章分类；
  3. 参考博客平时写什么：博客自己的分类分布作为先验，但压平过——只能帮有一点证据的文章拿主意，不能单凭博客给文章定类；
  4. 小类没把握、大类有把握时只标大类（如「技术」），都没把握归「其他」。
  在约 8200 篇文章上，约 75% 有分类，抽查大类基本正确；「其他」多是标题和摘要都看不出主题的短文。博客的分类是其文章中占比 ≥ 25% 的大类。
- **类型**（界面上叫「类型」，和「数据来源」不是一回事）：博客 / 资讯 / 周刊 / 播客 / 视频 / 论坛 / 其他。配置里可以指定；没指定时从 RSS 判断：多数条目带音频 → 播客；多数条目链接到视频网站的视频页（YouTube、B 站、PeerTube……）→ 视频，只是附带视频片段的博客仍算博客；Discourse 等论坛网址 → 论坛；标题含「周刊 / Weekly」→ 周刊。
- **相似文章**：标签和关键词按全站稀有程度加权成单位向量（`post_terms` 表），余弦相似度排序；只共享一个词的匹配降权，同分类加分、同大类小加分、大类不同减分，每个博客最多 2 篇。
- **三种混排**：「发现」只看「新鲜度 + 随机扰动 + 同博客降权」，对所有人一样；「推荐」再加上你的偏好——与你喜欢、收藏、读过的文章和关注的博主的内容相似度，你常看的分类，关注的博主，并把已经看过的往后放、刷到过几次却没点开的稍微往后放（免得同样的文章一刷再刷）；「反推荐」把偏好反过来用——和你平时读的越像、越是你常看的分类、越是你关注或读过的博客，就越往后，每个博客也只取最新 3 篇，用来发现平时遇不到的博客和领域。偏好数据只在请求时以文章 / 博客编号的形式发给服务器，不写入数据库，这类请求也不进本站的响应缓存（`Cache-Control: private, no-store`）。
- 新文章入库时就打好标签和分类；全站统计和分类器每 6 小时、`export` 之前和 `feed-server analyze` 时重算。

## 备份到 GitHub 私有仓库

`[backup]` 打开后，`serve` 每 `every_hours`（默认 6）小时备份一次：先把仓库里已有的合并进数据库，再把合并后的结果写成文本快照，有变化就 `git commit` 并 `git push`：

```text
README.md            说明
meta.json            格式版本、博客数、文章数
sites/<博客地址>.json  一个博客一个文件：博客信息、所属目录、全部文章（按首次抓到的时间排序）
```

每次提交只包含有变化的博客，diff 可以直接读；抓取过程中的临时状态（ETag、刷新时间、错误）和能重新算出来的内容（分类、标签向量、评论数）不进备份。目前约 1600 个博客、一万多篇文章，快照十几 MB，git 压缩后几百 KB。

**可以多处一起往同一个仓库备份**（家里的服务器、GitHub Actions……）。快照里没有数据库编号，博客按地址、文章按 guid 认；每次备份先合并：

- 对方有、自己没有的博客和文章补进自己的数据库（所以每一处的网站都能显示所有地方抓到的文章）；
- 对方找到了 RSS 而自己没找到的博客，用对方找到的地址，马上抓一次；
- 博主资料取较新的那份（读失败的空资料不会盖掉有内容的）；同一篇文章两边都有时以自己数据库里的为准，只是「首次抓到的时间」取两边较早的那个；
- 然后把「自己的数据库 ∪ 仓库里原有的」写回去。数据库里删掉的文章（比如按条数修剪掉的）在快照里照样保留。唯一的例外是博主要求不收录（`policy.exclude`）或收回了全文授权：快照里也随之去掉他的文章或全文，与数据库一致。

旧版的 `blogs/<id>.json` 会在第一次合并时读进来，改存成 `sites/`。

设置一次：

1. 在 GitHub 建一个**私有**仓库，比如 `blogscroll-data`（可以是空的）。
2. 生成一个只给这个仓库用的 SSH key，加到仓库的 Settings → Deploy keys，勾选 *Allow write access*；在 `~/.ssh/config` 里让 `github.com` 用这个 key（或者 remote 用带 fine-grained token 的 https 地址）。
3. 在 `config.toml` 里加：

   ```toml
   [backup]
   git_dir = "data/backup"                            # 相对 config.toml
   remote = "git@github.com:你/blogscroll-data.git"   # git_dir 还不是仓库时会先 clone
   every_hours = 6
   push = true
   ```

4. `cargo run -- backup` 先手动跑一次确认能推送，之后交给 `serve`。

用 GitHub Actions 部署（上文 B）时不需要 `[backup]`：设好 Secret `BACKUP_DEPLOY_KEY`（**私钥**：不带 `.pub` 的那个文件，从 `-----BEGIN` 到 `-----END` 整段；公钥是贴到备份仓库的 Deploy keys 里的）和 Variable `BACKUP_REPO`，工作流每次先用 `feed-server restore` 把这个仓库合并进缓存里的数据库（没有缓存时就是完整恢复），抓完再运行 `feed-server backup --git-dir … --remote …`；发布网站时也会先合并，所以网站上有所有抓取端的文章。服务器和 Actions 可以同时往这个仓库备份；不过两边按同一个间隔抓同一批博客会让对方站点的请求翻倍，同时开着时把一边的间隔调长些。

提交者显示为 `blogScroll <backup@blogscroll.invalid>`，不会用到你本机的 git 身份。推送失败时提交留在本地，下次一起推。Docker 镜像里已带 git 和 ssh，把 key 挂进容器即可。

**只追加、不覆盖。** 备份程序这边保证：

- 推送是 `git push origin HEAD`：明确写出的 refspec、不带 `+`，所以不会强推，git 配置里写了强推的 refspec 也不影响；
- 每次备份前先 `git fetch`，快进接上远端的新提交（比如你在 GitHub 上改了 README，或另一处推过）。本地还有没推上去的提交、和远端分叉时，把它们留在本地分支 `unpushed` 上，工作区换成远端的最新版本再做这一轮备份——那些提交里的内容都在数据库里，合并后会重新写进去；推送时如果正好有别处抢先推了，同样在对方之上重做，最多三次；
- `README.md` 只在第一次生成，之后归你编辑，备份不会改回去（还是旧版自动生成的原文时会换成新说明）；
- 合并只增不减，新快照的博客数或文章数比上一次少 10% 以上时不提交（只可能是 bug 或大量博主退出），确认是有意的再加 `--allow-shrink`；合并进数据库是一个事务，要么全部写入，要么什么都不写；
- ssh 以 BatchMode 运行，遇到陌生主机密钥或要输口令时直接失败，不会卡在终端里等输入（你自己配了 `core.sshCommand` 或 `GIT_SSH_COMMAND` 时用你的）。

GitHub 那边还可以再加一道锁：仓库 Settings → Rules → Rulesets 新建规则，目标选全部分支，勾选 *Block force pushes* 和 *Restrict deletions*，Bypass 列表留空（deploy key 也受约束）。注意 GitHub Free 账号的**私有**仓库不能用 rulesets 和分支保护，需要 GitHub Pro（学生可通过 GitHub Student Developer Pack 免费获得）；用不了时就只靠上面程序这边的保证。

deploy key 只给这一个仓库用，建议在 `~/.ssh/config` 里给它一个单独的主机名，免得和你平时的 GitHub key 混在一起：

```text
Host github-blogscroll-backup
  HostName github.com
  User git
  IdentityFile ~/.ssh/blogscroll_backup
  IdentitiesOnly yes
```

然后 `remote = "git@github-blogscroll-backup:<你>/<私有仓库>.git"`。

恢复或合并：`cargo run -- restore --from data/backup`，空数据库就是完整恢复，已有数据就是合并（同上，只补不删）。博客、目录关系、文章（含已授权的全文）都会写入，分类和标签当场重算；新加入的博客马上读一次 RSS，没找到 RSS 的一周后再找。

抓取规则变了（RSS 解析版本、`max_items_per_feed`、`max_post_age_days`，或寻找 RSS 的路径）之后第一次启动 `serve` 或 `crawl`，会把按旧规则读过的 RSS 全部排到马上完整重读一次，没找到 RSS 的博客也马上按新路径再找一次，不用等各自的刷新时间。

## 在页面右侧阅读原文

宽屏上点「阅读原文」「去原文评论」或博客主页上的首页、关于页链接，页面会分成两半：左边照常是本站（瀑布流按剩下的宽度重排，点别的文章接着在右边换着看），右边是原网站的页面，上面一条工具栏可以喜欢、收藏、刷新、在新标签页打开、关闭（或按 Esc）。

很多网站用 `X-Frame-Options` 或 CSP `frame-ancestors` 禁止被别的网站嵌入。本站不为此多发请求：每月读博客首页时（本来就要读）顺便看这两个响应头，记在 `blogs.framable`；抓 RSS 时如果响应头已经禁止，也先记为不能嵌入。已知不能嵌入的直接开新标签页；还不知道的先在右侧试，并在工具栏下提示「显示不出来就改用新标签页」，读者点过的会记在浏览器里，下次直接开新标签页。本站用 https 而原文是 http 时，浏览器不允许嵌入，也开新标签页。嵌入的页面带 `sandbox`，不能把整个页面跳走。

## 读者自己的数据

不需要账号，全部存在浏览器里：「关注」在 cookie 里（静态版也能用），「喜欢」「收藏」「浏览历史」「刷到过」、各频道看到的位置和频道设置在 localStorage 里。喜欢和收藏会连同文章卡片一起保存，文章从站点下架后仍可在「我的」页看到、搜索。

- **刷到过**：首页、博客页和搜索结果里，卡片有一半以上在屏幕上停留 0.6 秒就记一次（点没点开都算），保留最近 1000 篇，记着刷到几次、在哪个频道。用来找回「之前好像看到过」的文章，也让「推荐」「反推荐」少重复推没点开的文章。可以关闭或清空。
- **上次看到哪里**：按时间排的频道（最新、关注、分类或标签按最新）记住你看到过的最新和最旧的一篇。下次打开，更新的文章标「新」，顶部提示有几篇新文章，点「从上次看到的地方继续」就从那篇往下接着看（`?from=` 在网址里，可以前进后退）。

「我的 → 备份与恢复」可以下载一个 JSON 备份文件（包括以上全部），在别的浏览器或设备上恢复：「合并」保留现有记录并加入备份里没有的，「覆盖」完全换成备份内容。浏览历史和刷到过都可以关闭或清空。

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

- **简介**：每月读一次首页和关于页（每个博客 2 个请求）：meta 描述、头像、关于页链接与约 300 字摘要、社交链接（GitHub、B站、微博、知乎……与 `rel="me"`，同一平台最多 3 个账号，页面上显示为各平台图标；同一平台有多个账号时每个都带上账号名）、博客程序（generator）。
  - 头像按整页打分挑选：图片的 alt（「头像」「博主」「Profile Image」）、所在位置（作者卡片、侧栏、页头）加分，评论、友链、文章列表、聊天挂件里的头像不算，封面、横幅、壁纸、logo 不算或减分；懒加载的图取真实地址，跳过加载动画、占位图和主题自带的默认头像。首页没有就看关于页，再依次退到 apple-touch-icon、作者自己链接里的 GitHub 头像、RSS 自带的图片、站点最大的 favicon。选中前会真的请求一次，确认是图片才用；地址里有 `//`（如 `cdn.jsdelivr.net/gh//user/…`，jsDelivr 会回 400）时也试一下去掉多余斜杠的地址。
  - 博主可以在配置里直接指定头像、简介、社交链接、标签（`[[policy.profile]]`，见下），优先于抓到的。
  - 博主改了网站、想马上看到：`feed-server enrich --blog <id>` 立即重读这一个博客的资料（服务器的接口缓存最多 10 分钟后更新）。读取方式改了、想全部马上用上：`feed-server enrich --force` 重读所有博客的资料（中途停了再运行一次，会接着做没读的）。出过错（403、超时、DNS……）正在退避等待的博客，`feed-server crawl --all --retry` 让它们现在就再试一次（找 RSS 出错的也重新找）；回过 429/503 要求等待的不提前。
- **「我的 → 我的博客」**：博主找到自己的博客后，可以看到本站抓它的情况——RSS 地址和来源、上次 / 下次检查、连续失败和错误、文章数、全文和评论授权、资料上次读取时间、头像来源、社交链接、关于页、标签，哪些来自配置。需要修改时，按 `[server] repo` 给出 GitHub 链接：一键提 issue（自动带上博客地址和当前抓到的内容），或直接编辑 `repo_config`（默认 `deploy/config.actions.toml`）提 PR，页面上有按当前内容预填好的配置片段可复制。博客页上也有入口「这是你的博客？」。
- RSS 修补：有的生成器（如 Hexo 的 feed 插件）会删掉换行，把写成两行的 `<img\nsrc="…">` 变成 `<imgsrc="…">`，图片和链接就读不出来；解析前会把常见标签和属性之间的空格补回去。日期也更宽容：`HKT` 等时区缩写、只有日期没有时间（`Sun, 15 Mar 2026`）、中文的星期和月份（`周日, 31 8月 2025 22:21:10 +0800`）都能读；所有文章都是同一个时间（生成时间）、2077 年这类日期仍然不认。
- **评论系统识别**：只看首页和关于页里的配置值（Twikoo 的 `envId`、Waline 的 `serverURL`、Artalk 的 `server`、Giscus / Utterances 的仓库、Disqus、Halo、Valine），仅出现产品名不算；WordPress / Typecho 等按 generator 判断为内置评论。不读文章页。
- **评论数**：
  - RSS 自带的 `<slash:comments>` / `<thr:total>`，不需要额外请求；
  - Twikoo（自建地址）、Waline、Artalk、Halo 2：每天最多一次批量请求（最新 20 篇）；401/403/404 视为博主不允许，30 天内不再请求；不伪造 Referer / Origin。
  - Giscus、Utterances、Disqus 没有批量接口，只显示「去原文评论」。
- **内嵌评论区**（`[[policy.embed_comments]]` 授权后）：详情页加载博客自己的评论组件，评论读写都在原博客的评论服务。组件脚本固定版本、只从 jsDelivr / giscus.app / utteranc.es 加载，从不加载博客站点本身的脚本；博主需要把本站域名加入评论服务的允许列表。静态版（Pages）对授权了全文或内嵌评论的博客，会把它们的文章详情单独导出（`data/posts/<id>.json`），所以 Pages 上也能看全文、用评论区；其他博客只显示评论数和链接。

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

[[policy.embed_comments]]                     # 作者授权后，详情页内嵌其评论组件
site = "https://example.com"
note = "2026-11-01 邮件授权"

[[policy.about]]                              # 作者对关于页的要求
site = "https://example.com"
url = "https://example.com/about-me/"         # 指定关于页（不写则从首页找）
full = true                                   # 取整页（保留排版），而不是约 300 字摘要
# text = "自己写的介绍，可以多行。"              # 或者直接用作者给的介绍，优先于页面内容
note = "2026-11-10 博主要求"

[[policy.profile]]                            # 博主指定的资料，优先于从网站上抓到的
site = "https://example.com"
avatar = "https://example.com/me.png"         # 头像
description = "一句话介绍"                     # 简介
social = ["https://github.com/someone", "https://x.com/someone"]   # 社交链接（替换抓到的，平台按地址识别）
tags = ["Rust", "摄影"]                        # 标签（替换按文章统计的）
about = "https://example.com/about-me/"       # 关于页，同 policy.about 的 url / full / text，
about_full = true                             #   写在这里就不用另开一段
# about_text = "自己写的介绍"
note = "issue #12"

[[policy.feed_overrides]]                     # 手动指定 RSS 地址
site = "https://example.com"
feed = "https://example.com/custom.xml"
```

站点按「去掉 www 的域名 + 路径」匹配，`https://www.a.com/` 和 `a.com` 视为同一站。撤销授权（从列表移除）后，已存的全文会在下次启动或同步时清除。`policy.about` 改了 `url` 或 `full` 会在下一轮重读关于页；`text` 立即生效；移除后恢复成自动读取的摘要。`policy.profile` 立即生效；抓到的内容照常保存，移除后恢复显示。

关于页还会显示站长（`[server.maintainer]`，名字和博客地址）；如果站长的博客也在收录范围里，会附上它在本站的页面。

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
| `GET /api/posts?mode=discover&seed=&cursor=` | 发现：近期文章，按种子轻度打散，同一博客连续出现会降权 |
| …`mode=explore&follow=&like=&save=&read=` | 反推荐：把这些偏好反过来用，越陌生的越靠前，每个博客最多 3 篇 |
| …`&follow=1,2&like=3&save=4&read=7,8&shown=9` | 推荐：再按关注的博主、喜欢 / 收藏 / 读过的文章个性化；`shown` 是刷到过却没点开的，稍微往后放 |
| `GET /api/posts?mode=latest&cursor=&blog=&q=` | 最新 / 某博客 / 搜索（标题、摘要、博客名，或恰好是某个标签），游标分页 |
| …`&blogs=1,2,3` | 只看这些博客（「关注」频道），按时间排序 |
| …`&category=tech&tag=Docker&kind=podcast&dirtag=tech&directory=travellings&cover=1` | 两种模式通用的筛选：分类（大类含其小类，或 `tech/ai`）、标签（同义写法合并）、来源类型、目录标签、来源目录、只看有图（`cover=1`，`cover=none` 只看无图）、只看长摘要（`text=1`，摘要至少 50 字）、只看有评论（`comments=1`），可以同时用 |
| `GET /api/posts/{id}` | 详情；`content_html` 仅对授权全文的博客返回 |
| `GET /api/posts/{id}/related?limit=8` | 相似文章 |
| `GET /api/tags` | 分类树（含文章数）、来源类型、各目录的标签（含博客数、文章数、中文名）和近半年热门标签 |
| `GET /api/tags/search?q=&limit=` | 按名称找标签，用得多的在前；不带 `q` 时列出全部用过两次以上的标签 |
| `GET /api/blogs?q=&category=&kind=&dirtag=&ids=&sort=recent\|posts\|name&offset=` | 博客目录；有 `q` 时按匹配程度排序（名称 > 网址 > 简介 > 关于页 / 分类） |
| `GET /api/blogs/{id}` | 博客主页（含目录标签、分类、类型、常用标签） |
| `GET /api/meta` | 站点名、数据来源、统计 |
