# 稳定性分析

这份文档列出会上线运行的代码里，所有可能让程序 panic、抛出异常或直接退出的地方，以及出错后悄悄吞掉、没人知道的地方。另外还有一些不会崩溃、却会让系统运行失常的情况，比如资源越用越多、平台的配额、过期的 token，作为人工条目列在对应的代码下面。每一处都引用了代码，并说明三件事：为什么这样写、什么情况下会出事、要不要改。

测试代码不算在内：测试里的 `unwrap` 本来就是要在意外时立刻失败。

## 和代码保持一致

这份文档由 `scripts/stability.mjs` 检查。CI 的 frontend 任务每次推送都会跑它，文档和代码不一致时 CI 失败。

```sh
cd frontend && npm ci                      # 第一次运行前：脚本要用 TypeScript 和 Vue 的解析器
node ../scripts/stability.mjs              # 检查
node ../scripts/stability.mjs --skeleton   # 生成还没写进文档的地方的条目骨架
node ../scripts/stability.mjs --summary    # 按各条目的结论生成「要改的地方」表
```

它检查四件事：
- 代码里每一处符合下面规则的写法，都要在它所在文件那一节里被引用到。
- 文档里引用的每一段代码，都要能在源码里原样找到，而且只对应一个位置。比较时逐行进行，忽略缩进；`// …`（模板里写 `<!-- … -->`）表示中间省略了几行。
- 每个条目的「要不要改」只能是以下四种之一：不用改 / 可以改（低）/ 建议改（中）/ 建议改（高）。
- 「要改的地方」表必须正好列出所有「建议改」的条目。

改代码时这样更新：
- **新写的**：补一个条目。
- **改动的**：更新引用的代码和说明。
- **删掉的**：删掉条目。

检查只能保证引用的代码还在，不能保证说明仍然成立。改代码的人要顺手看一眼说明。标题里带「（人工）」的条目，是规则抓不到、但人工看过的地方。

## 检查哪些写法

| 代码 | 写法 | 出事的样子 |
|---|---|---|
| Rust（`backend/src`，不含 `#[cfg(test)]`） | `.unwrap()` `.expect()` `.unwrap_err()` `.expect_err()` | 值不是预期的 `Some` / `Ok` 时 panic |
| | `panic!` `unreachable!` `todo!` `unimplemented!` 和 `assert!` 系列 | 直接 panic |
| | `unsafe` | 写错时是未定义行为（目前没有） |
| | `process::exit` `process::abort` | 进程立即结束（目前没有） |
| | 下标 `v[i]`、切片 `s[a..b]` | 越界时 panic；字符串切在一个字的中间时也 panic。不算 `x[..]`，也不算 serde_json 的 `v["key"]`（缺了只是 null） |
| | `truncate` `split_at` `split_off` `insert_str` `replace_range` `drain`、`String` 的 `insert(i, 'c')`，以及参数不是数字常量的 `chunks` / `windows` | 越界、不在字符边界、或块大小为 0 时 panic |
| | `let _ =` | 丢掉结果：出错了也没人知道 |
| TypeScript / Vue（`frontend/src`，包括模板里的表达式）和 Worker（`worker/src`） | `throw` | 没人接住，就是未处理的错误 |
| | 非空断言 `x!` | 运行时不检查；x 真是 null 时，接下来的访问抛 TypeError |
| | `JSON.parse` `new URL` `decodeURIComponent` `decodeURI` `atob` `btoa`，以及参数是变量的 `new RegExp` | 输入格式不对时抛出 |
| | `.json()` | 响应不是 JSON 时 reject |
| | 直接访问 `localStorage` / `sessionStorage` | 存储被禁用时抛 SecurityError，写满时抛 QuotaExceededError |
| | 空的 `catch {}`，以及什么也不做的 `.catch(() => …)` | 吞掉错误：出错了也没人知道。会把原因记进「设置 → 诊断信息」的不算 |

**不在检查范围内的**：
- **Rust 的整数溢出**：release 构建用默认配置，溢出时回绕，不 panic。debug 构建会 panic。
- **整数除以 0、对 0 取模**：从语法上看不出是不是整数运算。有疑问的地方用「（人工）」条目列出。
- **返回 `Err`、用 `?` 往上传**：错误交给调用方处理，不算崩溃。启动阶段的错误会让进程退出，这是有意的：配置错了就不该启动。
- **前端 `await fetch()` 因网络失败而 reject**：到处都有，由各调用处的 try/catch 处理，不逐个列出。
- **其他**：内存耗尽、栈溢出、依赖库内部的 panic。

## 出事的后果取决于在哪里

| 运行环境 | 一处 panic 或未处理的异常会怎样 |
|---|---|
| `feed-server serve` 的抓取循环（`Crawler::run`：重算标签、同步目录、抓 RSS、读博主资料和评论数） | 整个抓取任务结束，**之后不再抓取**，直到重启；网站照常开着。返回 `Err` 只会记日志、继续下一轮，panic 不会 |
| `feed-server watch`（另一台机器只抓不建站） | 抓取循环直接在主任务里跑（`select!`），panic 会让整个进程退出 |
| 放在 `spawn_blocking` 里的计算（例如抓取时提取关键词） | panic 变成普通错误，只影响这一个博客或这一次计算 |
| 网页 API（axum 的请求处理） | 只断开这一个请求的连接，服务器继续运行 |
| 其他后台任务（定时备份、每小时算热度） | 这个任务结束，直到重启都不再执行 |
| GitHub Actions 里的命令（`crawl`、`export`、`backup`） | 进程以非 0 退出，这次运行失败；`export` 失败时网站不更新。抓取进度存在数据库里，所以同一个有问题的 RSS 会让以后每次运行都在同一处失败 |
| 启动阶段（`main` 开始服务之前） | 进程退出。配置或数据库有问题时，本来就该这样 |
| 用 `LazyLock` 保存的固定正则、CSS 选择器 | 只有写错的常量才会在第一次使用时 panic；有单元测试覆盖的，带不到线上 |
| 浏览器：事件处理、异步函数 | 控制台报错，这一个操作没完成；页面其他部分照常 |
| 浏览器：组件渲染（模板里的表达式）、`setup` | 这个组件渲染失败。应用没有全局的 `errorHandler` |
| 浏览器：模块第一次加载时执行的代码 | 整个应用起不来，白屏 |
| Worker 的 `fetch`（`/api/*`） | 路由外面包了 try/catch：这个请求返回 500，并记日志 |
| Worker 的 `scheduled`（每 5 分钟） | 没有 try/catch：这一次定时任务中断，后面的步骤（启动抓取、处理刷新请求）这次都跳过，5 分钟后再试 |

## 要改的地方

正常运行时，外部输入（RSS、网站、读者）都触发不了 panic 或白屏，代码里也没有 `unsafe` 和 `process::exit`。值得改的是下面这些，大多不是崩溃，而是「出了问题没人知道」或者「坏了以后自己恢复不了」。其余条目是「不用改」或「可以改（低）」，改法写在各条目里。

| 级别 | 文件 | 条目 | 怎么改 |
|---|---|---|---|
| 高 | `backend/src/sources/travellings.rs` | 开往返回空列表或大批异常状态时，博客整批下线（人工） | 开往的接口「成功」地返回空列表、字段改了名，或者它的检测服务把大批成员标成超时，只在开往里的约 1,200 个博客就会一起从网站上消失，至少一天，没有任何报警。列表为空时报错（会转去试备用地址）；活着的成员比上次少一半以上时，不保存这次结果，记一条 warn |
| 中 | `.github/workflows/publish.yml` | 缓存和备份都拿不到时，会发布一个空网站（人工） | 数据库缓存 7 天没用就被 GitHub 删掉；这时如果合并备份也失败（这一步允许失败），导出的是空数据库，空网站会覆盖正常的网站。导出前检查数据库里的博客数，太少就让发布失败，保留线上的旧版本 |
| 中 | `.github/workflows/pages.yml` | 第三方 action 被仓库策略禁掉时，每次运行都失败而没人知道（人工） | 这次真的发生了：仓库只允许自己名下的 action 后，每次抓取都在启动时失败，Worker 启动成功、GitHub 的后备定时又被它压住，几天没人发现。让 Worker 在启动抓取后检查上一次运行的结果，或者检查线上 `data/index.json` 的生成时间，太旧就在网站上提示 |
| 中 | `worker/src/index.js` | 启动抓取失败只写日志，token 过期没人知道（人工） | Worker 用的 fine-grained token 一年左右就过期，过期后「刷新」按钮照样说「约 N 分钟后更新」，实际什么也没发生，统计也停了，只有 Worker 日志里有记录。连续失败时在 `/api/capabilities` 里关掉刷新，并在 `feed-server status` / 诊断信息里显示 |
| 中 | `.github/workflows/pages.yml` | 排队中的运行会被下一个顶掉（人工） | 同一组里只能有一个排队的运行，新的会把旧的顶掉。访客的刷新批次已经记成「发出了」，被顶掉就丢了。刷新用自己的并发组，或者 Worker 发出前确认上一批已经开始 |
| 中 | `worker/wrangler.jsonc` | 在 Cloudflare 后台改的变量，下一次发布就没了（人工） | 每次发布 `wrangler deploy` 都会用配置文件里的值盖掉后台改过的普通变量，README 却让人去后台改 `CRAWL_EVERY_HOURS`。改成从 GitHub 的变量传入，或者在配置里写 `keep_vars: true` |
| 中 | `backend/src/backup.rs` | 单个博客的备份文件没有大小上限（人工） | 文章从不删除，一个高产的 RSS（比如读者订阅的新闻源）的备份文件总有一天超过 GitHub 的 100 MB 上限，之后每次备份推送都被拒，而这一步允许失败，没人知道。给单个文件设上限，超过时只备份最近的文章并记 warn |
| 中 | `backend/src/crawler.rs` | 轮询间隔夹在配置的上下限之间（人工） | 只把 `max_refresh_secs` 设得比 `min_refresh_secs` 小（比如只写 `max_refresh_secs = 3600`，默认的下限是 7200），第一次遇到没更新的 RSS 就 panic，抓取停掉。在 `Config::load` 里校验上下限，或改成 `.max(min).min(max)` |
| 中 | `frontend/src/library.ts` | 写回列表，存储满时腾地方（localStorage） | 存储满时腾地方的顺序反了：写「刷到过」失败时会把历史永久砍到 50 条；两次重试都失败时，新的喜欢、收藏只留在内存里，刷新就丢，也不提示。改成先缩短正在写的「刷到过」、先清可以重新读取的订阅缓存，还写不进就在「我的」页提示 |
| 中 | `frontend/src/sources.ts` | 写进本地存储（localStorage） | 自己订阅的预览缓存（`sources:read`）只增不减，导入几百个 RSS 后可能占满浏览器存储，连带上一条的问题。只保留还在预览的源、限制总大小，写不进时清掉它 |
| 中 | `frontend/src/sync.ts` | 服务器拒收：副本太大（throw） | nginx 默认只收 1 MB 的请求，同步副本最大可到 1.9 MB，超过 1 MB 的读者每次上传都失败。收到 413 时用更小的上限重新打包；README 里写明 `client_max_body_size 2m;` |
| 中 | `worker/src/index.js` | 统计推送时读 GitHub 的回应（.json()） | 定时任务里统计推送一出错，同一轮的抓取调度就跳过；如果每轮都出错，网站会一直不更新。在 `scheduled` 里给 `pushStats` 单独接住错误（`.catch(...)`） |

## 逐处说明

按文件排列：先是 Rust 服务端，再是前端的脚本、组件和页面，最后是 Worker。

## `backend/src/analyze.rs`

### 去掉网址和邮箱的正则（unwrap）

```rust
static URLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)https?://\S+|www\.\S+|\S+@\S+\.\w+").unwrap());
```

- **为什么这样写**：正则是写死的常量，第一次用时编译一次。编译失败只可能是正则本身写错了，属于编码错误，不是运行时情况。
- **什么时候会出事**：不会：正则是合法的，`keywords` 的单元测试每次都会编译它。就算以后改坏了，`keywords` 也只在 `spawn_blocking` 里跑（crawler.rs `store_items`，以及 `retag_all` 里的 `prepare`），panic 会变成 JoinError，按普通错误记日志。不过 LazyLock 初始化 panic 后会“中毒”，之后每次访问都再 panic，于是每次写入文章都会失败。这种改坏在 `cargo test` 就会暴露。
- **要不要改**：不用改——常量正则，有测试覆盖。

### 全局语料库的读写锁（unwrap）

```rust
pub fn corpus() -> Arc<Corpus> {
CORPUS.read().unwrap().clone()
```

```rust
*CORPUS.write().unwrap() = corpus;
```

- **为什么这样写**：`CORPUS` 是 `RwLock<Arc<Corpus>>`。`retag_all` 算出新语料库后整体换上，抓取时 `corpus()` 取一份 `Arc`。标准库的锁只在持锁线程 panic 后才会中毒，`unwrap` 只针对这种情况。
- **什么时候会出事**：不会：持锁时只做 `Arc::clone` 或一次赋值（顺带释放旧的 `Corpus`，就是几张 HashMap），都不会 panic，所以锁不会中毒。假如真的中毒了，之后每次 `corpus()`（crawler.rs `store_items`，直接在抓取任务里调用）都会 panic，抓取任务会停掉，直到重启。
- **要不要改**：不用改——持锁区间里没有会 panic 的代码。

### 判断成人内容时看匹配词前后的字符（slice）

```rust
text.match_indices(w).any(|(i, _)| {
let before = text[..i].chars().next_back();
let after = text[i + w.len()..].chars().next();
```

- **为什么这样写**：英文词要整词匹配（"r18" 不能匹配 "r1800"），所以要看匹配处前后各一个字符是不是字母或数字。
- **什么时候会出事**：不会：`i` 是 `match_indices` 在同一个 `text` 上找到的起点，`i + w.len()` 是这次匹配的终点。两者都在字符边界上，也不越界。标题全是中文时也一样（`text` 是转小写后的新字符串，下标也来自它）。
- **要不要改**：不用改——下标都来自同一字符串上的查找。

### 只留前几个词、前几个分类（truncate）

```rust
terms.truncate(TERMS_PER_POST);
```

```rust
out.truncate(3);
```

- **为什么这样写**：一篇文章的词向量只留权重最高的 24 个词；一个博客的大类最多列 3 个。
- **什么时候会出事**：不会：`terms` 和 `out` 都是 `Vec`。`Vec::truncate` 在长度不够时什么也不做，从不 panic。会 panic 的是 `String::truncate` 切在字符中间，这里不是字符串。
- **要不要改**：不用改——`Vec::truncate` 不会 panic。

### 按词表给每个主题打分（index）

```rust
for &i in TOPIC_WORDS.get(key.as_str()).into_iter().flatten() {
scores[i] += w;
```

- **为什么这样写**：`TOPIC_WORDS` 把词映射到 `TOPICS` 的下标，`scores` 也按 `TOPICS` 的顺序排。
- **什么时候会出事**：不会：`scores` 的长度是 `TOPICS.len()`，`TOPIC_WORDS` 里的下标来自 `TOPICS.iter().enumerate()`。`TOPICS` 是编译期常量，两边不会对不上。
- **要不要改**：不用改——下标和长度来自同一个常量表。

### 训练分类器时按主题计数（index）

```rust
for (blog, raw, topic) in seeds {
docs[topic] += 1.0;
blogs.entry(blog).or_insert_with(|| vec![0.0; n])[topic] += 1.0;
for (k, (w, _)) in raw {
counts.entry(k.clone()).or_insert_with(|| vec![0.0; n])[topic] += w;
```

- **为什么这样写**：朴素贝叶斯要统计每个主题的文章数、每个博客的主题分布、每个词在各主题下的权重。三者都用长度为 `n = TOPICS.len()` 的 Vec 存。
- **什么时候会出事**：不会：`topic` 来自 `seed(&word_scores(raw))`，是长度为 `TOPICS.len()` 的分数表里的下标，三个 Vec 的长度都是 `n`。另外这段在 `retag_all` 的 `spawn_blocking` 里跑，即使 panic 也只是这一轮重算失败。
- **要不要改**：不用改——下标与长度来自同一个常量表。

### 去掉训练集中太少见的词（index）

```rust
counts.retain(|k, _| df[k] >= CLASSIFIER_MIN_DF);
```

- **为什么这样写**：只在不到 3 篇训练文章里出现的词不当证据。`df` 是 HashMap，`df[k]` 在键不存在时会 panic。
- **什么时候会出事**：不会：上面的循环每往 `counts` 放一个词，同时也给 `df.entry(k)` 加一，所以 `counts` 的每个键都在 `df` 里。
- **要不要改**：不用改——两张表在同一个循环里同步建出来。

### 用博客自己的主题分布做先验（index）

```rust
let c = own.map_or(0.0, |c| c[i]);
```

- **为什么这样写**：`own` 是这个博客在训练集里各主题的文章数，`i` 是遍历 `prior` 时的下标，把博客的偏好和全站的先验混在一起。
- **什么时候会出事**：不会：`prior` 和 `blogs` 里的每个 Vec 都是同一次 `train` 按 `n = TOPICS.len()` 建出来的，长度相同。没训练过时 `prior` 为空，`classify` 不会调用 `posterior`。这段经 `derive` 直接在抓取任务里跑，panic 会让抓取停掉，所以这个不变式很重要，但现在的代码能保证它。
- **要不要改**：不用改——长度在 `train` 里一起定下来。

### 词表有把握时直接定类（index）

```rust
if let Some(i) = seed(&scores) {
return TOPICS[i].path();
```

```rust
Some((i, s)) if *s >= 0.5 => TOPICS[i].path(),
```

- **为什么这样写**：`scores` 和 `TOPICS` 一一对应，拿到下标就是主题。
- **什么时候会出事**：不会：两处的 `i` 都是 `scores`（长度 `TOPICS.len()`）的下标。
- **要不要改**：不用改——下标来自同长度的分数表。

### 按分类器的概率定类（index）

```rust
if pb >= TOPIC_CONFIDENCE {
return TOPICS[best].path();
```

- **为什么这样写**：概率够高就用最可能的主题。
- **什么时候会出事**：不会：`p` 是 `posterior` 的结果，长度等于 `prior.len()`，也就是训练时的 `TOPICS.len()`。`TOPICS` 是常量，运行中长度不变。`p` 为空时，前面的 `let … else` 已经返回 `other`。
- **要不要改**：不用改——概率表与 `TOPICS` 等长。

## `backend/src/api.rs`

### 大家在看只留前 30 个博客（truncate）

```rust
blogs.truncate(30);
```

- **为什么这样写**：查询最多取 60 个热门博客，过滤掉不公开的之后按分数只留前 30 个。
- **什么时候会出事**：不会：`blogs` 是 `Vec<TrendingBlog>`，`Vec::truncate` 不足 30 个时什么也不做，不会 panic。
- **要不要改**：不用改——`Vec::truncate` 不会 panic。

## `backend/src/backup.rs`

### 合并两份备份里的同一篇文章（index）

```rust
let Some(&i) = at.get(&o.guid) else {
at.insert(o.guid.clone(), self.posts.len());
self.posts.push(o);
continue;
};
let p = &mut self.posts[i];
```

- **为什么这样写**：`at` 记着 guid 在 `self.posts` 里的下标，用来找到同一篇文章、合并两边的字段。
- **什么时候会出事**：不会：`at` 里的下标要么来自对 `self.posts` 的 enumerate，要么是 push 之前的 `len()`。循环里 `self.posts` 只增不减。备份文件里 guid 重复也只是 map 记住其中一个下标，仍然有效。
- **要不要改**：不用改——下标总在范围内。

### 过长的备份文件名截短（truncate）

```rust
name.truncate(180);
while !name.is_char_boundary(name.len()) || name.ends_with('%') {
```

- **为什么这样写**：文件系统只允许 255 字节的文件名，过长的博客地址截到 180 字节，再接一段哈希保证不重名。
- **什么时候会出事**：不会：`name` 只由 ASCII 字母数字、`.`、`-`、`_` 和 `%XX` 组成，全是单字节，任何位置都是字符边界。而且只在 `name.len() > 200` 时才截。下一行的 `is_char_boundary` 检查是多余的保险。
- **要不要改**：不用改——`name` 全是 ASCII。

### 删掉旧版本留下的空目录（let _ =）

```rust
for path in json_files(&dir.join(LEGACY_BLOGS))? {
std::fs::remove_file(path)?;
stats.changed += 1;
}
let _ = std::fs::remove_dir(dir.join(LEGACY_BLOGS));
```

- **为什么这样写**：旧版本把每个博客写在 `LEGACY_BLOGS` 目录里。现在把里面的文件搬走以后，顺手删掉这个目录。目录本来就不存在（新的备份仓库），或者里面还有别人放的文件时，`remove_dir` 会报错；这两种情况都不该让备份失败，所以不看结果。
- **什么时候会出事**：不会：删不掉只是留下一个目录，git 不跟踪空目录，备份的内容不受影响。
- **要不要改**：不用改——失败无害，也不值得记日志。

### 备份时把整份快照和整个数据库读进内存（人工）

```rust
pub async fn write_snapshot(pool: &SqlitePool, dir: &Path, allow_shrink: bool) -> anyhow::Result<SnapshotStats> {
let mut files = read_snapshot(dir)?;
for (key, mut mine) in read_database(pool).await? {
```

- **为什么这样写**：备份只增不减：先读出仓库里每个 `sites/*.json` 和数据库里的每个博客、每篇文章，按地址合并，再逐个文件比较，只写有变化的。整份读进来最简单，也不会漏。
- **什么时候会出事**：文章从不删除（`max_posts_per_blog` 默认 0），快照和数据库只会越来越大。每次备份时内存里同时有两份全部文章（`read_snapshot` 一份，`read_database` 一份），合并后还要逐个 `to_string_pretty`；文章到几十万篇时，峰值内存可能上 GB。`restore` 也是整份读入，而每次备份都先做一遍 `restore`。另外 `read_snapshot`、`write_if_changed` 用的是 `std::fs`，JSON 的解析和生成也直接在这个 async 函数里做，没有放进 `spawn_blocking`。`serve` 配了备份时每 `every_hours`（默认 6 小时）做一次，这几秒到几十秒里会占住一个 tokio 工作线程。单核 VPS 上只有一个工作线程，这段时间网站的所有请求都要等。平时没人会注意到，除非内存不够被 OOM 杀掉；容器设了 `restart: unless-stopped`，重启后下一次备份还会这样。
- **要不要改**：可以改（低）——把读快照、合并和写文件整个放进 `spawn_blocking`；文章多了以后改成一个博客一个博客地处理，每次只读这个博客的数据库记录和它的快照文件。

### 单个博客的备份文件没有大小上限（人工）

```rust
let sites = dir.join(SITES);
std::fs::create_dir_all(&sites)?;
for (key, file) in &files {
stats.changed += usize::from(write_if_changed(&sites.join(file_name(key)), &pretty(file)?)?);
}
```

- **为什么这样写**：一个博客一个文件，提交时只改动有变化的博客，diff 也能逐行看。
- **什么时候会出事**：每个文件里是这个博客抓到过的每一篇文章（标题、摘要、`excerpt_html`、关键词，允许全文的还有正文）。备份只增不减，文件也只会变大。普通博客没问题，但读者可以添加任何 RSS，配置里也能加新闻站。一个每天几百条的源，按每条 1–3 KB 算，一两年就会超过 GitHub 单个文件 100 MB 的上限（50 MB 起就有警告）。从那以后每次 `git push` 都被 GitHub 拒绝（GH001 Large files detected）。这个文件每次备份都会重新生成，所以之后所有备份都推不上去。Actions 的备份步骤设了 `continue-on-error`，运行照样显示成功；`serve` 只是每小时记一条 warn。而且推送失败会被当成冲突，每次多等 10–20 分钟（见「推送失败一律当作冲突重试」）。没人会注意到，直到需要从备份恢复时，才发现备份停在了几个月前。
- **要不要改**：建议改（中）——写文件前检查大小，超过比如 40 MB 就按年份拆成 `sites/<site>/<year>.json`；或者给新闻、论坛这类非博客的源设一个默认的 `max_posts_per_blog`。推送失败的信息里有 `GH001` 时直接报错，并写进 `feed-server status`。

### git 被中途杀掉后留下锁文件，备份从此失败（人工）

```rust
let mut cmd = Command::new("git");
cmd.args(COMMITTER)
.args(args)
.current_dir(dir)
.env("GIT_TERMINAL_PROMPT", "0")
.kill_on_drop(true);
```

- **为什么这样写**：每个 git 命令最多等 `GIT_TIMEOUT`（5 分钟）；超时后 future 被丢弃，`kill_on_drop` 把 git 进程杀掉，免得一个卡住的推送挂住整个备份任务。
- **什么时候会出事**：git 在 `add`、`commit` 时会建 `.git/index.lock`，正常结束才删掉。被 SIGKILL 杀掉时，锁文件就留在了原地。有两种情况会这样：一是命令超时被 `kill_on_drop` 杀掉；二是服务器在备份进行中被停掉，`docker stop` 或重启时，`feed-server` 不处理 SIGTERM（见 main.rs「按 Ctrl-C 时优雅退出」），10 秒后整个容器连同 git 被 SIGKILL。此后每次备份都报 “Unable to create '…/index.lock': File exists”。`serve`、`watch` 的备份目录是持久的，所以备份从此一直失败，日志里每小时一条 warn，直到有人手动删掉这个文件。Actions 每次重新克隆，不受影响。
- **要不要改**：可以改（低）——备份开始前检查 `.git/index.lock`：比 `GIT_TIMEOUT` 还旧就删掉并记一条 warn；同时让 `serve`、`watch` 处理 SIGTERM。

### 推送失败一律当作冲突重试（人工）

```rust
if push && let Err(e) = git(dir, &["push", "--quiet", "--set-upstream", "origin", "HEAD"], b).await {
if attempt < PUSH_ATTEMPTS {
let [lo, hi] = cfg.conflict_wait_minutes;
let wait = crate::util::random_between(lo * 60, hi * 60);
```

- **为什么这样写**：两个抓取程序（Actions 和家里的 `watch`）同时推送时，后推的那个会被拒绝。这时等一个随机时间，拿进对方的快照再推，最多试 3 次。
- **什么时候会出事**：不管推送因为什么失败，都走这条路：部署密钥被删、仓库改名、权限变了、文件超过 GitHub 的大小上限、网络断了。这些情况等多久都不会好，但每次都要等两轮 5–10 分钟，每轮还要再合并、再写一遍整份快照。Actions 里这一步排在最多 100 分钟的抓取之后，再加上这 10–20 分钟，离 job 130 分钟的超时就不远了（见 pages.yml 那条）。而这一步设了 `continue-on-error`，运行显示成功，没人会注意到。
- **要不要改**：可以改（低）——只在 stderr 里是「非快进被拒」（`[rejected]`、`non-fast-forward`、`fetch first`）时才等待重试，其他错误立即返回。

## `backend/src/cache.rs`

### 响应缓存的锁（unwrap）

```rust
fn get(&self, key: &str) -> Option<Entry> {
let mut entries = self.entries.lock().unwrap();
```

```rust
fn put(&self, key: String, entry: Entry) {
let mut entries = self.entries.lock().unwrap();
```

- **为什么这样写**：不带个人参数的 GET `/api/*` 共用一个 `Mutex<HashMap>` 缓存。锁只在持锁线程 panic 后才会中毒。
- **什么时候会出事**：不会：持锁时只做 HashMap 的查找、删除、`retain`、插入，以及 `Entry` 的克隆（`Bytes`、`HeaderValue`），都不会 panic，锁不会中毒。假如真的中毒了，之后每个走缓存的 API 请求都会 panic：进程还在，但整个 API 都不可用，直到重启。
- **要不要改**：不用改——持锁区间里没有会 panic 的代码。

### 拼 ETag 和 Cache-Control 头（unwrap）

```rust
etag: HeaderValue::from_str(&format!("W/\"{:016x}\"", h.finish())).unwrap(),
```

```rust
let cache_control = HeaderValue::from_str(&format!(
"public, max-age={remaining}, stale-while-revalidate={}",
remaining * 5
))
.unwrap();
```

- **为什么这样写**：`HeaderValue::from_str` 遇到控制字符等非法字节才会报错，这两个头的内容都是自己格式化出来的。
- **什么时候会出事**：不会：ETag 是 `W/"` 加 16 位十六进制数再加 `"`。Cache-Control 是固定文字加两个整数：`remaining` 最多 600 秒，乘 5 也不会溢出。两者都是可见 ASCII。
- **要不要改**：不用改——内容不含任何外部输入。

### 刷新博客之后，列表还是缓存里的旧内容（人工）

```rust
fn ttl(path: &str) -> Duration {
let secs = if path.starts_with("/api/posts/") {
600
} else if path == "/api/meta" || path == "/api/tags" || path.starts_with("/api/blogs/") {
300
} else {
60
};
```

```rust
// Browsers may reuse it for whatever is left of our own TTL.
let remaining = entry.expires.saturating_duration_since(Instant::now()).as_secs();
```

- **为什么这样写**：不带个人参数的 GET 请求在内存里缓存 1–10 分钟，浏览器也按剩下的时间缓存，再加 5 倍的 stale-while-revalidate，省掉重复的数据库查询。新文章本来就是几小时抓一次，晚几分钟看到无所谓。
- **什么时候会出事**：服务器版的博客页上，读者点「刷新」：`/api/refresh` 当场抓完，回复「读到 3 篇新文章」，页面随即重新请求这个博客的文章列表。可这个请求和刚才的一模一样：在 `max-age` 之内，浏览器直接用自己缓存的结果；过了 `max-age`，按 stale-while-revalidate 也会先显示旧的；就算请求到了服务器，`ResponseCache` 里还有最多 60 秒前的结果。于是读者看到提示说有新文章，列表里却没有。博客资料（`/api/blogs/…`，缓存 5 分钟）里的「上次抓取」也还是旧的。前端只在 `queued` 分支里先显示浏览器自己读到的文章，`done` 分支没有这一步。
- **要不要改**：可以改（低）——刷新抓到新文章后，清掉缓存里这个博客相关的条目（把 `ResponseCache` 也交给 readers 的路由；最简单是整个清空）。前端在 `done` 之后重新请求时加 `cache: 'no-cache'` 或一个会变的参数。

## `backend/src/clean.rs`

### 编译固定的正则表达式（unwrap）

```rust
fn re(s: &str) -> Regex {
    Regex::new(s).unwrap()
}
```

- **为什么这样写**：`re()` 只给本文件的 `LazyLock<Regex>` 静态量用，传进来的都是写死的模式。`Regex::new` 只会因为模式本身写错而失败，那是编程错误，用 `unwrap` 让它在第一次使用时就暴露出来，省得每处都写一遍。
- **什么时候会出事**：不会：模式与输入无关，要么永远能编译，要么每次都 panic。clean.rs 的测试覆盖了所有经过 `re()` 的静态量：`excerpt_text` / `strip_markdown` 的测试用到 ESCAPED_HTML、BARE_LINKS、全部 MD_*、TEMPLATE_TAG、SPACES；`boilerplate_tails_are_removed` 用到 BOILERPLATE 和 TRAILING_ELLIPSIS；`tag` 的测试用到 TAG_PREFIX；`is_spam` 的测试用到 SPAM。改错了模式，`cargo test` 先失败。
- **要不要改**：不用改——常量模式由测试守着，没有输入能让它失败。

### 去掉作者名外面的括号（slice）

```rust
if s.starts_with('(') && s.ends_with(')') {
    s = s[1..s.len() - 1].trim().to_string();
}
```

- **为什么这样写**：feed 的作者字段常见 `(Name)` 这种带括号的写法（RSS `email (Name)` 惯例的残留），去掉首尾括号只留名字。
- **什么时候会出事**：不会：进入分支时 `s` 以 `(` 开头、以 `)` 结尾，两个都是 1 字节的 ASCII 字符，一个字符不可能同时是两者，所以 `len ≥ 2`，下标 1 和 `len - 1` 都落在字符边界上。`"()"` 得到空串，`"(小明)"` 得到 `小明`，都不会 panic。
- **要不要改**：不用改——边界由前面的 `starts_with` / `ends_with` 保证。

### 去掉标签里的「tag:」「分类：」前缀（slice）

```rust
Some(m) if m.end() < t.len() => t[m.end()..].trim(),
```

- **为什么这样写**：有的 feed 把种类写进分类名（`tag:Docker`、`category:教程`），切掉正则匹配到的前缀。守卫 `m.end() < t.len()` 是为了让只有前缀的 `tag:` 原样保留而不是变成空串（analyze.rs 的测试 `tag_display("tag:")`），不是为了防越界。
- **什么时候会出事**：不会：`m` 是在同一个字符串 `t` 上的匹配，regex 对 `&str` 的匹配位置总在字符边界上，且 `m.end() ≤ t.len()`。这个函数也会经 `analyze::tag_display` 被调用（包括 `spawn_blocking` 里的关键词提取），同样安全。
- **要不要改**：不用改——下标来自同一字符串上的匹配。

## `backend/src/comments.rs`

### 编译固定的正则和 CSS 选择器（unwrap）

```rust
fn key_value(keys: &str) -> Regex {
    Regex::new(&format!(
        r#"(?i)["']?\b(?:{keys})["']?\s*[:=]\s*["']([^"'\s<>]{{2,300}})["']"#
    ))
    .unwrap()
}
```

```rust
static DISQUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)//([a-z0-9-]{2,64})\.disqus\.com/embed\.js").unwrap());
static USE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)["']?use["']?\s*[:=]\s*\[?\s*["'](\w+)["']"#).unwrap());
```

```rust
fn script_attrs(html: &str, src_part: &str) -> Option<std::collections::HashMap<String, String>> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("script[src]").unwrap();
```

```rust
pub(crate) static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<(?:item|entry)[\s>].*?</(?:item|entry)>").unwrap());
static ITEM_COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(?:slash:comments|thr:total)>\s*(\d{1,7})\s*</(?:slash:comments|thr:total)>").unwrap()
});
```

- **为什么这样写**：模式都是常量。`key_value` 用 `format!` 拼模式，但 `keys` 只来自本文件 `LazyLock` 里写死的单词和 `|`（`"envId|env_id|…"`、`"server"` 等），没有外部输入。正则放在 `LazyLock` 里只编译一次；选择器每次调用 `script_attrs` 重新解析，也是常量。
- **什么时候会出事**：不会：只有改代码把模式写错才会 panic，而且每次都会。测试覆盖：`detect` 的测试编译了 TWIKOO_ENV、SERVER_URL、SERVER、SITE、REPO、VALINE_APP、DISQUS、USE 和 `script[src]`；REPO_ID、CATEGORY 等其余 `key_value` 用同一个模板，只是换了单词；ITEM / ITEM_COUNT 由 `rss_comment_counts_align_with_entries` 覆盖（ITEM 也被 feed.rs 的 `loose_entry_dates` 用）。
- **要不要改**：不用改——常量模式由测试守着。

### 在关键词附近按字节窗口找配置值（slice）

```rust
fn near(lower: &str, pos: usize, word: &str, before: usize, after: usize) -> bool {
    let start = lower.floor_char_boundary(pos.saturating_sub(before));
    let end = lower.ceil_char_boundary((pos + after).min(lower.len()));
    lower[start..end].contains(word)
}
```

```rust
fn value_after(html: &str, from: usize, window: usize, re: &Regex) -> Option<String> {
    let end = html.ceil_char_boundary((from + window).min(html.len()));
    re.captures(&html[from..end]).map(|c| unescape(&c[1]))
}
```

- **为什么这样写**：评论系统的配置值要在产品名附近才算数（前 1500 / 后 200 字节，或产品名之后 800 / 1200 字节），按字节取窗口最简单。页面多是中文，窗口边界先用 `floor_char_boundary` / `ceil_char_boundary` 挪到字符边界上再切。
- **什么时候会出事**：不会：`near` 的 `start` / `end` 都挪到了字符边界，`end` 不超过 `len`。`value_after` 的 `end` 同理；`from` 来自 `lower.match_indices("artalk")` 或 `lower.find("giscus")`，而 `lower = html.to_ascii_lowercase()`，它只改 ASCII 字节，字节长度和字符边界与 `html` 完全相同，匹配又以 ASCII 字母开头，所以 `from` 也是 `html` 的字符边界，且 `from < end`。`near` 里把 `html` 上的位置用在 `lower` 上，同理。
- **要不要改**：不用改——唯一的隐含前提是 `lower` 必须用 `to_ascii_lowercase()` 生成；若改成 `to_lowercase()`（有的字符小写后字节数会变，如 `İ`），`from` 就可能落在汉字中间，`value_after` 会在爬虫任务里 panic。改这里时要记得。

### 取正则的第 1 个捕获组（index）

```rust
re.captures(&html[from..end]).map(|c| unescape(&c[1]))
```

```rust
if let Some(c) = DISQUS.captures(html) {
    found.push(CommentSystem::Disqus {
        shortname: c[1].to_ascii_lowercase(),
```

```rust
let used: Vec<String> = USE.captures_iter(html).map(|c| c[1].to_ascii_lowercase()).collect();
```

```rust
.map(|m| ITEM_COUNT.captures(m.as_str()).and_then(|c| c[1].parse().ok()))
```

- **为什么这样写**：`c[1]` 比 `c.get(1).unwrap().as_str()` 简短。
- **什么时候会出事**：不会：`c[1]` 只在第 1 组可选且这次没参与匹配时 panic。这几个正则的第 1 组都不是可选的：`key_value` 的 `([^"'\s<>]{2,300})`、DISQUS 的 `([a-z0-9-]{2,64})`、USE 的 `(\w+)`、ITEM_COUNT 的 `(\d{1,7})`，整个正则匹配上时它一定有值。
- **要不要改**：不用改——捕获组是必选的。

### giscus / utterances 的 pathname 映射（slice）

```rust
fn pathname_term(path: &str) -> String {
    if path.len() < 2 {
        return "index".into();
    }
    let p = &path[1..];
    match p.rfind('.') {
        Some(i) if !p[i + 1..].is_empty() && p[i + 1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            p[..i].to_string()
        }
```

- **为什么这样写**：照 giscus / utterances 前端的 `pathname` 规则算搜索词：去掉开头的 `/` 和文件扩展名，根路径是 `index`。
- **什么时候会出事**：不会：`path` 来自 `Url::path()`，url 库会把非 ASCII 字符百分号编码，路径全是 ASCII，每个字节都是字符边界；`len < 2` 时先返回，所以 `[1..]` 不越界；`i` 是 `rfind('.')` 的位置，`i + 1 ≤ p.len()`。它在 HTTP 接口 `get_post`（api.rs 生成评论嵌入配置）里运行，就算出错也只断开一个请求。
- **要不要改**：不用改——下标都有保证。

## `backend/src/config.rs`

### 爬虫的数值设置都不检查（人工）

```rust
let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
cfg.apply_env();
cfg.server.columns = cfg.server.columns.clamp(1, 12);
```

- **为什么这样写**：`deny_unknown_fields` 让拼错的键在启动时就报错。数值只有 `columns` 夹了范围，其余照写的用。
- **什么时候会出事**：几个值写成 0 或写反时不会报错，程序照常启动，只是悄悄做错事（`min_refresh_secs` 和 `max_refresh_secs` 写反的情况已在 crawler.rs 单独列出）。`request_timeout_secs = 0` 时 reqwest 的超时是 0，每个请求马上超时，所有博客都记成失败，一路退避到一周一次。`directory_sync_secs = 0` 时每个 tick（30 秒）都重新拉一次目录；`discovery_retry_secs = 0` 时，没找到 RSS 的博客每个 tick 都重新探测一遍。这两种都会频繁打扰开往和这些博客。`refresh_secs` 小于 `min_refresh_secs` 时，抓到新文章后下一次只隔 `refresh_secs`（`ingest` 直接用它，不受下限约束）。`per_host_interval_secs = 0` 时，同一主机的请求不再间隔。这些只能从日志里，或者对方站长的抱怨里发现。
- **要不要改**：可以改（低）——在 `Config::load` 里统一校验：超时和各种间隔必须大于 0（或者有合理的下限），`refresh_secs` 在 `[min_refresh_secs, max_refresh_secs]` 之间，不满足就启动失败。

## `backend/src/crawler.rs`

### 只保留最新的 N 条（truncate）

```rust
if self.c().max_items_per_feed > 0 {
    items.truncate(self.c().max_items_per_feed);
}
```

- **为什么这样写**：配置了 `max_items_per_feed` 时只读 feed 里最新的 N 条（已按发布时间倒序排好）。
- **什么时候会出事**：不会：这是 `Vec::truncate`，长度不足时什么也不做，从不 panic（会因字符边界 panic 的是 `String::truncate`）。
- **要不要改**：不用改——`Vec::truncate` 不会失败。

### 按下标取对应条目的 backlog 位置（index）

```rust
let backlog_at: Vec<Option<i64>> = items
    .iter()
    .enumerate()
    .map(|(idx, item)| {
```

```rust
for (idx, (item, kw)) in items.iter().zip(&keywords).enumerate() {
    let content = if full_text { feed_full_text(item) } else { None };
    let content_ref = content.as_deref().map(|c| (c, "feed"));
    let kw_json = serde_json::to_string(kw)?;
    let entry = db::Entry {
        position: idx as i64,
        backlog_at: backlog_at[idx],
```

- **为什么这样写**：`backlog_at` 是先按 `items` 顺序算好的一列，入库时按同一个下标取。
- **什么时候会出事**：不会：`backlog_at` 由 `items.iter().map(...)` 生成，长度等于 `items.len()`；`idx` 来自对 `items`（与同样由 `items` 生成的 `keywords` zip）的 `enumerate`，一定小于 `items.len()`。
- **要不要改**：不用改——两个列表由同一个 `items` 生成，长度相同。

### 轮询间隔夹在配置的上下限之间（人工）

```rust
fn slower(&self, current: i64) -> i64 {
    (current * 3 / 2).clamp(self.c().min_refresh_secs as i64, self.c().max_refresh_secs as i64)
}
```

- **为什么这样写**：feed 没有变化时把下次轮询的间隔拉长 1.5 倍，并限制在配置的 `min_refresh_secs` 和 `max_refresh_secs` 之间。
- **什么时候会出事**：`Ord::clamp` 在 `min > max` 时无条件 panic（`assert!(min <= max)`，release 也一样），而 `Config::load` 不检查这两个值的大小关系。比如运营者只把 `max_refresh_secs` 改成 3600（默认 `min_refresh_secs` 是 7200），那么第一个返回 304 或内容没变的 feed（`unchanged`），或一次没有新文章的抓取（`ingest`），就会触发。`serve` 下爬虫任务就此结束，直到重启都不再抓取；`crawl` / GitHub Actions 每次都在这里退出。
- **要不要改**：建议改（中）——在 `Config::load` 里校验 `min_refresh_secs <= max_refresh_secs`（不满足就报错），或在这里改成 `.max(min).min(max)`，不再 panic。

### 读博主资料失败后，照样记下「读过了」（let _ =）

```rust
if let Err(e) = self.refresh_profile(&b, None).await {
tracing::debug!(blog = %b.homepage_url, error = %e, "profile refresh failed");
let _ = db::touch_profile(&self.pool, b.id, now()).await;
}
```

```rust
Err(e) => {
tracing::debug!(blog = %blog.homepage_url, error = %e, "profile refresh failed");
let _ = db::touch_profile(&self.pool, blog.id, t).await;
}
```

- **为什么这样写**：主页打不开、超时的时候，也要更新这个博客的 `profile_at`，否则它每一轮都会被重新挑出来再试，挤占别的博客。这一步写数据库失败时也没有更好的办法：下一轮挑到它就再试一次，所以不看结果。读资料失败只记 debug 级别的日志：每个博客失败的原因不重要，数量又大，默认的 info 级别不显示。
- **什么时候会出事**：数据库写失败时，这个博客下一轮还会被选中重读资料，多发几次请求（同一主机 10 秒的间隔照样遵守）。数据库一直写不进时，抓取的其他步骤也会报错，日志里看得到。读资料失败本身，只有设了 `RUST_LOG=feed_server::crawler=debug` 才看得到。
- **要不要改**：可以改（低）——把 `let _ =` 换成 `if let Err(e) = … { tracing::warn!(…) }`，数据库写失败时至少留下日志；读资料失败仍然记 debug 即可。

### 评论系统不提供评论数，过一阵再看（let _ =）

```rust
if !sys.supports_counts() {
// Nothing to ask; look again after the next profile refresh.
let _ = db::save_comment_counts(&self.pool, blog.id, &[], t + profile_every.max(86400)).await;
return;
```

- **为什么这样写**：这个博客用的评论系统没有评论数可查时，把下次检查推迟到下次读资料以后。所在的函数没有返回值，写失败也没有别处可交。
- **什么时候会出事**：写失败时下次检查没有推迟，下一轮还会挑到它；但这一步不发任何请求，只是多一次数据库写，无害。
- **要不要改**：不用改。

### Retry-After 不设上限（人工）

```rust
FetchError::Status { retry_after, .. } => retry_after.map(|d| d.as_secs() as i64),
```

```rust
fn backoff(&self, blog: &BlogRow, retry_after: Option<i64>) -> i64 {
let base = blog.refresh_secs.max(self.c().min_refresh_secs as i64);
let exp = base
.saturating_mul(1 << blog.failures.clamp(0, 10))
.min(MAX_BACKOFF_SECS);
jitter(exp.max(retry_after.unwrap_or(0)), 10, blog.id)
}
```

- **为什么这样写**：对方回 429/503 并带 `Retry-After` 时，至少按对方说的时间等；自己的指数退避最长一周。
- **什么时候会出事**：`Retry-After` 只当下限、没有上限，有两种出错的情况。第一种，值很大但没有溢出：对方配置错了，比如把毫秒当成秒，`Retry-After: 3600000` 就是 41 天，这个博客就停这么久。`crawl --retry` 特意跳过 429/503；服务器版的刷新按钮会说「N 分钟后可以再刷新」，N 可能有几万；只能手动改数据库。第二种，值接近 `i64::MAX`（有的服务器用它表示「永远别来」）：`jitter` 里的 `secs * percent` 回绕成负数，于是原样返回；接着 `t + backoff` 也溢出，得到一个很大的负数，`next_check_at` 永远早于现在。结果正好相反：这个博客每一轮都到期，`serve` 和 `watch` 每 30 秒请求它一次；`crawl --all` 每轮都有到期的博客，要一直转到 `--max-minutes` 才停（见下一条）。release 构建不会 panic，debug 构建会在 `jitter` 里 panic。两种情况都没人会注意到，除非对方站长来投诉。
- **要不要改**：可以改（低）——把 `retry_after` 夹到一个上限，比如 30 天，和 `MAX_BACKOFF_SECS` 一个量级；`next_check_at` 用 `saturating_add` 算。

### crawl --all 可能一直转到截止时间（人工）

```rust
while std::time::Instant::now() < deadline {
let s = self.crawl_due(round.max(1)).await?;
if s.checked == 0 {
break;
}
tracing::info!(?s, "crawl round finished");
total.add(&s);
}
while std::time::Instant::now() < deadline && self.enrich_due(ENRICH_ROUND).await? > 0 {}
```

```rust
let Some(sys) = system.filter(|_| count_every > 0 && blog.comments_next_at <= t) else {
return;
};
```

- **为什么这样写**：Actions 的定时运行在时间预算内一轮一轮地抓，直到没有到期的博客，然后把到期的博主资料和评论数也补完。截止时间只在两轮之间检查，这样一轮里的博客总能抓完。
- **什么时候会出事**：两个循环都要等「处理过的博客不再到期」才会结束。只要有一个博客处理完还是到期，循环就要转到截止时间。第二个循环的例子：`due_for_enrich` 选的是 `comment_system` 不为空、`comments_next_at` 已到的博客；可 `comment_system` 存的如果是这个版本不认识的评论系统（家里的 `watch` 和 Actions 版本不同，通过备份互相合并时就会这样），解析结果是 None，`enrich` 在上面第二段代码处直接返回，`comments_next_at` 没变。这个博客永远到期，循环每次查一遍数据库、什么也不做，一直空转到 `--max-minutes`（100 分钟）。`feed-server enrich --all` 没有截止时间，就成了死循环。第一个循环的例子：`next_check_at` 被写成过去时间的博客（见上一条 Retry-After 溢出），每一轮都会被选中。另外，截止时间只在两轮之间检查，一轮 200 个博客碰上大量超时、找 RSS 的探测（同一主机隔 10 秒）和读资料，可能要十几二十分钟，会越过截止时间这么久。后果都是 Actions 的运行变长、网站晚更新、离 130 分钟的超时更近；日志里只能看到不停重复的记录。
- **要不要改**：可以改（低）——`enrich` 里 `comment_system` 解析失败时也把 `comments_next_at` 推后（或者清掉它，等下次读资料时重新识别）；两个循环在「这一轮的博客和上一轮完全一样」时停下；把截止时间也传给 `crawl_blogs`，过了截止时间就不再开始新的博客。

### 解析 feed 和网页的 CPU 活在异步线程上（人工）

```rust
let parsed = match feed::parse(body, feed_url) {
```

- **为什么这样写**：抓取主要是在等网络，一次解析通常很快；提取关键词（分词）这种明显吃 CPU 的已经放进了 `spawn_blocking`。
- **什么时候会出事**：`feed::parse` 要处理的正文最大 4 MB（`max_body_bytes`），要解析 XML，还要给每一条生成并清理摘要 HTML。同样直接在异步任务里跑的还有：找 RSS 和读资料时用 scraper 解析整页 HTML（`discovery`、`profile::from_home`、`comments::detect`、`fulltext::main_content`），`store_items` 事务里的 `feed_full_text`（`html::sanitize`）和 `analyze::derive`，以及 readers.rs 的 `/api/fetch` 遇到网页时调用的 `html::feed_links`。碰上大的输入，一次可能占住工作线程几百毫秒到一两秒。`serve` 里抓取和网站共用一个 tokio 运行时，工作线程数等于 CPU 核数，单核 VPS 上这段时间所有 API 请求都在排队。第一次抓取，或者规则变了要全部重抓时最明显。表现只是网站偶尔变慢，没人会想到是抓取引起的。
- **要不要改**：可以改（低）——把 `feed::parse` 和对整页 HTML 的解析放进 `spawn_blocking`（输入是 `Bytes` 或 `String`，可以直接 move 进去）；或者让 `serve` 的抓取跑在单独的运行时里。

## `backend/src/db.rs`

### 别的写操作占着数据库超过 10 秒时（人工）

```rust
.journal_mode(SqliteJournalMode::Wal)
.foreign_keys(true)
.busy_timeout(Duration::from_secs(10));
let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
```

- **为什么这样写**：WAL 模式下读和写互不阻塞，同一时刻只能有一个写事务；别人正在写时，最多等 10 秒。
- **什么时候会出事**：有的写事务很长。`backup::restore` 为了「全有或全无」，把整份快照的合并放在一个事务里，`serve` 每次定时备份都要先做一遍；另一台机器推上来的改动多时（比如它升级了解析器，每篇文章的 `excerpt_html` 都变了），要更新几十万行。`store_items` 在事务里还要做 HTML 清理和打标签。手动对 `serve` 正在用的数据库跑 `feed-server restore`、`analyze`、`backup` 也一样。这时其他写操作等满 10 秒就报 `database is locked`：读者的 `/api/sync` 上传、`/api/events` 回 500，前端稍后重试。抓取里的 `store_items` 失败时，`crawl_blog` 把它当成这个博客的错误记下来：`failures` 加一、按指数退避，`last_error` 写的是数据库的报错，算到了无辜的博客头上。`record_failure` 本身也可能因为同样的原因写不进去，那就只有一条 error 日志。
- **要不要改**：可以改（低）——`busy_timeout` 提到 30–60 秒；`restore` 改成分批提交，比如每 200 个博客一个事务（合并做两遍和做一遍结果一样，中途失败下次从头再来也无害）；`crawl_blog` 遇到 SQLite 的 BUSY/LOCKED 错误时，不算作博客的失败。

## `backend/src/discovery.rs`

### 从文件地址退回到它所在的目录（let _ =）

```rust
if last.contains('.') {
let _ = u.path_segments_mut().map(|mut s| {
s.pop().push("");
});
```

- **为什么这样写**：找 RSS 时要从 `/blog/post.html` 退回到 `/blog/`。`path_segments_mut()` 只有在地址不能带路径时（`mailto:`、`data:` 这类）才返回 `Err(())`，`let _ =` 只是丢掉这个 `Result`。
- **什么时候会出事**：不会：这里的地址都是博客主页，在收录时已经确认是 http(s)。万一不是，路径保持原样，只是多试一个候选地址。
- **要不要改**：不用改。

## `backend/src/export.rs`

### 把最新文章分片写成文件（chunks）

```rust
let shard_size = opts.shard_size.max(1);
let shards: Vec<&[Value]> = posts.chunks(shard_size).collect();
```

- **为什么这样写**：前端按片加载最新文章（`feed-N.json`）。`chunks(0)` 会 panic。
- **什么时候会出事**：不会：`shard_size` 来自 `--shard-size`，即使传 0 也会被上一行的 `.max(1)` 改成 1。
- **要不要改**：不用改——已经有 `.max(1)` 兜底。

### 读者自己的订阅源文件（index）

```rust
own_posts.insert(*id, (blog, posts));
}
let (blog, posts) = &own_posts[id];
```

- **为什么这样写**：同一个博客可能被几位读者用不同地址订阅（`own` 里同一个 `id` 出现多次），博客和文章只取一次，用 HashMap 记住。
- **什么时候会出事**：不会：上面刚判断过 `contains_key`，没有就插入，所以 `own_posts[id]` 一定存在。
- **要不要改**：不用改——取之前一定已经插入。

### 分页取完后截到上限（truncate）

```rust
items.truncate(max);
```

- **为什么这样写**：按 `next_cursor` 一页页取，最后一页可能超过要的数量，截到 `max`。
- **什么时候会出事**：不会：`items` 是 `Vec<Value>`，`Vec::truncate` 不会 panic（`max` 为 `usize::MAX` 时什么也不做）。
- **要不要改**：不用改——`Vec::truncate` 不会 panic。

## `backend/src/feed.rs`

### 编译固定的正则表达式（unwrap）

```rust
static PEERTUBE_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/(?:w|videos/watch)/[\w-]+/?$").unwrap());
```

```rust
static FORUM_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?:t/[^/]+/\d+|thread-\d+-|d/\d+|topic/\d+)").unwrap());
```

```rust
static URL_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/((?:19|20)\d{2})[/-](\d{1,2})[/-](\d{1,2})(?:/|[-_.]|$)|/((?:19|20)\d{2})(\d{2})(\d{2})(\d{2})?(\d{2})?(?:/|[-_.]|$)").unwrap()
});
```

```rust
static REPLY_ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^post_\d+$").unwrap());
```

```rust
static ENTRY_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(published|pubDate|dc:date|issued|updated|modified|lastBuildDate)>\s*([^<]{8,40}?)\s*</").unwrap()
});
```

- **为什么这样写**：常量模式放在 `LazyLock` 里，第一次用时编译一次；只有模式写错才会失败，那是编程错误。
- **什么时候会出事**：不会：与 feed 内容无关。测试都会编译到：`video_channels_link_to_video_pages` 用到 PEERTUBE_URL 和 FORUM_URL（普通博客那组走到论坛判断），`dates_from_urls` 用到 URL_DATE，`forum_replies_collapse_into_their_topic` 用到 REPLY_ANCHOR，`reads_dates_feed_rs_rejects` 用到 ENTRY_DATE。
- **要不要改**：不用改——常量模式由测试守着。

### 每篇最多留 6 个标签（truncate）

```rust
tags.truncate(6);
```

- **为什么这样写**：分类太多的条目只留前 6 个，显示和打标签都够用。
- **什么时候会出事**：不会：`tags` 是 `Vec<String>`，`Vec::truncate` 在长度不足时什么也不做。
- **要不要改**：不用改——`Vec::truncate` 不会失败。

### 取日期正则的两个捕获组（unwrap）

```rust
let mut found: Vec<(&str, &str)> = ENTRY_DATE
    .captures_iter(m.as_str())
    .map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str()))
```

- **为什么这样写**：同时要标签名（第 1 组）和日期文本（第 2 组）。
- **什么时候会出事**：不会：ENTRY_DATE 的两个组 `(published|…)` 和 `([^<]{8,40}?)` 都不是可选的，正则匹配上时都有值。
- **要不要改**：不用改——捕获组是必选的。

## `backend/src/fulltext.rs`

### 逐个解析固定的正文容器选择器（unwrap）

```rust
for sel in SELECTORS {
    let sel = Selector::parse(sel).unwrap();
```

- **为什么这样写**：`SELECTORS` 是写死的常见主题正文容器列表，按顺序试。选择器是常量，解析失败只可能是写错了。
- **什么时候会出事**：不会：与页面内容无关。测试 `short_pages_yield_nothing` 的页面只有一个很短的 `<article>`，会把整个列表都解析一遍。`profile::about_text` 也用这个列表。
- **要不要改**：不用改——常量选择器由测试守着（每次调用都重新解析只是性能问题）。

## `backend/src/html.rs`

### 编译固定的正则和 CSS 选择器（unwrap）

```rust
static GLUED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)<(img|a|source|video|audio|iframe|span|div|p)(src|href|alt|class|id|width|height|title|style|srcset|loading|data-[a-z-]+)=",
    )
    .unwrap()
});
```

```rust
let doc = Html::parse_fragment(html);
let sel = Selector::parse("img").unwrap();
```

```rust
let sel = Selector::parse("link[rel][href]").unwrap();
```

- **为什么这样写**：模式和选择器都是常量，只有写错才会失败。
- **什么时候会出事**：不会：与输入无关。GLUED 由 feed.rs 的 `glued_attributes_are_read`（以及所有 `feed::parse` 测试）覆盖，`img` 由 `first_image_prefers_lazy_attrs_and_skips_junk` 覆盖，`link[rel][href]` 由 `feed_links_are_found_and_ranked` 覆盖。
- **要不要改**：不用改——常量由测试守着。

## `backend/src/http.rs`

### 取响应体开头 1024 字节判断是不是 SVG（slice）

```rust
let head = String::from_utf8_lossy(&body[..body.len().min(1024)]).to_ascii_lowercase();
```

- **为什么这样写**：只看开头一小段就够判断 `<svg`，不必把整个响应体转成字符串。
- **什么时候会出事**：不会：这是字节切片 `&[u8]`，上界 `min(len, 1024)` 不超过长度；字节切片没有字符边界的问题，被截断的半个 UTF-8 字符由 `from_utf8_lossy` 换成替换字符。
- **要不要改**：不用改——上界由 `min` 保证。

### 每个主机的下次可请求时间表（unwrap）

```rust
let start = {
    let mut slots = self.next_slot.lock().unwrap();
    let now = Instant::now();
    if slots.len() > 4096 {
        slots.retain(|_, t| *t > now);
    }
    let start = slots.get(host).copied().filter(|t| *t > now).unwrap_or(now);
    slots.insert(host.to_string(), start + self.per_host_interval);
    start
};
```

- **为什么这样写**：std `Mutex`，锁只在这个同步块里持有，块结束才 `sleep_until`，不跨 `await`，所以不需要 tokio 的锁。`lock()` 只在锁中毒时返回 `Err`。
- **什么时候会出事**：只有持锁期间 panic 过，锁才会中毒。锁内只有 `HashMap` 操作和 `start + self.per_host_interval`，后者只在 `per_host_interval_secs` 配成接近 `u64` 上限这种荒唐值时因 `Instant` 加法溢出而 panic。真到那一步，之后每个请求都会在这里 panic，爬虫任务结束。
- **要不要改**：不用改——正常配置下锁内没有会 panic 的代码。

### 全局并发信号量（expect）

```rust
pub async fn get(&self, url: &Url, accept: &str, validators: Option<&Validators>) -> Result<Fetched, FetchError> {
    self.wait_for_host(url.host_str().unwrap_or_default()).await;
    let _permit = self.global.acquire().await.expect("semaphore never closed");
```

```rust
pub async fn post_json(&self, url: &Url, body: &serde_json::Value) -> Result<serde_json::Value, FetchError> {
    self.wait_for_host(url.host_str().unwrap_or_default()).await;
    let _permit = self.global.acquire().await.expect("semaphore never closed");
```

- **为什么这样写**：tokio 的 `Semaphore::acquire` 只在信号量被 `close()` 之后返回 `Err`，这里用 `expect` 写明这个前提。
- **什么时候会出事**：不会：`global` 是私有字段，整个项目没有对它调用 `close()`（项目里唯一的 `close()` 是关闭数据库连接池）。
- **要不要改**：不用改——信号量从不关闭。

### Accept 请求头（unwrap）

```rust
let mut headers = HeaderMap::new();
headers.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
```

- **为什么这样写**：`accept` 由调用方传入，`HeaderValue::from_str` 只拒绝换行等控制字符。
- **什么时候会出事**：不会：所有调用方传的都是常量（本文件的 `ACCEPT_FEED` / `ACCEPT_HTML` / `ACCEPT_JSON` / `ACCEPT_IMAGE`，independent.rs 的 `ACCEPT_CSV`），都是可见 ASCII；`ACCEPT_FEED` 里的 `\` 续行会连同换行和缩进一起去掉。discovery.rs 的测试实际用 `ACCEPT_HTML` 和 `ACCEPT_FEED` 发过请求。
- **要不要改**：不用改——值都是常量；以后若有调用方传运行时字符串，再改成 `&'static str` 加 `HeaderValue::from_static`，或把错误转成 `FetchError`。

### 排队时，同一主机的两次请求可能挨得很近（人工）

```rust
slots.insert(host.to_string(), start + self.per_host_interval);
start
};
tokio::time::sleep_until(start).await;
}
```

- **为什么这样写**：先为主机预约一个时间（上一个预约加 10 秒），睡到那时再去拿全局并发许可（默认 4 个）。等预约时间时不占许可，别的主机的请求可以先走。
- **什么时候会出事**：间隔是按「预约的时间」算的，不是按请求真正发出的时间。一轮抓取有 64 个任务抢 4 个许可，许可要排队（tokio 的信号量先来先得）。比如同一主机的 A 预约在第 0 秒、B 预约在第 10 秒；A 排在队尾，等到第 40 秒才拿到许可，B 第 10 秒排进队、正好在 A 后面，两次请求实际只隔一两秒，甚至同时发出。一个任务里接连访问同一站点（RSS、首页、关于页、头像、全文页）也一样：上一个请求排队排晚了，下一个的预约时间早已过去，不用再等。所以 README 里说的「同一主机两次请求间隔 ≥ 10 秒」，在抓取繁忙时并不成立。共用一个主机名的博客平台会收到成串的请求，可能招来 429 和退避。这很难发现。
- **要不要改**：可以改（低）——为每个主机记下「上一次真正发出」的时间：拿到许可后再检查一次，不够间隔就放回许可，按实际时间重新预约。

## `backend/src/main.rs`

### restore 已在前面处理（unreachable!）

```rust
Command::Restore { .. } => unreachable!("handled above"),
```

- **为什么这样写**：`restore` 必须在任何东西写数据库之前执行（在 `apply_policy`、`sync_local_directories` 之前），所以在 match 之前用 `if let Some(Command::Restore { from })` 单独处理并 `return`。match 要求穷尽，这一分支只是为了编译通过。
- **什么时候会出事**：不会：走到 match 时 `cli.command` 一定不是 `Restore`，否则前面已经 return 了。两处之间没有代码改 `cli.command`。
- **要不要改**：不用改——前面的提前返回保证走不到这里。

### crawl --all 的截止时间溢出（人工）

```rust
let deadline = std::time::Instant::now() + std::time::Duration::from_secs(max_minutes * 60);
```

- **为什么这样写**：`crawl --all` 在时间预算内一轮轮地抓。`--max-minutes` 默认 90，GitHub Actions 传 100。
- **什么时候会出事**：`Instant + Duration` 溢出时即使在 release 下也会 panic（overflow when adding duration to instant）。这要 `--max-minutes` 大到约 1.5×10^17 才会发生（`max_minutes * 60` 在 release 下还会回绕）。只有手动传荒唐的值才会触发，结果是这次命令行运行失败退出。
- **要不要改**：可以改（低）——不会自然发生。要防的话改用 `max_minutes.saturating_mul(60)` 和 `Instant::now().checked_add(..)`，溢出时当作没有截止时间。

### 按 Ctrl-C 时优雅退出（let _ =）

```rust
axum::serve(listener, app)
.with_graceful_shutdown(async {
let _ = tokio::signal::ctrl_c().await;
})
```

- **为什么这样写**：在终端里按 Ctrl-C 时，等正在处理的请求结束再退出。
- **什么时候会出事**：两种情况。① `ctrl_c()` 只有在装不上信号处理（很少见的受限环境）时才返回错误，但错误被 `let _ =` 丢掉后这个 async 块立刻结束，axum 当作「该关了」：服务器启动后马上退出，日志里只有一行 listening，没有原因。② 它只等 SIGINT，而 `docker compose stop` 发的是 SIGTERM：服务器不会优雅退出，10 秒后被强行结束。SQLite 有事务保护，数据不会坏，只是正在处理的请求会断开。
- **要不要改**：可以改（低）——出错时记一条 warn 然后一直等（`std::future::pending::<()>().await`）；同时用 `tokio::signal::unix::signal(SignalKind::terminate())` 监听 SIGTERM。

## `backend/src/profile.rs`

### 编译固定的 CSS 选择器和正则（unwrap）

```rust
fn sel(s: &str) -> Selector {
    Selector::parse(s).unwrap()
}
```

```rust
static SCRIPT_SOCIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
// …
    .unwrap()
});
```

```rust
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+").unwrap());
```

- **为什么这样写**：`sel()` 的调用方传的都是字面量（`meta[content]`、`link[rel][href]`、`img`、`a[href]`、`script` 等，以及 `about_text` 里的 `fulltext::SELECTORS` 加 `.content`、`#content`、`body`）；两个正则也是常量。只有写错才会失败。
- **什么时候会出事**：不会：与页面内容无关。`from_home`、`social_from_page`、`about_text` 的测试覆盖了这些选择器和 SCRIPT_SOCIAL、EMAIL；`about_text` 的测试在 `article` 就返回了，后面的 `.content`、`#content`、`body` 测试没走到，但它们是最简单的选择器，`main` 由 fulltext.rs 的测试覆盖。
- **要不要改**：不用改——常量由测试守着。

### 主题脚本里的社交链接取捕获组（index）

```rust
for script in doc.select(&sel("script")) {
    let text: String = script.text().collect();
    for c in SCRIPT_SOCIAL.captures_iter(&text) {
        let raw = c[1].replace("\\/", "/");
```

- **为什么这样写**：第 1 组是链接本身，直接取。
- **什么时候会出事**：不会：SCRIPT_SOCIAL 的第 1 组 `(https?:(?:\\?/){2}[^"'\s]+)` 不是可选的，匹配上就有值。
- **要不要改**：不用改——捕获组是必选的。

### 单段路径是不是功能页（index）

```rust
if segs.len() == 1 && NOT_ACCOUNTS.contains(&segs[0]) {
    return None;
}
```

- **为什么这样写**：`github.com/features`、`x.com/intent` 这类单段路径不是账号页。
- **什么时候会出事**：不会：`&&` 短路，`segs.len() == 1` 成立时才取 `segs[0]`。
- **要不要改**：不用改——长度先检查过。

## `backend/src/readers.rs`

### 同步数据的 ETag 头（unwrap）

```rust
(header::ETAG, HeaderValue::from_str(&format!("\"{version}\"")).unwrap())
```

- **为什么这样写**：版本号放进 ETag，客户端下次用 `If-Match` 带回来，防止两台设备互相覆盖。
- **什么时候会出事**：不会：内容是引号加一个 i64 的十进制数（最多带个负号），总是合法的头。版本号来自数据库，或者是写入成功后的 `base + 1`。
- **要不要改**：不用改——内容只有数字和引号。

### 读者 If-Match 版本号加一（人工）

```rust
.map(|r| (r.rows_affected() == 1).then_some(base + 1))
```

- **为什么这样写**：写入成功时把新版本号返回给客户端。
- **什么时候会出事**：`base` 来自请求头 `If-Match`，公网上谁都能发 `If-Match: "9223372036854775807"`（i64 最大值）。`then_some` 的参数无论如何都会先算，`base + 1` 溢出。release 版回绕成负数，又因为 `rows_affected` 是 0 被丢掉，没有影响。debug 版（本地 `cargo run`、测试）会 panic，断开这一个请求。
- **要不要改**：可以改（低）——只影响 debug 构建。改成 `.then(|| base + 1)` 就只在写入成功（库里版本恰好等于 `base`）时才加，不会溢出。

### 订阅源代理的缓存锁（unwrap）

```rust
if let Some((at, answer)) = self.cache.lock().unwrap().get(&key)
```

```rust
let mut cache = self.cache.lock().unwrap();
```

- **为什么这样写**：`/api/fetch` 的结果缓存 15 分钟（失败的 10 分钟，或按对方的 Retry-After），订阅同一源的读者共用一次抓取。
- **什么时候会出事**：不会：持锁时只有 HashMap 查找、`Answer` 克隆（`Bytes`、`String`）、`retain`、`clear`、`insert`。`at.elapsed()` 用的是 tokio 的 `Instant`，减法是饱和的。`ttl()` 里 `clamp(60, 6 * 3600)` 的上下限是常量。真正的抓取 `self.fetch(url).await` 在两次加锁之间，不持锁。所以锁不会中毒。假如中毒，之后每个 `/api/fetch` 请求都会 panic，只影响“读者自己加的源”这个功能，进程不退。
- **要不要改**：不用改——持锁区间里没有会 panic 的代码。

### 看响应开头判断是不是订阅源（slice）

```rust
let head = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_ascii_lowercase();
```

- **为什么这样写**：只看前 4 KB，就能分出 RSS/Atom/OPML/JSON Feed、网页和其他内容。
- **什么时候会出事**：不会：切的是字节切片 `&[u8]`，没有字符边界问题。上限取了 `min(body.len())`，不会越界。切断的多字节字符由 `from_utf8_lossy` 变成替换字符。对方网站返回什么内容都一样。
- **要不要改**：不用改——字节切片，长度已取 min。

### 判断 IPv6 地址是否公网（index）

```rust
let s = v6.segments();
!(v6.is_loopback()
|| v6.is_unspecified()
|| v6.is_multicast()
|| (s[0] & 0xfe00) == 0xfc00 // unique local
|| (s[0] & 0xffc0) == 0xfe80 // link-local
|| (s[0] == 0x2001 && s[1] == 0xdb8) // documentation
|| (s[0] == 0x64 && s[1] == 0xff9b)) // NAT64
```

- **为什么这样写**：防止借代理访问内网（SSRF）。唯一本地、链路本地、文档和 NAT64 地址段要看前一两段来判断。
- **什么时候会出事**：不会：`segments()` 返回固定长度的 `[u16; 8]` 数组，下标 0、1 是常量，编译期就能确定不越界。
- **要不要改**：不用改——定长数组的常量下标。

### 统计事件一次最多收 200 条（slice）

```rust
let list = &e.events[..e.events.len().min(200)];
```

- **为什么这样写**：一批事件只记前 200 条，免得一个请求写太多。
- **什么时候会出事**：不会：上限取了 `min(len)`，Vec 切片不会越界。请求体整体还受 `DefaultBodyLimit`（1.9 MB）限制。
- **要不要改**：不用改——长度已取 min。

### 订阅源代理的缓存按条数算，满了就全部清掉（人工）

```rust
const MAX_FEED_BYTES: usize = 4 * 1024 * 1024;
const FEED_TTL: Duration = Duration::from_secs(15 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHED_FEEDS: usize = 256;
```

```rust
if cache.len() >= CACHED_FEEDS {
cache.retain(|_, (at, a)| at.elapsed() < a.ttl());
if cache.len() >= CACHED_FEEDS {
cache.clear();
}
}
```

- **为什么这样写**：同一个源一刻钟内只替所有读者抓一次。最多存 256 条，满了先去掉过期的，还是满就整个清空，代码最简单。
- **什么时候会出事**：有两个问题。一是内存只按条数限制，每条里存着整个 feed 正文（最大 4 MB），256 条最多约 1 GB。`/api/fetch` 不需要登录，谁都能用 256 个不同的地址把它填满（同一个大文件加上不同的查询参数就行）；同时进行的抓取也没有数量上限，每个都在内存里攒最多 4 MB。小内存的 VPS 可能因此被 OOM 杀掉。二是读者多了以后，一刻钟内被请求的源超过 256 个，几乎每次插入都会触发 `clear()`，「大家共用一次抓取」就不成立了：对方网站和本站的流量都会翻倍，但从外面看不出来。
- **要不要改**：可以改（低）——按总字节数限制（比如 64 MB），满了淘汰最旧的几条，而不是全部清空；再用一个信号量限制同时进行的抓取数（比如 8 个）。

## `backend/src/rich.rs`

### 编译固定的正则表达式（unwrap）

```rust
static BARE_LINKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)https?://\S+|\S+\.(?:jpe?g|png|webp|gif|avif|svg)\b").unwrap());
```

```rust
static TEMPLATE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{%.*?%\}").unwrap());
```

```rust
static ESCAPED_HTML: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)</(?:p|h[1-6]|li|ol|ul|a|strong|em|b|blockquote|span|figure|figcaption|table|section|article)>|<br\s*/?>",
    )
    .unwrap()
});
```

- **为什么这样写**：常量模式，`LazyLock` 里编译一次，只有写错才会失败。
- **什么时候会出事**：不会：与 feed 内容无关。`excerpt_html` 的测试覆盖三个：纯文本输入（`"Plain text, no structure."`）走到 ESCAPED_HTML 和 TEMPLATE_TAG，任何有文字的输入都经过 `Writer::text` 里的 BARE_LINKS。
- **要不要改**：不用改——常量由测试守着。

### 关闭标签时弹栈（unwrap）

```rust
fn pop_to(&mut self, i: usize) {
    while self.stack.len() > i {
        let o = self.stack.pop().unwrap();
```

- **为什么这样写**：把下标 `i` 及以上的开标签都弹出并写闭合标签。
- **什么时候会出事**：不会：循环条件 `len > i ≥ 0` 保证栈非空，`pop()` 一定是 `Some`。
- **要不要改**：不用改——循环条件保证。

### 字数用完时在最后一个字后面插入省略号（insert）

```rust
if self.left < 1 + usize::from(gap) {
    self.out.insert(self.text_end, '…');
    self.done = true;
    return;
}
```

- **为什么这样写**：省略号要紧跟最后一个显示的字，放在已经写出的闭合标签（如 `</strong>`）前面，所以记下最后一个字的结束位置 `text_end`，在那里插入。
- **什么时候会出事**：不会：`text_end` 只在写入一个字符之后被设为 `self.out.len()`（初值 0），而 `out` 只会在末尾追加（`push` / `push_str`），从不截断或改中间，所以 `text_end ≤ out.len()` 且始终在字符边界上，中文也一样。插入之后 `done = true`，`text` / `open_*` / `close` 都直接返回，不会再插第二次。
- **要不要改**：不用改——不变式成立。注意 `excerpt_html` 在 `feed::parse` 里、爬虫任务上运行，这里一旦 panic 会让爬虫停掉；以后若加了修改 `out` 中间内容的代码，要同步更新 `text_end`。

### 找最内层的块（index）

```rust
let block = self.stack.iter().rposition(|o| !is_inline(o.tag));
if block.is_some_and(|i| holds_text(self.stack[i].tag)) {
```

- **为什么这样写**：判断最内层的块能不能直接放文字。
- **什么时候会出事**：不会：`i` 是同一个 `stack` 上 `rposition` 的结果，中间 `stack` 没有变，一定小于长度。
- **要不要改**：不用改——下标来自同一个栈。

### 在栈中间插入段落（人工）

```rust
let at = block.map_or(0, |i| i + 1);
self.stack.insert(
    at,
```

- **为什么这样写**：文字需要一个能放文字的块，就在最内层块的上面（还没写出的行内标签下面）补一个 `<p>`。
- **什么时候会出事**：不会：`Vec::insert` 在下标大于长度时 panic，这里 `at` 是 0 或 `i + 1`，`i` 是上一行 `rposition` 给出的有效下标，所以 `at ≤ len`。
- **要不要改**：不用改——下标不会超过长度。

## `backend/src/seo.rs`

### 替换模板里的标题（slice）

```rust
(Some(a), Some(b)) if a < b => format!("{}<title>{}</title>{}", &html[..a], esc(title), &html[b + 8..]),
```

- **为什么这样写**：把构建出来的 index.html 里的 `<title>…</title>` 换成每页自己的标题。
- **什么时候会出事**：不会：`a`、`b` 都是在同一个 `html` 上 `find` 出来的位置，`b + 8` 正好是 `</title>`（8 个 ASCII 字节）的结尾，都在字符边界上，也不越界。顺序反了（`a < b` 不成立）时走另一个分支。模板是我们自己构建的 index.html，不是外部输入。
- **要不要改**：不用改——下标来自同一字符串上的查找。

### 删掉模板自带的 description（slice）

```rust
let Some(len) = html[start..].find('>') else {
return html.to_string();
};
format!(
"{}{}",
&html[..start],
html[start + len + 1..].trim_start_matches(['\n', ' '])
```

- **为什么这样写**：模板里原有的 `<meta name="description" …>` 要去掉，每页换成自己的描述。
- **什么时候会出事**：不会：`start` 来自 `html.find`，`len` 来自 `html[start..].find('>')`，`start + len` 是 `>` 的位置。`>` 是单字节，`+ 1` 仍在字符边界上，且不超过长度。找不到 `>` 时直接返回原文。
- **要不要改**：不用改——下标来自同一字符串上的查找。

### 从 JSON 里取字符串字段（index）

```rust
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
v[key].as_str().unwrap_or_default()
```

- **为什么这样写**：页面要的标题、链接、摘要都从 API 的 JSON 里取，缺了就当空字符串。
- **什么时候会出事**：不会：`v` 是 `serde_json::Value`。只读的字符串下标在键不存在、或 `v` 不是对象时返回 `Null`，不会 panic。
- **要不要改**：不用改——`Value` 的只读下标不会 panic。

## `backend/src/sources/manual.rs`

### 编译固定的正则表达式（unwrap）

```rust
static OUTLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<outline\b([^>]*)>").unwrap());
static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"([\w:]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
```

- **为什么这样写**：OPML 只需要读 `<outline>` 的几个属性，用两个常量正则即可，不用引入 XML 解析器。
- **什么时候会出事**：不会：与 OPML 内容无关，`opml_outlines_become_feeds` 测试会编译两个正则。
- **要不要改**：不用改——常量由测试守着。

### 属性名捕获组（index）

```rust
ATTR.captures_iter(attrs)
    .find(|a| a[1].eq_ignore_ascii_case(name))
    .and_then(|a| a.get(2).or(a.get(3)))
```

- **为什么这样写**：第 1 组是属性名；属性值在第 2 组（双引号）或第 3 组（单引号），这两个是可选的，所以用 `get`。
- **什么时候会出事**：不会：第 1 组 `([\w:]+)` 不是可选的，匹配上就有值。
- **要不要改**：不用改——必选组用下标，可选组用 `get`，写法正确。

## `backend/src/sources/travellings.rs`

### 开往返回空列表或大批异常状态时，博客整批下线（人工）

```rust
#[serde(default)]
data: Vec<Member>,
```

```rust
let alive = self.cfg.live_statuses.iter().any(|s| s.eq_ignore_ascii_case(&status));
```

- **为什么这样写**：开往的接口返回全部成员和开往每天检测的状态，只有 RUN 和 LOST 算活着。`data` 缺了就当空列表。
- **什么时候会出事**：`save_directory` 会把这次列表里没有的成员标成不活跃，`alive` 为假的也一样；只被开往收录的博客随之从网站上消失、不再抓取。所以只要接口「成功」地返回了不完整的数据，就会整批下线。可能的情况有：接口返回 `{"success": true, "data": []}`（维护中、数据库出错）；字段改了名，`data` 换成别的名字，`serde(default)` 让它变成空列表而不是报错；开往自己的检测服务出了问题，一天里大批成员被标成 TIMEOUT 或 ERROR。这几种都不会转去试备用地址，也不报错。目录一天才同步一次，至少要到第二天才恢复；如果是字段改名，就一直不会恢复。中文独立博客列表（independent.rs）已经对空列表报错，这里没有。只能从网站上博客数突然变少看出来。
- **要不要改**：建议改（高）——外面一个服务出点小问题，就能让网站少掉四成博客（约 1,200 个只在开往里的），而且没人报警。和 independent.rs 一样，列表为空时报错（这样会转去试备用地址）。再加一个类似 backup.rs `check_shrink` 的检查：这次活着的成员比上次少了一大截（比如一半）时，不保存这次的结果，记一条 warn，等下一次同步。

## `backend/src/stats.rs`

### 「今天」按 UTC 算（人工）

```rust
pub fn today() -> String {
chrono::Utc::now().format("%Y-%m-%d").to_string()
}
```

- **为什么这样写**：服务器、Worker（`dayOf`）和浏览器（telemetry.ts 里的 `toISOString().slice(0, 10)`）都用 UTC 日期，按天计数的口径一致，不受服务器所在时区影响。
- **什么时候会出事**：读者几乎都在 UTC+8。「大家在看」页上的「今天 N 位访客」，在北京时间早上 8 点清零，凌晨 0 点到 8 点的访客都算在前一天。浏览器每天报一次「来过」也按 UTC 日期，早上 8 点前后各访问一次就算两天。静态版更明显：数字是导出那一刻的，页面可能几小时后才有人看，跨过 UTC 零点后显示的「今天」其实是前一天。只是显示上不准，不影响热度。
- **要不要改**：可以改（低）——服务器、Worker、浏览器三处统一用一个固定时区（例如 Asia/Shanghai，可以在配置里改）；或者把文案改成「过去 24 小时」，按时间戳算。

### 统计只增不减，每次合并备份都把所有日子重新导入（人工）

```rust
pub async fn import(pool: &SqlitePool, dir: &Path) -> anyhow::Result<usize> {
let stats = dir.join("stats");
let Ok(read) = std::fs::read_dir(&stats) else {
return Ok(0);
};
```

- **为什么这样写**：备份仓库里每天一个 `stats/<day>.json`（Worker 每小时提交一次）。合并备份时一律以文件为准：先删掉这一天，再插入，简单，也不会重复计数。
- **什么时候会出事**：`restore` 每次都会调用它。Actions 每次抓取运行要合并两次，每次发布再合并一次，`serve` 每次定时备份也合并一次。文件每天多一个、从不删除，每次都要把所有日子读一遍，逐行删除再插入，耗时随天数线性增长。数据库里的 `stats` 表也从不清理，而 `refresh_heat` 的条件 `day >= ?` 不在主键的开头，每小时都要扫一遍整张表。热度只用最近 30 天（`HEAT_DAYS`），更早的日子每次都白导入一遍。几年后每次合并要多花几十秒，表现为 Actions 运行越来越慢，没人会专门注意。
- **要不要改**：可以改（低）——只导入最近 `HEAT_DAYS` 天的文件和数据库里还没有的日子，或者记下每个文件的哈希，没变就跳过；给 `stats` 表加一个 `(day)` 索引。

## `backend/src/status.rs`

### 去掉错误信息里的网址（unwrap）

```rust
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s)]+").unwrap());
```

- **为什么这样写**：写死的常量正则。同一种错误发生在不同网站上时，去掉网址才能合在一起数。
- **什么时候会出事**：不会：正则是合法的，`status` 的单元测试每次都会用到它；而且只在 `feed-server status` 命令里用。
- **要不要改**：不用改。

## `backend/src/util.rs`

### 按字符数截断（slice）

```rust
pub fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((idx, _)) => format!("{}…", s[..idx].trim_end()),
```

- **为什么这样写**：按字符（不是字节）截断标题、摘要、简介，中文不会被切坏。
- **什么时候会出事**：不会：`idx` 是 `char_indices` 给出的第 `max` 个字符的起点，一定在字符边界上。`truncate_counts_chars` 测试用中文覆盖了这一点。
- **要不要改**：不用改——下标来自 `char_indices`。

### 除数是变量的取余（人工）

```rust
pub fn random_between(lo: u64, hi: u64) -> u64 {
    let r = RandomState::new().hash_one(now());
    lo + r % (hi.saturating_sub(lo) + 1)
}
```

```rust
let r = RandomState::new().hash_one((salt, now())) as i64;
secs + r.rem_euclid(2 * span + 1) - span
```

- **为什么这样写**：`random_between` 在区间里取一个随机数（备份推送冲突后的等待时间）；`jitter` 给间隔加 ±percent% 的抖动，免得一起加入的 feed 总在同一时刻被抓。
- **什么时候会出事**：不会：`%` / `rem_euclid` 只在除数为 0（或 `MIN % -1`）时 panic，release 也一样。`random_between` 的除数只有 `lo = 0`、`hi = u64::MAX` 时才会回绕成 0，唯一调用方 backup.rs 传的是 `lo * 60` / `hi * 60`，乘积是偶数，不可能等于 `u64::MAX`。`jitter` 前面已确认 `span > 0`，`2 * span + 1` 是奇数，不会是 0；`span ≤ i64::MAX / 100`，也不会回绕成 -1。另外，`secs * percent` 在 `secs` 极大时会溢出：`secs` 可以来自服务器的 `Retry-After`（crawler.rs 的 `backoff` 只把它当下限，不设上限），release 下溢出静默回绕，只会算出离谱的下次抓取时间，不会 panic。
- **要不要改**：不用改——除数不会为 0。

## `frontend/src/api.ts`

### 拼出服务器接口的地址（new URL）

```ts
async function get<T>(path: string, params: Params = {}): Promise<T> {
  const url = new URL(path, location.origin)
  for (const [k, v] of Object.entries(params)) {
    if (v !== null && v !== undefined && v !== '') url.searchParams.set(k, String(v))
  }
```

- **为什么这样写**：查询参数要用 `searchParams` 逐个加（自动转义，空值不发），而 `URL` 需要绝对地址，所以以当前页面的 `location.origin` 为基准。`get` 只在服务器版用，静态版走 `staticApi`。
- **什么时候会出事**：不会：`path` 全是代码里写死的 `/api/...`，最多拼进数字 id 或路由里的博客 key；以 `/` 开头的相对路径配合任何字符串都能解析（特殊字符被转义）。基准是浏览器给的 origin，页面从 http(s) 打开时总是有效。只有 origin 为 `"null"`（如 Firefox 从 file:// 打开）才会抛 TypeError，那时本来也连不上服务器；而且抛在 async 函数里，会变成 rejection，由调用方当作加载失败处理（见下一条）。
- **要不要改**：不用改——不会在实际部署里失败。

### 请求失败或回来的不是 JSON（throw、.json()）

```ts
  const res = await fetch(url)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return (await res.json()) as T
```

- **为什么这样写**：`get` 是服务器版所有接口的出口。`fetch` 对 4xx/5xx 不报错，所以手动把非 2xx 变成 rejection，调用方只需要一个 catch 就能处理所有失败。
- **什么时候会出事**：服务器出错、重启中、反向代理回 502、博客或文章已删（404）时在 throw 那行拒绝。回 200 但正文不是 JSON 时 `res.json()` 拒绝：服务器对不存在的 `/api/...` 路径会走 SPA 兜底，以 200 返回 index.html（main.rs 的 `fallback_service`），所以前端已更新、服务进程还是旧的（第 104 行注释说的情况）时调用新接口（如 `/api/trending`）就会走到这里。所有调用方都接住了：Waterfall 显示「加载失败，点此重试」，BlogView 显示找不到，PostDetail 显示「详情加载失败」或「找不到这篇文章」，TrendingView 显示失败，SearchView、SettingsView、MyBlog、MeView 各自 catch，AppHeader 用 `Promise.allSettled`，`loadMeta`/`loadTags` 静默 catch，`blogDetail` 失败时从缓存里删掉以便重试。错误消息本身没人显示，也没人按 `'HTTP 404'` 区分。
- **要不要改**：不用改——抛出就是约定好的失败通道，每个调用方都处理了。

### 读者自己的订阅源：文章详情和博客资料（non-null !）

```ts
  post: (id) => (id < 0 ? (ownDetail(id) ? Promise.resolve(ownDetail(id)!) : notFound()) : siteApi.post(id)),
  related: (id, limit) => (id < 0 ? Promise.resolve([]) : siteApi.related(id, limit)),
  blog: (id) => (id < 0 ? (ownBlog(id) ? Promise.resolve(ownBlog(id)!) : notFound()) : siteApi.blog(id)),
```

- **为什么这样写**：负数 id 是读者自己加的源（sources.ts），只在本机有。第一次调用判断有没有，第二次取值；TypeScript 不知道两次结果相同，所以加 `!`。
- **什么时候会出事**：不会：`ownDetail` / `ownBlog` 是对内存里 `ownFeeds`、`reads` 的同步查找，两次调用在同一个同步表达式里，中间没有任何代码能改这些数据，结果一定相同。找不到时走 `notFound()`（拒绝 `'HTTP 404'`），调用方（PostDetail、BlogView、MyBlog、`blogDetail`）都 catch。
- **要不要改**：不用改——想去掉 `!` 可以先存进变量：`const d = ownDetail(id); return d ? Promise.resolve(d) : notFound()`，还能少建一次对象，但与稳定性无关。

### 博客资料缓存超过 200 个时丢掉最早的（non-null !）

```ts
    p.catch(() => blogCache.delete(id))
    blogCache.set(id, p)
    if (blogCache.size > 200) blogCache.delete(blogCache.keys().next().value!)
```

- **为什么这样写**：Map 按插入顺序迭代，第一个键就是最早放进去的；`next().value` 的类型含 undefined（迭代器可能已结束），所以加 `!`。
- **什么时候会出事**：不会：只有 `size > 200` 时才取，Map 非空，第一个键一定存在。
- **要不要改**：不用改。

### 分类的显示名「技术 › AI」（non-null !）

```ts
  const parent = tagsInfo.value?.categories.find((x) => x.id === top)
  if (!sub) return parent?.label ?? c
  const child = parent?.children?.find((x) => x.id === c)
  return child ? `${parent!.label} › ${child.label}` : c
```

- **为什么这样写**：`child` 是从 `parent?.children` 里找到的，找到了就说明 `parent` 存在；TypeScript 推不出这层关系。
- **什么时候会出事**：不会：`parent` 为 undefined 时 `child` 也是 undefined，走 `: c` 分支。这个函数在渲染时调用（HomeView、AppHeader、TrendingView、SettingsView 的模板），抛错会让组件渲染失败——但它不会抛。
- **要不要改**：不用改。

### 网站信息和标签信息读不到时不提示（.catch 忽略）

```ts
.then((m) => {
meta.value = m
document.title = m.site_name
})
.catch(() => {})
```

```ts
.then((t) => (tagsInfo.value = upgradeTags(t)))
.catch(() => {})
```

- **为什么这样写**：`loadMeta()`、`loadTags()` 在应用启动时调用，结果只用来补充页面：网站名、目录列表、标签的中文名。拿不到时页面用默认值照常显示，所以不打扰读者。
- **什么时候会出事**：服务器暂时连不上，或者静态网站的数据文件缺失时，标题保持默认，设置里「本站的来源」是空的，标签显示原文。这次访问里不会再试，刷新页面才再试；原因也不会进「诊断信息」。
- **要不要改**：可以改（低）——改成 `.catch((e) => noteProblem('读网站信息', e))`，出问题时在诊断信息里看得到原因。

## `frontend/src/backup.ts`

### 导出备份时复制当前设置（JSON.parse）

```ts
    channels: JSON.parse(JSON.stringify(prefs.value)),
    settings: { ...settings.value },
    options: JSON.parse(JSON.stringify(options.value)),
    sources: JSON.parse(JSON.stringify(ownSources.value)),
```

- **为什么这样写**：深拷贝：把 Vue 的响应式对象变成普通数据，备份对象不和正在用的设置共享嵌套的数组和对象。
- **什么时候会出事**：不会：三个值都是经过 `cleanPrefs` / `cleanOptions` / `cleanSources` 整理的普通 JSON 数据（字符串、数字、布尔、数组、对象），没有循环引用、BigInt 或函数；也永远不是 undefined（那样 `JSON.stringify` 会返回 undefined，`JSON.parse` 才会抛）。`makeBackup` 只在「下载备份文件」按钮（MeView）里调用。
- **要不要改**：不用改。

### 读入读者选的备份文件（JSON.parse、throw）

```ts
export function parseBackup(text: string): Backup {
  let v: Record<string, unknown>
  try {
    v = JSON.parse(text)
  } catch {
    throw new Error('不是有效的 JSON 文件')
  }
  if (v?.format !== FORMAT) throw new Error('这不是本站导出的备份文件')
  if (typeof v.version !== 'number' || v.version > VERSION) throw new Error('备份来自更新的版本，请先刷新页面')
```

- **为什么这样写**：文件是读者自己选的，内容什么都可能是。把 JSON 语法错误换成读得懂的中文消息，再核对 `format` 和 `version`，不对就整体拒绝，而不是把别的 JSON 当备份恢复、覆盖读者的数据。之后的字段逐个用 `clean*` 清洗，不合格的丢掉。
- **什么时候会出事**：选错文件（图片、别的 JSON；`null`、数字、数组也会在 `format` 那行被拒）、文件被截断、或者用没刷新的旧页面导入新版本导出的备份。唯一调用方 MeView 的 `pickFile` 用 try/catch 接住，把 `err.message` 显示在恢复区（`restoreError`），本机数据不动。即使后面的清洗意外抛错，也会被同一个 catch 接住。
- **要不要改**：不用改——抛出带消息的错误就是设计好的处理方式。

## `frontend/src/channels.ts`

### 读写频道栏设置（JSON.parse、localStorage）

```ts
function load(): Prefs {
  try {
    return cleanPrefs(JSON.parse(localStorage.getItem(KEY) ?? 'null'))
  } catch {
    return cleanPrefs(null)
  }
}

export const prefs = ref<Prefs>(load())

watch(
  prefs,
  (p) => {
    try {
      localStorage.setItem(KEY, JSON.stringify(p))
    } catch (e) {
      // Applies to this visit only.
      noteProblem('保存频道设置', e)
    }
  },
  { deep: true },
)
```

- **为什么这样写**：`prefs` 是模块顶层的 `ref(load())`，应用启动导入模块时就运行（main.ts 先导入 sync.ts，它导入 channels.ts）。这里一旦抛错整个应用起不来（白屏），所以读取、解析、清洗全部包在 try 里，失败用默认频道栏；写入同样包住。另一个标签页改了设置时（`storage` 事件）也走 `load()`。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：存储被禁用（隐私设置、沙箱 iframe；Firefox 关掉 DOM 存储时 `localStorage` 是 null）时 `getItem`/`setItem` 抛错；值被手改或扩展写坏时 `JSON.parse` 抛错；存储满时 `setItem` 抛 QuotaExceededError。全部被 catch：读失败 → 默认频道栏；写失败 → 本次访问照常生效，刷新后回到上次存下的设置。不会白屏。
- **要不要改**：不用改。

## `frontend/src/diagnostics.ts`

### 读写问题记录（JSON.parse、localStorage、catch {}）

```ts
function load(): Problem[] {
try {
const v = JSON.parse(localStorage.getItem(KEY) ?? '[]')
return Array.isArray(v) ? v.filter((p) => typeof p?.message === 'string').slice(0, MAX) : []
} catch {
return []
}
}
```

```ts
function save() {
try {
localStorage.setItem(KEY, JSON.stringify(problems.value))
} catch {
// Storage is the likely problem itself; kept for this visit.
}
}
```

- **为什么这样写**：问题记录在模块加载时读取（main.ts 第一个导入的就是它），读不出来必须不抛，否则整个应用白屏。写不进去时不能再记一条问题，否则会一直循环下去，只能留在内存里。
- **什么时候会出事**：存储被禁用或写满时，记录只保留到关闭页面；页面上照常显示，「复制全部」照样能用。
- **要不要改**：不用改。

## `frontend/src/feedParse.ts`

### 源读不出来时报错（throw、JSON.parse）

```ts
const again = parser.parseFromString(fixed, 'application/xml')
if (broken(again)) throw new Error('格式有误，读不出来')
```

```ts
try {
json = JSON.parse(source)
} catch {
throw new Error('格式有误，读不出来')
}
// …
throw new Error('不是 RSS、Atom 或 OPML')
```

```ts
const parsed = parseSource(source, location.href)
if (!('list' in parsed)) throw new Error('这个文件不是 OPML 订阅列表')
```

- **为什么这样写**：parseSource 是浏览器里解析源的唯一入口。读不出来时抛一个带中文说明的 Error，由调用方决定怎么显示；JSON.parse 的 SyntaxError 也被换成同一句中文。parseXml 先按原文解析，失败后修掉常见毛病（裸 &、HTML 实体、控制字符）再试一次，还不行才抛。parseOpmlFile 只接受 OPML。
- **什么时候会出事**：地址返回的 XML/JSON 坏了（标签没闭合、内容被截断），或者根本不是 RSS/Atom/OPML（比如 sitemap.xml），或者读者导入文件时选了一个 RSS 而不是 OPML。所有调用方都接住了：readFeed/readList 把消息记进 reads，设置页「我添加的」显示「上次读取失败：格式有误，读不出来」，这个源的页面（BlogView）上也有；lookUp/fromFile 的错误由 SettingsView 的 add() 显示在输入框下面；readNow 在 BlogView 里有 `.catch(() => [])`。JSON Feed 的字段类型不对（比如 items 不是数组）会在后面抛 TypeError，也被同样接住，只是提示变成浏览器的英文报错。
- **要不要改**：不用改——抛错本来就是要的处理方式，每个调用方都接住了，并把消息给读者看。

### 把源里的地址变成绝对地址（new URL）

```ts
function absolute(raw: string | null | undefined, base: string): string {
if (!raw) return ''
try {
const u = new URL(raw.trim(), base)
return u.protocol === 'http:' || u.protocol === 'https:' ? u.href : ''
} catch {
return ''
}
```

- **为什么这样写**：源里的文章链接、图片、主页地址常常是相对的，或者干脆写错了，所以全部经过这里转换。解析失败的、不是 http(s) 的（javascript:、data: 等）都当作没有。
- **什么时候会出事**：不会：放在 try/catch 里，坏地址返回 ''。
- **要不要改**：不用改——已经接住。

### 源没写主页时用它所在网站的根（new URL）

```ts
const homepage = absolute(text(kid(channel, 'link')) || atomLink?.getAttribute('href'), base) || new URL(base).origin
```

```ts
const homepage = absolute(alternate(feed), base) || new URL(base).origin
```

```ts
const homepage = absolute(f.home_page_url, base) || new URL(base).origin
```

- **为什么这样写**：RSS、Atom、JSON Feed 都可能不写主页，或者写坏了，这时用源所在网站的根作为主页。base 是取到这个源的地址，按约定一定是合法的绝对地址。
- **什么时候会出事**：不会。base 只有这几种来源：fetchRemote 返回的 `x-final-url`（服务器的 reqwest 或 Worker 的 fetch 跟完跳转后的地址，由 URL 类型序列化，一定合法）；没有这个头时，是请求用的那个地址，而服务器刚用同样的 WHATWG 规则解析过它并取回了内容；lookUp 还先用 `new URL(url).href` 规范化过；parseOpmlFile 传的是 location.href。万一真抛了，也在 parseSource 里面，会被上一条说的调用方接住，只影响这一个源。
- **要不要改**：不用改——base 在进来之前已经被解析过了。

### OPML 里没写标题的订阅用域名代替（new URL）

```ts
if (!url || seen.has(url)) continue
seen.add(url)
feeds.push({
url,
title: o.getAttribute('title') || o.getAttribute('text') || new URL(url).hostname,
```

- **为什么这样写**：OPML 的 outline 不一定有 title/text，用域名作为显示名。
- **什么时候会出事**：不会：url 是 absolute() 的返回值，要么是 ''（上面已经 continue 跳过），要么是刚刚由 new URL 成功解析出来的 http(s) href，再解析一次一定成功。
- **要不要改**：不用改——前一步已经保证地址合法。

## `frontend/src/format.ts`

### 链接显示成域名（new URL）

```ts
export function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, '')
  } catch {
    return url
  }
}
```

- **为什么这样写**：卡片、博客页、搜索建议、大家在看等处把主页地址显示成域名；地址来自服务器数据或读者输入（MyBlog 的搜索框），不一定是合法的绝对 URL。
- **什么时候会出事**：不会出事：缺协议（`example.com/blog`）或空串时 `new URL` 抛 TypeError，被 catch，原样返回字符串。它在模板里调用，不抛才能保证组件照常渲染——确实不抛。
- **要不要改**：不用改。

## `frontend/src/library.ts`

### 启动时读出喜欢、收藏、历史、刷到过（JSON.parse、localStorage）

```ts
function load(name: ListName): Entry[] {
  try {
    return cleanEntries(JSON.parse(localStorage.getItem(key(name)) ?? '[]')).slice(0, LIMITS[name])
  } catch {
    return []
  }
}
```

```ts
const lists: Record<ListName, Ref<Entry[]>> = {
  likes: ref(load('likes')),
```

- **为什么这样写**：`lists` 是模块顶层对象，启动时（sync.ts 导入 library.ts）就调用 `load`，抛错会白屏，所以整段包在 try 里，失败当作空列表。`cleanEntries` 逐条丢掉不合格的项，坏一条不影响其他条。另一个标签页写入时（`storage` 事件）也重新 `load`。
- **什么时候会出事**：存储被禁用 → 四张列表为空，本次访问照常能用；值被手改或扩展写坏、`JSON.parse` 失败 → 这张列表显示为空，下次写入会用新列表覆盖坏值（`setItem` 是整值写入，正常使用不会写出半截 JSON，坏值本来也读不回来）。不会白屏。
- **要不要改**：不用改。

### 写回列表，存储满时腾地方（localStorage）

```ts
function store(name: ListName, entries: Entry[]) {
  try {
    localStorage.setItem(key(name), JSON.stringify(entries))
  } catch (failed) {
    // Storage full or blocked. Make room by shortening the least precious
    // lists (刷到过, then the history), once.
    for (const [victim, keep] of [['seen', 200], ['history', 50]] as const) {
      if (name === victim || lists[victim].value.length <= keep) continue
      lists[victim].value = lists[victim].value.slice(0, keep)
      try {
        localStorage.setItem(key(victim), JSON.stringify(lists[victim].value))
        localStorage.setItem(key(name), JSON.stringify(entries))
        return
      } catch {
        // Try the next one.
      }
    }
    // Kept for this visit only.
    noteProblem(`保存${LIST_LABELS[name]}`, failed)
  }
```

- **为什么这样写**：每次喜欢、收藏、打开、刷到过之后（`put`、`remove`、`clear`、`flushShown`、`restoreList`，都在点击或定时器里）把整张列表写回。`setItem` 在存储满（QuotaExceededError）或被禁用时会抛；这里全部接住，先把最不重要的刷到过缩到 200 条、再把历史缩到 50 条腾出空间重试，都不行就只留在内存里。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：不会抛到外面，但存储满并不算稀奇：四张列表满额（收藏 2000、喜欢 1000、历史 500、刷到过 1000 条，每条带一份卡片快照，几百到一千多字符）就有约 4M 字符，而浏览器给每个网站的 localStorage 通常只有 5 MB 左右，同一份配额还要装读者自己订阅源的缓存（sources.ts 的 `sources:read`，最多 500 个源 × 20 篇）。满了之后有两个问题：① 写的是刷到过本身时，`name === victim` 跳过了 `seen`，转而把历史永久砍到 50 条，和注释说的「先牺牲最不重要的刷到过」相反——滚动时每 2 秒一次的 `flushShown` 就能把历史砍掉；② 两轮都失败时，新的喜欢和收藏只在内存里，刷新后消失；读者不会被提醒，只有打开「设置 → 诊断信息」才看得到「保存喜欢：QuotaExceededError」这样的记录。（存储被禁用时每次都走到这里，第一次写就把内存里的刷到过和历史截短，但那时本来就只在本次访问有效，影响很小。）
- **要不要改**：建议改（中）——① `name` 是 `'seen'` 时先用更短的 `entries`（如前 200 条）重试，不去动历史；② 腾空间时先让可以重新读取的订阅源缓存（`sources:read`）让位，比如由 sources.ts 提供一个清缓存的函数在这里先调用；③ 仍然写不进去时设一个响应式标记（如 `storageFull`），在「我的」页提示「浏览器存储已满，新的喜欢/收藏不会保存，请下载备份并清理刷到过」。

### 历史和刷到过的记录开关（JSON.parse、localStorage）

```ts
function loadSettings(): Settings {
  try {
    const v = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? '{}')
    return { history: v?.history !== false, seen: v?.seen !== false }
  } catch {
    return { history: true, seen: true }
  }
}

const settingsRef = ref<Settings>(loadSettings())
// …
try {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settingsRef.value))
} catch (e) {
  // Applies to this visit only.
  noteProblem('保存历史记录的设置', e)
}
```

- **为什么这样写**：`settingsRef` 也在模块顶层初始化，读失败必须不抛，默认两个开关都打开；`v?.` 让存了 `null` 时也能用默认值。写入在 `setSettings` 里（MeView 的开关、恢复备份、同步），先改内存再写存储，写不进去只影响以后的访问。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：存储被禁用或值被写坏 → 两个开关回到默认的「开」；存储满时写失败 → 本次访问按新设置，刷新后回到旧设置。不会白屏，也不会打断点击。
- **要不要改**：不用改。

## `frontend/src/options.ts`

### 读写设置页的选项（JSON.parse、localStorage）

```ts
function load(): Options {
  try {
    return cleanOptions(JSON.parse(localStorage.getItem(KEY) ?? 'null'))
  } catch {
    return cleanOptions(null)
  }
}

export const options = ref<Options>(load())

watch(
  options,
  (o) => {
    try {
      localStorage.setItem(KEY, JSON.stringify(o))
    } catch (e) {
      // Applies to this visit only.
      noteProblem('保存设置', e)
    }
  },
  { deep: true },
)
```

- **为什么这样写**：`options` 是模块顶层的 `ref(load())`，api.ts、telemetry.ts、sync.ts 在启动时都导入它，抛错就白屏；所以读、解析、`cleanOptions` 清洗都在 try 里，失败用默认设置。写入在深度 watch 里，同样包住。`storage` 事件也走 `load()`。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：存储被禁用或值被写坏 → 默认设置（权重、屏蔽、字号等回到默认）；存储满时写失败 → 本次访问按新设置，刷新后回到上次存下的。都被 catch，不会白屏。
- **要不要改**：不用改。

## `frontend/src/positions.ts`

### 读写各信息流读到的位置（JSON.parse、localStorage）

```ts
function load(): Record<string, Position> {
  try {
    return cleanPositions(JSON.parse(localStorage.getItem(KEY) ?? '{}'))
  } catch {
    return {}
  }
}

let positions = load()
// …
positions = Object.fromEntries(keep)
try {
  localStorage.setItem(KEY, JSON.stringify(positions))
} catch (e) {
  // Kept for this visit only.
  noteProblem('保存读到哪里', e)
}
```

- **为什么这样写**：`positions` 在模块顶层加载（sync.ts 启动时导入），读失败当作没有记录，免得白屏；`save` 在 `notePosition` 之后 1.5 秒的定时器里和 `pagehide` 时运行，写失败放弃，内存里的位置照用。最多 50 个信息流，数据很小。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：存储被禁用或值被写坏 → 「从上次读到的地方继续」和「新文章」标记这次没有；存储满时写失败 → 下次访问没有这次的位置。都被 catch，不会白屏，也不会打断定时器。
- **要不要改**：不用改。

### 给备份和同步一份位置的副本（JSON.parse）

```ts
export function allPositions(): Record<string, Position> {
  return JSON.parse(JSON.stringify(positions))
}
```

- **为什么这样写**：返回深拷贝，调用方（`makeBackup`、sync.ts 上传和比较）改不到模块里的 `positions`。
- **什么时候会出事**：不会：`positions` 总是一个对象，里面只有数字和字符串——来自 `cleanPositions`（存储、备份、同步的输入都经过它）或 `notePosition` 写入的 `sort_at`/`id`/`title`/`at`，没有循环引用和 BigInt。
- **要不要改**：不用改。

## `frontend/src/reader.ts`

### 阅读栏占窗口多宽（localStorage）

```ts
function loadShare(): number {
  try {
    const v = Number(localStorage.getItem(SHARE_KEY))
    return v >= 0.3 && v <= 0.7 ? v : 0.5
  } catch {
    return 0.5
  }
}

/** Share of the window's width the pane takes. */
export const readerShare = ref(loadShare())
// …
if (!save) return
try {
  localStorage.setItem(SHARE_KEY, String(readerShare.value))
} catch (e) {
  // Kept for this visit only.
  noteProblem('保存阅读窗格的宽度', e)
}
```

- **为什么这样写**：`readerShare` 是模块顶层 ref（App 导入 ReaderPane、PostDetail，它们导入 reader.ts），读必须不抛；`Number(null)` 是 0、坏值是 NaN，都不在 0.3–0.7 之间，用 0.5。写只在拖完分隔条时（ReaderPane 的 `endResize`，`save=true`）发生，值已经在内存的 ref 里。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：存储被禁用 → 默认一半宽，拖动照常；写失败 → 刷新后回到上次存下的宽度。不会白屏，不会打断拖动。
- **要不要改**：不用改。

### 记住哪些博客在阅读栏里打不开（JSON.parse、localStorage）

```ts
function blocked(): number[] {
  try {
    const v = JSON.parse(localStorage.getItem(BLOCKED_KEY) ?? '[]')
    return Array.isArray(v) ? v.filter(Number.isInteger) : []
  } catch {
    return []
  }
}

/** Remembers that this blog's pages would not show in the pane. */
export function markBlocked(blogId: number) {
  try {
    localStorage.setItem(BLOCKED_KEY, JSON.stringify([blogId, ...blocked().filter((x) => x !== blogId)].slice(0, 500)))
  } catch (e) {
    // Applies to this visit only.
    noteProblem('保存不能在窗格里打开的博客', e)
  }
```

- **为什么这样写**：`blocked()` 在每次点链接（`openInReader`）和指针停在链接上（`readerIntent`）时由 `canShow` 调用，抛错会让点击或悬停处理失败，所以读、解析都包住，坏值当作空表。`markBlocked` 在读者点阅读栏的「新标签页打开」（`giveUp`）时记下这个博客，写失败就放弃。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：不会抛到外面。存储被禁用或满时，注释说的「Applies to this visit only」其实不成立：名单每次都从存储读，内存里没有副本，所以这次访问里再点这个博客的链接，仍会先在阅读栏打开、显示不出来，读者得再点一次「新标签页打开」。
- **要不要改**：可以改（低）——只在存储不可用时多点一次；想修可以加一个模块级的 `Set<number>`，`markBlocked` 同时记进去，`canShow` 一并检查。

### 预先连接文章原文所在的网站（new URL）

```ts
export function warmUp(url: string) {
  let origin: string
  try {
    origin = new URL(url).origin
  } catch {
    return
  }
```

- **为什么这样写**：打开文章详情时（PostDetail）给原文网站加 dns-prefetch / preconnect，只需要 origin；文章地址来自服务器或读者自己订阅源的数据，不一定是合法的绝对 URL。
- **什么时候会出事**：不会出事：地址不合法（相对地址、空串）时 `new URL` 抛 TypeError，被 catch，直接不预连，只是少一点加速。
- **要不要改**：不用改。

## `frontend/src/server.ts`

### 问服务器能做什么（.json()）

```ts
asking ??= fetch('/api/capabilities', { cache: 'no-store' })
.then(async (r) => {
const c = r.ok && r.headers.get('content-type')?.includes('json') ? await r.json() : {}
// …
.catch(() => {
// Offline: ask again next time.
asking = null
```

- **为什么这样写**：在静态托管（GitHub Pages）上，/api/capabilities 返回的是 404 页面，或者 SPA 回退的 index.html，所以只有在 2xx 并且 content-type 是 JSON 时才解析，否则当作什么都不提供。`c?.sync === true` 这种写法也挡住了 null 和缺字段的情况。
- **什么时候会出事**：r.json() 只在服务器回了标着 JSON、实际却解析不了的内容时才会 reject（服务器 bug、响应被截断）。这种情况会落到 .catch，当作离线：返回全 false，并清掉 asking，下次再问，不会抛给调用方。旁边那条分支要留意：服务器临时回 5xx（重启中、反向代理 502）时 `r.ok` 为假，全 false 的结果会被留在 asking 里，这次访问里不会再问。于是设置页说「这里的服务器不提供同步」，并隐藏添加来源的表单；syncNow 每次都报这一句，刷新页面才恢复。
- **要不要改**：可以改（低）——.json() 本身没问题。可以让 5xx 和离线走同一条路（不缓存结果，清掉 asking），只有 404 或非 JSON 回答才认定「不提供」。

### 连不上本站或本站拒绝时报错（throw）

```ts
try {
res = await fetch(`/api/fetch?url=${encodeURIComponent(url)}`)
} catch {
throw new FetchFailed('连不上本站的服务器')
}
```

```ts
const e = new FetchFailed(message)
const wait = Number(res.headers.get('retry-after'))
if (wait > 0) e.retryAfter = wait
throw e
```

- **为什么这样写**：fetchRemote 抛 FetchFailed，消息是给读者看的中文说明，retryAfter 用来告诉调用方对方网站要求等多久（429/503）。
- **什么时候会出事**：断网、本站服务器挂了，或者对方网站出错、超时、限流（服务器回 4xx/5xx 和中文的 error）。调用方都接住了：readFeed/readList 交给 failed()，记下错误，并按 Retry-After 推迟下一次读取，设置页和这个源的页面上显示「上次读取失败：…」；lookUp 的错误由 add() 显示；readNow 被 `.catch(() => [])` 吞掉。
- **要不要改**：不用改——抛错是这里要的处理方式，消息会给读者看到。

### 读出服务器给的错误说明（.json()）

```ts
let message = `HTTP ${res.status}`
try {
message = (await res.json()).error ?? message
} catch {
if (res.status === 404) message = '这里的服务器不能代为读取 RSS'
}
```

- **为什么这样写**：本站服务器（readers.rs 和 Worker 的 error()）出错时回 `{"error": "对方网站返回 HTTP 404"}` 这样的中文说明，这里把它取出来给读者看；取不到就显示「HTTP 状态码」。
- **什么时候会出事**：不会抛出去：非 JSON 的错误页（反向代理的 502 HTML、静态托管的 404 页）和值为 null 的 JSON 都落到 catch；如果是 404，还会换成「这里的服务器不能代为读取 RSS」。
- **要不要改**：不用改——已经接住。

### 读出网页上找到的 RSS（.json()）

```ts
if (res.headers.get('x-kind') === 'page') {
const j = await res.json()
return { kind: 'page', feeds: Array.isArray(j.feeds) ? j.feeds : [], url: j.final_url || url }
}
```

- **为什么这样写**：地址是网页时，服务器回 `x-kind: page` 和 `{"feeds": [...], "final_url": ...}`。这是服务器自己用 Json(...) / JSON.stringify 生成的，所以直接解析，只防了 feeds 不是数组的情况。
- **什么时候会出事**：只有响应体读到一半时连接断了（res.json() 以网络错误 reject），或者中间某一层把响应体改坏了才会出事；j 是 null 时，`j.feeds` 也会抛 TypeError。这些都会抛给调用方：lookUp 的错误由 add() 显示，但读者看到的是浏览器的英文报错；readFeed/readList 把它记成这个源的读取错误；readNow 的错误被吞掉。下面的 `await res.arrayBuffer()` 也一样。
- **要不要改**：可以改（低）——把这两处读响应体的调用包进 try，失败时 `throw new FetchFailed('服务器的回答没读完整，稍后再试')`，提示就和其他失败一样了。

## `frontend/src/social.ts`

### 从社交链接里取账号名（new URL、decodeURIComponent）

```ts
function handleOf(kind: string, url: string): string {
  try {
    const u = new URL(url)
    const host = u.hostname.replace(/^www\./, '')
    if (kind === 'link') return host
    // NetEase Music: #/user/home?id=<id>
    if (u.hostname === 'music.163.com') return new URLSearchParams(u.hash.split('?')[1] ?? '').get('id') ?? ''
    const seg = u.pathname
      .split('/')
      .filter(Boolean)
      .find((s) => !PREFIXES.has(s))
    return seg ? decodeURIComponent(seg) : host
  } catch {
    return ''
  }
}
```

- **为什么这样写**：同一平台有几个账号时，用链接里的账号名区分（「X (wznmickeyEn)」）。链接是从博客资料页抓来的，可能不合法；路径段是百分号编码的（中文用户名），要解码才好看。
- **什么时候会出事**：不会出事：链接不是合法 URL 时 `new URL` 抛 TypeError，路径里有残缺的百分号编码（如 `https://example.com/%E4%B8`）时 `decodeURIComponent` 抛 URIError，两者都在同一个 try 里，返回 `''`；`socialItems` 用 `|| null` 把它当作没有账号名，标签只显示平台名。它在 SocialLinks 的 computed 里（渲染时）调用，不抛才不会让博客资料卡渲染失败——确实不抛。
- **要不要改**：不用改——解码失败时也可以退回显示原样的 `seg`，但影响只是一个标签少了括号里的名字。

## `frontend/src/sources.ts`

### 读取存着的来源和缓存（JSON.parse、localStorage）

```ts
function load<T>(key: string, fallback: T): T {
try {
return JSON.parse(localStorage.getItem(key) ?? 'null') ?? fallback
} catch {
return fallback
}
```

- **为什么这样写**：ownSources、reads、crawled 在模块加载时就读。这个模块被首页、设置页、同步 import，如果在顶层抛错，整个应用都起不来，所以存储被禁用（SecurityError）或值被改坏时都退回默认值。
- **什么时候会出事**：不会抛：两种异常都在 try 里。只是读出来的值不检查结构：KEY 的值还要经过 cleanSources 清洗，READ_KEY、CRAWLED_KEY 的值则原样使用，只有手动把 localStorage 改成奇怪的结构（比如某个 items 不是数组）才会在后面的 readState 等处抛 TypeError。
- **要不要改**：不用改——已经接住；只有手改数据才会碰到结构问题。

### 写进本地存储（localStorage）

```ts
function store(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch (e) {
    // Storage full or blocked: kept for this visit.
    noteProblem(`保存自己的订阅（${key}）`, e)
  }
```

- **为什么这样写**：存储满了或被禁用时不抛错，数据留在内存里，这次访问照常能用。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：这一行不会抛。问题在于存储满时写入会被悄悄丢掉，而这里写的最大一项 `sources:read`（READ_KEY）只会变大不会变小：每个在浏览器里读的源最多缓存 20 张卡片，一张的 JSON 约 0.8K 字符（中文摘要 240 字）；删掉的源、后来被本站抓取的源，它们的条目都一直留在 reads 里。导入一个两三百个订阅的 OPML，单这一项就有三四百万字符，和浏览器给每个网站的 localStorage 额度（通常 5MB 左右）是同一个量级，还要加上 library.ts 最多 4500 条的喜欢、收藏、历史和刷到过。额度用完以后：这一项写不进去，旧值还占着地方；同一网站下的其他写入也会失败——library.ts 写不进时会先把「刷到过」砍到 200 条、历史砍到 50 条来腾地方，还不够的话，新的喜欢和收藏只在这次访问里有效，刷新就没了（没开同步时就是真丢了）；新加的来源（KEY）也写不进去，刷新后就不见了。
- **要不要改**：建议改（中）——READ_KEY 只是可以重新读到的缓存，应该给读者自己的数据让地方：noteRead 时只保留还在 previewFeeds 里的源的条目；给序列化后的总长度设个上限（比如超过 1M 字符时每个源只留最近几条）；store(READ_KEY) 失败时 removeItem(READ_KEY)，不要让旧的大值一直占着空间。

### 告诉服务器读者有哪些源（.json()）

```ts
if (!res.ok) return
const answer = await res.json()
for (const [url, id] of Object.entries(answer?.blogs ?? {})) {
```

- **为什么这样写**：服务器回 `{"blogs": {地址: 博客 id}}`，这里记下哪些源已经由本站抓取。
- **什么时候会出事**：不会抛出去：整段在 try 里，失败就等下次（"Next time."）；answer 是 null 时，`answer?.blogs ?? {}` 也挡住了。
- **要不要改**：不用改——已经接住。

### 定时读源时，源变成了别的东西（throw）

```ts
const got = await fetchRemote(feed.url)
if (got.kind !== 'feed') throw new Error('这个地址现在是网页，不是 RSS')
const parsed = parseSource(got.text, got.url)
if (!('feed' in parsed)) throw new Error('这是订阅列表，不是 RSS')
// …
} catch (e) {
failed(feed.url, e)
}
```

```ts
const got = await fetchRemote(list.url)
if (got.kind !== 'feed') throw new Error('这个地址现在是网页，不是 OPML')
const parsed = parseSource(got.text, got.url)
if (!('list' in parsed)) throw new Error('这个地址现在是 RSS，不是订阅列表')
// …
} catch (e) {
failed(list.url, e)
}
```

- **为什么这样写**：源在添加时是 RSS 或 OPML，之后对方网站可能改了。用 throw 把这种情况和「取不到」「解析失败」并到同一个 catch 里处理。
- **什么时候会出事**：对方网站改版，或者 RSS 地址被跳转到首页。同一个函数里的 catch 会接住：failed() 把消息记进 reads，设置页「我添加的」显示「上次读取失败：这个地址现在是网页，不是 RSS」，这个源的页面上也显示；之前读到的文章还在。
- **要不要改**：不用改——在本函数内接住，并显示给读者。

### 按地址或文件添加来源时找不到东西（throw）

```ts
if (got.kind === 'page') {
if (!got.feeds.length) throw new Error('这个网页上没有找到 RSS')
url = viaOurRsshub(got.feeds[0])
got = await fetchRemote(url)
if (got.kind !== 'feed') throw new Error('这个网页上没有找到 RSS')
}
// …
if ('list' in parsed) {
if (!parsed.list.feeds.length) throw new Error('这个订阅列表是空的')
```

```ts
const list = parseOpmlFile(await file.text())
if (!list.feeds.length) throw new Error('这个文件里没有 RSS 地址')
```

- **为什么这样写**：读者在设置页输入地址或选择文件，找不到能添加的东西时，用中文告诉他原因。
- **什么时候会出事**：输入的网页没有 `<link rel="alternate">`、OPML 是空的，这些都是正常的输入错误。SettingsView 的 add() 会接住，把消息用红字显示在输入框下面，「读取中…」也会复原。
- **要不要改**：不用改——抛错是预期的处理方式，调用方会把消息给读者看。

### 规范化要添加的地址（new URL）

```ts
// Known by the address asked for (redirects or not): the crawler finds a
// feed it already has by it, with its history.
got = { ...got, url: new URL(url).href }
```

- **为什么这样写**：用读者输入的地址（或网页上找到的地址）作为源的标识，而不是跳转后的地址。`new URL().href` 会把它规范化（主机名转小写、补上 `/`），和服务器上的写法一致。
- **什么时候会出事**：不会：走到这里之前，fetchRemote(url) 已经成功了，说明服务器（Rust 的 url crate 或 Worker 的 new URL，都按 WHATWG 规则）解析过同一个字符串；url 要么是读者的输入（缺协议时补了 https://），要么是服务器返回的绝对地址。就算抛了也会被 add() 接住，只是提示变成英文。
- **要不要改**：不用改——同一个地址已经被服务器解析过。

### 算出本站对博客的标识（new URL）

```ts
export function siteKey(url: string): string | null {
try {
const u = new URL(url)
// …
} catch {
return null
}
```

- **为什么这样写**：添加源时，用和服务器 util.rs `site_key` 一样的规则算出 example.com/blog，查本站是不是已经收录了这个博客。
- **什么时候会出事**：不会：在 try 里，坏地址返回 null，比如源没写主页（homepage 是 ''）时就是这样。调用方（SettingsView 的 onSite）拿到 null 就不去查。
- **要不要改**：不用改——已经接住。

### 向服务器登记自己的订阅（catch {}）

```ts
if (!res.ok) return
const answer = await res.json()
for (const [url, id] of Object.entries(answer?.blogs ?? {})) {
if (Number.isInteger(id) && (id as number) > 0) noteCrawled(url, id as number)
}
store(REGISTERED_KEY, Date.now())
} catch {
// Next time.
}
```

- **为什么这样写**：登记是为了让服务器（或 Worker）也去抓这些 RSS，并告诉浏览器哪些已经由网站在抓。失败（离线、服务器出错、回应不是 JSON）时什么也不用做：订阅列表一变，2 秒后会再登记，每天第一次打开也会登记一次。
- **什么时候会出事**：一直失败时（例如服务器的 `/api/readers/feeds` 坏了），读者的订阅只由浏览器自己读：网站不会抓它们，它们也不会出现在「读者在订阅」里。读者看不出区别，只是旧文章少（浏览器每个源只留 20 篇），也没有任何提示或记录。
- **要不要改**：可以改（低）——在 catch 里和 `!res.ok` 时调用 `noteProblem('登记自己的订阅', …)`（带上状态码），这样「诊断信息」里看得到。

## `frontend/src/staticApi.ts`

### 读取导出的 JSON 文件（throw、.json()）

```ts
async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  return (await res.json()) as T
}
```

- **为什么这样写**：静态版的所有数据都经过这里（经 `index()` 和 `file()`），和服务器版 `get` 用同一个约定：非 2xx 变成 `HTTP <状态码>` 的 rejection，调用方一个 catch 处理所有失败。
- **什么时候会出事**：数据文件不存在时，Cloudflare（wrangler.jsonc 的 `not_found_handling: "404-page"`）和 GitHub Pages 都回 404 状态，在 throw 那行拒绝，不会把 404.html 当 JSON 解析；重新部署期间文件不全、断网也在这里失败。网络中途断开导致正文截断、或强制门户/代理回 200 的 HTML 时，`res.json()` 拒绝。`file()` 失败时把缓存删掉，下次重试；rejection 经 `staticApi` 的 async 函数传给和服务器版相同的调用方，它们都 catch（见 api.ts 的「请求失败或回来的不是 JSON」）。读者自己的源还没被爬到时 `own/<hash>.json` 必然 404，`ownFile` 把它 catch 成 null，是正常情况。
- **要不要改**：不用改——抛出是约定的失败通道，调用方都处理了。

### index.json 取不到时清掉缓存再抛（throw）

```ts
function index(): Promise<Index> {
  indexPromise ??= fetchJson<Index>(`${DATA}index.json`, { cache: 'no-cache' }).catch((e) => {
    indexPromise = null
    throw e
  })
  return indexPromise
}
```

- **为什么这样写**：`indexPromise` 缓存 index.json 的请求。失败的 promise 如果留在缓存里，以后每次调用都会立刻失败，直到刷新页面；所以 catch 里先清空缓存，再把原错误抛出，这次的调用方照常收到失败，下次调用会重新请求。不抛的话 `.catch` 会返回 undefined，调用方随后读 `generated_at` 时才以更难懂的 TypeError 失败。
- **什么时候会出事**：断网或部署中取不到 index.json 时。调用方都 catch（显示「加载失败，点此重试」等），网络恢复后点重试就能加载。
- **要不要改**：不用改。

### 博客文件取不到时找读者自己的源（throw）

```ts
const blogFile = (id: number) =>
  file<BlogFile>(`blogs/${id}.json`).catch(async (e) => {
    const own = (await ownFiles()).find((f) => f.blog.id === id)
    if (!own) throw e
    return { blog: own.blog, posts: own.posts }
  })
```

- **为什么这样写**：读者自己的源被爬到后有站点的正数 id，但不在 `blogs/` 下，只在 `own/<hash>.json` 里；所以 `blogs/<id>.json` 取不到时先在自己的源里找，找不到才把原来的错误（通常是 `HTTP 404`）原样抛出，调用方看到的失败和服务器版一致。
- **什么时候会出事**：打开一个已从导出中移除的博客的旧链接（`/blog/123`），或查询 `blog=` 一个不存在的 id。BlogView 的 watch catch 后显示找不到，Waterfall 显示加载失败。`ownFiles()` 本身不会抛：每个 `ownFile` 都 catch 成 null，没有 `crypto.subtle` 时返回空数组。
- **要不要改**：不用改。

### 找不到文章或博客时按 404 拒绝（throw）

```ts
  const found = (await allPosts()).find((p) => p.id === id)
  if (found) return { ...found, content_html: null }
  throw new Error('HTTP 404')
```

```ts
    const found = (await allBlogs()).find((b) => b.key === key) ?? (await ownFiles()).find((f) => f.blog.key === key)?.blog
    if (!found) throw new Error('HTTP 404')
```

- **为什么这样写**：模拟服务器版的 404，让调用方用同一套失败处理。
- **什么时候会出事**：静态版只导出最新的若干篇文章（feed 分片，README 说约 3000 篇）；用 `?p=<id>` 打开一篇更早的文章，或打开一个已不在 blogs.json、也不是自己源的博客地址时。PostDetail catch：卡片已知（从列表或信息流点进来）时照常显示卡片和「详情加载失败，可直接阅读原文」，否则显示「找不到这篇文章」；BlogView 显示找不到。
- **要不要改**：不用改——这就是预期的「找不到」。

### 按分类筛选博客（non-null !）

```ts
  const all = (await allBlogs())
    .filter((b) => !q.dirtag || b.tags.some((t) => t.tag === q.dirtag))
    .filter((b) => !q.category || (b.categories ?? []).some((c) => c.id === topOf(q.category!)))
```

- **为什么这样写**：`!q.category ||` 已经保证后面的 `q.category` 有值，但 `!` 所在的是内层回调 `(c) => ...`，TypeScript 在回调里不保留对参数属性的收窄，所以加 `!`。
- **什么时候会出事**：不会：内层回调在 `.some` 里同步执行，就在检查之后，中间没有代码能改 `q.category`。
- **要不要改**：不用改。

### 网站替读者抓的订阅文件（.catch 忽略）

```ts
p = sha256(url)
.then((h) => file<OwnFile>(`own/${h}.json`))
.then((f) => {
noteCrawled(url, f.blog.id)
return f
})
.catch(() => null)
```

- **为什么这样写**：静态网站上，网站替读者抓过的订阅各有一个 `own/<哈希>.json`。还没抓过（刚登记，下一次发布之前）时文件不存在，返回 null，调用方改用浏览器自己读到的文章。
- **什么时候会出事**：不会出错。但网络问题、文件损坏也会得到 null，同样当作「还没抓」，而且这个结果缓存在 `ownCache` 里，直到刷新页面。
- **要不要改**：不用改——缺文件是常态，其他原因的结果也一样（用浏览器自己读的）。

### 哪些文章有单独的详情文件（.catch 忽略）

```ts
const detailed = await file<number[]>('details.json').catch(() => [] as number[])
if (detailed.includes(id)) return file<PostDetail>(`posts/${id}.json`)
```

- **为什么这样写**：只有允许全文或内嵌评论的博客，文章才有单独的文件，清单在 `details.json` 里。清单读不到时当作没有，改用列表里的摘要。
- **什么时候会出事**：`details.json` 读不到时，允许全文的那些文章（目前只有站长自己的博客）只显示摘要，也没有内嵌评论；其他文章不受影响。
- **要不要改**：不用改。

### 静态版的数据，一个标签页只读一次（人工）

```ts
let indexPromise: Promise<Index> | null = null
function index(): Promise<Index> {
indexPromise ??= fetchJson<Index>(`${DATA}index.json`, { cache: 'no-cache' }).catch((e) => {
```

```ts
.catch(() => null)
ownCache.set(url, p)
```

- **为什么这样写**：`index.json` 每次都向服务器确认；其余文件带着导出时间 `?v=`，可以一直缓存。读过的文件放在 `files` 里，同一个标签页里不再重复下载。
- **什么时候会出事**：`indexPromise` 成功之后再也不会清空，所以一个标签页从打开起就固定在那一次导出上。标签页开着过夜，或者手机浏览器一直留在后台时，之后几次发布的新文章都看不到，只有整页刷新才行。博客页点「刷新」后提示「已请求，约 N 分钟后网站更新」，可等网站真的更新了，这个页面读的还是旧数据，读者会以为刷新没成功。读者刚添加、网站还没抓到的源，`ownFile` 的结果 null 也缓存着，网站抓到以后这个标签页也不会去读它的文件。另外 `files` 只增不减：每打开一个博客页，这个博客的全部文章（`blogs/<id>.json`）就留在内存里，浏览几百个博客后可能占几十上百 MB。
- **要不要改**：可以改（低）——页面重新可见时，或者距上次读 `index.json` 超过比如 10 分钟时，重新读一次；`generated_at` 变了就清空 `files` 和 `ownCache`。`files` 里的博客文件只保留最近用过的几十个。

## `frontend/src/sync.ts`

### 读取同步状态（JSON.parse、localStorage）

```ts
function read<T>(key: string): T | null {
try {
return JSON.parse(localStorage.getItem(key) ?? 'null')
} catch {
return null
}
```

- **为什么这样写**：LINK_KEY、MARKS_KEY 在模块加载时就要读（初始化 syncStatus 和 marks），模块顶层不能抛错，所以存储被禁用或值被改坏时都当作没有。
- **什么时候会出事**：不会抛。读出来以后：Link 由 readLink 检查 code 是不是合法的同步码；Marks 只取四个字段，缺的补默认值，只有手动改成奇怪的结构，才可能在合并时出错。
- **要不要改**：不用改——已经接住。

### 写入同步状态（localStorage）

```ts
function write(key: string, value: unknown) {
  try {
    if (value === null) localStorage.removeItem(key)
    else localStorage.setItem(key, JSON.stringify(value))
  } catch (e) {
    // Storage blocked: this visit only.
    noteProblem('保存同步状态', e)
  }
```

- **为什么这样写**：存储被禁用或满了时不抛错，不让同步流程中断。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：这两行不会抛。但注释里说的「这次访问照用」对 LINK_KEY 不成立：同步码只存在 localStorage 里，readLink() 每次都从存储读，内存里没有副本。存储被禁用（某些隐私设置、沙箱 iframe）或已经满了（见 sources.ts「写进本地存储」那条）时，读者点「开启同步」：startSync 的 write 悄悄失败，syncStatus 却被设成 on，界面显示「已开启」，同步码那里是空的；接着 syncNow 因为 readLink() 是 null 直接返回，什么都没同步，也没有任何提示。成功同步后如果是写 version 失败，就没有害处：下次多下载一次，再合并一遍。
- **要不要改**：可以改（低）——startSync 写完以后检查 readLink()，读不回来就 `throw new Error('浏览器不让这个网站保存数据，没法同步')`，begin() 会把它显示出来。

### 同步码前面已经验过（non-null !）

```ts
if (decodeCode(m[1])) joinOffer.value = encodeCode(decodeCode(m[1])!)
```

```ts
const l = read<Link>(LINK_KEY)
return l && typeof l.code === 'string' && decodeCode(l.code) ? l : null
// …
const secret = decodeCode(code)!
```

- **为什么这样写**：decodeCode 是纯函数，同一个字符串第一次返回非 null，第二次也一定是非 null；TypeScript 看不出这一点，所以用 `!` 告诉它。keysFor 的 code 只来自 readLink()（once、deleteSync），而 readLink 只在 decodeCode(l.code) 成功时才返回 link。
- **什么时候会出事**：不会：两处都在调用前检查过。将来如果有人绕过 readLink 把 code 传给 keysFor，importKey 收到 null 会 reject，而且这个被拒绝的 promise 会留在 keyCache 里；在 once 里调用时，会被 syncNow 接住并显示出来。
- **要不要改**：不用改——前提已经检查过。

### 打包副本时复制频道栏和设置（JSON.parse）

```ts
channels: { at: marks.touched.channels ?? 0, value: JSON.parse(JSON.stringify(prefs.value)) },
options: { at: marks.touched.options ?? 0, value: JSON.parse(JSON.stringify(options.value)) },
```

- **为什么这样写**：深拷贝，去掉 Vue 的响应式代理，让副本不受之后修改的影响，也和存进 localStorage 的样子一致。
- **什么时候会出事**：不会：prefs、options 始终是 cleanPrefs / cleanOptions 造出来的纯 JSON 对象（字符串、数字、布尔值、数组），没有循环引用、BigInt，根值也不是 undefined，stringify 之后一定能 parse 回来。
- **要不要改**：不用改——数据一定能被 JSON 表示。

### 解开服务器上的副本（throw、JSON.parse）

```ts
async function unseal(bytes: Uint8Array, key: CryptoKey): Promise<Doc> {
if (bytes[0] !== 1) throw new Error('同步数据来自更新的版本，请先刷新页面')
let zipped: ArrayBuffer
try {
zipped = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: bytes.slice(1, 13), additionalData: AAD }, key, bytes.slice(13))
} catch {
throw new Error('同步数据解不开，可能已损坏')
}
const doc = JSON.parse(new TextDecoder().decode(await pipe(new Uint8Array(zipped), new DecompressionStream('gzip'))))
if (doc?.v !== 1) throw new Error('同步数据来自更新的版本，请先刷新页面')
return doc
}
```

- **为什么这样写**：副本的格式带版本号（首字节和 doc.v）。遇到新版前端写的、旧版不认识的数据就停下，不去合并或覆盖，因为覆盖会丢掉新版多出来的数据。AES-GCM 自带认证，解密成功就说明这份数据是某台设备用同一个同步码完整写进去的，所以后面的解压和 JSON.parse 没有再单独包 try。
- **什么时候会出事**：① 将来格式升级后，另一台设备已经用上新版，这台还开着旧页面：报「同步数据来自更新的版本，请先刷新页面」，刷新就好；② 服务器上的数据被截断或损坏：报「同步数据解不开，可能已损坏」。两种都发生在 once() 里，由 syncNow 的 catch 接住：设置页显示「没能同步：…」，从 30 秒开始翻倍重试，最长间隔 10 分钟，本地数据不受影响。② 不会自己恢复，读者要在设置里「删除服务器上的数据」后重新开启同步。解压或 JSON.parse 失败只可能是写入那一方有 bug，也会被同样接住，只是提示是英文。
- **要不要改**：不用改——不认识就停、不覆盖，是保护数据的做法，消息也会给读者看到。

### 副本压到最小还是太大（throw）

```ts
const sealed = await seal(d, key)
if (sealed.length <= MAX_BYTES) return sealed
}
throw new Error('要同步的数据太多了')
```

- **为什么这样写**：服务器只收 1.9MB 以内的副本（readers.rs 和 Worker 的 MAX_SYNC_BYTES 跟这里的 MAX_BYTES 一样）。放不下时，先依次缩短「刷到过」（300、50、0 条）和历史（100 条），还放不下才报错。
- **什么时候会出事**：几乎不会：要在喜欢（最多 1000 条）和收藏（最多 2000 条）之外，再有大量来源列表之类的数据，压缩加密后才会超过 1.9MB。真遇到时，syncNow 会接住并显示「没能同步：要同步的数据太多了」，然后一直按退避间隔重试，直到读者删掉一些收藏。
- **要不要改**：不用改——现实中达不到，达到了也有提示。

### 服务器拒收：副本太大（throw）

```ts
if (put.status === 413) throw new Error('要同步的数据太多了')
```

- **为什么这样写**：sealFitting 已经保证不超过本站服务器的上限，所以 413 理论上不会来自本站服务器；这里兜个底，给一句读者能看懂的话。
- **什么时候会出事**：README 建议在服务器前面挂 Caddy 或 nginx 做 HTTPS，而 nginx 的 `client_max_body_size` 默认只有 1MB。喜欢、收藏、历史多的读者，副本压缩加密后在 1MB 到 1.9MB 之间时，每次写入都会被 nginx 回 413：设置页一直显示「没能同步：要同步的数据太多了」，最长每 10 分钟重试一次，结果都一样，这台设备的改动永远传不上去（读取其他设备的改动仍然正常，因为合并发生在写入之前）。sealFitting 只按 1.9MB 裁剪，收到 413 后不会再裁小。
- **要不要改**：建议改（中）——收到 413 时，用更小的上限重新 sealFitting（比如给它加一个上限参数，413 后依次试 1MB、500KB，逐级去掉刷到过和历史）；同时在 README 的部署说明里写明 nginx 要设 `client_max_body_size 2m;`。

### 服务器不提供同步（throw）

```ts
if (!(await loadCapabilities()).sync) throw new Error('这里的服务器不提供同步')
```

- **为什么这样写**：静态托管上没有 /api/sync，不去请求；抛错是为了让 syncNow 的 catch 统一显示和重试。
- **什么时候会出事**：只有服务器确实不提供同步时，这句话才是对的。loadCapabilities 在离线（fetch 失败）时也返回全 false，所以如果这次访问里第一次询问时就断网了，之后的同步（定时 pull、切回标签页）都会显示「没能同步：这里的服务器不提供同步」，误导读者；恢复联网后，online 事件会触发重新询问和同步，提示就消失了。如果询问时服务器恰好回了 5xx，结果会被缓存（见 server.ts「问服务器能做什么」），这次访问里一直显示这句，刷新页面才好。
- **要不要改**：可以改（低）——让 loadCapabilities 区分「不知道」（离线、5xx）和「不提供」，「不知道」时这里抛「连不上服务器，稍后自动重试」。

### 读写副本失败（throw）

```ts
throw new Error('连不上服务器，稍后自动重试')
}
// …
if (res.status !== 200 && res.status !== 304 && res.status !== 404) throw new Error(`服务器出错（HTTP ${res.status}）`)
```

```ts
if (put.status === 412) continue
// …
if (!put.ok) throw new Error(`服务器出错（HTTP ${put.status}）`)
```

```ts
throw new Error('别的设备正在同步，稍后自动重试')
```

- **为什么这样写**：once() 只负责做一次同步，出问题就抛一个带中文说明的 Error，由 syncNow 统一处理：在设置页显示，并按退避重试（30 秒起翻倍，最长 10 分钟）。404 表示服务器上还没有副本（第一台设备），不算错；412 表示别的设备刚写过，那就重新读、合并、再写，最多试 4 次。
- **什么时候会出事**：断网、服务器重启（502/503）、数据库出错（500），或者几台设备几乎同时不停地写（4 次都碰上 412）。读者在设置页看到「没能同步：…」，过一会儿会自己恢复，本地数据不受影响。有两个小问题：PUT 那次 fetch 没有像 GET 那样包在 try 里，写入时断网，显示的是浏览器的英文报错（如 “Failed to fetch”）；服务器上的同步码数量已满（MAX_SYNC_DOCS = 20000，第一次写入会回 507）时，只显示「服务器出错（HTTP 507）」，而且会一直重试。
- **要不要改**：可以改（低）——这几处抛错是预期的处理方式。可以顺手把 PUT 的 fetch 也包进 try（抛「连不上服务器，稍后自动重试」），再把 507 译成「服务器上的同步名额满了」。

### 另一台设备删除了副本（throw）

```ts
if (res.status === 410) throw new Gone()
```

```ts
if (put.status === 410) throw new Gone()
```

- **为什么这样写**：410 表示有设备点了「删除服务器上的数据」，这个同步码已经作废。用专门的 Gone 类型抛出，syncNow 就能把它和别的错误分开：清掉本机的 link，显示「另一台设备删除了服务器上的同步数据，同步已停止」，不再重试。
- **什么时候会出事**：这是预期的流程，不是故障；本机的数据都保留。
- **要不要改**：不用改——这是有意的控制流，结果会显示给读者。

### 在设置页开启或删除同步时出错（throw）

```ts
const bytes = decodeCode(code)
if (!bytes) throw new Error('同步码不对：应该是 24 个字母和数字')
```

```ts
const res = await fetch('/api/sync', { method: 'DELETE', headers: { authorization: `Bearer ${token}` } })
if (!res.ok) throw new Error(`没能删除（HTTP ${res.status}）`)
```

- **为什么这样写**：这两个函数只由设置页的按钮调用，错误抛给 SettingsView 的 begin() 和 removeAll()，它们 catch 之后把消息放进 syncError，显示在同步那一节的下面。
- **什么时候会出事**：读者把同步码输错或少输了几位，或者删除时服务器出错，都会看到提示。删除失败时，本机还在同步，可以再点一次。deleteSync 里的 fetch 本身没有包 try，断网时显示的是浏览器的英文报错（同样会被 removeAll 接住）。
- **要不要改**：不用改——调用方接住了，并把消息给读者看。

## `frontend/src/syncDoc.ts`

### 合并靠各设备自己的时钟，删除记录 120 天后失效（人工）

```ts
export const REMOVED_DAYS = 120
// …
const gone = (col: string, key: string | number, at: number) => (marks[col]?.[String(key)] ?? -1) >= at
```

- **为什么这样写**：没有账户，服务器也看不懂内容，只能在浏览器里合并。每个关注、喜欢、收藏、来源都带着加入的时间，删除会留下「删除于」的记录，两边比时间：删除晚于加入，就删掉。删除记录只留 120 天，免得同步的副本越来越大。
- **什么时候会出事**：有两种情况。一是时间用的是各设备自己的 `Date.now()`。如果一台设备的时钟快了几个小时（手动调过时间、双系统电脑把硬件时钟当本地时间、没开自动对时），在这几个小时里，它做的删除会压过别的设备上的重新添加：比如在时钟快的电脑上取消关注一个博客，过一会儿在手机上重新关注，下一次同步后又没了。它改的设置（`objects` 的 `at`）也总是赢。时钟慢的设备正好相反，它的删除会被别处更早的添加压回去。读者只会觉得同步「有时不听话」。二是一台很久没打开的设备（旧平板、不常用的电脑）过了 120 天再同步时：别的设备早就删掉的喜欢、收藏、关注和来源，它那里还有，而删除记录已经过期，合并后这些东西全都回来了，再同步到所有设备。
- **要不要改**：可以改（低）——给时间戳加逻辑时钟：本地新记下的时间取 `max(Date.now(), 见过的最大时间 + 1)`，或者用服务器回复的 `Date` 头校正。设备的上次同步时间早于 `REMOVED_DAYS` 时，不合并它本地的旧内容，以服务器上的副本为准，并提示读者。

## `frontend/src/telemetry.ts`

### 统计用的本机 id 和「今天来过」标记（localStorage）

```ts
function stored(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}
function keep(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // This visit only.
  }
}
```

```ts
export function statsId(): string {
  let id = stored(ID_KEY)
// …
function daily() {
  const today = new Date().toISOString().slice(0, 10)
  if (!sendingStats() || stored(DAY_KEY) === today) return
  keep(DAY_KEY, today)
// …
daily()
setInterval(daily, 3600_000)
```

- **为什么这样写**：统计绝不能影响正常使用，所以读写都包一层，读失败当作没有，写失败直接放弃。`daily()` 在模块加载时就运行（library.ts 导入 telemetry.ts，启动时加载），这里抛错就是白屏，包住是必要的。
- **什么时候会出事**：不会抛到外面。但存储被禁用时注释说的「This visit only」并不成立，因为内存里也没留：`stored` 总返回 null，于是 `statsId()` 每次发送（`flush`）都生成一个新的随机 id，`daily()` 每小时都认为今天还没来过，又发一次 `visit` 和全部 `setting`。服务器按 (day, who) 去重访客（stats.rs 的 `visitors` 表，Worker 同样），这样的读者开着页面会每小时被算成一位新访客，访客数和设置统计偏高一点。读者自己看不到任何问题。
- **要不要改**：可以改（低）——在模块里留一份内存副本：如 `let memId: string | null = null` 让 `statsId` 先用它、生成后存进去；`let lastDay = ''` 让 `daily` 先比较它。这样存储不可用时也是「每次访问一个 id、一天一次 visit」。

### 统计发不出去就算了（.catch 忽略）

```ts
fetch('/api/events', { method: 'POST', headers: { 'content-type': 'application/json' }, body, keepalive: true }).catch(
() => {},
)
```

- **为什么这样写**：匿名统计是尽力而为：失败不重试、不提示，免得打扰读者，也不值得占用诊断记录。
- **什么时候会出事**：离线、或者被广告拦截插件拦下时，这一批计数丢失，热度略偏低；其他功能不受影响。
- **要不要改**：不用改。

## `frontend/src/components/BlogAboutCard.vue`

### 博主资料读不到（catch {}）

```vue
try {
detail.value = await blogDetail(id)
} catch {
// The card still shows the name and avatar from the post itself.
}
```

- **为什么这样写**：卡片上已经有文章里带的博主名字和头像，详细资料只是补充；读不到时就保持简单的样子。
- **什么时候会出事**：服务器出错，或者静态网站缺了这个博客的数据文件时，卡片上没有简介和社交链接，也没有提示。
- **要不要改**：不用改。

## `frontend/src/components/CommentEmbed.vue`

### 把 giscus / utterances 的脚本放进评论框（non-null !）

```vue
for (const [k, v] of Object.entries(attrs)) if (v) s.setAttribute(k, v)
box.value!.appendChild(s)
```

- **为什么这样写**：giscus / utterances 是「`<script>` 放在哪里就在哪里画评论区」的挂件，要放进 `<div ref="box">`。这个 div 在模板里没有 `v-if`，组件挂载后 `box` 一定有值；`scriptWidget` 只由 `mount()` 调用，而 `mount()` 只在 `onMounted` 里建的 IntersectionObserver 回调中触发。
- **什么时候会出事**：几乎不会：giscus / utterances 分支在 `mount()` 的第一个 `await` 之前同步执行，这时组件还挂着。唯一的边角情况是观察回调已经排进队列、同一帧里组件被卸载（评论区刚滚进视野就关掉了文章详情）：卸载后 Vue 把 `box` 置为 null，这里抛 TypeError。但它在 `mount()` 的 try/catch 里，只会 `console.warn('comment widget failed', …)` 并把一个已经不显示的组件的 `state` 设成 `error`，读者什么也看不到。
- **要不要改**：不用改——元素无条件渲染，极端情况下的异常也已被 `mount()` 的 try/catch 接住。

### 评论脚本加载完后调用它的全局对象（non-null !）

```vue
await loadScript(CDN.twikoo)
await g.twikoo!.init({ envId: c.env_id, el: `#${boxId}`, path: c.path, lang: 'zh-CN' })
```

```vue
await loadScript(CDN.artalk)
const a = g.Artalk!.init({
```

- **为什么这样写**：`loadScript` 在 `<script>` 的 `onload` 之后才 resolve，固定版本的 twikoo / Artalk CDN 构建执行后会挂上 `window.twikoo` / `window.Artalk`；TypeScript 不知道这一点，所以写 `!`。脚本加载失败（`onerror`）时 `loadScript` 已经 reject，走不到这一行。
- **什么时候会出事**：脚本「加载成功」却没有定义全局对象时，例如浏览器的拦截扩展把这个 CDN 脚本换成了空脚本：`g.twikoo` / `g.Artalk` 是 undefined，调用抛 TypeError。`init` 本身也可能抛错或 reject，比如配置不对，或者脚本加载期间读者已经关掉详情、`#comments-…` 元素不在了。这些都在 `mount()` 的 try/catch 里：读者看到「评论区加载失败，可以到原文评论。」，控制台有 `comment widget failed`，页面其他部分不受影响。
- **要不要改**：不用改——这正是 try/catch 要处理的失败，提示也合适；`!` 只是让类型检查通过。

### 观察评论区何时滚进视野（non-null !）

```vue
observer.observe(root.value!)
```

- **为什么这样写**：评论挂件的脚本等评论区快滚到时才加载。`root` 是模板的根 `<div ref="root">`，没有 `v-if`，到 `onMounted` 时模板引用已经赋值。
- **什么时候会出事**：不会：`onMounted` 只在挂载完成后调用一次，这时 `root.value` 一定是那个 div。
- **要不要改**：不用改——根元素总是存在，`!` 只是告诉 TypeScript 这一点。

## `frontend/src/components/DiagnosticsPanel.vue`

### 统计本机存储用了多少（localStorage）

```vue
try {
const out = []
for (let i = 0; i < localStorage.length; i++) {
const key = localStorage.key(i) ?? ''
out.push({ key, chars: key.length + (localStorage.getItem(key)?.length ?? 0) })
}
return out.sort((a, b) => b.chars - a.chars)
} catch {
return null
}
```

- **为什么这样写**：列出每一项占多少，用来判断存储是不是快满了。整段包在 try 里。
- **什么时候会出事**：不会：存储被禁用时返回 null，面板上显示「读不到：浏览器不让这个网站保存数据」，这本身就是有用的诊断信息。
- **要不要改**：不用改。

## `frontend/src/components/FeedCard.vue`

### 卡片封面图地址（non-null !）

```vue
<div v-if="showImage" class="cover" :style="{ aspectRatio: `1 / ${ratio ?? COVER_DEFAULT}` }">
<img
:src="post.cover!"
```

- **为什么这样写**：`cover` 的类型是 `string | null`。`showImage` 等于 `!!props.post.cover && !failed.value`，外层 div 的 `v-if` 已经保证有封面，`!` 只是告诉 TypeScript 这一点。
- **什么时候会出事**：不会：父元素 `v-if` 为假时整个 `<img>` 都不渲染；而且就算是 null，`:src="null"` 也只会让 Vue 去掉 `src` 属性，不会抛错。
- **要不要改**：不用改——父元素的 `v-if` 已经保护，运行时 `!` 不做任何事。

## `frontend/src/components/MyBlog.vue`

### 记住博主选的「我的博客」（localStorage）

```vue
try {
  const id = Number(localStorage.getItem(KEY))
  return Number.isInteger(id) && id > 0 ? id : null
} catch {
  return null
}
```

```vue
try {
  if (id) localStorage.setItem(KEY, String(id))
  else localStorage.removeItem(KEY)
} catch (e) {
  // Remembered for this visit only.
  noteProblem('保存「我的博客」', e)
}
```

- **为什么这样写**：「我的」页的博主标签（`/me?tab=blog`）记住上次选的博客，下次打开直接显示它的抓取情况。读和写都包在 try/catch 里。写不进去时，把原因记进「设置 → 诊断信息」（diagnostics.ts），查问题时看得到。
- **什么时候会出事**：浏览器禁止网站存数据（设置里屏蔽 Cookie 和网站数据、某些隐私模式）时访问 `localStorage` 抛 SecurityError，存储满了 `setItem` 抛 QuotaExceededError。这些都被接住：读取当作没记住，写入失败时只在这次访问里记得，下次要重新选。
- **要不要改**：不用改——失败已经处理，影响只是下次要再选一次。

## `frontend/src/components/PostDetail.vue`

### 详情页封面图地址（non-null !）

```vue
v-if="hasCover"
:src="post.cover!"
```

```vue
v-if="narrow && hasCover"
:src="post.cover!"
```

- **为什么这样写**：`hasCover` 等于 `!!post.value?.cover && !coverFailed.value`。两个 `<img>` 自己的 `v-if` 都含 `hasCover`，而元素自己的 `v-if` 先于它的其他绑定求值，所以求 `:src` 时 `cover` 一定非空。
- **什么时候会出事**：不会：`v-if` 为假时 `:src` 不会求值；就算求值时是 null，也只会去掉 `src` 属性。
- **要不要改**：不用改——元素自己的 `v-if` 已经保护，运行时 `!` 不做任何事。

### 「去原文评论」链接的地址（non-null !）

```vue
:href="commentsLink"
target="_blank"
rel="noopener"
class="comments-go"
@pointerenter="ahead(commentsLink!, `评论 · ${post.title}`)"
@pointerleave="readerIntentEnd"
@focus="ahead(commentsLink!, `评论 · ${post.title}`)"
@pointerdown="ahead(commentsLink!, `评论 · ${post.title}`, 0)"
@click="openBeside($event, commentsLink!, `评论 · ${post.title}`)"
```

- **为什么这样写**：`commentsLink` 等于 `post.value?.comments_url ?? post.value?.url`，只有在还没有文章数据时才是 undefined。这些绑定都在 `<template v-if="post">` 里，而 `post.url` 的类型是必有的 `string`，所以这里一定是字符串；`!` 只是去掉类型里的 undefined。
- **什么时候会出事**：不会：这些是事件处理器，只在指针移入、获得焦点、按下、点击时执行，而那时元素已经在 `v-if="post"` 之内渲染出来了，`commentsLink` 至少是文章的原文地址。
- **要不要改**：不用改——外层 `v-if="post"` 已经保证有值。

### 相关文章读不到（.catch 忽略）

```vue
api
.related(props.id, 6)
.then((r) => {
rememberPosts(r)
related.value = r
})
.catch(() => {})
```

- **为什么这样写**：详情页下面的「相关文章」是附加内容，读不到时就不显示这一栏，不影响正文。
- **什么时候会出事**：服务器出错，或者静态网站的 `related.json` 读不到时，没有相关文章，也没有提示。
- **要不要改**：不用改。

## `frontend/src/components/Waterfall.vue`

### 观察瀑布流容器的宽度和底部哨兵（non-null !）

```vue
resizeObserver.observe(root.value!)
// …
intersectionObserver.observe(sentinel.value!)
```

- **为什么这样写**：按容器宽度决定列数，底部哨兵进入视野时加载下一页。`root`（根 div）和 `sentinel`（底部状态 div）在模板里都没有 `v-if`，到 `onMounted` 时都已赋值。
- **什么时候会出事**：不会：`onMounted` 只在挂载后执行一次。被 `<KeepAlive>` 缓存时组件不会卸载，两个元素一直都在（缓存时宽度为 0 的情况已在 ResizeObserver 回调里单独处理）。
- **要不要改**：不用改——两个元素总是存在，`!` 只是告诉 TypeScript 这一点。

## `frontend/src/views/BlogView.vue`

### 读「刷新这个博客」的回应（.json()）

```vue
const r = await res.json().catch(() => ({}))
if (r.status === 'done') {
```

- **为什么这样写**：`/api/refresh` 由后端（readers.rs）或 Cloudflare Worker 回答，正常都是 JSON；但请求也可能落到别的地方（代理的 502 HTML 页、请求体格式不对时的纯文本报错）。`.catch(() => ({}))` 把非 JSON 的回应当成空对象，最后落到 `r.error ?? 没能刷新（HTTP 状态码）` 这条提示。
- **什么时候会出事**：不会：有了 `.catch`，这个 promise 不会 reject。只有回应体正好是 JSON `null` 时，`r.status` 才会抛 TypeError，但整个 `refresh()` 外面有 try/catch，读者看到「没能刷新，稍后再试」，`refreshing` 也在 finally 里复位。后端和 Worker 都只回对象，不会回 `null`。
- **要不要改**：不用改——非 JSON 的回应已经变成一条可读的提示。

### 请求刷新后，浏览器先自己读一次（.catch 忽略）

```vue
justRead.value = (await readNow(feed, ref).catch(() => [])).filter((p) => p.sort_at > newest)
```

- **为什么这样写**：静态网站上，「刷新」只是请求下一次抓取，几分钟后网站才更新。为了马上看到新文章，浏览器同时自己读一次 RSS。读不到（对方网站不让读、代读服务出错）时，就不显示「刚读到」的部分，提示文字照常。
- **什么时候会出事**：读不到时，读者只看到「已请求，约 N 分钟后网站更新」，没有新文章预览，也看不到原因。
- **要不要改**：不用改。

## `frontend/src/views/BlogsView.vue`

### 观察博客列表底部哨兵（non-null !）

```vue
observer.observe(sentinel.value!)
```

- **为什么这样写**：底部哨兵进入视野（提前 800px）就加载下一页博客。`<div ref="sentinel">` 在根元素里、没有 `v-if`，到 `onMounted` 时已赋值。
- **什么时候会出事**：不会：`onMounted` 只在挂载后执行一次；页面被 `<KeepAlive>` 缓存（用了 `onActivated`）时也不会卸载，哨兵一直在。
- **要不要改**：不用改——元素总是存在，`!` 只是告诉 TypeScript 这一点。

## `frontend/src/views/SearchView.vue`

### 博主、标签查询失败时，其余部分照常（catch {}）

```vue
blogOffset.value += page.items.length
moreBlogs.value = !!page.next_cursor
} catch {
// The other sections still show.
} finally {
```

```vue
if (needle === q.value) tags.value = found
} catch {
// As above.
}
```

- **为什么这样写**：搜索页同时查文章、博主、标签三部分，一部分失败时其余照常显示，不让一个错误清空整页。
- **什么时候会出事**：博主或标签查询失败时那一栏是空的，看起来和「没有搜到」一样，读者不知道其实是出错了。
- **要不要改**：可以改（低）——失败时在那一栏写「暂时查不到」，或者调用 `noteProblem('搜索', e)`。

## `frontend/src/views/SettingsView.vue`

### 「在 GitHub 上提建议」「推荐给本站」的链接（non-null !）

```vue
<p v-if="suggestLink()" class="hint">
希望本站收录某个博客？<a :href="suggestLink()!" target="_blank" rel="noopener">在 GitHub 上提建议</a>
```

```vue
<a v-if="s.kind === 'feed' && suggestLink(s)" :href="suggestLink(s)!" target="_blank" rel="noopener" class="link">推荐给本站</a>
```

- **为什么这样写**：站点的仓库不在 GitHub 上时 `suggestLink` 返回 null，链接就不显示。所以先在 `v-if` 里调用一次判断（第一处是父元素 `<p>` 的 `v-if`，第二处是 `<a>` 自己的 `v-if`），再在 `:href` 里调用第二次；TypeScript 不知道两次结果相同，所以写 `!`。
- **什么时候会出事**：不会：`suggestLink` 只读 `meta.value?.repo?.url` 和参数，同一次渲染里两次调用结果一样；而且就算是 null，`:href="null"` 也只会去掉属性，不会抛错。
- **要不要改**：不用改——`v-if` 已经保护，运行时 `!` 不做任何事。

### 读者自己添加的来源的读取状态（non-null !）

```vue
<small v-if="readText(s)" :class="{ warn: readText(s)!.bad }">{{ readText(s)!.text }}</small>
```

- **为什么这样写**：`readText` 只对从文件导入、本站又没在抓取的列表（`file:` 地址）返回 null。元素自己的 `v-if` 先判断非空，再取 `.bad` 和 `.text`。
- **什么时候会出事**：不会：元素自己的 `v-if` 先求值，为假时 `:class` 和插值都不执行。`readText` 只读响应式状态（`crawled`、`reads`），渲染是同步的，三次调用之间状态不会变，所以 `v-if` 为真时后两次也一定非空。如果真在这里拿到 null，`.bad` 会抛 TypeError，又没有全局 errorHandler，整个设置页都渲染不出来，不过按上面的理由走不到这一步。
- **要不要改**：不用改——同一次渲染里结果不变，`v-if` 已经保护。如果以后 `readText` 的结果会在两次调用之间变化（依赖时间、有副作用），就改成每个来源先算一次再用。

### 兴趣画像、来源对应的博客读不到（.catch 忽略）

```vue
profile.value = hasSignals.value ? await api.interest(s).catch(() => null) : null
```

```vue
return key ? api.blogByKey(key).catch(() => null) : null
```

- **为什么这样写**：「推荐眼中的你」读不到时就不显示画像。添加来源时会查一下本站是不是已经收录了这个博客（收录了就改成关注它）；查不到就当没收录，照常加成自己的订阅。
- **什么时候会出事**：前者只是少一块说明。后者在服务器出错时，已经收录的博客会被加进「我添加的」，变成重复：同一个 RSS 由网站和浏览器各读一次。不影响使用。
- **要不要改**：不用改。

## `worker/src/index.js`

### 解析请求地址（new URL）

```js
async fetch(request, env, ctx) {
const url = new URL(request.url)
try {
```

- **为什么这样写**：路由要看 `pathname` 和查询参数；`url` 在 try 之后的 404 和 `env.ASSETS` 分支里也要用，所以放在 try 之前。
- **什么时候会出事**：不会：`request.url` 是 Workers 运行时按收到的 HTTP 请求生成的绝对地址，总能解析。
- **要不要改**：不用改——输入由运行时保证合法。

### 建表失败时重新抛出（throw）

```js
).catch((e) => {
schema = undefined
throw e
})
```

- **为什么这样写**：`schema` 缓存建表的 promise，同一个 isolate 里只建一次。失败时先清空缓存再抛出：这一次的调用者知道失败了，下一次调用会重新建表，而不是一直拿着一个已经失败的 promise。
- **什么时候会出事**：D1 暂时不可用，或数据库没绑定、没建好（部署配置错）时。`/api/sync`、`/api/readers/feeds`、`/api/refresh`、`/api/events` 里抛出的，被 fetch 处理器的 try/catch 变成这一个请求的 500 `server error`；`seen()` 在 `ctx.waitUntil` 里，失败只进日志；`scheduled` 第一行就是 `await ready(env)`，失败时这一轮什么都不做，5 分钟后下一轮重试。没有数据库时后面的清理、统计和调度（都要读写 D1）本来也做不成。
- **要不要改**：不用改——有意抛出，失败后会自动重试。

### 解析网页里指向 RSS 的链接（new URL）

```js
try {
const u = new URL(href, base)
if (/^https?:$/.test(u.protocol) && !found.some((f) => f.url === u.href)) found.push({ rank, url: u.href })
} catch {
// Not an address.
}
```

- **为什么这样写**：`/api/fetch` 拿到的是网页时，从 `<link rel="alternate">` 里找 RSS 地址；`href` 可能是相对地址，所以以页面的最终地址为基准解析。
- **什么时候会出事**：博客页面里的 `href` 写错（如 `http://`、主机名里有空格）时会抛，但就在 try/catch 里，只是跳过这一条链接。
- **要不要改**：不用改——已经接住，坏链接被忽略正是想要的结果。

### 跟随对方网站的重定向（new URL）

```js
if (hop >= 5) return failed(502, '对方网站转了太多次')
url = new URL(next, url)
```

- **为什么这样写**：请求用 `redirect: 'manual'`，自己跟随重定向，这样每一跳都先用 `allowed()` 检查目标不在内网。`Location` 可以是相对地址，所以以当前地址为基准解析。
- **什么时候会出事**：对方网站回的 `Location` 无法解析时，比如 `Location: http://`（没有主机名）、主机名里有空格、端口超过 65535。循环里只有 `fetch` 包在 try 里，这一行没有；异常一路传到 fetch 处理器的 try/catch，读者在设置页添加这个来源时看到英文的「server error」，而不是像其他失败那样的中文原因。因为异常发生在写缓存之前，这个失败也不会被缓存 10 分钟，每次重试都会再访问一次对方网站。只影响这一次请求。
- **要不要改**：可以改（低）——很少见，影响只是一条看不懂的错误信息。改法：`try { url = new URL(next, url) } catch { return failed(502, '对方网站转到了无效的地址') }`，这样读者看到原因，失败也按 `FAILURE_TTL` 缓存。

### 解析读者输入的地址（new URL）

```js
const raw = (url.searchParams.get('url') ?? '').trim()
target = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`)
} catch {
return error(400, '地址无效')
```

- **为什么这样写**：读者输入的地址可以不带 `https://`，补上后再解析。
- **什么时候会出事**：读者输入无法解析的内容（空、只有 `https://`、主机名里有空格）时会抛，但就在 try/catch 里，回 400「地址无效」，设置页显示这句话。
- **要不要改**：不用改——抛错后回 400 就是想要的行为。

### 共享缓存的键（new URL）

```js
const key = new Request(new URL(`/api/fetch-cache/${encodeURIComponent(target.href)}`, url))
```

- **为什么这样写**：Cache API 用 Request 当键；把目标地址编码进本站域名下的一个路径，读同一个 feed 的读者一刻钟内共用一次抓取。
- **什么时候会出事**：不会：基准 `url` 是已经解析好的请求地址，路径以 `/` 开头，`encodeURIComponent` 的结果只有安全字符和 `%XX`，这样拼出来的总能解析。
- **要不要改**：不用改——输入都已经是合法的。

### 读请求体（.json()）

```js
async function register(request, env, ctx) {
const body = await request.json().catch(() => null)
const feeds = (Array.isArray(body?.feeds) ? body.feeds : [])
```

```js
async function events(request, env) {
const body = await request.json().catch(() => null)
const list = Array.isArray(body?.events) ? body.events.slice(0, 200) : []
```

```js
if (!env.GITHUB_TOKEN) return error(503, '这里还不能请求刷新')
const body = await request.json().catch(() => null)
const site = typeof body?.site === 'string' ? body.site.trim().replace(/\/+$/, '') : ''
```

- **为什么这样写**：这几个接口谁都能调用，请求体可能为空或不是 JSON。`.catch(() => null)` 把这些情况当作没有内容，后面一律用 `body?.…` 读字段并检查类型。
- **什么时候会出事**：不会：有了 `.catch`，这个 promise 不会 reject。`body` 为 null 或任意 JSON 值时，后面的 `?.`、`Array.isArray`、`typeof` 都能处理，分别得到空的登记（`{ blogs: {}, added: 0 }`）、`{ counted: 0 }` 和 404「找不到这个博客」。
- **要不要改**：不用改——坏请求体已经变成正常的空结果或 404。

### 只登记公网上的 RSS 地址（new URL）

```js
try {
return allowed(new URL(f))
} catch {
return false
```

- **为什么这样写**：读者浏览器发来的订阅列表要先过滤，只登记公网上的 http(s) 地址给抓取程序。
- **什么时候会出事**：列表里有无法解析的字符串（读者手动改了同步数据、旧版本留下的坏数据）时会抛，但就在 try/catch 里，只是丢掉这一条。
- **要不要改**：不用改——已经接住，坏地址被丢掉正是想要的结果。

### 记下统计推送被拒的接口（new URL）

```js
const later = (res) => {
console.error('stats push refused', res.status, new URL(res.url).pathname)
```

- **为什么这样写**：日志里只记接口路径（如 `/repos/…/git/ref/heads/main`），一眼看出是哪一步被拒（注释说的「第一次调用就 404：token 看不到 DATA_REPO」）。
- **什么时候会出事**：不会：传进来的 `res` 都是 `github()` 里 `fetch` 返回的回应，它的 `url` 是实际请求的绝对地址（`GITHUB_API` 或 `https://api.github.com` 加路径），总能解析。如果这里真的抛了，后果会比较重：`stats_retry_at` 来不及设，异常从 `pushStats` 传到没有 try/catch 的 `scheduled`，这一轮的刷新和定时抓取调度都被跳过，而且每一轮都会重来一遍；但 `fetch` 回应的 `url` 不会是空的，所以走不到这一步。
- **要不要改**：不用改——输入总是合法的绝对地址。只有以后把自己构造的 Response（`url` 为空字符串）也传给 `later` 时，才需要改成直接记 `res.url`。

### 统计推送时读 GitHub 的回应（.json()）

```js
const ref = await gh('GET', `/git/ref/heads/${branch}`)
if (!ref.ok) return later(ref)
const head = (await ref.json()).object.sha
const parent = await gh('GET', `/git/commits/${head}`)
if (!parent.ok) return later(parent)
const tree = await gh('POST', '/git/trees', {
base_tree: (await parent.json()).tree.sha,
// …
if (!tree.ok) return later(tree)
const commit = await gh('POST', '/git/commits', {
// …
tree: (await tree.json()).sha,
// …
if (!commit.ok) return later(commit)
// …
const moved = await gh('PATCH', `/git/refs/heads/${branch}`, { sha: (await commit.json()).sha, force: false })
if (!moved.ok) return later(moved)
```

- **为什么这样写**：每小时把统计提交到备份仓库：取分支头 → 取父提交的树 → 建树 → 建提交 → 移动分支。每一步先看 `ok`，不 ok 就 `later()`（记日志，5–10 分钟后再试）；ok 了才读 JSON 取 `sha`，这些字段是 GitHub 成功回应里一定有的。
- **什么时候会出事**：`ok` 之后的 `.json()` 只有在回应体读到一半连接断了，或者 2xx 回应不是 GitHub 的 JSON（例如 `GITHUB_API` 指向了不对的替身）时才会 reject。同一段里的 `await gh(...)` 更容易出问题：`fetch` 遇到网络错误（连接被重置、DNS 失败）会直接 reject，`github()` 没有接住。不管哪种，`pushStats` 抛出后，没有 try/catch 的 `scheduled` 这一轮就停了：排在它后面的 `dispatchRefreshes(env)` 和定时抓取的 `dispatch(...)` 都不执行，`stats_retry_at` 也没设。因为 `crawled` 时间戳要到 dispatch 前才写，5 分钟后的下一轮会把统计和调度重新做一遍，所以一次网络抖动的代价是读者请求的刷新和定时抓取晚 5 分钟、统计晚一轮。如果每一轮都抛，抓取就一直不会被启动。GitHub API 一直连不上时，`dispatch` 本身也发不出去，这种情况两边一起坏。但 `pushStats` 里还有和 GitHub 无关的步骤，例如 `statsFiles` 的 D1 查询；它们要是持续出错（表变大后超时、碰上免费版每次调用的查询数上限），就会只因为统计而让定时抓取一直停着。
- **要不要改**：建议改（中）——触发条件少见，但后果是整个网站不再更新，而改起来只要一行。统计推送是附带功能，不该挡住抓取调度：在 `scheduled` 里写 `await pushStats(env).catch((e) => console.error('stats push failed', e))`（可以顺便在 catch 里设 `stats_retry_at`，免得持续出错时每 5 分钟重试一次），这样统计相关的任何异常（包括其中的 D1 查询）都只影响统计本身。

### 刷新请求核对是不是本站的博客（new URL）

```js
} else if (/^[a-z0-9.-]+(\/[^\s?#]*)?$/i.test(site)) {
// A blog of the site has its page.
const page = await env.ASSETS.fetch(new Request(new URL(`/blog/${site}/`, request.url)))
```

- **为什么这样写**：静态站里每个收录的博客都有 `/blog/<site>/` 页面；用 `ASSETS` 取一下，200 就说明是本站的博客（没有的页面按 `not_found_handling: "404-page"` 回 404）。
- **什么时候会出事**：不会：`site` 已经过了正则（开头只有字母、数字、点、横线，后面的路径里没有空白、`?`、`#`），拼成以 `/` 开头的相对路径，以合法的请求地址为基准总能解析。万一抛了，也在 fetch 处理器的 try/catch 里，只是这一次请求 500。
- **要不要改**：不用改——输入已经过正则检查，基准地址合法。

### 启动抓取失败只写日志，token 过期没人知道（人工）

```js
if (every > 0 && now - (await getState(env, 'crawled')) >= every) {
// Counted as started even if GitHub refused, so a broken token is not retried every five minutes.
await setState(env, 'crawled', now)
await dispatch(env, { crawl: 'true', scheduled: 'true' })
}
```

```js
if (!res.ok) console.error('dispatch failed', res.status, await res.text())
return res.ok
```

- **为什么这样写**：抓取由 Worker 的定时器启动。GitHub 拒绝了也算启动过，免得 token 坏掉时每 5 分钟去撞一次；原因写进 Worker 的日志。
- **什么时候会出事**：`GITHUB_TOKEN` 是细粒度 token，一般都设了过期时间。token 过期、被撤销，或者仓库改了名以后，每次 `dispatch` 都失败（401 或 404），影响有三处。定时抓取：Worker 不再启动它；好在 pages.yml 的 GitHub 定时任务会在「4 小时内没有 Worker 启动的运行」时顶上，网站照常每 3 小时更新；但公开仓库 60 天没有改动时，GitHub 会停掉定时任务，那以后就完全不抓了。读者的「刷新」：`/api/capabilities` 只看有没有设 token，按钮照常显示，点了回复「已请求，约 N 分钟后网站更新」，然后什么都不会发生；读者新加的源也要等下一次定时抓取。没设 `DATA_TOKEN` 时，统计推送用的也是这个 token，同样停了（见下面统计那条）。这些错误只出现在 Worker 的日志里，没人会去看。GitHub 会在 token 到期前发邮件，但邮件里不会说是哪个功能在用它。
- **要不要改**：建议改（中）——把最近一次 `dispatch` 的结果（状态码和时间）存进 `state`。连续失败时 `/api/capabilities` 回 `refresh: false`，并在关于页或设置页提示，比如「抓取启动失败：GitHub 返回 401」。README 里提醒给 token 设长一点的有效期，并记下到期日。

### 刷新请求先记成已发出再发，失败或被顶掉就丢了（人工）

```js
await env.DB.prepare('UPDATE refreshes SET dispatched_at = ? WHERE site IN (SELECT value FROM json_each(?))')
.bind(now, JSON.stringify(sites))
.run()
// …
await dispatch(env, { crawl: 'true', sites: sites.join(',') })
```

```js
return json({ status: 'queued', eta_minutes: Math.ceil(wait / 60) + 5 })
```

- **为什么这样写**：一批最多 30 个博客合成一次短运行，最多每 3 分钟一次、每小时 12 次。发出前先记下 `dispatched_at`，同一个博客 30 分钟内不再接受刷新。
- **什么时候会出事**：`dispatch` 的返回值没人检查。GitHub 拒绝时（token 过期、限流、5xx），这批博客已经记成「已发出」（`dispatched_at >= requested_at`），以后不会再发。读者看到的是「已请求」，30 分钟内再点是「刚请求过」。GitHub 接受了也不一定会跑：同一组里排队等待的运行会被后来的顶掉（见 pages.yml 那条），被顶掉的那一批同样不会再发。给读者的「约 N 分钟」是「等到下次能发的时间 + 5」，可一次短运行要构建、合并备份、抓取、备份、再构建、导出、部署，估计要十几分钟；就算网站更新了，已经打开的静态页也看不到（见 staticApi.ts 那条）。另外，两个刷新请求几乎同时到达时，两个 `dispatchRefreshes` 都会通过 3 分钟的检查，同一批博客会被发两次。
- **要不要改**：可以改（低）——`dispatch` 失败时把这一批的 `dispatched_at` 改回原来的值；挑博客时把最近 20 分钟内发出过的也带上，这样被顶掉的那一次能由下一次补上；ETA 按实际的运行时长估，比如从 15 分钟起。

### 统计只推今天和昨天，中断超过一天就永久缺几天（人工）

```js
for (const day of [dayOf(now - 86400), dayOf(now)]) {
```

```js
env.DB.prepare('DELETE FROM stats WHERE day < ?').bind(dayOf(now - 7 * 86400)),
```

- **为什么这样写**：只有今天和昨天的计数还会变，所以每小时只提交这两天（和读者订阅数）。以前的每一天都已经在备份仓库里，D1 只留 7 天。
- **什么时候会出事**：推送连续失败超过一天（token 过期或没有写权限、`DATA_REPO` 写错、GitHub 故障、`pushStats` 抛错）后再恢复，只会补推那时的「今天」和「昨天」。中间那几天虽然还在 D1 里，却再也不会提交，7 天后被删掉。备份仓库里就永远缺了这几天，热度和「大家在看」也少了这几天，没人会注意到。
- **要不要改**：可以改（低）——提交 D1 里还留着的所有日子中内容变了的。用一条 `WHERE day >= ?` 取出 7 天的数据，哈希也一条查询取齐，免得 D1 查询数接近每次调用 50 条的上限（现在一次定时任务最多约 39 条）。

### /api/fetch 在免费版上的两个限制（人工）

```js
const cached = await caches.default.match(key)
// …
ctx.waitUntil(caches.default.put(key, respond(a, 'public')))
```

```js
if (kind === 'page') return { feeds: await feedLinks(body, url), url: url.href }
```

- **为什么这样写**：同一个地址一刻钟内只抓一次，所有读者共用（Cache API）；拿到网页时，用 HTMLRewriter 找页面里的 RSS 链接。正文最大 4 MB。
- **什么时候会出事**：这两处都依赖 Cloudflare 的运行环境，出问题时不报错。第一，据 Cloudflare 的文档，Cache API 在 `*.workers.dev` 域名上不起作用：`put` 存不下，`match` 总是找不到。README 建议先在 workers.dev 上试；如果网站一直只用这个地址，「一刻钟一次、大家共用」就不成立，每个读者的每次预览都会去请求对方网站（只有浏览器自己按 `max-age` 缓存 15 分钟）。绑定自己的域名后才正常。第二，免费版每个请求只有 10 ms CPU。用 HTMLRewriter 解析几 MB 的网页、把 4 MB 的 feed 拼接拷贝，可能超出这个限制，请求以 1102 错误结束。失败的结果没有写进缓存，读者每重试一次就再抓一次。读者只会觉得预览慢，或者偶尔失败。
- **要不要改**：可以改（低）——README 写明要绑定自己的域名，缓存才生效（或者改用 KV 或 D1 存这一刻钟的结果）；网页超过比如 1 MB 时只看前 1 MB（RSS 链接都在 `<head>` 里）。

## `worker/wrangler.jsonc`

### 在 Cloudflare 后台改的变量，下一次发布就没了（人工）

```jsonc
"vars": {
"USER_AGENT": "blogScroll (RSS reader)",
"GITHUB_REPO": "",
"DATA_REPO": "",
"CRAWL_EVERY_HOURS": "3"
},
```

- **为什么这样写**：Worker 的变量写在这里，发布时由 wrangler 一起设置；仓库名、备份仓库和 User-Agent 由 publish.yml 用 `--var` 传入。
- **什么时候会出事**：`wrangler deploy` 默认用这个文件里的 `vars` 替换后台的变量（这里没设 `keep_vars`）：后台加的普通变量会被删掉，同名的会被改回这里的值。README 说定时抓取的间隔是「Worker 的变量 `CRAWL_EVERY_HOURS`」，站长很自然会到后台把它改成 6，或者改成 0（家里已经有 `watch` 在抓，不想让同一批博客被抓两遍）。可是每次抓取结束都会发布一次，最多 3 小时后它就悄悄变回 "3"。`GITHUB_REF`、`DATA_BRANCH`（默认分支不叫 main 时需要设）这里没有列出，在后台设成普通变量的话，下一次发布也会被删掉，Worker 退回 `main`：启动抓取收到 422，统计推向不存在的分支，只在 Worker 日志里留下错误。另外，`CRAWL_EVERY_HOURS` 写成 "3h" 这类值时，`Number()` 得到 NaN，`every > 0` 为假，等于关掉了定时抓取，也不报错。只有 Secret（`GITHUB_TOKEN`、`DATA_TOKEN`）不受影响。
- **要不要改**：建议改（中）——加上 `"keep_vars": true`，把 `CRAWL_EVERY_HOURS` 从这里拿掉（代码里已经默认 3），后台设的值就能保留下来；或者在 README 里写明这些变量只能改这个文件，或通过 GitHub 的 Variables 由 publish.yml 传入。`GITHUB_REF`、`DATA_BRANCH` 也写进文档。Worker 读到的 `CRAWL_EVERY_HOURS` 不是有效数字时，记一条日志并按 3 处理。

## `.github/workflows/pages.yml`

### 排队中的运行会被下一个顶掉（人工）

```yaml
concurrency:
# Refreshes do not wait behind a long crawl; their pushes meet in the backup's retries.
group: ${{ inputs.sites && 'pages-refresh' || 'pages-crawl' }}
cancel-in-progress: false
```

- **为什么这样写**：同一组的运行一个接一个跑，不会有两个同时改同一份数据库缓存。刷新单独一组，不用排在两个小时的大抓取后面。
- **什么时候会出事**：GitHub 的一个并发组里，只有一个运行在跑，最多一个在等。再来一个时，正在等的那个会被直接取消；`cancel-in-progress: false` 只保护正在跑的那个。一次刷新运行（构建、合并备份、抓取、备份、发布）估计要十几分钟，而 Worker 每 3 分钟就能再发一次（每小时最多 12 次）。读者稍多一点就会这样：A 在跑，B 在等，C 一来，B 就被取消了。Worker 已经把 B 那批博客记成「已发出」，不会再发（见 worker 那条），这些读者的刷新就丢了，读者新加的源也要等下一次定时抓取。抓取组也一样：大抓取在跑时，排队的定时抓取会被后来的运行（比如另一台机器发来的 `crawl=false`）顶掉，这一轮就没抓。这些只在 Actions 页面上表现为几个「cancelled」，没人会注意。
- **要不要改**：建议改（中）——Worker 每次发刷新时，把最近 20 分钟内发出过的博客也一起带上（`dispatched_at > now - 1200`），被顶掉的那批由下一次补上；或者把 `REFRESH_GAP` 调到比一次运行的时长还长。

### 第三方 action 被仓库策略禁掉时，每次运行都失败而没人知道（人工）

```yaml
- uses: actions/checkout@v7
- uses: dtolnay/rust-toolchain@stable
- uses: Swatinem/rust-cache@v2
```

- **为什么这样写**：用 `dtolnay/rust-toolchain` 装 Rust、`Swatinem/rust-cache` 缓存编译结果，是 Rust 项目的常见写法；publish.yml 里也一样。
- **什么时候会出事**：仓库的 Actions 设置（Settings → Actions → General）只允许 GitHub 自己的 action，或者只允许列出来的几个时，用到这两个 action 的运行一启动就失败，一步都不执行。这真的发生过：抓取和发布连续失败了好几天都没人发现。Worker 每 3 小时照常 dispatch，GitHub 也照常接受（回 204），所以 Worker 的日志里没有错误。失败的运行也算「4 小时内有 Worker 启动的运行」，后备的定时任务就不跑（跑了也一样失败）。网站停在最后一次成功发布的样子，关于页上的「数据生成于」没人会去看。失败的运行最多给触发它的账号发一封通知邮件，而这种通知也可能被关掉了。更糟的是，连续 7 天没有运行用到数据库缓存时，GitHub 会把缓存删掉（见 publish.yml 那条）。
- **要不要改**：建议改（中）——把这两个 action 换成 `run:` 步骤（runner 上已经有 rustup；缓存改用 GitHub 自己的 `actions/cache`，存 `~/.cargo` 和 `backend/target`），少依赖第三方。更重要的是加一个「网站多久没更新」的检查：比如 Worker 的定时任务读 `data/index.json` 的 `generated_at`，超过 12 小时就通过 `/api/capabilities` 告诉前端显示提示，或者调用 GitHub API 开一个 issue。

### job 超时和 --max-minutes 之间的余量不够（人工）

```yaml
timeout-minutes: 130
// …
run: ./target/release/feed-server crawl --sync --all --limit 200 --max-minutes 100
```

- **为什么这样写**：抓取最多跑 100 分钟，剩下 30 分钟留给构建、合并备份、备份和保存缓存；超过 130 分钟由 GitHub 结束这个 job。
- **什么时候会出事**：100 分钟只是「不再开始新一轮」的时间，正在进行的一轮还要做完（见 crawler.rs「crawl --all 可能一直转到截止时间」，可能要十几二十分钟）。rust-cache 失效时（工具链每 6 周更新一次，或者 Cargo.lock 变了），构建要十几分钟；没有数据库缓存时，要从备份仓库重建整个数据库；备份步骤推送被拒时，还要再等两轮 5–10 分钟（见 backup.rs 那条）。几项碰在一起就会超过 130 分钟，而这往往正是积压最多的时候：第一次运行、规则变了要全部重抓、缓存丢了。超时后 job 被取消，正在进行的抓取或备份被打断。如果连保存缓存的步骤也没来得及执行，下一次运行又从上一份缓存开始，同样的积压还得再来一遍。在 Actions 页面上看是一次失败的运行，网站照常发布。
- **要不要改**：可以改（低）——按 `timeout-minutes` 减去其他步骤的实际耗时来定 `--max-minutes`（比如改成 80），或者把 `timeout-minutes` 提到 150；让抓取过了截止时间就不再开始新的博客。

### 后备的定时任务把读者的刷新也当成 Worker 启动的（人工）

```yaml
recent=$(gh run list -R "$GITHUB_REPOSITORY" --workflow pages.yml --event workflow_dispatch -L 20 \
--json createdAt --jq "[.[] | select(.createdAt > \"$since\")] | length")
```

- **为什么这样写**：GitHub 自己的定时任务只是 Worker 定时器的后备：4 小时内有 Worker 启动的运行，就什么也不做，免得同一批博客被抓两次。
- **什么时候会出事**：这里数的是所有 `workflow_dispatch` 运行，不只是 Worker 定时器发的那种：读者的刷新运行（`sites`）、另一台机器发来的 `crawl=false`、手动运行，连失败了的运行都算。所以当 Worker 的定时启动出了问题、而刷新还正常时（比如 `CRAWL_EVERY_HOURS` 被改坏，见 wrangler.jsonc 那条），只要每 4 小时里有人点过一次刷新，完整的抓取就一次也不会跑，网站上只有被刷新的那几个博客在更新。
- **要不要改**：可以改（低）——只数 Worker 定时器发的运行：用 `--json createdAt,displayTitle`，筛选 `displayTitle == "Crawl (Worker timer)"`（`run-name` 已经是这样命名的）。

## `.github/workflows/publish.yml`

### 缓存和备份都拿不到时，会发布一个空网站（人工）

```yaml
key: feed-db-publish-${{ github.run_id }}
restore-keys: feed-db-
```

```yaml
- name: Merge the backup into the database
if: steps.key.outputs.ok == 'true'
continue-on-error: true
```

- **为什么这样写**：发布时用最新的数据库缓存，再合并备份仓库，所以别的抓取程序的数据也能发布出去。合并失败不挡发布，至少还有缓存里的数据。
- **什么时候会出事**：数据库缓存可能被 GitHub 删掉：7 天没有被访问（比如抓取和发布连续失败了一周，见 pages.yml 第三方 action 那条），或者仓库的缓存总量超过 10 GB。这时如果没配 `BACKUP_REPO`，或者克隆失败（部署密钥失效、GitHub 故障；这一步设了 `continue-on-error`），`feed-server export` 拿到的是一个刚建好的空数据库，照样导出、照样部署。结果网站上一篇文章都没有，直到下一次抓取跑完。没有备份仓库的话，那次抓取也是从零开始（找 RSS 要一个小时左右），以前的文章只剩各个 RSS 里还列着的最近几篇。推送代码后由 CI 触发的发布，最容易先碰上这种情况。读者一打开就能发现，但那时旧网站已经被覆盖了。
- **要不要改**：建议改（中）——数据库里没有文章，或者博客数比线上那份（`data/blogs.json`）少了一大截时，`export` 报错退出，旧网站就保留下来；工作流里，没有命中缓存、也没有成功合并备份时，直接跳过发布。
