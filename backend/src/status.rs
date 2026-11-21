//! `feed-server status`: how crawling goes, read from the database, as
//! Markdown (readable in a terminal, and shown on each crawl workflow run's
//! page). Starts with what needs looking at, if anything.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::LazyLock;

use regex::Regex;
use sqlx::SqlitePool;

use crate::api::DirectoryInfo;
use crate::util::{now, truncate_chars};
use crate::{backup, crawler, db};

/// Nothing read for this long: the crawl has probably stopped.
const QUIET_SECS: i64 = 6 * 3600;
/// A blog whose next check is this late counts as overdue.
const OVERDUE_SECS: i64 = 3600;

static ADDRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s)]+").unwrap());

pub async fn report(pool: &SqlitePool, directories: &[DirectoryInfo]) -> anyhow::Result<String> {
    let t = now();
    let (blogs, with_feed, failing, failing_long, overdue): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(feed_url), COALESCE(SUM(failures > 0), 0), COALESCE(SUM(failures >= 5), 0),
                COALESCE(SUM(next_check_at < ?1), 0)
         FROM blogs WHERE active = 1 AND excluded = 0",
    )
    .bind(t - OVERDUE_SECS)
    .fetch_one(pool)
    .await?;
    let (checked, succeeded): (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT MAX(last_checked_at), MAX(last_success_at) FROM blogs")
            .fetch_one(pool)
            .await?;
    let (posts, new_posts): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(first_seen_at >= ?), 0) FROM posts")
            .bind(t - 86400)
            .fetch_one(pool)
            .await?;
    let listed: Vec<(String, i64)> =
        // Directories no longer configured keep their entries, inactive.
        sqlx::query_as("SELECT directory, SUM(active) AS n FROM directory_entries GROUP BY directory HAVING n > 0")
            .fetch_all(pool)
            .await?;
    let errors: Vec<(String,)> = sqlx::query_as(
        "SELECT last_error FROM blogs
         WHERE active = 1 AND excluded = 0 AND failures > 0 AND last_error IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    let synced = kv_time(pool, crawler::LAST_SYNC_KEY).await?;
    let backed_up = kv_time(pool, backup::LAST_BACKUP_KEY).await?;

    let mut warnings = Vec::new();
    if checked.is_none_or(|c| t - c > QUIET_SECS) {
        warnings.push("6 小时内没有检查过任何 RSS：抓取可能停了。".to_string());
    } else if succeeded.is_none_or(|s| t - s > QUIET_SECS) {
        warnings.push("6 小时内没有成功读到任何 RSS：可能是网络出了问题。".to_string());
    }
    if with_feed > 0 && failing * 100 / with_feed > 30 {
        warnings.push(format!("{failing} 个博客正在失败，超过有 RSS 的三成。"));
    }
    if blogs > 0 && overdue * 100 / blogs > 20 {
        warnings.push(format!(
            "{overdue} 个博客逾期 1 小时以上没有检查：抓取跟不上，或者停了。"
        ));
    }
    if synced.is_some_and(|s| t - s > 3 * 86400) {
        warnings.push("博客目录 3 天以上没有同步。".to_string());
    }
    if backed_up.is_some_and(|b| t - b > 3 * 86400) {
        warnings.push("3 天以上没有备份到 git 仓库。".to_string());
    }

    let mut out = String::new();
    writeln!(out, "## 运行状况（{}）\n", when(t))?;
    if warnings.is_empty() {
        writeln!(out, "没有发现问题。\n")?;
    } else {
        for w in &warnings {
            writeln!(out, "- ⚠️ {w}")?;
        }
        writeln!(out)?;
    }
    writeln!(out, "| | |\n|---|---|")?;
    writeln!(out, "| 最近一次检查 RSS | {} |", ago(t, checked))?;
    writeln!(out, "| 最近一次读到 RSS | {} |", ago(t, succeeded))?;
    writeln!(
        out,
        "| 收录的博客 | {blogs}（有 RSS {with_feed}，还没找到 {}） |",
        blogs - with_feed
    )?;
    writeln!(out, "| 正在失败 | {failing}（连续 5 次以上 {failing_long}） |")?;
    writeln!(out, "| 逾期 1 小时以上未检查 | {overdue} |")?;
    writeln!(out, "| 文章 | {posts}（24 小时内新读到 {new_posts}） |")?;
    for (id, n) in &listed {
        let name = directories.iter().find(|d| d.id == id).map_or_else(
            || if id == db::READERS { "读者自己的订阅" } else { id }.to_string(),
            |d| d.name.to_string(),
        );
        writeln!(out, "| 目录：{name} | {n} 个 |")?;
    }
    writeln!(out, "| 上次同步目录 | {} |", ago(t, synced))?;
    writeln!(out, "| 上次备份 | {} |", ago(t, backed_up))?;

    if !errors.is_empty() {
        // The same failure on different sites reads the same without the addresses.
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (e,) in &errors {
            *counts
                .entry(truncate_chars(&ADDRESS.replace_all(e, "…"), 120))
                .or_default() += 1;
        }
        let mut top: Vec<_> = counts.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        writeln!(out, "\n### 最常见的失败原因\n\n| 博客数 | 错误 |\n|---|---|")?;
        for (e, n) in top.iter().take(10) {
            writeln!(out, "| {n} | {} |", e.replace('|', "\\|").replace('\n', " "))?;
        }
    }
    Ok(out)
}

async fn kv_time(pool: &SqlitePool, key: &str) -> sqlx::Result<Option<i64>> {
    Ok(db::kv_get(pool, key).await?.and_then(|v| v.parse().ok()))
}

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map_or_else(|| t.to_string(), |d| d.format("%Y-%m-%d %H:%M UTC").to_string())
}

fn ago(now: i64, t: Option<i64>) -> String {
    let Some(t) = t else {
        return "从来没有".into();
    };
    let secs = (now - t).max(0);
    let rough = match secs {
        s if s < 120 => "刚刚".to_string(),
        s if s < 7200 => format!("{} 分钟前", s / 60),
        s if s < 2 * 86400 => format!("{} 小时前", s / 3600),
        s => format!("{} 天前", s / 86400),
    };
    format!("{rough}（{}）", when(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_failures_grouped_and_warns_when_quiet() {
        let st = crate::api::tests::seeded().await;
        sqlx::query(
            "UPDATE blogs SET failures = 6, last_error = 'HTTP 404 for https://' || site_key || '/feed', last_checked_at = 1",
        )
        .execute(&st.pool)
        .await
        .unwrap();
        let text = report(&st.pool, &st.directories).await.unwrap();
        assert!(text.contains("6 小时内没有检查过任何 RSS"), "{text}");
        assert!(
            text.contains("| 2 | HTTP 404 for … |"),
            "the two live blogs' errors as one: {text}"
        );
        assert!(text.contains("正在失败 | 2（连续 5 次以上 2）"), "{text}");
    }
}
