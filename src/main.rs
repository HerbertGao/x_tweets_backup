mod config;
mod downloader;
mod setup;
mod updater;
mod x_api;
mod markdown_generator;
mod tweet_parser;

// 调试日志宏
macro_rules! debug_log {
    ($config:expr, $($arg:tt)*) => {
        if $config.debug_logs {
            println!($($arg)*);
        }
    };
}

use anyhow::Result;
use clap::{Parser, Subcommand};

use config::Config;
use downloader::Downloader;
use setup::SetupArgs;
use updater::Updater;
use x_api::XApi;
use markdown_generator::MarkdownGenerator;

#[derive(Parser)]
#[command(name = "x_tweets_backup")]
#[command(about = "X推文备份器 - 备份某人的全部推文媒体")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

    #[derive(Subcommand)]
    enum Commands {
        /// 初始化配置
        Setup(SetupArgs),
        /// 备份用户推文媒体
        Backup {
            /// 目标用户名（可选，不指定则使用配置文件中的设置）
            username: Option<String>,
        },
        /// 检查并更新到最新版本
        Update,
    }

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

        match &cli.command {
            Commands::Setup(args) => {
                setup::run_setup(args.clone())?;
            }
            Commands::Backup { username } => {
                run_backup(username).await?;
            }
            Commands::Update => {
                let updater = Updater::new()?;
                updater.update().await?;
            }
        }

    Ok(())
}

async fn run_backup(username: &Option<String>) -> Result<()> {
    // 加载配置
    let mut config = Config::load()?;
    
    debug_log!(config, "[DEBUG] 开始备份流程");
    debug_log!(config, "[DEBUG] 加载配置文件...");
    
    // 如果命令行指定了用户名，则覆盖配置文件中的设置
    if let Some(ref uname) = username {
        debug_log!(config, "[DEBUG] 使用命令行指定的用户名: {}", uname);
        config.target_user_id = uname.clone();
    } else {
        debug_log!(config, "[DEBUG] 使用配置文件中的用户名: {}", config.target_user_id);
    }
    
    // 验证用户名设置
    if config.target_user_id.is_empty() {
        return Err(anyhow::anyhow!("请指定目标用户名。使用方法：\n1. 通过命令行参数：./x_tweets_backup backup <username>\n2. 通过环境变量：TARGET_USER_ID=<username>\n3. 通过配置文件：在.env文件中设置TARGET_USER_ID"));
    }
    
    // 创建API客户端
    debug_log!(config, "[DEBUG] 创建API客户端...");
    let api = XApi::new(config.clone())?;
    
    // 查询用户名对应的用户ID
    debug_log!(config, "[DEBUG] 正在查询用户信息: {}", config.target_user_id);
    let original_username = config.target_user_id.clone();
    let user_id = api.get_user_id_by_username(&config.target_user_id).await?;
    debug_log!(config, "[DEBUG] 用户 @{} 的ID: {}", original_username.trim_start_matches('@'), user_id);
    
    // 保存原始用户名并更新用户ID
    config.target_username = original_username.trim_start_matches('@').to_string();
    config.target_user_id = user_id;
    
    // 重新创建API客户端以使用更新后的用户ID
    debug_log!(config, "[DEBUG] 重新创建API客户端...");
    let api = XApi::new(config.clone())?;
    
    // 创建下载器
    debug_log!(config, "[DEBUG] 创建下载器...");
    let mut downloader = Downloader::new(config.clone())?;
    
    // 创建Markdown生成器
    debug_log!(config, "[DEBUG] 创建Markdown生成器...");
    let mut markdown_generator = MarkdownGenerator::new(config.clone());
    
    // 获取用户推文
    debug_log!(config, "[DEBUG] 开始获取用户推文...");
    let tweets = api.get_user_tweets_internal().await?;

    debug_log!(config, "[DEBUG] 从 API 获取到 {} 条用户 tweet 数据", tweets.len());
    debug_log!(config, "[DEBUG] 已记录的下载ID数量: {}", downloader.downloaded_count());

    let mut processed_count = 0;
    let mut download_success_count = 0;
    let mut download_failed_count = 0;
    let mut content_saved_count = 0;

    let tweets_count = tweets.len();
    
    for entry in tweets {
        // 提取推文数据，使用与测试程序相同的逻辑
        let tweet_data = entry
            .get("content")
            .and_then(|c| c.get("itemContent"))
            .and_then(|ic| ic.get("tweet_results"))
            .and_then(|tr| tr.get("result"))
            .unwrap_or(&entry);

        // 可选：仅保留作者本人推文（过滤掉转推的他人内容）
        if config.only_self {
            // 统一获取 tweet 对象
            let tweet_obj = tweet_data.get("tweet").unwrap_or(tweet_data);

            // 提取作者ID
            let author_id = tweet_obj
                .get("core")
                .and_then(|c| c.get("user_results"))
                .and_then(|ur| ur.get("result"))
                .and_then(|r| r.get("rest_id"))
                .and_then(|id| id.as_str())
                .or_else(|| {
                    tweet_obj
                        .get("core")
                        .and_then(|c| c.get("user_results"))
                        .and_then(|ur| ur.get("result"))
                        .and_then(|r| r.get("legacy"))
                        .and_then(|l| l.get("id_str"))
                        .and_then(|s| s.as_str())
                });

            // 转推活动检测：如果 legacy 中存在 retweeted_status_result 或 retweeted_status_id_str，则判定为转推，跳过
            let legacy = tweet_obj.get("legacy").unwrap_or(&serde_json::Value::Null);
            let is_retweet_activity = legacy.get("retweeted_status_result").is_some()
                || legacy.get("retweeted_status_id_str").is_some();

            match author_id {
                Some(aid) if aid == config.target_user_id && !is_retweet_activity => { /* 保留 */ }
                _ => {
                    // 过滤掉非本人推文
                    continue;
                }
            }
        }
        let tweet_id = tweet_data
            .get("rest_id")
            .and_then(|id| id.as_str())
            .or_else(|| {
                tweet_data
                    .get("tweet")
                    .and_then(|t| t.get("rest_id"))
                    .and_then(|id| id.as_str())
            });

        if let Some(tweet_id) = tweet_id {
            if downloader.is_downloaded(tweet_id) {
                // 即使已下载媒体，也要保存内容到Markdown
                if config.save_markdown {
                    match MarkdownGenerator::extract_tweet_content(tweet_data) {
                        Ok(tweet_content) => {
                            markdown_generator.add_tweet(tweet_content);
                            content_saved_count += 1;
                        }
                        Err(e) => {
                            println!("提取推文内容失败 {}: {}", tweet_id, e);
                        }
                    }
                }
                continue;
            }

            // 下载媒体文件
            let media_result = downloader.call_media_downloader(tweet_data, tweet_id).await;
            
            // 保存推文内容到Markdown
            if config.save_markdown {
                match MarkdownGenerator::extract_tweet_content(tweet_data) {
                    Ok(tweet_content) => {
                        markdown_generator.add_tweet(tweet_content);
                        content_saved_count += 1;
                    }
                    Err(e) => {
                        println!("提取推文内容失败 {}: {}", tweet_id, e);
                    }
                }
            }

            match media_result {
                Ok(Some(true)) => {
                    // 成功下载了媒体文件
                    processed_count += 1;
                    println!("处理 tweet ({}): {}", processed_count, tweet_id);
                    downloader.save_downloaded_id(tweet_id)?;
                    download_success_count += 1;
                    println!("✓ 成功下载并记录tweet ID: {}", tweet_id);
                }
                Ok(Some(false)) => {
                    // 实际尝试下载但失败了
                    processed_count += 1;
                    println!("处理 tweet ({}): {}", processed_count, tweet_id);
                    download_failed_count += 1;
                    println!("✗ 下载失败,不记录tweet ID: {}", tweet_id);
                }
                Ok(None) => {
                    // 没有媒体文件，但内容已保存
                    processed_count += 1;
                    println!("处理 tweet ({}): {} (无媒体文件)", processed_count, tweet_id);
                }
                Err(e) => {
                    println!("处理 tweet {} 时发生错误: {}", tweet_id, e);
                }
            }
        }
    }

    // 保存Markdown文件
    if config.save_markdown {
        println!("正在生成Markdown文件...");
        markdown_generator.save_markdown()?;
    }

    println!("\n=== 处理总结 ===");
    println!("总tweet数量: {}", tweets_count);
    println!("已处理数量: {}", processed_count);
    println!("下载成功数量: {}", download_success_count);
    println!("下载失败数量: {}", download_failed_count);
    println!("内容保存数量: {}", content_saved_count);
    println!("全部处理完成.");

    Ok(())
}

