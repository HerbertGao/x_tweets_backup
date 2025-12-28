use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use serde_json::Value;
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::tweet_parser;

// 调试日志辅助宏
macro_rules! debug_log {
    ($config:expr, $($arg:tt)*) => {
        if $config.debug_logs {
            println!($($arg)*);
        }
    };
}

#[derive(Debug, Clone)]
pub struct TweetContent {
    pub id: String,
    pub text: String,
    pub created_at: Option<DateTime<Utc>>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub media_urls: Vec<String>,
    pub is_retweet: bool,
    pub retweeted_by: Option<String>,
    pub reply_to: Option<String>,
    pub quote_tweet: Option<String>,
}

pub struct MarkdownGenerator {
    config: Config,
    tweets: Vec<TweetContent>,
}

impl MarkdownGenerator {
    pub fn new(config: Config) -> Self {
        debug_log!(config, "[DEBUG] 初始化Markdown生成器");
        MarkdownGenerator {
            config,
            tweets: Vec::new(),
        }
    }

    pub fn add_tweet(&mut self, tweet: TweetContent) {
        debug_log!(self.config, "[DEBUG] 添加推文到Markdown: {} - {}", tweet.id, tweet.text.chars().take(50).collect::<String>());
        self.tweets.push(tweet);
    }

    pub fn generate_markdown(&self) -> Result<String> {
        if self.tweets.is_empty() {
            return Ok(String::new());
        }

        let mut markdown = String::new();
        
        // 添加标题
        let default_username = "Unknown".to_string();
        let username = self.tweets.first()
            .and_then(|t| t.username.as_ref())
            .unwrap_or(&default_username);
        
        markdown.push_str(&format!("# {} 的推文备份\n\n", username));
        markdown.push_str(&format!("备份时间: {}\n", Local::now().format("%Y-%m-%d %H:%M:%S")));
        markdown.push_str(&format!("推文总数: {}\n\n", self.tweets.len()));
        markdown.push_str("---\n\n");

        // 按时间倒序排列（最新的在前）
        let mut sorted_tweets = self.tweets.clone();
        sorted_tweets.sort_by(|a, b| {
            match (a.created_at, b.created_at) {
                (Some(a_time), Some(b_time)) => b_time.cmp(&a_time),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        // 生成每个推文的内容
        for (index, tweet) in sorted_tweets.iter().enumerate() {
            markdown.push_str(&self.format_single_tweet(tweet, index + 1)?);
            markdown.push_str("\n---\n\n");
        }

        Ok(markdown)
    }

    pub fn save_markdown(&self) -> Result<()> {
        debug_log!(self.config, "[DEBUG] 开始生成Markdown文件，包含 {} 条推文", self.tweets.len());
        let markdown_content = self.generate_markdown()?;
        
        // 确保目录存在
        if let Some(parent) = Path::new(&self.config.markdown_output).parent() {
            fs::create_dir_all(parent)?;
        }

        let content_len = markdown_content.len();
        fs::write(&self.config.markdown_output, markdown_content)?;
        debug_log!(self.config, "[DEBUG] Markdown文件已保存到: {} (大小: {} 字节)", 
            self.config.markdown_output, 
            content_len);
        
        Ok(())
    }

    fn format_single_tweet(&self, tweet: &TweetContent, index: usize) -> Result<String> {
        let mut content = String::new();

        // 推文标题
        content.push_str(&format!("## 推文 #{}\n\n", index));

        // 推文信息
        if let Some(created_at) = tweet.created_at {
            content.push_str(&format!("**发布时间**: {}\n\n", created_at.format("%Y-%m-%d %H:%M:%S UTC")));
        }

        if let Some(username) = &tweet.username {
            content.push_str(&format!("**用户名**: @{}\n\n", username));
        }

        if let Some(display_name) = &tweet.display_name {
            content.push_str(&format!("**显示名**: {}\n\n", display_name));
        }

        // 推文内容
        content.push_str("**推文内容**:\n\n");
        content.push_str(&format!("{}\n\n", tweet.text));

        // 特殊类型标识
        if tweet.is_retweet {
            if let Some(retweeted_by) = &tweet.retweeted_by {
                content.push_str(&format!("*🔄 转推自 @{}*\n\n", retweeted_by));
            } else {
                content.push_str("*🔄 转推*\n\n");
            }
        }

        if let Some(reply_to) = &tweet.reply_to {
            content.push_str(&format!("*💬 回复 @{}*\n\n", reply_to));
        }

        if let Some(quote_tweet) = &tweet.quote_tweet {
            content.push_str(&format!("*💭 引用推文: {}*\n\n", quote_tweet));
        }

        // 媒体文件
        if !tweet.media_urls.is_empty() {
            content.push_str("**媒体文件**:\n\n");
            for (i, media_url) in tweet.media_urls.iter().enumerate() {
                // 尝试生成本地文件路径
                let local_path = self.generate_local_media_path(&tweet, i, media_url);
                
                // HTML转义辅助函数
                let escape_html = |s: &str| -> String {
                    s.replace("&", "&amp;")
                        .replace("<", "&lt;")
                        .replace(">", "&gt;")
                        .replace("\"", "&quot;")
                        .replace("'", "&#x27;")
                };
                
                if media_url.contains(".mp4") || media_url.contains(".mov") || media_url.contains(".webm") {
                    // 根据文件扩展名确定正确的 MIME 类型
                    let mime_type = if media_url.contains(".webm") {
                        "video/webm"
                    } else if media_url.contains(".mov") {
                        "video/quicktime"
                    } else {
                        "video/mp4" // 默认或 .mp4
                    };
                    
                    // 使用HTML video标签嵌入视频（转义用户提供的内容）
                    content.push_str(&format!("🎥 **视频 {}**:\n", i + 1));
                    content.push_str(&format!("<video controls width=\"100%\" style=\"max-width: 600px;\">\n"));
                    content.push_str(&format!("  <source src=\"{}\" type=\"{}\">\n", escape_html(&local_path), escape_html(mime_type)));
                    content.push_str(&format!("  您的浏览器不支持视频播放。\n"));
                    content.push_str(&format!("</video>\n"));
                    content.push_str(&format!("<br/>\n"));
                    content.push_str(&format!("<small>📎 [下载视频]({}) | [原始链接]({})</small>\n\n", escape_html(&local_path), escape_html(media_url)));
                } else {
                    // 使用HTML img标签嵌入图片（转义用户提供的内容）
                    content.push_str(&format!("🖼️ **图片 {}**:\n", i + 1));
                    content.push_str(&format!("<img src=\"{}\" alt=\"推文图片 {}\" style=\"max-width: 100%; height: auto; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,0.1);\">\n", escape_html(&local_path), i + 1));
                    content.push_str(&format!("<br/>\n"));
                    content.push_str(&format!("<small>📎 [查看原图]({}) | [原始链接]({})</small>\n\n", escape_html(&local_path), escape_html(media_url)));
                }
            }
        }

        // 推文链接
        content.push_str(&format!("**推文链接**: [https://x.com/{}/status/{}](https://x.com/{}/status/{})\n\n", 
            tweet.username.as_deref().unwrap_or("unknown"), 
            tweet.id,
            tweet.username.as_deref().unwrap_or("unknown"), 
            tweet.id
        ));

        Ok(content)
    }

    fn generate_local_media_path(&self, tweet: &TweetContent, _media_index: usize, media_url: &str) -> String {
        // 从URL中提取原始文件名
        let url_parts: Vec<&str> = media_url.split('/').collect();
        let original_filename = url_parts.last().map_or("unknown", |v| v);
        
        // 提取原始文件名（去掉查询参数）
        let clean_filename = if let Some(query_start) = original_filename.find('?') {
            &original_filename[..query_start]
        } else {
            original_filename
        };
        
        // 生成本地文件名，使用与下载器相同的格式：[username][tweetid][origin_filename]
        let username_str = tweet.username.as_deref().unwrap_or("");
        let local_filename = if username_str.is_empty() {
            // 如果用户名为空，只使用推文ID
            format!("[{}][{}]", tweet.id, clean_filename)
        } else {
            // 格式：[username][tweetid][origin_filename]
            format!("[{}][{}][{}]", username_str, tweet.id, clean_filename)
        };
        
        // 计算从 markdown 文件到下载目录的相对路径
        let markdown_path = Path::new(&self.config.markdown_output);
        let download_dir = Path::new(&self.config.download_dir);
        
        // 获取 markdown 文件的父目录
        let markdown_parent = markdown_path.parent()
            .unwrap_or_else(|| Path::new("."));
        
        // 计算相对路径
        let relative_path = if let Some(rel_path) = pathdiff::diff_paths(download_dir, markdown_parent) {
            // 如果计算成功，使用计算出的相对路径
            if let Some(rel_str) = rel_path.to_str() {
                // 确保路径以 ./ 开头（相对路径）
                if rel_str.starts_with("..") {
                    format!("{}/{}", rel_str, local_filename)
                } else if rel_str == "." || rel_str.is_empty() {
                    // 如果下载目录和 markdown 在同一目录，使用当前目录
                    format!("./{}/{}", download_dir.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("downloads"), local_filename)
                } else {
                    format!("./{}/{}", rel_str, local_filename)
                }
            } else {
                // 如果路径包含非 UTF-8 字符，回退到简单路径
                format!("./{}/{}", download_dir.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("downloads"), local_filename)
            }
        } else {
            // 如果路径计算失败（例如跨磁盘），使用下载目录的最后一个组件
            format!("./{}/{}", download_dir.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("downloads"), local_filename)
        };
        
        relative_path
    }

    pub fn extract_tweet_content(tweet_data: &Value) -> Result<TweetContent> {
        let tweet_obj = tweet_parser::extract_tweet_object(tweet_data)
            .ok_or_else(|| anyhow::anyhow!("无法提取推文对象"))?;
        
        let id = tweet_obj
            .get("rest_id")
            .and_then(|id| id.as_str())
            .unwrap_or("unknown")
            .to_string();

        let legacy = tweet_obj.get("legacy").unwrap_or(&Value::Null);
        
        // 提取推文文本
        let text = legacy
            .get("full_text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        // 提取创建时间
        let created_at = tweet_parser::extract_tweet_timestamp_datetime(tweet_obj)?;

        // 提取用户信息
        let (username, display_name) = tweet_parser::extract_user_info(tweet_obj)?;

        // 提取媒体URL
        let media_urls = tweet_parser::extract_media_urls(tweet_obj)?;

        // 检查是否为转推
        let (is_retweet, retweeted_by) = Self::extract_retweet_info(legacy)?;

        // 检查是否为回复
        let reply_to = Self::extract_reply_info(legacy)?;

        // 检查是否为引用推文
        let quote_tweet = Self::extract_quote_tweet_info(legacy)?;

        Ok(TweetContent {
            id,
            text,
            created_at,
            username,
            display_name,
            media_urls,
            is_retweet,
            retweeted_by,
            reply_to,
            quote_tweet,
        })
    }


    fn extract_retweet_info(legacy: &Value) -> Result<(bool, Option<String>)> {
        let is_retweet = legacy.get("retweeted").and_then(|r| r.as_bool()).unwrap_or(false);
        
        let retweeted_by = if is_retweet {
            // 这里需要从父级数据结构中获取转推者信息
            // 实际实现可能需要更复杂的逻辑
            None
        } else {
            None
        };

        Ok((is_retweet, retweeted_by))
    }

    fn extract_reply_info(legacy: &Value) -> Result<Option<String>> {
        if let Some(in_reply_to_screen_name) = legacy.get("in_reply_to_screen_name").and_then(|s| s.as_str()) {
            Ok(Some(in_reply_to_screen_name.to_string()))
        } else {
            Ok(None)
        }
    }

    fn extract_quote_tweet_info(legacy: &Value) -> Result<Option<String>> {
        if let Some(quoted_status_id) = legacy.get("quoted_status_id").and_then(|id| id.as_str()) {
            Ok(Some(quoted_status_id.to_string()))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            target_user_id: "test_user".to_string(),
            target_username: "test_user".to_string(),
            only_self: false,
            save_markdown: true,
            markdown_output: "data/test_output.md".to_string(),
            debug_logs: false,
            user_by_screen_name_query_id: "6ND0OKRCgPajU_yJbcWSVw".to_string(),
            user_tweets_query_id: "V3vRrAJh5U6n9m1ZJ8xYQw".to_string(),
            max_pages: 50,
            download_timeout_secs: 30,
        }
    }

    #[test]
    fn test_markdown_generator_new() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        assert_eq!(generator.tweets.len(), 0);
    }

    #[test]
    fn test_markdown_generator_add_tweet() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Test tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: Some("Test User".to_string()),
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        assert_eq!(generator.tweets.len(), 1);
    }

    #[test]
    fn test_generate_markdown_empty() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        let markdown = generator.generate_markdown().unwrap();
        assert!(markdown.is_empty());
    }

    #[test]
    fn test_generate_markdown_with_tweets() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Test tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: Some("Test User".to_string()),
            media_urls: vec!["https://example.com/image.jpg".to_string()],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        let markdown = generator.generate_markdown().unwrap();
        assert!(markdown.contains("test_user"));
        assert!(markdown.contains("Test tweet"));
        assert!(markdown.contains("推文 #1"));
    }

    #[test]
    fn test_extract_tweet_object_path1() {
        let tweet = json!({
            "content": {
                "itemContent": {
                    "tweet_results": {
                        "result": {
                            "tweet": {
                                "rest_id": "123",
                                "legacy": {
                                    "full_text": "Test"
                                }
                            }
                        }
                    }
                }
            }
        });
        
        let result = tweet_parser::extract_tweet_object(&tweet);
        assert!(result.is_some());
        assert_eq!(result.unwrap().get("rest_id").and_then(|v| v.as_str()), Some("123"));
    }

    #[test]
    fn test_extract_tweet_object_path2() {
        let tweet = json!({
            "tweet": {
                "rest_id": "456",
                "legacy": {
                    "full_text": "Test"
                }
            }
        });
        
        let result = tweet_parser::extract_tweet_object(&tweet);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.get("rest_id").and_then(|v| v.as_str()), Some("456"));
    }

    #[test]
    fn test_extract_tweet_object_path3() {
        let tweet = json!({
            "rest_id": "789",
            "legacy": {
                "full_text": "Test"
            }
        });
        
        let result = tweet_parser::extract_tweet_object(&tweet);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.get("rest_id").and_then(|v| v.as_str()), Some("789"));
    }

    #[test]
    fn test_extract_tweet_timestamp_from_ms_string() {
        let legacy = json!({
            "created_at_ms": "1609459200000"
        });
        
        let result = tweet_parser::extract_tweet_timestamp_datetime(&legacy).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_tweet_timestamp_from_ms_number() {
        let legacy = json!({
            "created_at_ms": 1609459200000i64
        });
        
        let result = tweet_parser::extract_tweet_timestamp_datetime(&legacy).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_tweet_timestamp_from_string() {
        let legacy = json!({
            "created_at": "Thu Apr 06 15:24:15 +0000 2017"
        });
        
        let result = tweet_parser::extract_tweet_timestamp_datetime(&legacy).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_tweet_timestamp_none() {
        let legacy = json!({});
        let result = tweet_parser::extract_tweet_timestamp_datetime(&legacy).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_user_info() {
        let tweet_obj = json!({
            "core": {
                "user_results": {
                    "result": {
                        "legacy": {
                            "screen_name": "test_user",
                            "name": "Test User"
                        }
                    }
                }
            }
        });
        
        let (username, display_name) = tweet_parser::extract_user_info(&tweet_obj).unwrap();
        assert_eq!(username, Some("test_user".to_string()));
        assert_eq!(display_name, Some("Test User".to_string()));
    }

    #[test]
    fn test_extract_media_urls_photo() {
        let legacy = json!({
            "extended_entities": {
                "media": [
                    {
                        "type": "photo",
                        "media_url_https": "https://example.com/image.jpg"
                    }
                ]
            }
        });
        
        let result = tweet_parser::extract_media_urls(&legacy).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/image.jpg");
    }

    #[test]
    fn test_extract_media_urls_video() {
        let legacy = json!({
            "extended_entities": {
                "media": [
                    {
                        "type": "video",
                        "video_info": {
                            "variants": [
                                {
                                    "bitrate": 1000,
                                    "url": "https://example.com/video.mp4"
                                },
                                {
                                    "bitrate": 2000,
                                    "url": "https://example.com/video_hd.mp4"
                                }
                            ]
                        }
                    }
                ]
            }
        });
        
        let result = tweet_parser::extract_media_urls(&legacy).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/video_hd.mp4"); // 应该选择最高bitrate
    }

    #[test]
    fn test_extract_retweet_info() {
        let legacy = json!({
            "retweeted": true
        });
        
        let (is_retweet, _) = MarkdownGenerator::extract_retweet_info(&legacy).unwrap();
        assert!(is_retweet);
    }

    #[test]
    fn test_extract_reply_info() {
        let legacy = json!({
            "in_reply_to_screen_name": "reply_user"
        });
        
        let result = MarkdownGenerator::extract_reply_info(&legacy).unwrap();
        assert_eq!(result, Some("reply_user".to_string()));
    }

    #[test]
    fn test_extract_quote_tweet_info() {
        let legacy = json!({
            "quoted_status_id": "12345"
        });
        
        let result = MarkdownGenerator::extract_quote_tweet_info(&legacy).unwrap();
        assert_eq!(result, Some("12345".to_string()));
    }

    #[test]
    fn test_generate_local_media_path() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123456".to_string(),
            text: "Test".to_string(),
            created_at: None,
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let path = generator.generate_local_media_path(&tweet, 0, "https://example.com/image.jpg?param=value");
        assert!(path.contains("test_user"));
        assert!(path.contains("123456"));
        assert!(path.contains("image.jpg"));
        assert!(!path.contains("param=value")); // 应该去掉查询参数
    }

    #[test]
    fn test_generate_local_media_path_no_username() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123456".to_string(),
            text: "Test".to_string(),
            created_at: None,
            username: None,
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let path = generator.generate_local_media_path(&tweet, 0, "https://example.com/image.jpg");
        assert!(path.contains("123456"));
        assert!(path.contains("image.jpg"));
        assert!(!path.contains("test_user"));
    }

    #[test]
    fn test_generate_local_media_path_no_query() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123456".to_string(),
            text: "Test".to_string(),
            created_at: None,
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let path = generator.generate_local_media_path(&tweet, 0, "https://example.com/image.jpg");
        assert!(path.contains("test_user"));
        assert!(path.contains("123456"));
        assert!(path.contains("image.jpg"));
    }

    #[test]
    fn test_generate_local_media_path_uses_config_download_dir() {
        // 测试自定义下载目录的情况
        let mut config = create_test_config();
        config.download_dir = "custom/media".to_string();
        config.markdown_output = "custom/output.md".to_string();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123456".to_string(),
            text: "Test".to_string(),
            created_at: None,
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let path = generator.generate_local_media_path(&tweet, 0, "https://example.com/image.jpg");
        // 应该使用相对路径指向 custom/media 目录
        assert!(path.contains("media"));
        assert!(path.contains("test_user"));
        assert!(path.contains("123456"));
        assert!(path.contains("image.jpg"));
        // 不应该硬编码 ./downloads/
        assert!(!path.contains("./downloads/"));
    }

    #[test]
    fn test_extract_tweet_content_full() {
        let tweet_data = json!({
            "rest_id": "123456",
            "legacy": {
                "full_text": "Test tweet content",
                "created_at_ms": "1609459200000",
                "retweeted": false,
                "in_reply_to_screen_name": null,
                "quoted_status_id": null
            },
            "core": {
                "user_results": {
                    "result": {
                        "legacy": {
                            "screen_name": "test_user",
                            "name": "Test User"
                        }
                    }
                }
            }
        });
        
        let result = MarkdownGenerator::extract_tweet_content(&tweet_data).unwrap();
        assert_eq!(result.id, "123456");
        assert_eq!(result.text, "Test tweet content");
        assert_eq!(result.username, Some("test_user".to_string()));
        assert_eq!(result.display_name, Some("Test User".to_string()));
        assert!(!result.is_retweet);
    }

    #[test]
    fn test_extract_tweet_content_with_tweet_wrapper() {
        let tweet_data = json!({
            "content": {
                "itemContent": {
                    "tweet_results": {
                        "result": {
                            "tweet": {
                                "rest_id": "789",
                                "legacy": {
                                    "full_text": "Wrapped tweet"
                                }
                            }
                        }
                    }
                }
            }
        });
        
        let result = MarkdownGenerator::extract_tweet_content(&tweet_data).unwrap();
        assert_eq!(result.id, "789");
        assert_eq!(result.text, "Wrapped tweet");
    }

    #[test]
    fn test_extract_user_info_direct_core() {
        let tweet_obj = json!({
            "core": {
                "screen_name": "direct_user",
                "name": "Direct User"
            }
        });
        
        let (username, display_name) = tweet_parser::extract_user_info(&tweet_obj).unwrap();
        assert_eq!(username, Some("direct_user".to_string()));
        assert_eq!(display_name, Some("Direct User".to_string()));
    }

    #[test]
    fn test_extract_user_info_alternative_path() {
        let tweet_obj = json!({
            "core": {
                "user_results": {
                    "result": {
                        "core": {
                            "screen_name": "alt_user",
                            "name": "Alt User"
                        }
                    }
                }
            }
        });
        
        let (username, display_name) = tweet_parser::extract_user_info(&tweet_obj).unwrap();
        assert_eq!(username, Some("alt_user".to_string()));
        assert_eq!(display_name, Some("Alt User".to_string()));
    }

    #[test]
    fn test_extract_user_info_none() {
        let tweet_obj = json!({});
        
        let (username, display_name) = tweet_parser::extract_user_info(&tweet_obj).unwrap();
        assert_eq!(username, None);
        assert_eq!(display_name, None);
    }

    #[test]
    fn test_extract_media_urls_from_entities() {
        let legacy = json!({
            "entities": {
                "media": [
                    {
                        "type": "photo",
                        "media_url_https": "https://example.com/entity_photo.jpg"
                    }
                ]
            }
        });
        
        let result = tweet_parser::extract_media_urls(&legacy).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/entity_photo.jpg");
    }

    #[test]
    fn test_extract_media_urls_video_no_bitrate() {
        let legacy = json!({
            "extended_entities": {
                "media": [
                    {
                        "type": "video",
                        "video_info": {
                            "variants": [
                                {
                                    "url": "https://example.com/video.mp4"
                                }
                            ]
                        }
                    }
                ]
            }
        });
        
        let result = tweet_parser::extract_media_urls(&legacy).unwrap();
        // 没有bitrate的variant不会被选择
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_extract_media_urls_unknown_type() {
        let legacy = json!({
            "extended_entities": {
                "media": [
                    {
                        "type": "unknown",
                        "media_url_https": "https://example.com/unknown.jpg"
                    }
                ]
            }
        });
        
        let result = tweet_parser::extract_media_urls(&legacy).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_generate_markdown_with_retweet() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Retweeted content".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: true,
            retweeted_by: Some("original_user".to_string()),
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        let markdown = generator.generate_markdown().unwrap();
        assert!(markdown.contains("转推"));
    }

    #[test]
    fn test_generate_markdown_with_reply() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Reply content".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: Some("reply_user".to_string()),
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        let markdown = generator.generate_markdown().unwrap();
        assert!(markdown.contains("回复"));
    }

    #[test]
    fn test_generate_markdown_with_quote() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Quote content".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: Some("quoted_tweet_id".to_string()),
        };
        
        generator.add_tweet(tweet);
        let markdown = generator.generate_markdown().unwrap();
        assert!(markdown.contains("引用推文"));
    }

    #[test]
    fn test_generate_markdown_sorted_by_time() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet1 = TweetContent {
            id: "1".to_string(),
            text: "Older tweet".to_string(),
            created_at: Some(DateTime::from_timestamp(1000, 0).unwrap()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let tweet2 = TweetContent {
            id: "2".to_string(),
            text: "Newer tweet".to_string(),
            created_at: Some(DateTime::from_timestamp(2000, 0).unwrap()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet1);
        generator.add_tweet(tweet2);
        let markdown = generator.generate_markdown().unwrap();
        
        // 新推文应该在前面
        let newer_pos = markdown.find("Newer tweet").unwrap();
        let older_pos = markdown.find("Older tweet").unwrap();
        assert!(newer_pos < older_pos);
    }

    #[test]
    fn test_generate_markdown_sorted_with_none_time() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet1 = TweetContent {
            id: "1".to_string(),
            text: "No time tweet".to_string(),
            created_at: None,
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let tweet2 = TweetContent {
            id: "2".to_string(),
            text: "Has time tweet".to_string(),
            created_at: Some(DateTime::from_timestamp(2000, 0).unwrap()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet1);
        generator.add_tweet(tweet2);
        let markdown = generator.generate_markdown().unwrap();
        
        // 有时间戳的推文应该在前面
        let has_time_pos = markdown.find("Has time tweet").unwrap();
        let no_time_pos = markdown.find("No time tweet").unwrap();
        assert!(has_time_pos < no_time_pos);
    }

    #[test]
    fn test_extract_tweet_content_with_media() {
        let tweet_data = json!({
            "rest_id": "123",
            "legacy": {
                "full_text": "Tweet with media",
                "extended_entities": {
                    "media": [
                        {
                            "type": "photo",
                            "media_url_https": "https://example.com/photo.jpg"
                        }
                    ]
                }
            }
        });
        
        let result = MarkdownGenerator::extract_tweet_content(&tweet_data).unwrap();
        assert_eq!(result.media_urls.len(), 1);
        assert_eq!(result.media_urls[0], "https://example.com/photo.jpg");
    }

    #[test]
    fn test_extract_tweet_content_with_reply() {
        let tweet_data = json!({
            "rest_id": "123",
            "legacy": {
                "full_text": "Reply tweet",
                "in_reply_to_screen_name": "reply_target"
            }
        });
        
        let result = MarkdownGenerator::extract_tweet_content(&tweet_data).unwrap();
        assert_eq!(result.reply_to, Some("reply_target".to_string()));
    }

    #[test]
    fn test_extract_tweet_content_with_quote() {
        let tweet_data = json!({
            "rest_id": "123",
            "legacy": {
                "full_text": "Quote tweet",
                "quoted_status_id": "quoted123"
            }
        });
        
        let result = MarkdownGenerator::extract_tweet_content(&tweet_data).unwrap();
        assert_eq!(result.quote_tweet, Some("quoted123".to_string()));
    }

    #[test]
    fn test_extract_tweet_content_minimal() {
        let tweet_data = json!({
            "rest_id": "minimal",
            "legacy": {
                "full_text": "Minimal tweet"
            }
        });
        
        let result = MarkdownGenerator::extract_tweet_content(&tweet_data).unwrap();
        assert_eq!(result.id, "minimal");
        assert_eq!(result.text, "Minimal tweet");
        assert_eq!(result.username, None);
        assert_eq!(result.display_name, None);
        assert_eq!(result.media_urls.len(), 0);
        assert!(!result.is_retweet);
    }

    #[test]
    fn test_format_single_tweet_with_all_fields() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Complete tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: Some("Test User".to_string()),
            media_urls: vec!["https://example.com/image.jpg".to_string()],
            is_retweet: true,
            retweeted_by: Some("original_user".to_string()),
            reply_to: Some("reply_user".to_string()),
            quote_tweet: Some("quote_id".to_string()),
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("Complete tweet"));
        assert!(formatted.contains("test_user"));
        assert!(formatted.contains("Test User"));
        assert!(formatted.contains("转推"));
        assert!(formatted.contains("回复"));
        assert!(formatted.contains("引用推文"));
    }

    #[test]
    fn test_format_single_tweet_video_media() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Video tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec!["https://example.com/video.mp4".to_string()],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("video"));
        assert!(formatted.contains("<video"));
        assert!(formatted.contains("type=\"video/mp4\""));
    }

    #[test]
    fn test_format_single_tweet_video_webm() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "WebM video tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec!["https://example.com/video.webm".to_string()],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("video"));
        assert!(formatted.contains("<video"));
        assert!(formatted.contains("type=\"video/webm\""));
        assert!(!formatted.contains("type=\"video/mp4\""));
    }

    #[test]
    fn test_format_single_tweet_video_mov() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "MOV video tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec!["https://example.com/video.mov".to_string()],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("video"));
        assert!(formatted.contains("<video"));
        assert!(formatted.contains("type=\"video/quicktime\""));
        assert!(!formatted.contains("type=\"video/mp4\""));
    }

    #[test]
    fn test_format_single_tweet_multiple_video_formats() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Multiple video formats".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![
                "https://example.com/video1.mp4".to_string(),
                "https://example.com/video2.webm".to_string(),
                "https://example.com/video3.mov".to_string(),
            ],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("type=\"video/mp4\""));
        assert!(formatted.contains("type=\"video/webm\""));
        assert!(formatted.contains("type=\"video/quicktime\""));
    }

    #[test]
    fn test_save_markdown() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let output_file = temp_dir.path().join("test_output.md");
        
        let mut config = create_test_config();
        config.markdown_output = output_file.to_str().unwrap().to_string();
        
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Test tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        generator.save_markdown().unwrap();
        
        assert!(output_file.exists());
        let content = std::fs::read_to_string(&output_file).unwrap();
        assert!(content.contains("Test tweet"));
        assert!(content.contains("test_user"));
    }

    #[test]
    fn test_save_markdown_empty() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let output_file = temp_dir.path().join("empty_output.md");
        
        let mut config = create_test_config();
        config.markdown_output = output_file.to_str().unwrap().to_string();
        
        let generator = MarkdownGenerator::new(config);
        generator.save_markdown().unwrap();
        
        assert!(output_file.exists());
        let content = std::fs::read_to_string(&output_file).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_save_markdown_creates_directory() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let output_file = temp_dir.path().join("subdir/test_output.md");
        
        let mut config = create_test_config();
        config.markdown_output = output_file.to_str().unwrap().to_string();
        
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Test".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        generator.save_markdown().unwrap();
        
        assert!(output_file.exists());
        assert!(output_file.parent().unwrap().exists());
    }

    #[test]
    fn test_format_single_tweet_no_username() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Tweet without username".to_string(),
            created_at: Some(Utc::now()),
            username: None,
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("Tweet without username"));
        assert!(formatted.contains("unknown")); // 应该使用unknown作为默认用户名
    }

    #[test]
    fn test_format_single_tweet_no_created_at() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Tweet without time".to_string(),
            created_at: None,
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("Tweet without time"));
        // 不应该包含发布时间
        assert!(!formatted.contains("发布时间"));
    }

    #[test]
    fn test_generate_markdown_multiple_media() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Multi media tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![
                "https://example.com/image1.jpg".to_string(),
                "https://example.com/image2.jpg".to_string(),
                "https://example.com/video.mp4".to_string(),
            ],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        let markdown = generator.generate_markdown().unwrap();
        assert!(markdown.contains("image1.jpg"));
        assert!(markdown.contains("image2.jpg"));
        assert!(markdown.contains("video.mp4"));
    }

    #[test]
    fn test_generate_markdown_sorting_all_none() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet1 = TweetContent {
            id: "1".to_string(),
            text: "No time 1".to_string(),
            created_at: None,
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let tweet2 = TweetContent {
            id: "2".to_string(),
            text: "No time 2".to_string(),
            created_at: None,
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet1);
        generator.add_tweet(tweet2);
        let markdown = generator.generate_markdown().unwrap();
        // 两个都没有时间，应该都能生成
        assert!(markdown.contains("No time 1"));
        assert!(markdown.contains("No time 2"));
    }

    #[test]
    fn test_generate_markdown_username_fallback() {
        let config = create_test_config();
        let mut generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Tweet".to_string(),
            created_at: Some(Utc::now()),
            username: None,
            display_name: None,
            media_urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        generator.add_tweet(tweet);
        let markdown = generator.generate_markdown().unwrap();
        // 应该使用 "Unknown" 作为默认用户名
        assert!(markdown.contains("Unknown"));
    }

    #[test]
    fn test_format_single_tweet_image_media() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Image tweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec!["https://example.com/image.jpg".to_string()],
            is_retweet: false,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("image.jpg"));
        assert!(formatted.contains("<img"));
        assert!(!formatted.contains("<video"));
    }

    #[test]
    fn test_format_single_tweet_retweet_no_user() {
        let config = create_test_config();
        let generator = MarkdownGenerator::new(config);
        
        let tweet = TweetContent {
            id: "123".to_string(),
            text: "Retweet".to_string(),
            created_at: Some(Utc::now()),
            username: Some("test_user".to_string()),
            display_name: None,
            media_urls: vec![],
            is_retweet: true,
            retweeted_by: None,
            reply_to: None,
            quote_tweet: None,
        };
        
        let formatted = generator.format_single_tweet(&tweet, 1).unwrap();
        assert!(formatted.contains("转推"));
    }
}
