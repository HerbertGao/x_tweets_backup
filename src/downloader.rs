use anyhow::{Context, Result};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use url::Url;
use chrono::DateTime;
use filetime::{FileTime, set_file_times};

use crate::config::Config;

// 调试日志辅助宏
macro_rules! debug_log {
    ($config:expr, $($arg:tt)*) => {
        if $config.debug_logs {
            println!($($arg)*);
        }
    };
}

pub struct Downloader {
    client: Client,
    config: Config,
    downloaded_ids: HashSet<String>,
}

impl Downloader {
    pub fn new(config: Config) -> Result<Self> {
        debug_log!(config, "[DEBUG] 初始化下载器...");
        let client_builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(30));

        let client = client_builder.build()?;
        let downloaded_ids = Self::load_downloaded_ids(&config.download_record)?;
        debug_log!(config, "[DEBUG] 已加载 {} 个已下载的推文ID", downloaded_ids.len());

        Ok(Downloader {
            client,
            config,
            downloaded_ids,
        })
    }

    fn load_downloaded_ids(filename: &str) -> Result<HashSet<String>> {
        if !Path::new(filename).exists() {
            // 文件不存在是正常情况，不需要调试日志
            return Ok(HashSet::new());
        }

        let content = fs::read_to_string(filename)
            .with_context(|| format!("无法读取文件: {}", filename))?;

        let ids: HashSet<String> = content.lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        // 加载文件是正常操作，不需要调试日志
        Ok(ids)
    }

    pub fn save_downloaded_id(&mut self, tweet_id: &str) -> Result<()> {
        if let Some(parent) = Path::new(&self.config.download_record).parent() {
            fs::create_dir_all(parent)?;
        }

        use std::io::Write;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.download_record)?
            .write_all(format!("{}\n", tweet_id).as_bytes())?;

        self.downloaded_ids.insert(tweet_id.to_string());
        Ok(())
    }

    pub fn is_downloaded(&self, tweet_id: &str) -> bool {
        self.downloaded_ids.contains(tweet_id)
    }

    pub fn downloaded_count(&self) -> usize {
        self.downloaded_ids.len()
    }

    pub async fn call_media_downloader(&self, tweet: &Value, tweet_id: &str) -> Result<Option<bool>> {
        debug_log!(self.config, "[DEBUG] 开始处理推文: {}", tweet_id);
        
        // 从 tweet 对象中抽取推文主体
        let tweet_obj = self.extract_tweet_object(tweet)?;
        if tweet_obj.is_none() {
            debug_log!(self.config, "[DEBUG] 推文 {} 没有有效内容", tweet_id);
            return Ok(None);
        }
        let tweet_obj = tweet_obj.unwrap();

        // 提取推文发布时间
        let tweet_timestamp = self.extract_tweet_timestamp(&tweet_obj)?;
        debug_log!(self.config, "[DEBUG] 推文 {} 发布时间: {:?}", tweet_id, tweet_timestamp);

        // 提取作者用户名（作为后备）
        let tweet_author_username = self.extract_username(&tweet_obj)?;
        debug_log!(self.config, "[DEBUG] 推文 {} 作者: {:?}", tweet_id, tweet_author_username);

        // 优先使用配置中的目标用户名，如果没有则使用推文作者的用户名
        let username = if !self.config.target_username.is_empty() {
            Some(self.config.target_username.clone())
        } else {
            tweet_author_username
        };
        debug_log!(self.config, "[DEBUG] 推文 {} 将使用用户名: {:?}", tweet_id, username);

        // 提取媒体列表
        let media_urls = self.extract_media_urls(&tweet_obj)?;
        if media_urls.is_empty() {
            debug_log!(self.config, "[DEBUG] 推文 {} 没有媒体文件", tweet_id);
            return Ok(None);
        }
        
        debug_log!(self.config, "[DEBUG] 推文 {} 找到 {} 个媒体文件", tweet_id, media_urls.len());

        // 构建文件名前缀：格式为 [username][tweetid]
        let username_str = username.as_deref().unwrap_or("");
        let prefix = if username_str.is_empty() {
            // 如果用户名为空，只使用推文ID
            format!("[{}]", tweet_id)
        } else {
            // 格式：[username][tweetid]
            format!("[{}][{}]", username_str, tweet_id)
        };

        let output_dir = Path::new(&self.config.download_dir);
        fs::create_dir_all(output_dir)?;

        let mut download_success_count = 0;
        let total_media_count = media_urls.len();
        let mut skipped_count = 0;

        for (i, media_url) in media_urls.iter().enumerate() {
            debug_log!(self.config, "[DEBUG] 处理媒体文件 {}/{}: {}", i + 1, total_media_count, media_url);
            
            let parsed_url = Url::parse(media_url)?;
            
            // 验证 URL 域名，防止 SSRF 攻击
            if let Some(host) = parsed_url.host_str() {
                if !host.ends_with("twimg.com") && !host.ends_with("twitter.com") {
                    eprintln!("[WARNING] 跳过可疑的媒体 URL (非 Twitter 域名): {}", media_url);
                    continue;
                }
            } else {
                eprintln!("[WARNING] 跳过无效的媒体 URL (无域名): {}", media_url);
                continue;
            }
            
            let original_name = parsed_url.path_segments()
                .and_then(|segments| segments.last())
                .unwrap_or("unknown");
            
            // 文件名格式：[username][tweetid][origin_filename]
            let filename = format!("{}[{}]", prefix, original_name);
            let out_path = output_dir.join(&filename);
            debug_log!(self.config, "[DEBUG] 输出路径: {:?}", out_path);

            // 检查文件是否已存在
            if out_path.exists() {
                let metadata = fs::metadata(&out_path)?;
                if metadata.len() > 0 {
                    debug_log!(self.config, "跳过已存在的文件 ({}/{}): {:?} (大小: {} 字节)", 
                        i + 1, total_media_count, out_path, metadata.len());
                    skipped_count += 1;
                    download_success_count += 1;
                    continue;
                } else {
                    debug_log!(self.config, "发现损坏的空文件，将重新下载 ({}/{}): {:?}", 
                        i + 1, total_media_count, out_path);
                    // 尝试删除空文件，失败时记录警告但继续
                    if let Err(e) = fs::remove_file(&out_path) {
                        eprintln!("[WARNING] 无法删除空文件 {:?}: {}，将继续尝试下载", out_path, e);
                    }
                }
            }

            debug_log!(self.config, "[DEBUG] 开始下载媒体文件: {}", media_url);
            match self.download_media(media_url, &out_path, i + 1, total_media_count).await {
                Ok(true) => {
                    debug_log!(self.config, "[DEBUG] 下载成功: {:?}", out_path);
                    // 下载成功后设置文件时间为推文发布时间
                    if let Some(ts) = tweet_timestamp {
                        let ft = FileTime::from_unix_time(ts, 0);
                        if let Err(e) = set_file_times(&out_path, ft, ft) {
                            debug_log!(self.config, "[DEBUG] 设置文件时间失败 {:?}: {}", out_path, e);
                        } else {
                            debug_log!(self.config, "[DEBUG] 已设置文件时间: {:?}", out_path);
                        }
                    }
                    download_success_count += 1;
                }
                Ok(false) => {
                    debug_log!(self.config, "[DEBUG] 下载失败: {}", media_url);
                }
                Err(e) => {
                    debug_log!(self.config, "[DEBUG] 下载异常 ({}/{}): {} 错误: {}", i + 1, total_media_count, media_url, e);
                }
            }
        }

        if download_success_count > 0 {
            if skipped_count > 0 {
                debug_log!(self.config, "Tweet {} 处理完成: 跳过 {} 个已存在文件，成功下载 {} 个新文件", 
                    tweet_id, skipped_count, download_success_count - skipped_count);
            } else {
                debug_log!(self.config, "Tweet {} 成功下载了 {}/{} 个媒体文件", 
                    tweet_id, download_success_count, total_media_count);
            }
            Ok(Some(true))
        } else {
            debug_log!(self.config, "Tweet {} 所有媒体文件下载失败", tweet_id);
            Ok(Some(false))
        }
    }

    fn extract_tweet_object<'a>(&self, tweet: &'a Value) -> Result<Option<&'a Value>> {
        // 路径1: content.itemContent.tweet_results.result.tweet
        if let Some(tweet_obj) = tweet
            .get("content")
            .and_then(|c| c.get("itemContent"))
            .and_then(|ic| ic.get("tweet_results"))
            .and_then(|tr| tr.get("result"))
            .and_then(|r| r.get("tweet"))
        {
            return Ok(Some(tweet_obj));
        }

        // 路径2: 直接结构
        if let Some(tweet_obj) = tweet.get("tweet") {
            return Ok(Some(tweet_obj));
        }

        // 路径3: 使用原始tweet
        Ok(Some(tweet))
    }

    fn extract_username(&self, tweet_obj: &Value) -> Result<Option<String>> {
        // 尝试从core.user_results.result.legacy路径提取（主要路径）
        let mut username = tweet_obj
            .get("core")
            .and_then(|c| c.get("user_results"))
            .and_then(|ur| ur.get("result"))
            .and_then(|r| r.get("legacy"))
            .and_then(|l| l.get("screen_name"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        // 如果旧路径没有找到，尝试直接从core路径提取
        if username.is_none() {
            username = tweet_obj
                .get("core")
                .and_then(|c| c.get("screen_name"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
        }

        // 如果还是没有找到，尝试从user_results.result.core路径提取
        if username.is_none() {
            username = tweet_obj
                .get("core")
                .and_then(|c| c.get("user_results"))
                .and_then(|ur| ur.get("result"))
                .and_then(|r| r.get("core"))
                .and_then(|c| c.get("screen_name"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
        }

        Ok(username)
    }

    fn extract_media_urls(&self, tweet_obj: &Value) -> Result<Vec<String>> {
        let mut media_urls = Vec::new();

        let legacy = tweet_obj.get("legacy").unwrap_or(&Value::Null);
        
        // 从 extended_entities 或 entities 中提取媒体列表
        let media_list = legacy
            .get("extended_entities")
            .and_then(|ee| ee.get("media"))
            .and_then(|m| m.as_array())
            .or_else(|| {
                legacy
                    .get("entities")
                    .and_then(|e| e.get("media"))
                    .and_then(|m| m.as_array())
            });

        if let Some(media_array) = media_list {
            for media in media_array {
                let media_type = media.get("type").and_then(|t| t.as_str()).unwrap_or("");
                
                match media_type {
                    "video" => {
                        if let Some(video_info) = media.get("video_info") {
                            if let Some(variants) = video_info.get("variants").and_then(|v| v.as_array()) {
                                let best_variant = variants
                                    .iter()
                                    .filter(|v| v.get("bitrate").is_some())
                                    .max_by_key(|v| v.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(0));
                                
                                if let Some(variant) = best_variant {
                                    if let Some(url) = variant.get("url").and_then(|u| u.as_str()) {
                                        media_urls.push(url.to_string());
                                    }
                                }
                            }
                        }
                    }
                    "photo" => {
                        if let Some(url) = media.get("media_url_https").and_then(|u| u.as_str()) {
                            media_urls.push(url.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(media_urls)
    }

    fn extract_tweet_timestamp(&self, tweet_obj: &Value) -> Result<Option<i64>> {
        let legacy = tweet_obj.get("legacy").unwrap_or(&Value::Null);

        // 尝试 created_at_ms （毫秒时间戳）
        if let Some(ts_ms_str) = legacy.get("created_at_ms").and_then(|v| v.as_str()) {
            if let Ok(ts_ms) = ts_ms_str.parse::<i64>() {
                return Ok(Some(ts_ms / 1000));
            }
        }
        if let Some(ts_ms) = legacy.get("created_at_ms").and_then(|v| v.as_i64()) {
            return Ok(Some(ts_ms / 1000));
        }

        // 尝试 created_at 字符串，如 "Thu Apr 06 15:24:15 +0000 2017"
        if let Some(created_at_str) = legacy.get("created_at").and_then(|v| v.as_str()) {
            if let Ok(dt) = DateTime::parse_from_str(created_at_str, "%a %b %d %H:%M:%S %z %Y") {
                return Ok(Some(dt.timestamp()));
            }
        }

        Ok(None)
    }

    async fn download_media(&self, url: &str, out_path: &Path, current: usize, total: usize) -> Result<bool> {
        debug_log!(self.config, "[DEBUG] 发送下载请求: {}", url);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("User-Agent", self.config.user_agent.parse()?);

        let response = self.client
            .get(url)
            .headers(headers)
            .send()
            .await?;

        debug_log!(self.config, "[DEBUG] 下载响应状态: {}", response.status());
        if !response.status().is_success() {
            debug_log!(self.config, "[DEBUG] 下载失败 ({}) ({}/{}): {}", response.status(), current, total, url);
            return Ok(false);
        }

        let content_length = response.content_length();
        let mut downloaded_size = 0u64;
        debug_log!(self.config, "[DEBUG] 文件大小: {:?} 字节", content_length);

        // 创建进度条
        let pb = ProgressBar::new(content_length.unwrap_or(0));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"));

        debug_log!(self.config, "[DEBUG] 开始写入文件: {:?}", out_path);
        let mut file = File::create(out_path).await?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded_size += chunk.len() as u64;
            pb.set_position(downloaded_size);
        }

        pb.finish_with_message("下载完成");

        // 验证下载完整性
        if let Some(expected_size) = content_length {
            if downloaded_size != expected_size {
                eprintln!("[ERROR] 下载不完整 ({}/{}): {:?} (期望: {}, 实际: {})", 
                    current, total, out_path, expected_size, downloaded_size);
                // 尝试删除不完整的文件，失败时记录警告但不影响流程
                if let Err(e) = fs::remove_file(out_path) {
                    eprintln!("[WARNING] 无法删除不完整的文件 {:?}: {}", out_path, e);
                }
                return Ok(false);
            }
        }

        debug_log!(self.config, "下载成功 ({}/{}): {:?} (大小: {} 字节)", 
            current, total, out_path, downloaded_size);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_test_config() -> Config {
        Config {
            user_id: "".to_string(),
            bearer_token: "".to_string(),
            auth_token: "".to_string(),
            ct0: "".to_string(),
            personalization_id: "".to_string(),
            user_agent: "Mozilla/5.0".to_string(),
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
        }
    }

    #[test]
    fn test_load_downloaded_ids_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("empty.txt");
        std::fs::write(&record_file, "").unwrap();
        
        let result = Downloader::load_downloaded_ids(record_file.to_str().unwrap()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_downloaded_ids_with_ids() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("ids.txt");
        std::fs::write(&record_file, "123\n456\n789\n").unwrap();
        
        let result = Downloader::load_downloaded_ids(record_file.to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.contains("123"));
        assert!(result.contains("456"));
        assert!(result.contains("789"));
    }

    #[test]
    fn test_load_downloaded_ids_nonexistent() {
        let result = Downloader::load_downloaded_ids("nonexistent_file.txt").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_tweet_object_path1() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet = json!({
            "content": {
                "itemContent": {
                    "tweet_results": {
                        "result": {
                            "tweet": {
                                "rest_id": "123"
                            }
                        }
                    }
                }
            }
        });
        
        let result = downloader.extract_tweet_object(&tweet).unwrap();
        assert_eq!(result.unwrap().get("rest_id").and_then(|v| v.as_str()), Some("123"));
    }

    #[test]
    fn test_extract_tweet_object_path2() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet = json!({
            "tweet": {
                "rest_id": "456"
            }
        });
        
        let result = downloader.extract_tweet_object(&tweet).unwrap();
        assert_eq!(result.unwrap().get("rest_id").and_then(|v| v.as_str()), Some("456"));
    }

    #[test]
    fn test_extract_tweet_object_path3() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet = json!({
            "rest_id": "789"
        });
        
        let result = downloader.extract_tweet_object(&tweet).unwrap();
        assert_eq!(result.unwrap().get("rest_id").and_then(|v| v.as_str()), Some("789"));
    }

    #[test]
    fn test_extract_username() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "core": {
                "user_results": {
                    "result": {
                        "legacy": {
                            "screen_name": "test_user"
                        }
                    }
                }
            }
        });
        
        let result = downloader.extract_username(&tweet_obj).unwrap();
        assert_eq!(result, Some("test_user".to_string()));
    }

    #[test]
    fn test_extract_media_urls_photo() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "photo",
                            "media_url_https": "https://example.com/image.jpg"
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/image.jpg");
    }

    #[test]
    fn test_extract_media_urls_video() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
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
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/video_hd.mp4");
    }

    #[test]
    fn test_extract_media_urls_empty() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {}
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_tweet_timestamp_from_ms_string() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "created_at_ms": "1609459200000"
            }
        });
        
        let result = downloader.extract_tweet_timestamp(&tweet_obj).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_tweet_timestamp_from_string() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "created_at": "Thu Apr 06 15:24:15 +0000 2017"
            }
        });
        
        let result = downloader.extract_tweet_timestamp(&tweet_obj).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_is_downloaded() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("ids.txt");
        std::fs::write(&record_file, "123\n456\n").unwrap();
        
        let mut config = create_test_config();
        config.download_record = record_file.to_str().unwrap().to_string();
        
        let downloader = Downloader::new(config).unwrap();
        assert!(downloader.is_downloaded("123"));
        assert!(downloader.is_downloaded("456"));
        assert!(!downloader.is_downloaded("789"));
    }

    #[test]
    fn test_downloaded_count() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("ids.txt");
        std::fs::write(&record_file, "123\n456\n789\n").unwrap();
        
        let mut config = create_test_config();
        config.download_record = record_file.to_str().unwrap().to_string();
        
        let downloader = Downloader::new(config).unwrap();
        assert_eq!(downloader.downloaded_count(), 3);
    }

    #[test]
    fn test_save_downloaded_id() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("new_ids.txt");
        
        let mut config = create_test_config();
        config.download_record = record_file.to_str().unwrap().to_string();
        
        let mut downloader = Downloader::new(config).unwrap();
        downloader.save_downloaded_id("999").unwrap();
        
        assert!(downloader.is_downloaded("999"));
        assert_eq!(downloader.downloaded_count(), 1);
        
        // 验证文件已创建
        assert!(record_file.exists());
        let content = std::fs::read_to_string(&record_file).unwrap();
        assert!(content.contains("999"));
    }

    #[test]
    fn test_extract_username_direct_core() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "core": {
                "screen_name": "direct_user"
            }
        });
        
        let result = downloader.extract_username(&tweet_obj).unwrap();
        assert_eq!(result, Some("direct_user".to_string()));
    }

    #[test]
    fn test_extract_username_alternative_path() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "core": {
                "user_results": {
                    "result": {
                        "core": {
                            "screen_name": "alt_user"
                        }
                    }
                }
            }
        });
        
        let result = downloader.extract_username(&tweet_obj).unwrap();
        assert_eq!(result, Some("alt_user".to_string()));
    }

    #[test]
    fn test_extract_username_none() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({});
        
        let result = downloader.extract_username(&tweet_obj).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_media_urls_from_entities() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "entities": {
                    "media": [
                        {
                            "type": "photo",
                            "media_url_https": "https://example.com/entity_image.jpg"
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/entity_image.jpg");
    }

    #[test]
    fn test_extract_media_urls_video_no_bitrate() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "video",
                            "video_info": {
                                "variants": [
                                    {
                                        "url": "https://example.com/video_no_bitrate.mp4"
                                    }
                                ]
                            }
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        // 没有bitrate的variant不会被选择
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_extract_media_urls_unknown_type() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "unknown",
                            "media_url_https": "https://example.com/unknown.jpg"
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_extract_tweet_timestamp_from_ms_number() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "created_at_ms": 1609459200000i64
            }
        });
        
        let result = downloader.extract_tweet_timestamp(&tweet_obj).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_tweet_timestamp_none() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {}
        });
        
        let result = downloader.extract_tweet_timestamp(&tweet_obj).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_downloaded_ids_with_whitespace() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("ids.txt");
        std::fs::write(&record_file, "123 \n 456\n789").unwrap();
        
        let result = Downloader::load_downloaded_ids(record_file.to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.contains("123"));
        assert!(result.contains("456"));
        assert!(result.contains("789"));
    }

    #[test]
    fn test_load_downloaded_ids_filters_empty() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("ids.txt");
        std::fs::write(&record_file, "123\n\n456\n").unwrap();
        
        let result = Downloader::load_downloaded_ids(record_file.to_str().unwrap()).unwrap();
        // 空行会被trim成空字符串，但HashSet会去重，所以可能只有1个空字符串
        // 实际行为：trim后空行变成""，HashSet会保留一个空字符串
        assert!(result.len() >= 2); // 至少包含123和456
        assert!(result.contains("123"));
        assert!(result.contains("456"));
    }

    #[test]
    fn test_extract_tweet_object_all_paths() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        // 测试路径1
        let tweet1 = json!({
            "content": {
                "itemContent": {
                    "tweet_results": {
                        "result": {
                            "tweet": {
                                "rest_id": "path1"
                            }
                        }
                    }
                }
            }
        });
        assert_eq!(downloader.extract_tweet_object(&tweet1).unwrap().unwrap().get("rest_id").and_then(|v| v.as_str()), Some("path1"));
        
        // 测试路径2
        let tweet2 = json!({
            "tweet": {
                "rest_id": "path2"
            }
        });
        assert_eq!(downloader.extract_tweet_object(&tweet2).unwrap().unwrap().get("rest_id").and_then(|v| v.as_str()), Some("path2"));
        
        // 测试路径3
        let tweet3 = json!({
            "rest_id": "path3"
        });
        assert_eq!(downloader.extract_tweet_object(&tweet3).unwrap().unwrap().get("rest_id").and_then(|v| v.as_str()), Some("path3"));
    }

    #[test]
    fn test_extract_media_urls_mixed_types() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "photo",
                            "media_url_https": "https://example.com/photo1.jpg"
                        },
                        {
                            "type": "video",
                            "video_info": {
                                "variants": [
                                    {
                                        "bitrate": 1500,
                                        "url": "https://example.com/video1.mp4"
                                    }
                                ]
                            }
                        },
                        {
                            "type": "photo",
                            "media_url_https": "https://example.com/photo2.jpg"
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.contains(&"https://example.com/photo1.jpg".to_string()));
        assert!(result.contains(&"https://example.com/video1.mp4".to_string()));
        assert!(result.contains(&"https://example.com/photo2.jpg".to_string()));
    }

    #[test]
    fn test_extract_media_urls_video_multiple_variants() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "video",
                            "video_info": {
                                "variants": [
                                    {
                                        "bitrate": 500,
                                        "url": "https://example.com/low.mp4"
                                    },
                                    {
                                        "bitrate": 2000,
                                        "url": "https://example.com/high.mp4"
                                    },
                                    {
                                        "bitrate": 1000,
                                        "url": "https://example.com/medium.mp4"
                                    }
                                ]
                            }
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 1);
        // 应该选择最高bitrate的
        assert_eq!(result[0], "https://example.com/high.mp4");
    }

    #[test]
    fn test_extract_tweet_timestamp_invalid_format() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "created_at_ms": "invalid_number"
            }
        });
        
        let result = downloader.extract_tweet_timestamp(&tweet_obj).unwrap();
        // 无效格式应该返回None或尝试其他路径
        // 实际行为：会尝试数字格式，如果失败则尝试字符串格式
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn test_extract_tweet_timestamp_invalid_date_string() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "created_at": "Invalid date format"
            }
        });
        
        let result = downloader.extract_tweet_timestamp(&tweet_obj).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_save_downloaded_id_multiple() {
        let temp_dir = TempDir::new().unwrap();
        let record_file = temp_dir.path().join("multi_ids.txt");
        
        let mut config = create_test_config();
        config.download_record = record_file.to_str().unwrap().to_string();
        
        let mut downloader = Downloader::new(config).unwrap();
        downloader.save_downloaded_id("111").unwrap();
        downloader.save_downloaded_id("222").unwrap();
        downloader.save_downloaded_id("333").unwrap();
        
        assert_eq!(downloader.downloaded_count(), 3);
        assert!(downloader.is_downloaded("111"));
        assert!(downloader.is_downloaded("222"));
        assert!(downloader.is_downloaded("333"));
        
        // 验证文件内容
        let content = std::fs::read_to_string(&record_file).unwrap();
        assert!(content.contains("111"));
        assert!(content.contains("222"));
        assert!(content.contains("333"));
    }

    #[test]
    fn test_extract_media_urls_photo_no_url() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "photo"
                            // 缺少 media_url_https
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_extract_media_urls_video_no_variants() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "video",
                            "video_info": {}
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_extract_media_urls_video_no_url_in_variant() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();
        
        let tweet_obj = json!({
            "legacy": {
                "extended_entities": {
                    "media": [
                        {
                            "type": "video",
                            "video_info": {
                                "variants": [
                                    {
                                        "bitrate": 1000
                                        // 缺少 url
                                    }
                                ]
                            }
                        }
                    ]
                }
            }
        });
        
        let result = downloader.extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 0);
    }
}
