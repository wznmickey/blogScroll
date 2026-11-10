//! What a post is about, from what its feed carries: the title, the author's
//! own categories, the excerpt and the feed's body text. Article pages are
//! never fetched for this.
//!
//! - keywords: jieba segmentation + TF-IDF, a dozen per post;
//! - tags: the author's categories (normalised, synonyms folded) plus keywords
//!   that are tags elsewhere (used by at least three blogs, or known words);
//! - topics: a fixed set of content areas, matched from tags and keywords;
//!   a blog's topics are the areas most of its posts fall into;
//! - term vectors: tags and keywords weighted by how rare they are across all
//!   posts, unit length, stored in `post_terms`. A dot product is then a
//!   cosine similarity, which gives related posts and personalised discover.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, RwLock};

use jieba_rs::{Jieba, KeywordExtract, TfIdf};
use regex::Regex;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::html;

const KEYWORDS_PER_POST: usize = 12;
const AUTO_TAGS_PER_POST: usize = 4;
const TERMS_PER_POST: usize = 24;
/// Body text beyond this adds little and costs segmentation time.
const BODY_CHARS: usize = 4000;
/// Own tags used by this many blogs become tags that keywords can turn into.
const SHARED_TAG_BLOGS: usize = 3;

pub struct Topic {
    pub id: &'static str,
    pub label: &'static str,
    /// Tag keys (lowercase, after synonym folding) that mark the topic.
    words: &'static [&'static str],
}

pub const TOPICS: &[Topic] = &[
    Topic {
        id: "dev",
        label: "编程开发",
        words: &[
            "编程",
            "代码",
            "开发",
            "程序",
            "程序员",
            "前端",
            "后端",
            "全栈",
            "算法",
            "数据结构",
            "编译",
            "编译器",
            "框架",
            "源码",
            "重构",
            "调试",
            "设计模式",
            "并发",
            "数据库",
            "接口",
            "api",
            "sdk",
            "javascript",
            "typescript",
            "python",
            "rust",
            "go",
            "java",
            "c++",
            "c#",
            "php",
            "kotlin",
            "swift",
            "vue",
            "react",
            "node.js",
            "css",
            "html",
            "sql",
            "mysql",
            "redis",
            "postgresql",
            "git",
            "github",
            "vs code",
            "leetcode",
            "正则表达式",
            "小程序",
            "flutter",
            "webassembly",
            "爬虫",
            "开源",
            "编程语言",
            "刷题",
            "软件开发",
        ],
    },
    Topic {
        id: "ai",
        label: "人工智能",
        words: &[
            "ai",
            "aigc",
            "llm",
            "chatgpt",
            "gpt",
            "claude",
            "deepseek",
            "gemini",
            "openai",
            "mcp",
            "agent",
            "rag",
            "prompt",
            "机器学习",
            "深度学习",
            "神经网络",
            "微调",
            "stable diffusion",
            "ai绘画",
            "智能体",
            "vibe coding",
            "cursor",
            "copilot",
            "提示词",
            "openclaw",
        ],
    },
    Topic {
        id: "ops",
        label: "运维网络",
        words: &[
            "linux",
            "ubuntu",
            "debian",
            "centos",
            "arch linux",
            "nixos",
            "docker",
            "kubernetes",
            "nginx",
            "caddy",
            "cloudflare",
            "cdn",
            "dns",
            "vps",
            "nas",
            "openwrt",
            "群晖",
            "homelab",
            "frp",
            "tailscale",
            "wireguard",
            "服务器",
            "运维",
            "部署",
            "内网穿透",
            "软路由",
            "自建",
            "自托管",
            "网络",
            "代理",
            "宝塔",
            "1panel",
            "云服务器",
            "备份",
            "ssl",
            "证书",
            "域名",
            "反向代理",
            "虚拟机",
            "pve",
            "树莓派",
            "shell",
            "命令行",
        ],
    },
    Topic {
        id: "security",
        label: "网络安全",
        words: &[
            "安全",
            "网络安全",
            "信息安全",
            "ctf",
            "渗透",
            "渗透测试",
            "漏洞",
            "逆向",
            "cve",
            "web安全",
            "红队",
            "攻防",
            "xss",
            "sql注入",
            "免杀",
            "应急响应",
            "靶场",
            "pwn",
            "密码学",
        ],
    },
    Topic {
        id: "web",
        label: "建站博客",
        words: &[
            "博客",
            "建站",
            "网站",
            "hexo",
            "hugo",
            "wordpress",
            "typecho",
            "halo",
            "astro",
            "vitepress",
            "butterfly",
            "主题",
            "博客美化",
            "友链",
            "评论系统",
            "静态博客",
            "seo",
            "个人网站",
            "开往",
            "十年之约",
            "rss",
            "独立博客",
            "博客搭建",
            "网站优化",
        ],
    },
    Topic {
        id: "gadget",
        label: "数码工具",
        words: &[
            "数码",
            "手机",
            "iphone",
            "ipad",
            "android",
            "mac",
            "macos",
            "ios",
            "windows",
            "电脑",
            "笔记本",
            "软件",
            "工具",
            "效率",
            "插件",
            "浏览器",
            "评测",
            "开箱",
            "键盘",
            "耳机",
            "显示器",
            "路由器",
            "硬件",
            "装机",
            "小米",
            "华为",
            "苹果",
            "chrome",
            "效率工具",
            "应用推荐",
        ],
    },
    Topic {
        id: "study",
        label: "学习学术",
        words: &[
            "学习",
            "笔记",
            "学习笔记",
            "考研",
            "高考",
            "大学",
            "数学",
            "物理",
            "化学",
            "论文",
            "学术",
            "课程",
            "保研",
            "oi",
            "acm",
            "竞赛",
            "英语",
            "考试",
            "复习",
            "期末",
            "线性代数",
            "高等数学",
            "微积分",
            "概率论",
            "研究生",
            "留学",
            "教育",
        ],
    },
    Topic {
        id: "life",
        label: "生活随笔",
        words: &[
            "生活",
            "日常",
            "随笔",
            "日记",
            "杂谈",
            "碎碎念",
            "心情",
            "感悟",
            "年终总结",
            "年度总结",
            "周记",
            "月记",
            "回忆",
            "成长",
            "家人",
            "情感",
            "人生",
            "散文",
            "闲聊",
            "生活记录",
        ],
    },
    Topic {
        id: "travel",
        label: "旅行",
        words: &[
            "旅行", "旅游", "游记", "出行", "自驾", "攻略", "风景", "徒步", "露营", "爬山", "景点", "citywalk", "出游",
        ],
    },
    Topic {
        id: "photo",
        label: "摄影",
        words: &[
            "摄影",
            "相机",
            "照片",
            "胶片",
            "拍照",
            "镜头",
            "风光摄影",
            "人像",
            "扫街",
            "后期",
            "lightroom",
        ],
    },
    Topic {
        id: "culture",
        label: "阅读影视",
        words: &[
            "读书",
            "书评",
            "书单",
            "电影",
            "影评",
            "观影",
            "电视剧",
            "纪录片",
            "音乐",
            "歌单",
            "小说",
            "文学",
            "诗",
            "诗歌",
            "历史",
            "哲学",
            "艺术",
            "书摘",
        ],
    },
    Topic {
        id: "acg",
        label: "游戏动漫",
        words: &[
            "游戏",
            "动漫",
            "番剧",
            "二次元",
            "acg",
            "galgame",
            "原神",
            "minecraft",
            "steam",
            "switch",
            "动画",
            "漫画",
            "轻小说",
            "手游",
            "星穹铁道",
            "塞尔达",
            "宝可梦",
            "明日方舟",
            "vtuber",
            "cosplay",
            "追番",
        ],
    },
    Topic {
        id: "food",
        label: "美食",
        words: &[
            "美食", "做饭", "食谱", "烹饪", "料理", "咖啡", "茶", "烘焙", "探店", "菜谱", "早餐", "火锅", "甜品",
            "厨房",
        ],
    },
    Topic {
        id: "career",
        label: "职场财经",
        words: &[
            "工作", "职场", "求职", "面试", "实习", "理财", "投资", "创业", "副业", "经济", "基金", "股票", "简历",
            "秋招", "春招", "校招", "跳槽", "离职", "打工",
        ],
    },
    Topic {
        id: "design",
        label: "设计创作",
        words: &[
            "设计",
            "ui",
            "ux",
            "字体",
            "配色",
            "figma",
            "排版",
            "插画",
            "绘画",
            "平面设计",
            "交互设计",
            "用户体验",
            "图标",
            "blender",
            "建模",
        ],
    },
    Topic {
        id: "sports",
        label: "运动健康",
        words: &[
            "运动",
            "跑步",
            "健身",
            "骑行",
            "游泳",
            "健康",
            "马拉松",
            "篮球",
            "足球",
            "羽毛球",
            "减肥",
            "睡眠",
            "养生",
            "心理健康",
        ],
    },
];

pub fn topic_label(id: &str) -> Option<&'static str> {
    TOPICS.iter().find(|t| t.id == id).map(|t| t.label)
}

/// Source kinds, with display names.
pub const KINDS: &[(&str, &str)] = &[
    ("blog", "博客"),
    ("news", "资讯"),
    ("newsletter", "周刊"),
    ("podcast", "播客"),
    ("video", "视频"),
    ("forum", "论坛"),
    ("other", "其他"),
];

pub fn kind_label(id: &str) -> &'static str {
    KINDS.iter().find(|(k, _)| *k == id).map_or("其他", |(_, l)| l)
}

/// Spellings folded together, and the display form of well-known names.
/// Keys are lowercase.
const CANONICAL: &[(&str, &str)] = &[
    ("ai", "AI"),
    ("人工智能", "AI"),
    ("aigc", "AIGC"),
    ("llm", "LLM"),
    ("大语言模型", "LLM"),
    ("大模型", "LLM"),
    ("chatgpt", "ChatGPT"),
    ("gpt", "GPT"),
    ("claude", "Claude"),
    ("deepseek", "DeepSeek"),
    ("gemini", "Gemini"),
    ("openai", "OpenAI"),
    ("mcp", "MCP"),
    ("agent", "Agent"),
    ("rag", "RAG"),
    ("prompt", "Prompt"),
    ("openclaw", "OpenClaw"),
    ("javascript", "JavaScript"),
    ("js", "JavaScript"),
    ("typescript", "TypeScript"),
    ("python", "Python"),
    ("rust", "Rust"),
    ("golang", "Go"),
    ("go语言", "Go"),
    ("java", "Java"),
    ("c++", "C++"),
    ("cpp", "C++"),
    ("c#", "C#"),
    ("php", "PHP"),
    ("kotlin", "Kotlin"),
    ("swift", "Swift"),
    ("vue", "Vue"),
    ("vue.js", "Vue"),
    ("vue3", "Vue"),
    ("react", "React"),
    ("node.js", "Node.js"),
    ("nodejs", "Node.js"),
    ("css", "CSS"),
    ("html", "HTML"),
    ("sql", "SQL"),
    ("mysql", "MySQL"),
    ("redis", "Redis"),
    ("postgresql", "PostgreSQL"),
    ("git", "Git"),
    ("github", "GitHub"),
    ("gitee", "Gitee"),
    ("vscode", "VS Code"),
    ("vs code", "VS Code"),
    ("leetcode", "LeetCode"),
    ("力扣", "LeetCode"),
    ("linux", "Linux"),
    ("ubuntu", "Ubuntu"),
    ("debian", "Debian"),
    ("centos", "CentOS"),
    ("archlinux", "Arch Linux"),
    ("arch linux", "Arch Linux"),
    ("nixos", "NixOS"),
    ("docker", "Docker"),
    ("kubernetes", "Kubernetes"),
    ("k8s", "Kubernetes"),
    ("nginx", "Nginx"),
    ("caddy", "Caddy"),
    ("cloudflare", "Cloudflare"),
    ("cdn", "CDN"),
    ("dns", "DNS"),
    ("vps", "VPS"),
    ("nas", "NAS"),
    ("openwrt", "OpenWrt"),
    ("homelab", "HomeLab"),
    ("tailscale", "Tailscale"),
    ("wireguard", "WireGuard"),
    ("ssl", "SSL"),
    ("pve", "PVE"),
    ("shell", "Shell"),
    ("ctf", "CTF"),
    ("cve", "CVE"),
    ("xss", "XSS"),
    ("pwn", "Pwn"),
    ("hexo", "Hexo"),
    ("hugo", "Hugo"),
    ("wordpress", "WordPress"),
    ("typecho", "Typecho"),
    ("halo", "Halo"),
    ("astro", "Astro"),
    ("vitepress", "VitePress"),
    ("butterfly", "Butterfly"),
    ("seo", "SEO"),
    ("rss", "RSS"),
    ("windows", "Windows"),
    ("macos", "macOS"),
    ("mac", "Mac"),
    ("ios", "iOS"),
    ("iphone", "iPhone"),
    ("ipad", "iPad"),
    ("android", "Android"),
    ("安卓", "Android"),
    ("chrome", "Chrome"),
    ("oi", "OI"),
    ("acm", "ACM"),
    ("acg", "ACG"),
    ("galgame", "Galgame"),
    ("minecraft", "Minecraft"),
    ("我的世界", "Minecraft"),
    ("steam", "Steam"),
    ("switch", "Switch"),
    ("vtuber", "VTuber"),
    ("cosplay", "Cosplay"),
    ("ui", "UI"),
    ("ux", "UX"),
    ("figma", "Figma"),
    ("blender", "Blender"),
    ("lightroom", "Lightroom"),
    ("citywalk", "Citywalk"),
    ("前端开发", "前端"),
    ("后端开发", "后端"),
    ("读书笔记", "读书"),
    ("阅读", "读书"),
    ("游戏杂谈", "游戏"),
    ("生活随笔", "随笔"),
    ("日常生活", "日常"),
    ("技术分享", "技术"),
    ("技术教程", "教程"),
];

static CANONICAL_MAP: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| CANONICAL.iter().copied().collect());

/// How a tag is shown: the canonical spelling when one is known.
pub fn tag_display(raw: &str) -> String {
    let t = raw.trim();
    CANONICAL_MAP
        .get(t.to_lowercase().as_str())
        .map_or_else(|| t.to_string(), |c| c.to_string())
}

/// Matching key of a tag or keyword: canonical spelling, lowercased.
pub fn tag_key(raw: &str) -> String {
    tag_display(raw).to_lowercase()
}

/// Words that are never useful keywords. jieba's IDF already discounts most
/// common words; these are the ones that still slip through in blog prose.
const STOP_WORDS: &[&str] = &[
    "时候",
    "时间",
    "问题",
    "东西",
    "方法",
    "方式",
    "事情",
    "地方",
    "情况",
    "内容",
    "部分",
    "感觉",
    "样子",
    "大家",
    "自己",
    "我们",
    "你们",
    "他们",
    "今天",
    "明天",
    "昨天",
    "现在",
    "之前",
    "之后",
    "以后",
    "以前",
    "最后",
    "开始",
    "结果",
    "原因",
    "目的",
    "过程",
    "作用",
    "意思",
    "一下",
    "一些",
    "一种",
    "一个",
    "这个",
    "那个",
    "什么",
    "怎么",
    "可以",
    "需要",
    "进行",
    "使用",
    "通过",
    "没有",
    "已经",
    "还是",
    "就是",
    "但是",
    "因为",
    "所以",
    "如果",
    "然后",
    "这样",
    "那么",
    "非常",
    "比较",
    "一直",
    "一样",
    "一起",
    "一点",
    "觉得",
    "知道",
    "发现",
    "希望",
    "感谢",
    "文章",
    "本文",
    "博主",
    "作者",
    "读者",
    "朋友们",
    "小伙伴",
    "链接",
    "地址",
    "图片",
    "截图",
    "页面",
    "点击",
    "下载",
    "评论",
    "留言",
    "更新",
    "版本",
    "首先",
    "其次",
    "总之",
    "其实",
    "比如",
    "例如",
    "方面",
    "所有",
    "同时",
    "另外",
    "目前",
    "当前",
    "最近",
    "最终",
    "之间",
    "这里",
    "那里",
    "这边",
    "每天",
    "一次",
    "两个",
    "几个",
    "全文",
    "原文",
    "阅读",
    "继续",
    "教程",
    "介绍",
    "分享",
    "推荐",
    "相关",
    "功能",
    "用户",
    "系统",
    "数据",
    "文件",
    "配置",
    "设置",
    "https",
    "http",
    "www",
    "com",
    "cn",
    "net",
    "org",
    "html",
    "htm",
    "php",
    "png",
    "jpg",
    "jpeg",
    "webp",
    "gif",
    "svg",
    "img",
    "src",
    "alt",
    "nbsp",
    "amp",
    "quot",
    "div",
    "span",
    "class",
    "style",
    "px",
    "the",
    "and",
    "you",
    "your",
    "this",
    "that",
    "with",
    "for",
    "are",
    "was",
    "will",
    "can",
    "not",
    "have",
    "has",
    "but",
    "they",
    "their",
    "there",
    "what",
    "when",
    "how",
    "why",
    "who",
    "use",
    "using",
    "used",
    "get",
    "one",
    "all",
    "just",
    "like",
    "also",
    "more",
    "some",
    "any",
    "new",
    "its",
    "into",
    "out",
    "about",
    "from",
    "such",
    "only",
    "other",
    "than",
    "then",
    "them",
    "these",
    "those",
    "been",
    "being",
    "were",
    "would",
    "should",
    "could",
    "may",
    "might",
    "must",
    "our",
    "my",
    "me",
    "his",
    "her",
    "she",
    "he",
    "it",
    "an",
    "of",
    "to",
    "in",
    "is",
    "on",
    "as",
    "at",
    "by",
    "or",
    "be",
    "so",
    "if",
    "no",
    "do",
    "does",
    "did",
    "done",
    "go",
    "let",
    "const",
    "var",
    "function",
    "return",
    "true",
    "false",
    "null",
    "none",
    "test",
    "example",
    "post",
    "posts",
    "blog",
    "page",
    "here",
    "now",
    "day",
    "year",
    "time",
    "way",
    "make",
    "made",
    "first",
    "last",
];

static STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| STOP_WORDS.iter().copied().collect());
static URLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)https?://\S+|www\.\S+|\S+@\S+\.\w+").unwrap());

struct Analyzer {
    jieba: Jieba,
    tfidf: TfIdf,
}

static ANALYZER: LazyLock<Analyzer> = LazyLock::new(|| {
    let mut jieba = Jieba::new();
    // Domain words, so "内网穿透" or "大模型" stay one token.
    for w in TOPICS
        .iter()
        .flat_map(|t| t.words)
        .chain(CANONICAL.iter().map(|(k, _)| k))
    {
        if !w.is_ascii() {
            jieba.add_word(w, Some(2000), Some("nz"));
        }
    }
    Analyzer {
        jieba,
        tfidf: TfIdf::default(),
    }
});

/// Part-of-speech tags kept as keyword candidates: nouns, names, places,
/// organisations, other proper nouns, verbal nouns, idioms, English words.
fn allowed_pos() -> Vec<String> {
    ["n", "nr", "nrt", "nrfg", "ns", "nt", "nz", "vn", "l", "j", "eng"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Keywords with weights scaled so the strongest is 1.0, at most a dozen.
/// The title and categories count more than body text.
pub fn keywords(title: &str, tags: &[String], excerpt: &str, body_html: Option<&str>) -> Vec<(String, f32)> {
    let body: String = body_html
        .map(html::to_text)
        .unwrap_or_default()
        .chars()
        .take(BODY_CHARS)
        .collect();
    let body = if body.contains(excerpt.trim_end_matches('…')) {
        body
    } else {
        format!("{excerpt} {body}")
    };
    let tags = tags.join(" ");
    let text = format!("{title}。{title}。{title}。{tags}。{tags}。{body}");
    let text = URLS.replace_all(&text, " ");

    let a = &*ANALYZER;
    let found = a
        .tfidf
        .extract_keywords(&a.jieba, &text, KEYWORDS_PER_POST * 3, allowed_pos());
    let mut out: Vec<(String, f32)> = Vec::new();
    for k in found {
        let word = k.keyword.trim();
        let key = tag_key(word);
        let useless = STOP.contains(key.as_str())
            || key.chars().count() < 2
            || key.chars().count() > 20
            || key.chars().all(|c| c.is_ascii_digit() || c.is_ascii_punctuation())
            || (key.chars().any(|c| c.is_ascii_digit()) && !CANONICAL_MAP.contains_key(key.as_str()));
        if useless || out.iter().any(|(w, _)| tag_key(w) == key) {
            continue;
        }
        out.push((tag_display(word), k.weight as f32));
        if out.len() == KEYWORDS_PER_POST {
            break;
        }
    }
    let max = out.iter().map(|(_, w)| *w).fold(0.0, f32::max);
    if max > 0.0 {
        for (_, w) in &mut out {
            *w = (*w / max * 1000.0).round() / 1000.0;
        }
    }
    out
}

/// Corpus statistics used to derive tags and weight terms.
#[derive(Default)]
pub struct Corpus {
    posts: usize,
    /// Posts containing each term key.
    df: HashMap<String, u32>,
    /// Keys a keyword may be promoted to a tag for.
    known_tags: HashSet<String>,
    /// The spelling authors use most for each tag key ("web" → "Web").
    labels: HashMap<String, String>,
}

impl Corpus {
    /// Terms seen in fewer than four posts are treated as four, so one-off
    /// noise words cannot dominate a vector.
    fn idf(&self, key: &str) -> f32 {
        let df = self.df.get(key).copied().unwrap_or(0).max(4) as f32;
        (1.0 + (self.posts as f32 + 1.0) / (df + 1.0)).ln()
    }

    fn is_known_tag(&self, key: &str) -> bool {
        (self.known_tags.contains(key) || TOPIC_WORDS.contains_key(key)) && !GENERIC.contains(key)
    }

    fn label(&self, key: &str, word: &str) -> String {
        self.labels.get(key).cloned().unwrap_or_else(|| tag_display(word))
    }
}

/// Fine as an author's own category, too vague to add on a post's behalf:
/// "记录" is as likely "排查记录" as a diary entry.
static GENERIC: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "记录", "博客", "工具", "生活", "部署", "开源", "代码", "网站", "笔记", "项目", "视频", "指南", "免费", "日志",
        "技术", "主题", "插件", "教程", "分享", "总结", "学习", "日常", "随笔", "思考", "经验", "方法", "问题", "工作",
        "软件", "设计", "安全", "网络", "系统", "数据", "模型", "应用", "服务", "开发", "程序", "图片", "效率", "测试",
        "体验", "折腾", "随想", "杂谈", "心得", "资源", "推荐", "实践", "个人", "优化", "web", "api",
    ]
    .into_iter()
    .collect()
});

static TOPIC_WORDS: LazyLock<HashMap<&'static str, Vec<&'static str>>> = LazyLock::new(|| {
    let mut m: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    for t in TOPICS {
        for w in t.words {
            m.entry(*w).or_default().push(t.id);
        }
    }
    m
});

static CORPUS: LazyLock<RwLock<Arc<Corpus>>> = LazyLock::new(Default::default);

pub fn corpus() -> Arc<Corpus> {
    CORPUS.read().unwrap().clone()
}

#[derive(Debug, Default, PartialEq)]
pub struct Derived {
    pub auto_tags: Vec<String>,
    pub topics: Vec<&'static str>,
    /// (key, weight, is_tag); unit length.
    pub terms: Vec<(String, f32, bool)>,
    /// Display form of each tag key.
    pub labels: Vec<(String, String)>,
}

/// Tags, topics and the term vector of one post.
pub fn derive(own_tags: &[String], keywords: &[(String, f32)], corpus: &Corpus) -> Derived {
    let own: Vec<(String, String)> = own_tags.iter().map(|t| (tag_key(t), tag_display(t))).collect();
    let mut auto_tags: Vec<(String, String)> = Vec::new();
    for (word, w) in keywords {
        let key = tag_key(word);
        if *w >= 0.15
            && corpus.is_known_tag(&key)
            && !own.iter().any(|(k, _)| *k == key)
            && auto_tags.len() < AUTO_TAGS_PER_POST
        {
            let label = corpus.label(&key, word);
            auto_tags.push((key, label));
        }
    }

    // Raw weights: own tags 1, keywords (and tags made from them) their score.
    let mut raw: HashMap<String, (f32, bool)> = HashMap::new();
    for (k, _) in &own {
        raw.insert(k.clone(), (1.0, true));
    }
    for (word, w) in keywords {
        let key = tag_key(word);
        let is_tag = auto_tags.iter().any(|(k, _)| *k == key);
        let e = raw.entry(key).or_insert((0.0, false));
        e.0 = e.0.max(*w);
        e.1 |= is_tag;
    }

    let mut scores: HashMap<&'static str, f32> = HashMap::new();
    for (key, (w, _)) in &raw {
        for topic in TOPIC_WORDS.get(key.as_str()).into_iter().flatten() {
            *scores.entry(topic).or_default() += w;
        }
    }
    let mut ranked: Vec<(&'static str, f32)> = scores.into_iter().filter(|(_, s)| *s >= 0.5).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    let best = ranked.first().map_or(0.0, |(_, s)| *s);
    let topics = ranked
        .into_iter()
        .filter(|(_, s)| *s >= best * 0.5)
        .take(2)
        .map(|(t, _)| t)
        .collect();

    let mut terms: Vec<(String, f32, bool)> = raw
        .into_iter()
        .map(|(k, (w, is_tag))| {
            let idf = corpus.idf(&k);
            (k, w * idf, is_tag)
        })
        .collect();
    terms.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    terms.truncate(TERMS_PER_POST);
    let norm = terms.iter().map(|(_, w, _)| w * w).sum::<f32>().sqrt();
    if norm > 0.0 {
        for t in &mut terms {
            t.1 = (t.1 / norm * 10000.0).round() / 10000.0;
        }
    }

    let labels = own.iter().chain(auto_tags.iter()).cloned().collect();
    Derived {
        auto_tags: auto_tags.into_iter().map(|(_, d)| d).collect(),
        topics,
        terms,
        labels,
    }
}

/// Writes a post's derived data, replacing its old terms.
pub async fn save_derived(conn: &mut sqlx::SqliteConnection, post_id: i64, d: &Derived) -> sqlx::Result<()> {
    sqlx::query("UPDATE posts SET auto_tags = ?, topics = ? WHERE id = ?")
        .bind(serde_json::to_string(&d.auto_tags).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&d.topics).unwrap_or_else(|_| "[]".into()))
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM post_terms WHERE post_id = ?")
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    for (term, weight, is_tag) in &d.terms {
        sqlx::query("INSERT INTO post_terms (post_id, term, weight, is_tag) VALUES (?, ?, ?, ?)")
            .bind(post_id)
            .bind(term)
            .bind(weight)
            .bind(is_tag)
            .execute(&mut *conn)
            .await?;
    }
    for (term, label) in &d.labels {
        sqlx::query("INSERT OR IGNORE INTO tag_labels (term, label) VALUES (?, ?)")
            .bind(term)
            .bind(label)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

#[derive(Debug, Serialize, serde::Deserialize, Clone, PartialEq)]
pub struct BlogTopic {
    pub topic: String,
    pub share: f32,
}

/// A blog's areas: those at least a quarter of its classified posts fall into.
pub async fn update_blog_topics(pool: &SqlitePool, blog_ids: &[i64]) -> sqlx::Result<()> {
    for &id in blog_ids {
        let rows: Vec<String> = sqlx::query_scalar("SELECT topics FROM posts WHERE blog_id = ? AND topics <> '[]'")
            .bind(id)
            .fetch_all(pool)
            .await?;
        let topics = blog_topics(&rows);
        sqlx::query("UPDATE blogs SET topics = ? WHERE id = ?")
            .bind(serde_json::to_string(&topics).unwrap_or_else(|_| "[]".into()))
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

fn blog_topics(post_topics: &[String]) -> Vec<BlogTopic> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for json in post_topics {
        let list: Vec<String> = serde_json::from_str(json).unwrap_or_default();
        for t in list {
            *counts.entry(t).or_default() += 1;
        }
    }
    let n = post_topics.len();
    let mut out: Vec<BlogTopic> = counts
        .into_iter()
        .filter(|(_, c)| *c * 4 >= n && (*c >= 2 || n < 3))
        .map(|(topic, c)| BlogTopic {
            topic,
            share: ((c as f32 / n.max(1) as f32) * 100.0).round() / 100.0,
        })
        .collect();
    out.sort_by(|a, b| b.share.total_cmp(&a.share).then_with(|| a.topic.cmp(&b.topic)));
    out.truncate(3);
    out
}

#[derive(Debug, Default)]
pub struct RetagStats {
    pub posts: usize,
    pub keyworded: usize,
    pub tagged: usize,
    pub blogs: usize,
}

#[derive(sqlx::FromRow)]
struct PostText {
    id: i64,
    blog_id: i64,
    title: String,
    excerpt: String,
    tags: String,
    keywords: String,
}

/// Recomputes corpus statistics, every post's tags, topics and term vector,
/// and every blog's topics. Posts that have no keywords yet (stored before
/// analysis existed) get them from their title, categories and excerpt.
pub async fn retag_all(pool: &SqlitePool) -> anyhow::Result<RetagStats> {
    let rows: Vec<PostText> = sqlx::query_as("SELECT id, blog_id, title, excerpt, tags, keywords FROM posts")
        .fetch_all(pool)
        .await?;
    let (posts, stats) = tokio::task::spawn_blocking(move || prepare(rows)).await?;

    let corpus = Arc::new(build_corpus(&posts));
    let mut stats = stats;
    for chunk in posts.chunks(500) {
        let mut tx = pool.begin().await?;
        for p in chunk {
            let d = derive(&p.tags, &p.keywords, &corpus);
            if p.keywords_changed {
                sqlx::query("UPDATE posts SET keywords = ? WHERE id = ?")
                    .bind(serde_json::to_string(&p.keywords)?)
                    .bind(p.id)
                    .execute(&mut *tx)
                    .await?;
            }
            stats.tagged += usize::from(!d.auto_tags.is_empty() || !p.tags.is_empty());
            save_derived(&mut tx, p.id, &d).await?;
        }
        tx.commit().await?;
    }
    let labels = preferred_labels(&posts);
    let mut tx = pool.begin().await?;
    for (term, label) in &labels {
        sqlx::query(
            "INSERT INTO tag_labels (term, label) VALUES (?, ?) ON CONFLICT(term) DO UPDATE SET label = excluded.label",
        )
        .bind(term)
        .bind(label)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let blogs: Vec<i64> = sqlx::query_scalar("SELECT id FROM blogs").fetch_all(pool).await?;
    update_blog_topics(pool, &blogs).await?;
    stats.blogs = blogs.len();
    *CORPUS.write().unwrap() = corpus;
    Ok(stats)
}

struct Prepared {
    id: i64,
    blog_id: i64,
    tags: Vec<String>,
    keywords: Vec<(String, f32)>,
    keywords_changed: bool,
}

fn prepare(rows: Vec<PostText>) -> (Vec<Prepared>, RetagStats) {
    let mut stats = RetagStats {
        posts: rows.len(),
        ..Default::default()
    };
    let posts = rows
        .into_iter()
        .map(|r| {
            let tags: Vec<String> = serde_json::from_str(&r.tags).unwrap_or_default();
            let stored: Vec<(String, f32)> = serde_json::from_str(&r.keywords).unwrap_or_default();
            let changed = stored.is_empty();
            let keywords = if changed {
                stats.keyworded += 1;
                keywords(&r.title, &tags, &r.excerpt, None)
            } else {
                stored
            };
            Prepared {
                id: r.id,
                blog_id: r.blog_id,
                tags,
                keywords,
                keywords_changed: changed,
            }
        })
        .collect();
    (posts, stats)
}

fn build_corpus(posts: &[Prepared]) -> Corpus {
    let mut df: HashMap<String, u32> = HashMap::new();
    let mut tag_blogs: HashMap<String, HashSet<i64>> = HashMap::new();
    for p in posts {
        let mut keys: HashSet<String> = p.keywords.iter().map(|(w, _)| tag_key(w)).collect();
        for t in &p.tags {
            let k = tag_key(t);
            tag_blogs.entry(k.clone()).or_default().insert(p.blog_id);
            keys.insert(k);
        }
        for k in keys {
            *df.entry(k).or_default() += 1;
        }
    }
    Corpus {
        posts: posts.len(),
        df,
        known_tags: tag_blogs
            .into_iter()
            .filter(|(_, b)| b.len() >= SHARED_TAG_BLOGS)
            .map(|(k, _)| k)
            .collect(),
        labels: preferred_labels(posts).into_iter().collect(),
    }
}

/// The most used spelling of each own tag (canonical names win).
fn preferred_labels(posts: &[Prepared]) -> Vec<(String, String)> {
    let mut seen: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for t in posts.iter().flat_map(|p| &p.tags) {
        *seen.entry(tag_key(t)).or_default().entry(tag_display(t)).or_default() += 1;
    }
    seen.into_iter()
        .filter_map(|(k, spellings)| {
            let best = spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
            Some((k, best.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_fold_synonyms_and_casing() {
        assert_eq!(tag_display("docker"), "Docker");
        assert_eq!(tag_display("人工智能"), "AI");
        assert_eq!(tag_key("大模型"), "llm");
        assert_eq!(tag_display("旅行"), "旅行");
        assert_eq!(tag_display("SomeNewThing"), "SomeNewThing");
    }

    #[test]
    fn keywords_come_from_title_tags_and_text() {
        let kw = keywords(
            "用 Tailscale 子网路由打通内网",
            &["Linux".into()],
            "Tailscale 子网路由可以打通内外网，让外网的设备可以通过内网 IP 访问内网的服务。",
            Some("<p>我最开始用 LXC 容器部署 Tailscale，然后配置了子网路由和内网穿透。</p>"),
        );
        let words: Vec<&str> = kw.iter().map(|(w, _)| w.as_str()).collect();
        assert!(words.contains(&"Tailscale"), "{words:?}");
        assert!(words.contains(&"内网穿透") || words.contains(&"内网"), "{words:?}");
        assert!(
            !words.iter().any(|w| STOP.contains(w.to_lowercase().as_str())),
            "{words:?}"
        );
        assert_eq!(kw[0].1, 1.0);
        assert!(kw.len() <= KEYWORDS_PER_POST);
    }

    #[test]
    fn derive_tags_topics_and_unit_vectors() {
        let corpus = Corpus {
            posts: 100,
            df: [
                ("docker".to_string(), 10),
                ("随笔".to_string(), 60),
                ("容器".to_string(), 30),
            ]
            .into_iter()
            .collect(),
            known_tags: ["随笔".to_string()].into_iter().collect(),
            labels: HashMap::new(),
        };
        let d = derive(
            &["随笔".into()],
            &[("Docker".into(), 1.0), ("容器".into(), 0.6), ("Nginx".into(), 0.1)],
            &corpus,
        );
        assert_eq!(d.auto_tags, ["Docker"], "known words become tags; weak ones do not");
        assert_eq!(d.topics, ["ops", "life"]);
        let norm: f32 = d.terms.iter().map(|(_, w, _)| w * w).sum();
        assert!((norm - 1.0).abs() < 1e-3);
        assert_eq!(d.terms[0].0, "docker", "rarer terms weigh more");
        assert!(d.terms.iter().any(|(k, _, tag)| k == "随笔" && *tag));
        assert!(d.terms.iter().any(|(k, _, tag)| k == "容器" && !*tag));
    }

    #[test]
    fn blog_topics_need_a_share() {
        let posts: Vec<String> = [
            "[\"dev\"]",
            "[\"dev\",\"ai\"]",
            "[\"dev\"]",
            "[\"life\"]",
            "[\"travel\"]",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let t = blog_topics(&posts);
        let ids: Vec<&str> = t.iter().map(|t| t.topic.as_str()).collect();
        assert_eq!(ids, ["dev"]);
        assert_eq!(t[0].share, 0.6);
    }
}
