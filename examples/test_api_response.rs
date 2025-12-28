use anyhow::Result;
use serde_json::Value;
use x_tweets_backup::{Config, Downloader, MarkdownGenerator};

#[tokio::main]
async fn main() -> Result<()> {
    println!("开始解析API响应数据...");
    
    // 创建基本配置
    let config = create_test_config();
    
    // 创建下载器
    let mut downloader = Downloader::new(config.clone())?;
    
    // 创建Markdown生成器
    let mut markdown_generator = MarkdownGenerator::new(config.clone());
    
    // 解析API响应数据
    // 注意: api_response.json 文件已从 git 中移除以保护隐私
    // 使用方法：
    // 1. 将测试 JSON 文件放在项目根目录
    // 2. 取消下面的注释并修改路径
    // 3. 运行: cargo run --example test_api_response
    
    let json_data = if let Ok(api_response) = std::fs::read_to_string("api_response.json") {
        serde_json::from_str(&api_response)?
    } else {
        eprintln!("[ERROR] 未找到 api_response.json 文件");
        eprintln!("[INFO] 请将测试 JSON 文件放在项目根目录");
        eprintln!("[INFO] 该文件已从 git 中移除以保护隐私");
        return Err(anyhow::anyhow!("缺少 api_response.json 文件"));
    };
    
    println!("成功解析JSON数据");
    
    // 提取推文数据
    let tweets = extract_tweets_from_response(&json_data)?;
    println!("提取到 {} 条推文", tweets.len());
    println!("已记录的下载ID数量: {}", downloader.downloaded_count());
    
    let mut processed_count = 0;
    let mut download_success_count = 0;
    let mut download_failed_count = 0;
    let mut content_saved_count = 0;
    
    // 处理每条推文
    for (index, tweet_data) in tweets.iter().enumerate() {
        let tweet_id = tweet_data.get("rest_id").and_then(|id| id.as_str()).unwrap_or("unknown");
        println!("处理推文 {}: {}", index + 1, tweet_id);
        
        // 检查是否已下载
        if downloader.is_downloaded(tweet_id) {
            println!("推文 {} 已下载，跳过", tweet_id);
            continue;
        }
        
        // 下载媒体文件
        let media_result = downloader.call_media_downloader(tweet_data, tweet_id).await;
        
        // 保存推文内容到Markdown
        match MarkdownGenerator::extract_tweet_content(tweet_data) {
            Ok(tweet_content) => {
                markdown_generator.add_tweet(tweet_content);
                content_saved_count += 1;
                println!("✓ 成功提取推文内容");
            }
            Err(e) => {
                println!("✗ 提取推文内容失败: {}", e);
            }
        }
        
        match media_result {
            Ok(Some(true)) => {
                // 成功下载了媒体文件
                processed_count += 1;
                downloader.save_downloaded_id(tweet_id)?;
                download_success_count += 1;
                println!("✓ 成功下载并记录tweet ID: {}", tweet_id);
            }
            Ok(Some(false)) => {
                // 实际尝试下载但失败了
                processed_count += 1;
                download_failed_count += 1;
                println!("✗ 下载失败，不记录tweet ID: {}", tweet_id);
            }
            Ok(None) => {
                // 没有媒体文件，但内容已保存
                processed_count += 1;
                println!("处理 tweet {}: {} (无媒体文件)", processed_count, tweet_id);
            }
            Err(e) => {
                println!("处理 tweet {} 时发生错误: {}", tweet_id, e);
            }
        }
    }
    
    // 生成并保存Markdown文件
    println!("正在生成Markdown文件...");
    markdown_generator.save_markdown()?;
    println!("✓ Markdown文件已保存到: {}", config.markdown_output);
    
    println!("\n=== 处理总结 ===");
    println!("总tweet数量: {}", tweets.len());
    println!("已处理数量: {}", processed_count);
    println!("下载成功数量: {}", download_success_count);
    println!("下载失败数量: {}", download_failed_count);
    println!("内容保存数量: {}", content_saved_count);
    println!("全部处理完成。");
    
    Ok(())
}

fn create_test_config() -> Config {
    Config {
        user_id: "".to_string(),
        bearer_token: "".to_string(),
        auth_token: "".to_string(),
        ct0: "".to_string(),
        personalization_id: "".to_string(),
        user_agent: "".to_string(),
        x_client_uuid: "".to_string(),
        x_client_transaction_id: "".to_string(),
        count: "20".to_string(),
        all: false,
        download_dir: "data/downloads".to_string(),
        download_record: "data/downloaded_tweet_ids.txt".to_string(),
        file_format: "{USERNAME} {ID}".to_string(),
        target_user_id: "ScottLatin21809".to_string(),
        target_username: "ScottLatin21809".to_string(),
        only_self: false,
        save_markdown: true,
        markdown_output: "data/test_tweets_backup.md".to_string(),
        debug_logs: true,
        user_by_screen_name_query_id: "6ND0OKRCgPajU_yJbcWSVw".to_string(),
        user_tweets_query_id: "V3vRrAJh5U6n9m1ZJ8xYQw".to_string(),
        max_pages: 50,
        download_timeout_secs: 30,
    }
}

fn extract_tweets_from_response(json_data: &Value) -> Result<Vec<&Value>> {
    let mut tweets = Vec::new();
    
    // 导航到推文数据的位置
    if let Some(instructions) = json_data
        .get("data")
        .and_then(|d| d.get("user"))
        .and_then(|u| u.get("result"))
        .and_then(|r| r.get("timeline"))
        .and_then(|t| t.get("timeline"))
        .and_then(|t| t.get("instructions"))
        .and_then(|i| i.as_array())
    {
        for instruction in instructions {
            if let Some(instruction_type) = instruction.get("type").and_then(|t| t.as_str()) {
                match instruction_type {
                    "TimelinePinEntry" => {
                        if let Some(entry) = instruction.get("entry") {
                            if let Some(tweet_data) = extract_tweet_from_entry(entry) {
                                tweets.push(tweet_data);
                            }
                        }
                    }
                    "TimelineAddEntries" => {
                        if let Some(entries) = instruction.get("entries").and_then(|e| e.as_array()) {
                            for entry in entries {
                                // 处理对话模块中的推文
                                if let Some(entry_id) = entry.get("entryId").and_then(|id| id.as_str()) {
                                    if entry_id.starts_with("profile-conversation-") {
                                        // 这是对话模块，提取其中的所有推文
                                        let conversation_tweets = extract_tweets_from_conversation_module(entry);
                                        tweets.extend(conversation_tweets);
                                    } else if entry_id.starts_with("tweet-") {
                                        // 这是单个推文
                                        if let Some(tweet_data) = extract_tweet_from_entry(entry) {
                                            tweets.push(tweet_data);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    
    Ok(tweets)
}

fn extract_tweet_from_entry(entry: &Value) -> Option<&Value> {
    // 处理单个推文条目
    if let Some(content) = entry.get("content") {
        if let Some(item_content) = content.get("itemContent") {
            if let Some(tweet_results) = item_content.get("tweet_results") {
                if let Some(result) = tweet_results.get("result") {
                    return Some(result);
                }
            }
        }
    }
    
    None
}

fn extract_tweets_from_conversation_module(entry: &Value) -> Vec<&Value> {
    let mut tweets = Vec::new();
    
    // 处理对话模块中的推文
    if let Some(content) = entry.get("content") {
        if let Some(items) = content.get("items").and_then(|i| i.as_array()) {
            for item in items {
                if let Some(item_obj) = item.get("item") {
                    if let Some(item_content) = item_obj.get("itemContent") {
                        if let Some(tweet_results) = item_content.get("tweet_results") {
                            if let Some(result) = tweet_results.get("result") {
                                tweets.push(result);
                            }
                        }
                    }
                }
            }
        }
    }
    
    tweets
}

