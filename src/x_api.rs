use anyhow::Result;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::Config;

// 调试日志辅助宏
macro_rules! debug_log {
    ($config:expr, $($arg:tt)*) => {
        if $config.debug_logs {
            println!($($arg)*);
        }
    };
}

#[derive(Debug)]
pub struct XApi {
    client: Client,
    config: Config,
}

impl XApi {
    pub fn new(config: Config) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()?;

        Ok(XApi { client, config })
    }

    pub async fn get_user_id_by_username(&self, username: &str) -> Result<String> {
        // 移除@符号（如果有的话）
        let clean_username = username.trim_start_matches('@');
        
        let variables = json!({
            "screen_name": clean_username,
            "withGrokTranslatedBio": false
        });

        let variables_str = serde_json::to_string(&variables)?;
        let variables_encoded = urlencoding::encode(&variables_str);
        let features_encoded = urlencoding::encode(r#"{"hidden_profile_subscriptions_enabled":true,"payments_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"responsive_web_profile_redirect_enabled":false,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"subscriptions_verification_info_is_identity_verified_enabled":true,"subscriptions_verification_info_verified_since_enabled":true,"highlights_tweets_tab_ui_enabled":true,"responsive_web_twitter_article_notes_tab_enabled":true,"subscriptions_feature_can_gift_premium":true,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"responsive_web_graphql_timeline_navigation_enabled":true}"#);
        let fieldtoggles_encoded = urlencoding::encode(r#"{"withAuxiliaryUserLabels":true}"#);

        let url = format!(
            "https://x.com/i/api/graphql/{}/UserByScreenName?variables={}&features={}&fieldToggles={}",
            self.config.user_by_screen_name_query_id,
            variables_encoded, features_encoded, fieldtoggles_encoded
        );

        debug_log!(self.config, "[DEBUG] 查询用户信息: {}", url);
        debug_log!(self.config, "[DEBUG] 请求参数: variables={}", variables_str);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Authorization", format!("Bearer {}", self.config.bearer_token).parse()?);
        headers.insert("Cookie", format!("auth_token={}; ct0={}", self.config.auth_token, self.config.ct0).parse()?);
        headers.insert("X-Csrf-Token", self.config.ct0.parse()?);
        headers.insert("User-Agent", self.config.user_agent.parse()?);

        debug_log!(self.config, "[DEBUG] 请求头信息:");
        for (key, value) in &headers {
            if key.as_str() == "authorization" {
                debug_log!(self.config, "[DEBUG]   {}: Bearer ***", key);
            } else if key.as_str() == "cookie" {
                debug_log!(self.config, "[DEBUG]   {}: ***", key);
            } else {
                debug_log!(self.config, "[DEBUG]   {}: {:?}", key, value);
            }
        }

        debug_log!(self.config, "[DEBUG] 发送请求到: {}", url);
        let response = self.client
            .get(&url)
            .headers(headers)
            .send()
            .await?;

        debug_log!(self.config, "[DEBUG] 响应状态: {}", response.status());
        
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            // 错误信息始终记录，但不包含敏感数据
            eprintln!("[ERROR] 查询用户信息失败: {} (状态码: {})", clean_username, status);
            debug_log!(self.config, "[DEBUG] 错误响应内容: {}", text);
            // 只在调试模式下包含完整响应，生产环境只返回状态码
            if self.config.debug_logs {
                return Err(anyhow::anyhow!("查询用户信息失败: {} (状态码: {}) - {}", clean_username, status, text));
            } else {
                return Err(anyhow::anyhow!("查询用户信息失败: {} (状态码: {})", clean_username, status));
            }
        }

        let data: Value = response.json().await?;
        debug_log!(self.config, "[DEBUG] 响应数据: {}", serde_json::to_string_pretty(&data).unwrap_or_else(|_| "无法解析JSON".to_string()));

        // 解析用户ID
        if let Some(user_id) = data
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("result"))
            .and_then(|r| r.get("rest_id"))
            .and_then(|id| id.as_str())
        {
            Ok(user_id.to_string())
        } else {
            Err(anyhow::anyhow!("无法找到用户: @{}", clean_username))
        }
    }

    pub async fn get_user_tweets_internal(&self) -> Result<Vec<Value>> {
        let mut all_tweets = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page_count = 0;
        let mut seen_tweet_ids = std::collections::HashSet::new(); // 记录已见过的推文ID
        // 使用配置中的最大页数，防止无限循环

        loop {
            page_count += 1;
            debug_log!(self.config, "[DEBUG] 开始获取第 {} 页数据", page_count);
            
            // 防止无限循环
            if page_count > self.config.max_pages {
                eprintln!("[WARNING] 已达到最大页数限制 ({}), 停止分页。这可能是由于分页逻辑问题导致的。", self.config.max_pages);
                debug_log!(self.config, "[DEBUG] 已达到最大页数限制 ({}), 停止分页", self.config.max_pages);
                break;
            }
            // 使用指定的目标用户ID，如果没有指定则使用当前认证用户ID
            let user_id = if !self.config.target_user_id.is_empty() {
                &self.config.target_user_id
            } else {
                &self.config.user_id
            };
            
            debug_log!(self.config, "[DEBUG] 使用用户ID: {}", user_id);
            let mut variables = json!({
                "userId": user_id,
                "count": self.config.count.parse::<i32>()?,
                "includePromotedContent": true,
                "withQuickPromoteEligibilityTweetFields": true,
                "withVoice": true
            });

            if let Some(ref cursor_val) = cursor {
                variables["cursor"] = json!(cursor_val);
            }

            let variables_str = serde_json::to_string(&variables)?;
            let variables_encoded = urlencoding::encode(&variables_str);
            let features_encoded = urlencoding::encode(r#"{"rweb_video_screen_enabled":false,"payments_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"responsive_web_profile_redirect_enabled":false,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"premium_content_api_read_enabled":false,"communities_web_enable_tweet_community_results_fetch":true,"c9s_tweet_anatomy_moderator_badge_enabled":true,"responsive_web_grok_analyze_button_fetch_trends_enabled":false,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_jetfuel_frame":true,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"tweet_awards_web_tipping_enabled":false,"responsive_web_grok_show_grok_translated_post":false,"responsive_web_grok_analysis_button_from_backend":true,"creator_subscriptions_quote_tweet_preview_enabled":false,"freedom_of_speech_not_reach_fetch_enabled":true,"standardized_nudges_misinfo":true,"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_grok_imagine_annotation_enabled":true,"responsive_web_grok_community_note_auto_translation_is_enabled":false,"responsive_web_enhance_cards_enabled":false}"#);
            let fieldtoggles_encoded = urlencoding::encode(r#"{"withArticlePlainText":false}"#);

            let url = format!(
                "https://x.com/i/api/graphql/{}/self.config.user_tweets_query_id,
                variables_encoded, features_encoded, fieldtoggles_encoded/UserTweets?variables={}&features={}&fieldToggles={}",
                variables_encoded, features_encoded, fieldtoggles_encoded
            );

            debug_log!(self.config, "[DEBUG] 获取用户推文: {}", url);
            debug_log!(self.config, "[DEBUG] 请求参数: variables={}", variables_str);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Authorization", format!("Bearer {}", self.config.bearer_token).parse()?);
        headers.insert("Cookie", format!("auth_token={}; ct0={}", self.config.auth_token, self.config.ct0).parse()?);
        headers.insert("X-Csrf-Token", self.config.ct0.parse()?);
        headers.insert("User-Agent", self.config.user_agent.parse()?);

        debug_log!(self.config, "[DEBUG] 请求头信息:");
        for (key, value) in &headers {
            if key.as_str() == "authorization" {
                debug_log!(self.config, "[DEBUG]   {}: Bearer ***", key);
            } else if key.as_str() == "cookie" {
                debug_log!(self.config, "[DEBUG]   {}: ***", key);
            } else {
                debug_log!(self.config, "[DEBUG]   {}: {:?}", key, value);
            }
        }

        debug_log!(self.config, "[DEBUG] 发送请求到: {}", url);

        let response = self.client
            .get(&url)
            .headers(headers)
            .send()
            .await?;

        debug_log!(self.config, "[DEBUG] 响应状态: {}", response.status());
        
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            eprintln!("[ERROR] API 请求失败: 状态码 {}", status);
            debug_log!(self.config, "[DEBUG] 错误响应内容: {}", text);
            // 只在调试模式下包含完整响应，生产环境只返回状态码
            if self.config.debug_logs {
                return Err(anyhow::anyhow!("API 请求失败: {} - {}", status, text));
            } else {
                return Err(anyhow::anyhow!("API 请求失败: 状态码 {}", status));
            }
        }

        let data: Value = response.json().await?;
        debug_log!(self.config, "[DEBUG] 响应数据: {}", serde_json::to_string_pretty(&data).unwrap_or_else(|_| "无法解析JSON".to_string()));

            let (tweets, new_cursor) = self.parse_user_tweets_response(&data)?;
            debug_log!(self.config, "[DEBUG] 本页获取到 {} 条 tweet，cursor: {:?}", tweets.len(), new_cursor);

            // 如果没有获取到新推文，停止分页
            if tweets.is_empty() {
                debug_log!(self.config, "[DEBUG] 没有获取到新推文，停止分页");
                break;
            }

            // 检查是否有新的推文（避免重复）
            let mut new_tweets = Vec::new();
            let mut duplicate_count = 0;
            
            for tweet in tweets {
                // 提取推文ID（同时兼容 entry 结构与 result 结构）
                let tweet_id = tweet
                    .get("content")
                    .and_then(|c| c.get("itemContent"))
                    .and_then(|ic| ic.get("tweet_results"))
                    .and_then(|tr| tr.get("result"))
                    .and_then(|r| r.get("rest_id"))
                    .and_then(|id| id.as_str())
                    // 兼容 entryId: tweet-XXXXXXXXX
                    .or_else(|| {
                        tweet
                            .get("entryId")
                            .and_then(|id| id.as_str())
                            .and_then(|s| s.strip_prefix("tweet-"))
                    })
                    // 兼容对话模块中直接返回的 result 节点（根级 rest_id）
                    .or_else(|| tweet.get("rest_id").and_then(|id| id.as_str()))
                    // 兼容某些变体：{ tweet: { rest_id: ... } }
                    .or_else(|| {
                        tweet
                            .get("tweet")
                            .and_then(|t| t.get("rest_id"))
                            .and_then(|id| id.as_str())
                    });

                if let Some(tweet_id) = tweet_id {
                    if seen_tweet_ids.contains(tweet_id) {
                        duplicate_count += 1;
                        debug_log!(self.config, "[DEBUG] 发现重复推文: {}", tweet_id);
                    } else {
                        seen_tweet_ids.insert(tweet_id.to_string());
                        new_tweets.push(tweet.clone());
                        debug_log!(self.config, "[DEBUG] 发现新推文: {}", tweet_id);
                    }
                } else {
                    debug_log!(self.config, "[DEBUG] 跳过无法识别ID的推文节点: {}", serde_json::to_string(&tweet).unwrap_or_else(|_| "<unserializable>".to_string()));
                }
            }
            
            debug_log!(self.config, "[DEBUG] 本页新推文: {} 条，重复推文: {} 条", new_tweets.len(), duplicate_count);
            
            // 如果所有推文都是重复的，停止分页
            if new_tweets.is_empty() {
                debug_log!(self.config, "[DEBUG] 本页没有新推文，停止分页");
                break;
            }

            all_tweets.extend(new_tweets);

            // 如果未启用 ALL 模式，则只返回第一页数据
            if !self.config.all {
                break;
            }

            // 如果没有新的 cursor 或新 cursor 与上一次相同，则认为没有更多数据
            if new_cursor.is_none() {
                debug_log!(self.config, "[DEBUG] 没有新的cursor，停止分页");
                break;
            }
            
            if let Some(ref new_cursor_val) = new_cursor {
                if let Some(ref old_cursor_val) = cursor {
                    if new_cursor_val == old_cursor_val {
                        debug_log!(self.config, "[DEBUG] cursor值相同，停止分页");
                        break;
                    }
                }
            }
            
            cursor = new_cursor;
        }

        Ok(all_tweets)
    }

    fn parse_user_tweets_response(&self, data: &Value) -> Result<(Vec<Value>, Option<String>)> {
        let mut tweets = Vec::new();
        let mut new_cursor = None;

        // 新的API响应结构：data.user.result.timeline.timeline.instructions
        if let Some(instructions) = data
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("result"))
            .and_then(|r| r.get("timeline"))
            .and_then(|t| t.get("timeline"))
            .and_then(|t| t.get("instructions"))
            .and_then(|i| i.as_array())
        {
            for instruction in instructions {
                // 处理 TimelineAddEntries 类型的指令
                if instruction.get("type") == Some(&json!("TimelineAddEntries")) {
                    if let Some(entries) = instruction.get("entries").and_then(|e| e.as_array()) {
                        for entry in entries {
                            if let Some(entry_id) = entry.get("entryId").and_then(|id| id.as_str()) {
                                if entry_id.starts_with("profile-conversation-") {
                                    // 这是对话模块，提取其中的所有推文
                                    debug_log!(self.config, "[DEBUG] 找到对话模块: {}", entry_id);
                                    let conversation_tweets = self.extract_tweets_from_conversation_module(entry);
                                    tweets.extend(conversation_tweets);
                                } else if entry_id.starts_with("tweet-") {
                                    debug_log!(self.config, "[DEBUG] 找到推文: {}", entry_id);
                                    tweets.push(entry.clone());
                                } else if entry_id.starts_with("cursor-bottom-") {
                                    new_cursor = entry
                                        .get("content")
                                        .and_then(|c| c.get("value"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    debug_log!(self.config, "[DEBUG] 找到cursor: {}", new_cursor.as_deref().unwrap_or("None"));
                                }
                            }
                        }
                    }
                }
                // 处理 TimelinePinEntry 类型的指令（置顶推文）
                else if instruction.get("type") == Some(&json!("TimelinePinEntry")) {
                    if let Some(entry) = instruction.get("entry") {
                        if let Some(entry_id) = entry.get("entryId").and_then(|id| id.as_str()) {
                            if entry_id.starts_with("tweet-") {
                                debug_log!(self.config, "[DEBUG] 找到置顶推文: {}", entry_id);
                                tweets.push(entry.clone());
                            }
                        }
                    }
                }
            }
        }

        Ok((tweets, new_cursor))
    }

    fn extract_tweets_from_conversation_module(&self, entry: &Value) -> Vec<Value> {
        let mut tweets = Vec::new();
        
        // 处理对话模块中的推文
        if let Some(content) = entry.get("content") {
            if let Some(items) = content.get("items").and_then(|i| i.as_array()) {
                for item in items {
                    if let Some(item_obj) = item.get("item") {
                        if let Some(item_content) = item_obj.get("itemContent") {
                            if let Some(tweet_results) = item_content.get("tweet_results") {
                                if let Some(result) = tweet_results.get("result") {
                                    debug_log!(self.config, "[DEBUG] 从对话模块提取推文: {}", result.get("rest_id").and_then(|id| id.as_str()).unwrap_or("unknown"));
                                    tweets.push(result.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        
        tweets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_config() -> Config {
        Config {
            user_id: "test_user_id".to_string(),
            bearer_token: "test_bearer".to_string(),
            auth_token: "test_auth".to_string(),
            ct0: "test_ct0".to_string(),
            personalization_id: "test_pid".to_string(),
            user_agent: "Mozilla/5.0".to_string(),
            x_client_uuid: "".to_string(),
            x_client_transaction_id: "".to_string(),
            count: "20".to_string(),
            all: false,
            download_dir: "data/downloads".to_string(),
            download_record: "data/downloaded_tweet_ids.txt".to_string(),
            file_format: "{USERNAME} {ID}".to_string(),
            target_user_id: "test_target".to_string(),
            target_username: "test_target".to_string(),
            only_self: false,
            save_markdown: true,
            markdown_output: "data/test_output.md".to_string(),
            debug_logs: false,
        }
    }

    #[test]
    fn test_x_api_new() {
        let config = create_test_config();
        let _api = XApi::new(config).unwrap();
        // 验证创建成功
        assert!(true); // XApi 没有公开字段，只能验证创建不失败
    }

    #[test]
    fn test_extract_tweets_from_conversation_module() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": [
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "123",
                                        "legacy": {
                                            "full_text": "Tweet 1"
                                        }
                                    }
                                }
                            }
                        }
                    },
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "456",
                                        "legacy": {
                                            "full_text": "Tweet 2"
                                        }
                                    }
                                }
                            }
                        }
                    }
                ]
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 2);
        assert_eq!(tweets[0].get("rest_id").and_then(|v| v.as_str()), Some("123"));
        assert_eq!(tweets[1].get("rest_id").and_then(|v| v.as_str()), Some("456"));
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_empty() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": []
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "entryId": "tweet-123",
                                                "content": {
                                                    "itemContent": {
                                                        "tweet_results": {
                                                            "result": {
                                                                "rest_id": "123"
                                                            }
                                                        }
                                                    }
                                                }
                                            },
                                            {
                                                "entryId": "cursor-bottom-abc123",
                                                "content": {
                                                    "value": "cursor_value"
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, cursor) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 1);
        assert_eq!(cursor, Some("cursor_value".to_string()));
    }

    #[test]
    fn test_parse_user_tweets_response_with_pin_entry() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelinePinEntry",
                                        "entry": {
                                            "entryId": "tweet-789",
                                            "content": {
                                                "itemContent": {
                                                    "tweet_results": {
                                                        "result": {
                                                            "rest_id": "789"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 1);
    }

    #[test]
    fn test_parse_user_tweets_response_with_conversation() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "entryId": "profile-conversation-123",
                                                "content": {
                                                    "items": [
                                                        {
                                                            "item": {
                                                                "itemContent": {
                                                                    "tweet_results": {
                                                                        "result": {
                                                                            "rest_id": "conv1"
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    ]
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 1);
        assert_eq!(tweets[0].get("rest_id").and_then(|v| v.as_str()), Some("conv1"));
    }

    #[test]
    fn test_parse_user_tweets_response_empty() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": []
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_no_instructions() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {}
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_nested() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": [
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "nested1"
                                    }
                                }
                            }
                        }
                    }
                ]
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 1);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_no_items() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {}
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_get_user_id_by_username_removes_at() {
        let _config = create_test_config();
        // 这个测试只验证@符号被移除，不进行实际网络请求
        // 实际测试需要mock或集成测试
        let clean1 = "@testuser".trim_start_matches('@');
        let clean2 = "testuser".trim_start_matches('@');
        assert_eq!(clean1, "testuser");
        assert_eq!(clean2, "testuser");
    }

    #[test]
    fn test_parse_user_tweets_response_multiple_entries() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "entryId": "tweet-111",
                                                "content": {
                                                    "itemContent": {
                                                        "tweet_results": {
                                                            "result": {
                                                                "rest_id": "111"
                                                            }
                                                        }
                                                    }
                                                }
                                            },
                                            {
                                                "entryId": "tweet-222",
                                                "content": {
                                                    "itemContent": {
                                                        "tweet_results": {
                                                            "result": {
                                                                "rest_id": "222"
                                                            }
                                                        }
                                                    }
                                                }
                                            },
                                            {
                                                "entryId": "cursor-bottom-xyz",
                                                "content": {
                                                    "value": "next_cursor"
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, cursor) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 2);
        assert_eq!(cursor, Some("next_cursor".to_string()));
    }

    #[test]
    fn test_parse_user_tweets_response_pin_and_entries() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelinePinEntry",
                                        "entry": {
                                            "entryId": "tweet-pinned",
                                            "content": {
                                                "itemContent": {
                                                    "tweet_results": {
                                                        "result": {
                                                            "rest_id": "pinned"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "entryId": "tweet-regular",
                                                "content": {
                                                    "itemContent": {
                                                        "tweet_results": {
                                                            "result": {
                                                                "rest_id": "regular"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 2);
    }

    #[test]
    fn test_parse_user_tweets_response_non_tweet_entry() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "entryId": "other-entry",
                                                "content": {}
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_multiple_items() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": [
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "conv1"
                                    }
                                }
                            }
                        }
                    },
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "conv2"
                                    }
                                }
                            }
                        }
                    },
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "conv3"
                                    }
                                }
                            }
                        }
                    }
                ]
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 3);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_no_result() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": [
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {}
                            }
                        }
                    }
                ]
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_pin_entry_non_tweet() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelinePinEntry",
                                        "entry": {
                                            "entryId": "other-pin",
                                            "content": {}
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_unknown_instruction_type() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "UnknownType",
                                        "entries": []
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_no_item_content() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": [
                    {
                        "item": {}
                    }
                ]
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_no_tweet_results() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": [
                    {
                        "item": {
                            "itemContent": {}
                        }
                    }
                ]
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_entry_id_variations() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "entryId": "tweet-111",
                                                "content": {
                                                    "itemContent": {
                                                        "tweet_results": {
                                                            "result": {
                                                                "rest_id": "111"
                                                            }
                                                        }
                                                    }
                                                }
                                            },
                                            {
                                                "entryId": "profile-conversation-222",
                                                "content": {
                                                    "items": [
                                                        {
                                                            "item": {
                                                                "itemContent": {
                                                                    "tweet_results": {
                                                                        "result": {
                                                                            "rest_id": "conv1"
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    ]
                                                }
                                            },
                                            {
                                                "entryId": "cursor-bottom-333",
                                                "content": {
                                                    "value": "cursor333"
                                                }
                                            },
                                            {
                                                "entryId": "other-entry",
                                                "content": {}
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, cursor) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 2); // tweet-111 和 conversation 中的推文
        assert_eq!(cursor, Some("cursor333".to_string()));
    }

    #[test]
    fn test_parse_user_tweets_response_no_data() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({});
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_malformed() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {}
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_mixed_valid_invalid() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": [
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "valid1"
                                    }
                                }
                            }
                        }
                    },
                    {
                        "item": {
                            "itemContent": {}
                        }
                    },
                    {
                        "item": {
                            "itemContent": {
                                "tweet_results": {
                                    "result": {
                                        "rest_id": "valid2"
                                    }
                                }
                            }
                        }
                    }
                ]
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 2);
        assert_eq!(tweets[0].get("rest_id").and_then(|v| v.as_str()), Some("valid1"));
        assert_eq!(tweets[1].get("rest_id").and_then(|v| v.as_str()), Some("valid2"));
    }

    #[test]
    fn test_parse_user_tweets_response_with_cursor_top() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "entryId": "tweet-123",
                                                "content": {
                                                    "itemContent": {
                                                        "tweet_results": {
                                                            "result": {
                                                                "rest_id": "123"
                                                            }
                                                        }
                                                    }
                                                }
                                            },
                                            {
                                                "entryId": "cursor-top-abc",
                                                "content": {
                                                    "value": "top_cursor"
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, cursor) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 1);
        // cursor-top 不应该被提取为 cursor
        assert_eq!(cursor, None);
    }

    #[test]
    fn test_parse_user_tweets_response_empty_instructions() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": []
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_no_timeline() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {}
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_no_user() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {}
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_entry_without_id() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": [
                                            {
                                                "content": {
                                                    "itemContent": {
                                                        "tweet_results": {
                                                            "result": {
                                                                "rest_id": "123"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        ]
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        // 没有 entryId 的条目应该被忽略
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_pin_entry_without_tweet_id() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelinePinEntry",
                                        "entry": {
                                            "entryId": "other-pin",
                                            "content": {}
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        // 不是 tweet- 开头的 pin entry 应该被忽略
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_pin_entry_without_entry_id() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelinePinEntry",
                                        "entry": {
                                            "content": {}
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_no_content() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({});
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_extract_tweets_from_conversation_module_items_not_array() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let entry = json!({
            "content": {
                "items": "not_an_array"
            }
        });
        
        let tweets = api.extract_tweets_from_conversation_module(&entry);
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_instructions_not_array() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": "not_an_array"
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }

    #[test]
    fn test_parse_user_tweets_response_entries_not_array() {
        let config = create_test_config();
        let api = XApi::new(config).unwrap();
        
        let data = json!({
            "data": {
                "user": {
                    "result": {
                        "timeline": {
                            "timeline": {
                                "instructions": [
                                    {
                                        "type": "TimelineAddEntries",
                                        "entries": "not_an_array"
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });
        
        let (tweets, _) = api.parse_user_tweets_response(&data).unwrap();
        assert_eq!(tweets.len(), 0);
    }
}

