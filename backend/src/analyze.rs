//! What a post is about, from what its feed carries: the title, the author's
//! own categories, the excerpt and the feed's body text. Article pages are
//! never fetched for this.
//!
//! - keywords: jieba segmentation + TF-IDF, a dozen per post;
//! - tags: the author's categories (normalised, synonyms folded) plus keywords
//!   that are tags elsewhere (used by at least three blogs, or known words);
//! - category: exactly one per post, from a two-level list (`tech`,
//!   `tech/ai`, `travel`, ... or `other`). Word lists classify the clear
//!   cases; a naive Bayes classifier trained on those places the rest, with
//!   the blog's usual mix as the prior. A blog's categories are the
//!   top-level ones most of its posts fall into;
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

/// A top-level category. Every post has exactly one category: a top-level
/// one, one of its sub-categories (`tech/ai`), or `other`.
pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
}

pub const OTHER: &str = "other";

/// In channel order.
pub const CATEGORIES: &[Category] = &[
    Category {
        id: "tech",
        label: "技术",
    },
    Category {
        id: "gadget",
        label: "数码",
    },
    Category {
        id: "life",
        label: "生活",
    },
    Category {
        id: "travel",
        label: "旅行",
    },
    Category {
        id: "photo",
        label: "摄影",
    },
    Category {
        id: "culture",
        label: "文娱",
    },
    Category {
        id: "study",
        label: "学习",
    },
    Category {
        id: "career",
        label: "职场财经",
    },
    Category {
        id: "design",
        label: "设计创作",
    },
    Category {
        id: OTHER,
        label: "其他",
    },
];

/// The finest level a post is classified at: a sub-category, or a top-level
/// category that has none (`id == parent`).
pub struct Topic {
    pub id: &'static str,
    pub parent: &'static str,
    pub label: &'static str,
    /// Tag keys (lowercase, after synonym folding) that mark the topic.
    words: &'static [&'static str],
}

impl Topic {
    /// The category id: `tech/ai`, or `travel` for a category without sub-categories.
    pub fn path(&self) -> String {
        if self.id == self.parent {
            self.id.to_string()
        } else {
            format!("{}/{}", self.parent, self.id)
        }
    }

    fn is_sub(&self) -> bool {
        self.id != self.parent
    }
}

pub const TOPICS: &[Topic] = &[
    Topic {
        id: "dev",
        parent: "tech",
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
            "golang",
            "c语言",
            "cpp",
            "spring",
            "spring boot",
            "mybatis",
            "django",
            "flask",
            "fastapi",
            "next.js",
            "nuxt",
            "tailwind",
            "vite",
            "npm",
            "lua",
            "ruby",
            "dart",
            "unity",
            "游戏开发",
            "嵌入式",
            "单片机",
            "stm32",
            "esp32",
            "arduino",
            "电路",
            "bug",
        ],
    },
    Topic {
        id: "ai",
        parent: "tech",
        label: "AI",
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
            "大模型",
            "ollama",
            "kimi",
            "qwen",
            "通义千问",
            "豆包",
            "midjourney",
            "comfyui",
            "transformer",
            "claude code",
            "codex",
            "ai编程",
            "slam",
            "计算机视觉",
        ],
    },
    Topic {
        id: "ops",
        parent: "tech",
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
            "vercel",
            "netlify",
            "edgeone",
            "github actions",
            "k3s",
            "systemd",
            "nextcloud",
            "jellyfin",
            "alist",
            "syncthing",
            "webdav",
            "home assistant",
            "监控",
        ],
    },
    Topic {
        id: "security",
        parent: "tech",
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
            "隐私",
            "加密",
            "木马",
            "钓鱼",
            "getshell",
            "反序列化",
            "提权",
            "webshell",
            "黑客",
        ],
    },
    Topic {
        id: "web",
        parent: "tech",
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
        parent: "gadget",
        label: "数码",
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
            "电池",
            "拆机",
            "充电器",
            "汽车",
            "电动车",
            "智能家居",
            "esim",
            "手机卡",
            "流量卡",
            "运营商",
            "平板",
            "智能手表",
            "kindle",
            "特斯拉",
            "输入法",
        ],
    },
    Topic {
        id: "study",
        parent: "study",
        label: "学习",
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
            "题解",
            "洛谷",
            "luogu",
            "codeforces",
            "atcoder",
            "noip",
            "信息学",
            "生物",
            "科普",
            "实验报告",
            "高中",
            "初中",
            "中考",
            "毕业设计",
            "托福",
            "雅思",
            "四六级",
            "外语",
            "单词",
        ],
    },
    Topic {
        id: "daily",
        parent: "life",
        label: "随笔",
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
            "闲聊",
            "生活记录",
            "daily",
            "碎语",
            "杂记",
            "近况",
            "生日",
            "过年",
            "春节",
        ],
    },
    Topic {
        id: "travel",
        parent: "travel",
        label: "旅行",
        words: &[
            "旅行",
            "旅游",
            "游记",
            "出行",
            "自驾",
            "攻略",
            "风景",
            "徒步",
            "露营",
            "爬山",
            "景点",
            "citywalk",
            "出游",
            "旅途",
            "游玩",
            "机票",
            "酒店",
            "民宿",
            "签证",
            "自由行",
            "旅拍",
            "度假",
            "古镇",
            "户外",
        ],
    },
    Topic {
        id: "photo",
        parent: "photo",
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
            "相册",
            "拍摄",
            "随手拍",
            "街拍",
            "富士",
            "索尼",
            "尼康",
            "佳能",
            "徕卡",
            "航拍",
            "无人机",
            "构图",
        ],
    },
    Topic {
        id: "reading",
        parent: "culture",
        label: "书影音",
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
            "阅读",
            "看书",
            "读后感",
            "观后感",
            "散文",
            "歌",
            "歌曲",
            "专辑",
            "演唱会",
            "话剧",
            "动画电影",
        ],
    },
    Topic {
        id: "acg",
        parent: "culture",
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
            "新番",
            "音游",
            "舞萌",
            "maimai",
            "机厅",
            "街机",
            "ps5",
            "xbox",
            "任天堂",
            "独立游戏",
            "崩坏",
            "鸣潮",
            "绝区零",
        ],
    },
    Topic {
        id: "food",
        parent: "life",
        label: "美食",
        words: &[
            "美食",
            "做饭",
            "食谱",
            "烹饪",
            "料理",
            "咖啡",
            "茶",
            "烘焙",
            "探店",
            "菜谱",
            "早餐",
            "火锅",
            "甜品",
            "厨房",
            "好吃",
            "烧烤",
            "小吃",
            "餐厅",
            "下厨",
            "家常菜",
            "奶茶",
            "啤酒",
            "红酒",
            "夜宵",
            "外卖",
            "零食",
            "蛋糕",
            "饺子",
            "美味",
            "食堂",
            "厨艺",
            "面包",
        ],
    },
    Topic {
        id: "career",
        parent: "career",
        label: "职场财经",
        words: &[
            "工作",
            "职场",
            "求职",
            "面试",
            "实习",
            "理财",
            "投资",
            "创业",
            "副业",
            "经济",
            "基金",
            "股票",
            "简历",
            "秋招",
            "春招",
            "校招",
            "跳槽",
            "离职",
            "打工",
            "工资",
            "薪资",
            "加班",
            "裁员",
            "找工作",
            "上班",
            "同事",
            "绩效",
            "晋升",
            "记账",
            "保险",
            "买房",
            "房贷",
        ],
    },
    Topic {
        id: "design",
        parent: "design",
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
            "创作",
            "原创",
            "手工",
            "手作",
            "画画",
            "手绘",
            "板绘",
            "海报",
            "logo",
            "设定",
        ],
    },
    Topic {
        id: "sports",
        parent: "life",
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
            "医院",
            "生病",
            "住院",
            "手术",
            "体检",
            "感冒",
            "锻炼",
            "瑜伽",
            "滑雪",
            "网球",
            "乒乓球",
            "攀岩",
            "训练",
            "体重",
        ],
    },
];

pub fn top_category(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id == id)
}

/// The top-level part of a category id (`tech/ai` → `tech`).
pub fn top_of(category: &str) -> &str {
    category.split('/').next().unwrap_or(category)
}

/// A category id's own label and, for a sub-category, its parent; `None`
/// for ids that do not exist.
pub fn category(id: &str) -> Option<(&'static str, Option<&'static Category>)> {
    match id.split_once('/') {
        None => top_category(id).map(|c| (c.label, None)),
        Some((parent, sub)) => {
            let t = TOPICS
                .iter()
                .find(|t| t.is_sub() && t.parent == parent && t.id == sub)?;
            Some((t.label, top_category(parent)))
        }
    }
}

/// Sub-categories of a top-level category.
pub fn sub_categories(parent: &str) -> impl Iterator<Item = &'static Topic> {
    TOPICS.iter().filter(move |t| t.is_sub() && t.parent == parent)
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
    let t = crate::clean::tag_prefix_free(raw.trim());
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

/// Corpus statistics used to derive tags and weight terms, and the category
/// classifier trained on the corpus.
#[derive(Default)]
pub struct Corpus {
    posts: usize,
    /// Posts containing each term key.
    df: HashMap<String, u32>,
    /// Keys a keyword may be promoted to a tag for.
    known_tags: HashSet<String>,
    /// The spelling authors use most for each tag key ("web" → "Web").
    labels: HashMap<String, String>,
    classifier: Classifier,
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

/// Word → the topics (indexes into `TOPICS`) it marks.
static TOPIC_WORDS: LazyLock<HashMap<&'static str, Vec<usize>>> = LazyLock::new(|| {
    let mut m: HashMap<&'static str, Vec<usize>> = HashMap::new();
    for (i, t) in TOPICS.iter().enumerate() {
        for w in t.words {
            m.entry(*w).or_default().push(i);
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
    /// `tech/ai`, `travel` or `other`; see `CATEGORIES`.
    pub category: String,
    /// (key, weight, is_tag); unit length.
    pub terms: Vec<(String, f32, bool)>,
    /// Display form of each tag key.
    pub labels: Vec<(String, String)>,
    /// Adult content (see `is_nsfw`); set by the caller, which has the title.
    pub nsfw: bool,
}

/// Words that mark adult content in a title or the author's tags. Latin ones
/// count as whole words only ("r18", not "r1800").
const NSFW_WORDS: &[&str] = &[
    "nsfw",
    "r18",
    "r-18",
    "hentai",
    "porn",
    "18禁",
    "成人向",
    "成人内容",
    "色情",
    "涩图",
    "色图",
    "里番",
    "工口",
    "エロ",
];

/// Whether a post is adult content (R18 / NSFW), hidden unless a reader asks.
pub fn is_nsfw(title: &str, tags: &[String]) -> bool {
    let has = |text: &str| {
        let text = text.to_lowercase();
        NSFW_WORDS.iter().any(|w| {
            if !w.is_ascii() {
                return text.contains(w);
            }
            text.match_indices(w).any(|(i, _)| {
                let before = text[..i].chars().next_back();
                let after = text[i + w.len()..].chars().next();
                !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
            })
        })
    };
    has(title) || tags.iter().any(|t| has(t))
}

/// Term key → (raw weight, is a tag): own tags weigh 1, keywords their score.
type RawTerms = HashMap<String, (f32, bool)>;

struct Features {
    /// (key, display) of the author's own tags, and of tags made from keywords.
    own: Vec<(String, String)>,
    auto_tags: Vec<(String, String)>,
    raw: RawTerms,
}

fn features(own_tags: &[String], keywords: &[(String, f32)], corpus: &Corpus) -> Features {
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

    let mut raw: RawTerms = HashMap::new();
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
    Features { own, auto_tags, raw }
}

/// Tags, category and term vector of one post of blog `blog_id`.
pub fn derive(blog_id: i64, own_tags: &[String], keywords: &[(String, f32)], corpus: &Corpus) -> Derived {
    let Features { own, auto_tags, raw } = features(own_tags, keywords, corpus);
    let category = corpus.classifier.classify(blog_id, &raw);

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
        category,
        terms,
        labels,
        nsfw: false,
    }
}

/// Word-list evidence for each topic, indexed like `TOPICS`. Broad words
/// ("生活", "日常") count less, so a specific one ("电影") outweighs them.
fn word_scores(raw: &RawTerms) -> Vec<f32> {
    let mut scores = vec![0.0; TOPICS.len()];
    for (key, (w, _)) in raw {
        let w = if GENERIC.contains(key.as_str()) { w * 0.3 } else { *w };
        for &i in TOPIC_WORDS.get(key.as_str()).into_iter().flatten() {
            scores[i] += w;
        }
    }
    scores
}

/// Word-list evidence that needs no second opinion: a strong keyword or a
/// tag, and clearly more than for any other topic.
const SEED_SCORE: f32 = 0.6;
const SEED_MARGIN: f32 = 1.5;

/// The topic the word lists are sure about, if any.
fn seed(scores: &[f32]) -> Option<usize> {
    let (best, &top) = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let second = scores
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != best)
        .map(|(_, s)| *s)
        .fold(0.0, f32::max);
    (top >= SEED_SCORE && top >= second * SEED_MARGIN).then_some(best)
}

/// Training posts a term must appear in to count as evidence.
const CLASSIFIER_MIN_DF: u32 = 3;
const SMOOTHING: f32 = 0.5;
/// How many posts' worth the corpus-wide prior counts against a blog's own
/// mix, and how much the prior counts against the post's words: flattened,
/// so the blog only tips a post that has some evidence of its own.
const BLOG_PRIOR_WEIGHT: f32 = 4.0;
const PRIOR_EXPONENT: f32 = 0.5;
/// Probability needed to name a topic; failing that, for its top-level
/// category (unsure which sub-category); failing that the post is `other`.
const TOPIC_CONFIDENCE: f32 = 0.6;
const CATEGORY_CONFIDENCE: f32 = 0.6;

/// Naive Bayes over tag and keyword keys, trained on the posts the word lists
/// are sure about. It then also knows the words that come with those ("k3s",
/// a camera model, a city) and can place the rest. Each blog's own mix of
/// topics is the prior, so a post with little text leans towards what its
/// blog usually writes about.
#[derive(Default)]
pub struct Classifier {
    /// ln P(term | topic) per topic, for terms seen in enough training posts.
    log_lik: HashMap<String, Vec<f32>>,
    /// Share of each topic among all training posts (smoothed).
    prior: Vec<f32>,
    /// Training posts per topic, per blog.
    blogs: HashMap<i64, Vec<f32>>,
}

impl Classifier {
    /// From (blog id, terms, topic index) of the training posts.
    fn train<'a>(seeds: impl IntoIterator<Item = (i64, &'a RawTerms, usize)>) -> Self {
        let n = TOPICS.len();
        let mut counts: HashMap<String, Vec<f32>> = HashMap::new();
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut docs = vec![0.0f32; n];
        let mut blogs: HashMap<i64, Vec<f32>> = HashMap::new();
        for (blog, raw, topic) in seeds {
            docs[topic] += 1.0;
            blogs.entry(blog).or_insert_with(|| vec![0.0; n])[topic] += 1.0;
            for (k, (w, _)) in raw {
                counts.entry(k.clone()).or_insert_with(|| vec![0.0; n])[topic] += w;
                *df.entry(k.clone()).or_default() += 1;
            }
        }
        let total_docs: f32 = docs.iter().sum();
        if total_docs == 0.0 {
            return Self::default();
        }
        counts.retain(|k, _| df[k] >= CLASSIFIER_MIN_DF);
        let mut totals = vec![0.0f32; n];
        for c in counts.values() {
            for (t, x) in totals.iter_mut().zip(c) {
                *t += x;
            }
        }
        let vocab = counts.len() as f32;
        let log_lik = counts
            .into_iter()
            .map(|(k, c)| {
                let ll = c
                    .iter()
                    .zip(&totals)
                    .map(|(x, t)| ((x + SMOOTHING) / (t + SMOOTHING * vocab)).ln())
                    .collect();
                (k, ll)
            })
            .collect();
        let prior = docs.iter().map(|d| (d + 1.0) / (total_docs + n as f32)).collect();
        Self { log_lik, prior, blogs }
    }

    /// P(topic | post) for each topic.
    fn posterior(&self, blog_id: i64, raw: &RawTerms) -> Vec<f32> {
        let own = self.blogs.get(&blog_id);
        let seen: f32 = own.map_or(0.0, |c| c.iter().sum());
        let mut logp: Vec<f32> = self
            .prior
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let c = own.map_or(0.0, |c| c[i]);
                PRIOR_EXPONENT * ((c + BLOG_PRIOR_WEIGHT * p) / (seen + BLOG_PRIOR_WEIGHT)).ln()
            })
            .collect();
        for (k, (w, _)) in raw {
            if let Some(ll) = self.log_lik.get(k) {
                for (lp, l) in logp.iter_mut().zip(ll) {
                    *lp += w * l;
                }
            }
        }
        let max = logp.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let exp: Vec<f32> = logp.iter().map(|l| (l - max).exp()).collect();
        let sum: f32 = exp.iter().sum();
        exp.into_iter().map(|e| e / sum).collect()
    }

    /// The category id for a post with these terms.
    pub fn classify(&self, blog_id: i64, raw: &RawTerms) -> String {
        let scores = word_scores(raw);
        if let Some(i) = seed(&scores) {
            return TOPICS[i].path();
        }
        if self.prior.is_empty() {
            // Not trained yet (a new database): the word lists' best guess.
            let best = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1));
            return match best {
                Some((i, s)) if *s >= 0.5 => TOPICS[i].path(),
                _ => OTHER.to_string(),
            };
        }
        decide(&self.posterior(blog_id, raw))
    }
}

/// The most specific category the probabilities support.
fn decide(p: &[f32]) -> String {
    let Some((best, &pb)) = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else {
        return OTHER.to_string();
    };
    if pb >= TOPIC_CONFIDENCE {
        return TOPICS[best].path();
    }
    CATEGORIES
        .iter()
        .map(|c| {
            let share: f32 = TOPICS
                .iter()
                .zip(p)
                .filter(|(t, _)| t.parent == c.id)
                .map(|(_, x)| x)
                .sum();
            (c.id, share)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, share)| *share >= CATEGORY_CONFIDENCE)
        .map_or_else(|| OTHER.to_string(), |(id, _)| id.to_string())
}

/// Writes a post's derived data, replacing its old terms.
pub async fn save_derived(conn: &mut sqlx::SqliteConnection, post_id: i64, d: &Derived) -> sqlx::Result<()> {
    sqlx::query("UPDATE posts SET auto_tags = ?, category = ?, nsfw = ? WHERE id = ?")
        .bind(serde_json::to_string(&d.auto_tags).unwrap_or_else(|_| "[]".into()))
        .bind(&d.category)
        .bind(d.nsfw)
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

/// A blog's main top-level category and its share of the blog's classified posts.
#[derive(Debug, Serialize, serde::Deserialize, Clone, PartialEq)]
pub struct BlogCategory {
    pub id: String,
    pub share: f32,
}

/// A blog's categories: the top-level ones at least a quarter of its
/// classified posts fall into.
pub async fn update_blog_categories(pool: &SqlitePool, blog_ids: &[i64]) -> sqlx::Result<()> {
    for &id in blog_ids {
        let rows: Vec<String> = sqlx::query_scalar("SELECT category FROM posts WHERE blog_id = ? AND category <> ?")
            .bind(id)
            .bind(OTHER)
            .fetch_all(pool)
            .await?;
        let categories = blog_categories(&rows);
        sqlx::query("UPDATE blogs SET categories = ? WHERE id = ?")
            .bind(serde_json::to_string(&categories).unwrap_or_else(|_| "[]".into()))
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

fn blog_categories(post_categories: &[String]) -> Vec<BlogCategory> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for c in post_categories {
        *counts.entry(top_of(c)).or_default() += 1;
    }
    let n = post_categories.len();
    let mut out: Vec<BlogCategory> = counts
        .into_iter()
        .filter(|(_, c)| *c * 4 >= n && (*c >= 2 || n < 3))
        .map(|(id, c)| BlogCategory {
            id: id.to_string(),
            share: ((c as f32 / n.max(1) as f32) * 100.0).round() / 100.0,
        })
        .collect();
    out.sort_by(|a, b| b.share.total_cmp(&a.share).then_with(|| a.id.cmp(&b.id)));
    out.truncate(3);
    out
}

#[derive(Debug, Default)]
pub struct RetagStats {
    pub posts: usize,
    pub keyworded: usize,
    pub tagged: usize,
    /// Posts the word lists alone classified (the classifier's training set).
    pub seeds: usize,
    /// Posts with a category other than `other`.
    pub categorized: usize,
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

/// Recomputes corpus statistics, retrains the category classifier, and
/// redoes every post's tags, category and term vector, and every blog's
/// categories. Posts that have no keywords yet (stored before analysis
/// existed) get them from their title, categories and excerpt.
pub async fn retag_all(pool: &SqlitePool) -> anyhow::Result<RetagStats> {
    let rows: Vec<PostText> = sqlx::query_as("SELECT id, blog_id, title, excerpt, tags, keywords FROM posts")
        .fetch_all(pool)
        .await?;
    let (posts, stats, corpus) = tokio::task::spawn_blocking(move || {
        let (posts, mut stats) = prepare(rows);
        let mut corpus = build_corpus(&posts);
        let raws: Vec<RawTerms> = posts
            .iter()
            .map(|p| features(&p.tags, &p.keywords, &corpus).raw)
            .collect();
        let seeds: Vec<(i64, &RawTerms, usize)> = posts
            .iter()
            .zip(&raws)
            .filter_map(|(p, raw)| seed(&word_scores(raw)).map(|t| (p.blog_id, raw, t)))
            .collect();
        stats.seeds = seeds.len();
        corpus.classifier = Classifier::train(seeds);
        (posts, stats, Arc::new(corpus))
    })
    .await?;

    let mut stats = stats;
    for chunk in posts.chunks(500) {
        let mut tx = pool.begin().await?;
        for p in chunk {
            let mut d = derive(p.blog_id, &p.tags, &p.keywords, &corpus);
            d.nsfw = p.nsfw;
            if p.keywords_changed {
                sqlx::query("UPDATE posts SET keywords = ? WHERE id = ?")
                    .bind(serde_json::to_string(&p.keywords)?)
                    .bind(p.id)
                    .execute(&mut *tx)
                    .await?;
            }
            stats.tagged += usize::from(!d.auto_tags.is_empty() || !p.tags.is_empty());
            stats.categorized += usize::from(d.category != OTHER);
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
    update_blog_categories(pool, &blogs).await?;
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
    nsfw: bool,
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
                nsfw: is_nsfw(&r.title, &tags),
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
        classifier: Classifier::default(),
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
    fn adult_content_by_title_or_tag() {
        assert!(is_nsfw("周末涩图分享", &[]));
        assert!(is_nsfw("Some art", &["R18".into()]));
        assert!(is_nsfw("[NSFW] gallery", &[]));
        assert!(!is_nsfw("R1800 路由器评测", &[]), "whole words only");
        assert!(!is_nsfw("成人高考经验", &["教育".into()]));
    }

    #[test]
    fn tags_fold_synonyms_and_casing() {
        assert_eq!(tag_display("docker"), "Docker");
        assert_eq!(tag_display("tag:Docker"), "Docker");
        assert_eq!(tag_key("category:教程"), "教程");
        assert_eq!(tag_display("tag:"), "tag:");
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
    fn derive_tags_category_and_unit_vectors() {
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
            ..Default::default()
        };
        let d = derive(
            1,
            &["随笔".into()],
            &[("Docker".into(), 1.0), ("容器".into(), 0.6), ("Nginx".into(), 0.1)],
            &corpus,
        );
        assert_eq!(d.auto_tags, ["Docker"], "known words become tags; weak ones do not");
        assert_eq!(d.category, "tech/ops", "one category, the strongest");
        let norm: f32 = d.terms.iter().map(|(_, w, _)| w * w).sum();
        assert!((norm - 1.0).abs() < 1e-3);
        assert_eq!(d.terms[0].0, "docker", "rarer terms weigh more");
        assert!(d.terms.iter().any(|(k, _, tag)| k == "随笔" && *tag));
        assert!(d.terms.iter().any(|(k, _, tag)| k == "容器" && !*tag));
    }

    fn raw(terms: &[(&str, f32)]) -> RawTerms {
        terms.iter().map(|(k, w)| (k.to_string(), (*w, false))).collect()
    }

    #[test]
    fn classifier_learns_words_from_clear_cases_and_uses_the_blog() {
        // Five clear posts per topic (ten for `ops`, all on blog 2): a word
        // from the topic's list plus two words of its own.
        let mut posts: Vec<(i64, RawTerms)> = Vec::new();
        for (i, t) in TOPICS.iter().enumerate() {
            let (a, b) = (format!("{}-a", t.id), format!("{}-b", t.id));
            let word = t.words.iter().find(|w| !GENERIC.contains(*w)).unwrap();
            let terms = raw(&[(word, 1.0), (&a, 1.0), (&b, 1.0)]);
            let (blog, n) = if t.id == "ops" { (2, 10) } else { (100 + i as i64, 5) };
            posts.extend((0..n).map(|_| (blog, terms.clone())));
        }
        let seeds: Vec<(i64, &RawTerms, usize)> = posts
            .iter()
            .filter_map(|(blog, terms)| seed(&word_scores(terms)).map(|t| (*blog, terms, t)))
            .collect();
        assert_eq!(seeds.len(), posts.len());
        let c = Classifier::train(seeds);
        assert_eq!(
            c.classify(3, &posts[0].1),
            TOPICS[0].path(),
            "clear cases keep their topic"
        );
        assert_eq!(
            c.classify(3, &raw(&[("travel-a", 1.0), ("travel-b", 1.0)])),
            "travel",
            "words learnt from the clear cases"
        );
        assert_eq!(c.classify(3, &raw(&[("ops-a", 1.0), ("ops-b", 0.8)])), "tech/ops");
        let weak = raw(&[("ops-a", 0.5)]);
        assert_eq!(
            c.classify(2, &weak),
            "tech/ops",
            "weak evidence, tipped by the blog's usual topic"
        );
        assert_eq!(
            c.classify(3, &weak),
            OTHER,
            "too weak on a blog that writes about anything"
        );
        assert_eq!(
            c.classify(2, &raw(&[("unknown", 1.0)])),
            OTHER,
            "the blog alone is not enough"
        );
    }

    #[test]
    fn unsure_sub_category_falls_back_to_its_parent() {
        let mut p = vec![0.0; TOPICS.len()];
        let at = |id: &str| TOPICS.iter().position(|t| t.id == id).unwrap();
        p[at("dev")] = 0.4;
        p[at("ops")] = 0.35;
        p[at("travel")] = 0.25;
        assert_eq!(decide(&p), "tech");
        p[at("dev")] = 0.7;
        p[at("ops")] = 0.05;
        assert_eq!(decide(&p), "tech/dev");
        let flat = vec![1.0 / TOPICS.len() as f32; TOPICS.len()];
        assert_eq!(decide(&flat), OTHER);
    }

    #[test]
    fn category_ids_and_labels() {
        assert_eq!(
            category("tech/ai").map(|(l, p)| (l, p.map(|p| p.label))),
            Some(("AI", Some("技术")))
        );
        assert_eq!(category("travel").map(|(l, p)| (l, p.is_none())), Some(("旅行", true)));
        assert!(category("tech/travel").is_none());
        assert!(category("nope").is_none());
        assert_eq!(top_of("life/food"), "life");
        assert!(TOPICS.iter().all(|t| top_category(t.parent).is_some()));
    }

    #[test]
    fn blog_categories_need_a_share() {
        let posts: Vec<String> = ["tech/dev", "tech/ai", "tech", "life/daily", "travel"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let t = blog_categories(&posts);
        let ids: Vec<&str> = t.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["tech"], "sub-categories count towards their parent");
        assert_eq!(t[0].share, 0.6);
    }
}
