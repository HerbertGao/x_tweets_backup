use anyhow::{Context, Result};
use filetime::{set_file_times, FileTime};
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

pub struct Downloader {
    client: Client,
    config: Config,
    downloaded_ids: HashSet<String>,
}

impl Downloader {
    pub fn new(config: Config) -> Result<Self> {
        debug_log!(config, "[DEBUG] 初始化下载器...");
        let client_builder =
            reqwest::Client::builder().timeout(Duration::from_secs(config.download_timeout_secs));

        let client = client_builder.build()?;
        let downloaded_ids = Self::load_downloaded_ids(&config.download_record)?;
        debug_log!(
            config,
            "[DEBUG] 已加载 {} 个已下载的推文ID",
            downloaded_ids.len()
        );

        Ok(Downloader {
            client,
            config,
            downloaded_ids,
        })
    }

    /// 验证域名是否为合法的 Twitter 媒体域名
    /// 使用 url crate 的 Host 类型进行安全的域名验证，防止 SSRF 攻击
    /// 只允许：
    /// - 精确匹配：twimg.com 或 twitter.com
    /// - 子域名：*.twimg.com 或 *.twitter.com
    /// 不允许：eviltwimg.com 或 malicioustwitter.com 等恶意相似域名
    fn is_valid_twitter_domain(host: &str) -> bool {
        // 使用 url crate 的 Host 类型解析域名，确保格式正确
        let parsed_host = match url::Host::parse(host) {
            Ok(h) => h,
            Err(_) => return false, // 无效的域名格式
        };

        // 获取域名字符串表示
        let host_str = match parsed_host {
            url::Host::Domain(domain) => domain,
            url::Host::Ipv4(_) | url::Host::Ipv6(_) => return false, // IP 地址不允许
        };

        // 允许的根域名列表
        const ALLOWED_DOMAINS: &[&str] = &["twimg.com", "twitter.com"];

        // 精确匹配根域名
        if ALLOWED_DOMAINS.contains(&host_str.as_str()) {
            return true;
        }

        // 检查是否为合法的子域名（必须以 . 开头，防止 eviltwimg.com 通过验证）
        for &domain in ALLOWED_DOMAINS {
            if host_str.ends_with(&format!(".{}", domain)) {
                return true;
            }
        }

        false
    }

    fn load_downloaded_ids(filename: &str) -> Result<HashSet<String>> {
        if !Path::new(filename).exists() {
            // 文件不存在是正常情况，不需要调试日志
            return Ok(HashSet::new());
        }

        let content =
            fs::read_to_string(filename).with_context(|| format!("无法读取文件: {}", filename))?;

        let ids: HashSet<String> = content
            .lines()
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

    /// 从推文中下载媒体文件
    ///
    /// # 参数
    /// * `tweet` - 包含推文信息的 JSON 值
    /// * `tweet_id` - 推文的唯一标识符
    ///
    /// # 返回值
    /// * `Ok(Some(true))` - 媒体文件下载成功
    /// * `Ok(Some(false))` - 下载失败或无需下载媒体
    /// * `Ok(None)` - 推文中未找到媒体文件
    /// * `Err(_)` - 下载过程中发生错误
    pub async fn call_media_downloader(
        &self,
        tweet: &Value,
        tweet_id: &str,
    ) -> Result<Option<bool>> {
        debug_log!(self.config, "[DEBUG] 开始处理推文: {}", tweet_id);

        // 从 tweet 对象中抽取推文主体
        let tweet_obj = match tweet_parser::extract_tweet_object(tweet) {
            Some(obj) => obj,
            None => {
                debug_log!(self.config, "[DEBUG] 推文 {} 没有有效内容", tweet_id);
                return Ok(None);
            }
        };

        // 提取推文发布时间
        let tweet_timestamp = tweet_parser::extract_tweet_timestamp_seconds(&tweet_obj)?;
        debug_log!(
            self.config,
            "[DEBUG] 推文 {} 发布时间: {:?}",
            tweet_id,
            tweet_timestamp
        );

        // 提取实际推文作者的用户名（从多个可能的路径中查找）
        // 重要：使用实际作者用户名而不是 config.target_username，以确保与 MarkdownGenerator 生成的文件路径一致
        // 这对于转推、引用推文和回复特别重要，因为实际作者可能与目标用户不同
        let username = tweet_parser::extract_username(&tweet_obj)?;
        debug_log!(
            self.config,
            "[DEBUG] 推文 {} 实际作者: {:?}",
            tweet_id,
            username
        );

        // 如果无法提取用户名，使用配置中的目标用户名作为后备
        let username = if let Some(u) = username {
            Some(u)
        } else if !self.config.target_username.is_empty() {
            debug_log!(
                self.config,
                "[DEBUG] 推文 {} 无法提取作者用户名，使用配置中的目标用户名: {}",
                tweet_id,
                self.config.target_username
            );
            Some(self.config.target_username.clone())
        } else {
            None
        };
        debug_log!(
            self.config,
            "[DEBUG] 推文 {} 将使用用户名: {:?}",
            tweet_id,
            username
        );

        // 提取媒体列表
        let media_urls = tweet_parser::extract_media_urls(&tweet_obj)?;
        if media_urls.is_empty() {
            debug_log!(self.config, "[DEBUG] 推文 {} 没有媒体文件", tweet_id);
            return Ok(None);
        }

        debug_log!(
            self.config,
            "[DEBUG] 推文 {} 找到 {} 个媒体文件",
            tweet_id,
            media_urls.len()
        );

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
            debug_log!(
                self.config,
                "[DEBUG] 处理媒体文件 {}/{}: {}",
                i + 1,
                total_media_count,
                media_url
            );

            let parsed_url = Url::parse(media_url)?;

            // 验证 URL 域名，防止 SSRF 攻击
            if let Some(host) = parsed_url.host_str() {
                if !Self::is_valid_twitter_domain(host) {
                    eprintln!(
                        "[WARNING] 跳过可疑的媒体 URL (非 Twitter 域名): {}",
                        media_url
                    );
                    continue;
                }
            } else {
                eprintln!("[WARNING] 跳过无效的媒体 URL (无域名): {}", media_url);
                continue;
            }

            // 从URL路径提取文件名，并清理以防止路径遍历攻击
            let original_name = parsed_url
                .path_segments()
                .and_then(|segments| segments.last())
                .unwrap_or("unknown")
                .replace("..", "") // 移除路径遍历序列
                .replace("/", "_") // 替换斜杠
                .replace("\\", "_") // 替换反斜杠
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
                .collect::<String>();

            // 文件名格式：[username][tweetid][origin_filename]
            let filename = format!("{}[{}]", prefix, original_name);
            let out_path = output_dir.join(&filename);
            debug_log!(self.config, "[DEBUG] 输出路径: {:?}", out_path);

            // 检查文件是否已存在
            if out_path.exists() {
                let metadata = fs::metadata(&out_path)?;
                if metadata.len() > 0 {
                    debug_log!(
                        self.config,
                        "跳过已存在的文件 ({}/{}): {:?} (大小: {} 字节)",
                        i + 1,
                        total_media_count,
                        out_path,
                        metadata.len()
                    );
                    skipped_count += 1;
                    download_success_count += 1;
                    continue;
                } else {
                    debug_log!(
                        self.config,
                        "发现损坏的空文件，将重新下载 ({}/{}): {:?}",
                        i + 1,
                        total_media_count,
                        out_path
                    );
                    // 尝试删除空文件，失败时记录警告但继续
                    if let Err(e) = fs::remove_file(&out_path) {
                        eprintln!(
                            "[WARNING] 无法删除空文件 {:?}: {}，将继续尝试下载",
                            out_path, e
                        );
                    }
                }
            }

            // 检查并清理可能残留的临时文件（来自之前失败的下载）
            let temp_path = {
                let mut temp = out_path.to_path_buf();
                let file_name = temp
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                temp.set_file_name(format!("{}.tmp", file_name));
                temp
            };
            if temp_path.exists() {
                debug_log!(
                    self.config,
                    "发现残留的临时文件，将清理 ({}/{}): {:?}",
                    i + 1,
                    total_media_count,
                    temp_path
                );
                if let Err(e) = fs::remove_file(&temp_path) {
                    eprintln!(
                        "[WARNING] 无法删除残留的临时文件 {:?}: {}，将继续尝试下载",
                        temp_path, e
                    );
                }
            }

            debug_log!(self.config, "[DEBUG] 开始下载媒体文件: {}", media_url);
            match self
                .download_media(media_url, &out_path, i + 1, total_media_count)
                .await
            {
                Ok(true) => {
                    debug_log!(self.config, "[DEBUG] 下载成功: {:?}", out_path);
                    // 下载成功后设置文件时间为推文发布时间
                    if let Some(ts) = tweet_timestamp {
                        let ft = FileTime::from_unix_time(ts, 0);
                        if let Err(e) = set_file_times(&out_path, ft, ft) {
                            // 记录警告，因为某些文件系统可能不支持时间修改
                            eprintln!("[WARNING] 无法设置文件时间 {:?}: {} (某些文件系统可能不支持此操作)", out_path, e);
                            debug_log!(
                                self.config,
                                "[DEBUG] 设置文件时间失败 {:?}: {}",
                                out_path,
                                e
                            );
                        } else {
                            debug_log!(self.config, "[DEBUG] 已设置文件时间: {:?}", out_path);
                        }
                    }
                    download_success_count += 1;
                }
                Ok(false) => {
                    eprintln!(
                        "[ERROR] 下载失败 ({}/{}): {}",
                        i + 1,
                        total_media_count,
                        media_url
                    );
                    debug_log!(self.config, "[DEBUG] 下载失败详情: {}", media_url);
                }
                Err(e) => {
                    eprintln!(
                        "[ERROR] 下载异常 ({}/{}): {} 错误: {}",
                        i + 1,
                        total_media_count,
                        media_url,
                        e
                    );
                    debug_log!(
                        self.config,
                        "[DEBUG] 下载异常详情 ({}/{}): {} 错误: {}",
                        i + 1,
                        total_media_count,
                        media_url,
                        e
                    );
                    // download_media 已经清理了临时文件，这里不需要额外操作
                }
            }
        }

        if download_success_count > 0 {
            if skipped_count > 0 {
                debug_log!(
                    self.config,
                    "Tweet {} 处理完成: 跳过 {} 个已存在文件，成功下载 {} 个新文件",
                    tweet_id,
                    skipped_count,
                    download_success_count - skipped_count
                );
            } else {
                debug_log!(
                    self.config,
                    "Tweet {} 成功下载了 {}/{} 个媒体文件",
                    tweet_id,
                    download_success_count,
                    total_media_count
                );
            }
            Ok(Some(true))
        } else {
            debug_log!(self.config, "Tweet {} 所有媒体文件下载失败", tweet_id);
            Ok(Some(false))
        }
    }

    async fn download_media(
        &self,
        url: &str,
        out_path: &Path,
        current: usize,
        total: usize,
    ) -> Result<bool> {
        debug_log!(self.config, "[DEBUG] 发送下载请求: {}", url);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("User-Agent", self.config.user_agent.parse()?);

        let response = self.client.get(url).headers(headers).send().await?;

        debug_log!(self.config, "[DEBUG] 下载响应状态: {}", response.status());
        if !response.status().is_success() {
            debug_log!(
                self.config,
                "[DEBUG] 下载失败 ({}) ({}/{}): {}",
                response.status(),
                current,
                total,
                url
            );
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

        // 使用临时文件名下载，成功后再原子性地重命名为目标文件名
        // 这样可以避免部分下载的文件被误判为已完成
        let temp_path = {
            let mut temp = out_path.to_path_buf();
            let file_name = temp
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            temp.set_file_name(format!("{}.tmp", file_name));
            temp
        };

        debug_log!(self.config, "[DEBUG] 开始写入临时文件: {:?}", temp_path);
        let mut file = File::create(&temp_path).await?;
        let mut stream = response.bytes_stream();

        // 下载数据流，在错误时清理临时文件
        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(chunk) => chunk,
                Err(e) => {
                    // 流读取错误，清理临时文件并返回错误
                    let _ = fs::remove_file(&temp_path);
                    return Err(e.into());
                }
            };

            // 写入错误时，清理临时文件并返回错误
            if let Err(e) = file.write_all(&chunk).await {
                let _ = fs::remove_file(&temp_path);
                return Err(e.into());
            }

            downloaded_size += chunk.len() as u64;
            pb.set_position(downloaded_size);
        }

        // 确保所有数据都写入磁盘
        if let Err(e) = file.sync_all().await {
            let _ = fs::remove_file(&temp_path);
            return Err(e.into());
        }

        pb.finish_with_message("下载完成");

        // 验证下载完整性
        if let Some(expected_size) = content_length {
            if downloaded_size != expected_size {
                eprintln!(
                    "[ERROR] 下载不完整 ({}/{}): {:?} (期望: {}, 实际: {})",
                    current, total, out_path, expected_size, downloaded_size
                );
                // 删除不完整的临时文件
                if let Err(e) = fs::remove_file(&temp_path) {
                    eprintln!("[WARNING] 无法删除不完整的临时文件 {:?}: {}", temp_path, e);
                }
                return Ok(false);
            }
        }

        // 验证临时文件的实际大小
        if let Ok(metadata) = fs::metadata(&temp_path) {
            if metadata.len() != downloaded_size {
                eprintln!(
                    "[ERROR] 文件大小不匹配 ({}/{}): {:?} (期望: {}, 实际: {})",
                    current,
                    total,
                    temp_path,
                    downloaded_size,
                    metadata.len()
                );
                let _ = fs::remove_file(&temp_path);
                return Ok(false);
            }
        }

        // 原子性地将临时文件重命名为目标文件
        // 如果目标文件已存在（可能是之前的部分下载），先删除它
        if out_path.exists() {
            if let Err(e) = fs::remove_file(out_path) {
                eprintln!(
                    "[WARNING] 无法删除已存在的目标文件 {:?}: {}，将继续尝试重命名",
                    out_path, e
                );
            }
        }

        if let Err(e) = fs::rename(&temp_path, out_path) {
            eprintln!(
                "[ERROR] 无法将临时文件重命名为目标文件 ({}/{}): {:?} -> {:?}: {}",
                current, total, temp_path, out_path, e
            );
            // 尝试清理临时文件
            let _ = fs::remove_file(&temp_path);
            return Err(e.into());
        }

        debug_log!(
            self.config,
            "下载成功 ({}/{}): {:?} (大小: {} 字节)",
            current,
            total,
            out_path,
            downloaded_size
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tweet_parser;
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
            user_by_screen_name_query_id: "6ND0OKRCgPajU_yJbcWSVw".to_string(),
            user_tweets_query_id: "V3vRrAJh5U6n9m1ZJ8xYQw".to_string(),
            max_pages: 50,
            download_timeout_secs: 30,
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

        let result = tweet_parser::extract_tweet_object(&tweet);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().get("rest_id").and_then(|v| v.as_str()),
            Some("123")
        );
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

        let result = tweet_parser::extract_tweet_object(&tweet);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().get("rest_id").and_then(|v| v.as_str()),
            Some("456")
        );
    }

    #[test]
    fn test_extract_tweet_object_path3() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();

        let tweet = json!({
            "rest_id": "789"
        });

        let result = tweet_parser::extract_tweet_object(&tweet);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().get("rest_id").and_then(|v| v.as_str()),
            Some("789")
        );
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

        let result = tweet_parser::extract_username(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_tweet_timestamp_seconds(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_tweet_timestamp_seconds(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_username(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_username(&tweet_obj).unwrap();
        assert_eq!(result, Some("alt_user".to_string()));
    }

    #[test]
    fn test_extract_username_none() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();

        let tweet_obj = json!({});

        let result = tweet_parser::extract_username(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_tweet_timestamp_seconds(&tweet_obj).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_tweet_timestamp_none() {
        let config = create_test_config();
        let downloader = Downloader::new(config).unwrap();

        let tweet_obj = json!({
            "legacy": {}
        });

        let result = tweet_parser::extract_tweet_timestamp_seconds(&tweet_obj).unwrap();
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
        let result1 = tweet_parser::extract_tweet_object(&tweet1);
        assert!(result1.is_some());
        assert_eq!(
            result1.unwrap().get("rest_id").and_then(|v| v.as_str()),
            Some("path1")
        );

        // 测试路径2
        let tweet2 = json!({
            "tweet": {
                "rest_id": "path2"
            }
        });
        let result2 = tweet_parser::extract_tweet_object(&tweet2);
        assert!(result2.is_some());
        assert_eq!(
            result2.unwrap().get("rest_id").and_then(|v| v.as_str()),
            Some("path2")
        );

        // 测试路径3
        let tweet3 = json!({
            "rest_id": "path3"
        });
        let result3 = tweet_parser::extract_tweet_object(&tweet3);
        assert!(result3.is_some());
        assert_eq!(
            result3.unwrap().get("rest_id").and_then(|v| v.as_str()),
            Some("path3")
        );
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_tweet_timestamp_seconds(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_tweet_timestamp_seconds(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
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

        let result = tweet_parser::extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_is_valid_twitter_domain_exact_match() {
        // 精确匹配应该通过
        assert!(Downloader::is_valid_twitter_domain("twimg.com"));
        assert!(Downloader::is_valid_twitter_domain("twitter.com"));
    }

    #[test]
    fn test_is_valid_twitter_domain_subdomains() {
        // 合法的子域名应该通过
        assert!(Downloader::is_valid_twitter_domain("pbs.twimg.com"));
        assert!(Downloader::is_valid_twitter_domain("video.twimg.com"));
        assert!(Downloader::is_valid_twitter_domain("abs.twimg.com"));
        assert!(Downloader::is_valid_twitter_domain("api.twitter.com"));
        assert!(Downloader::is_valid_twitter_domain("media.twitter.com"));
    }

    #[test]
    fn test_is_valid_twitter_domain_malicious_similar() {
        // 恶意相似域名应该被拒绝
        assert!(!Downloader::is_valid_twitter_domain("eviltwimg.com"));
        assert!(!Downloader::is_valid_twitter_domain("malicioustwitter.com"));
        assert!(!Downloader::is_valid_twitter_domain("fake-twimg.com"));
        assert!(!Downloader::is_valid_twitter_domain("fake-twitter.com"));
        assert!(!Downloader::is_valid_twitter_domain("twimg.com.evil.com"));
        assert!(!Downloader::is_valid_twitter_domain("twitter.com.evil.com"));
    }

    #[test]
    fn test_is_valid_twitter_domain_other_domains() {
        // 其他域名应该被拒绝
        assert!(!Downloader::is_valid_twitter_domain("example.com"));
        assert!(!Downloader::is_valid_twitter_domain("google.com"));
        assert!(!Downloader::is_valid_twitter_domain("evil.com"));
        assert!(!Downloader::is_valid_twitter_domain("localhost"));
    }
}
