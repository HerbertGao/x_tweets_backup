use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// 从推文数据中提取推文对象
/// 支持多种 JSON 结构路径
pub fn extract_tweet_object(tweet: &Value) -> Option<&Value> {
    // 路径1: content.itemContent.tweet_results.result.tweet
    if let Some(tweet_obj) = tweet
        .get("content")
        .and_then(|c| c.get("itemContent"))
        .and_then(|ic| ic.get("tweet_results"))
        .and_then(|tr| tr.get("result"))
        .and_then(|r| r.get("tweet"))
    {
        return Some(tweet_obj);
    }

    // 路径2: 直接结构
    if let Some(tweet_obj) = tweet.get("tweet") {
        return Some(tweet_obj);
    }

    // 路径3: 使用原始tweet
    Some(tweet)
}

/// 从推文对象中提取媒体 URL 列表
pub fn extract_media_urls(tweet_obj: &Value) -> Result<Vec<String>> {
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

/// 从推文对象中提取时间戳（Unix 时间戳，秒）
pub fn extract_tweet_timestamp_seconds(tweet_obj: &Value) -> Result<Option<i64>> {
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
        if let Ok(dt) = chrono::DateTime::parse_from_str(created_at_str, "%a %b %d %H:%M:%S %z %Y") {
            return Ok(Some(dt.timestamp()));
        }
    }

    Ok(None)
}

/// 从推文对象中提取时间戳（DateTime<Utc>）
pub fn extract_tweet_timestamp_datetime(tweet_obj: &Value) -> Result<Option<DateTime<Utc>>> {
    let legacy = tweet_obj.get("legacy").unwrap_or(&Value::Null);

    // 尝试 created_at_ms （毫秒时间戳）
    if let Some(ts_ms_str) = legacy.get("created_at_ms").and_then(|v| v.as_str()) {
        if let Ok(ts_ms) = ts_ms_str.parse::<i64>() {
            return Ok(DateTime::from_timestamp(ts_ms / 1000, 0));
        }
    }
    if let Some(ts_ms) = legacy.get("created_at_ms").and_then(|v| v.as_i64()) {
        return Ok(DateTime::from_timestamp(ts_ms / 1000, 0));
    }

    // 尝试 created_at 字符串，如 "Thu Apr 06 15:24:15 +0000 2017"
    if let Some(created_at_str) = legacy.get("created_at").and_then(|v| v.as_str()) {
        if let Ok(dt) = chrono::DateTime::parse_from_str(created_at_str, "%a %b %d %H:%M:%S %z %Y") {
            return Ok(Some(dt.with_timezone(&Utc)));
        }
    }

    Ok(None)
}

/// 从推文对象中提取用户信息（用户名和显示名）
pub fn extract_user_info(tweet_obj: &Value) -> Result<(Option<String>, Option<String>)> {
    // 尝试从core.user_results.result.legacy路径提取（主要路径）
    let mut username = tweet_obj
        .get("core")
        .and_then(|c| c.get("user_results"))
        .and_then(|ur| ur.get("result"))
        .and_then(|r| r.get("legacy"))
        .and_then(|l| l.get("screen_name"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let mut display_name = tweet_obj
        .get("core")
        .and_then(|c| c.get("user_results"))
        .and_then(|ur| ur.get("result"))
        .and_then(|r| r.get("legacy"))
        .and_then(|l| l.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string());

    // 如果旧路径没有找到，尝试直接从core路径提取
    if username.is_none() {
        username = tweet_obj
            .get("core")
            .and_then(|c| c.get("screen_name"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
    }

    if display_name.is_none() {
        display_name = tweet_obj
            .get("core")
            .and_then(|c| c.get("name"))
            .and_then(|n| n.as_str())
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

    if display_name.is_none() {
        display_name = tweet_obj
            .get("core")
            .and_then(|c| c.get("user_results"))
            .and_then(|ur| ur.get("result"))
            .and_then(|r| r.get("core"))
            .and_then(|c| c.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string());
    }

    Ok((username, display_name))
}

/// 从推文对象中提取用户名（仅用户名）
pub fn extract_username(tweet_obj: &Value) -> Result<Option<String>> {
    let (username, _) = extract_user_info(tweet_obj)?;
    Ok(username)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_tweet_object_path1() {
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
        
        let result = extract_tweet_object(&tweet);
        assert!(result.is_some());
        assert_eq!(result.unwrap().get("rest_id").and_then(|v| v.as_str()), Some("123"));
    }

    #[test]
    fn test_extract_tweet_object_path2() {
        let tweet = json!({
            "tweet": {
                "rest_id": "456"
            }
        });
        
        let result = extract_tweet_object(&tweet);
        assert!(result.is_some());
        assert_eq!(result.unwrap().get("rest_id").and_then(|v| v.as_str()), Some("456"));
    }

    #[test]
    fn test_extract_tweet_object_path3() {
        let tweet = json!({
            "rest_id": "789"
        });
        
        let result = extract_tweet_object(&tweet);
        assert!(result.is_some());
        assert_eq!(result.unwrap().get("rest_id").and_then(|v| v.as_str()), Some("789"));
    }

    #[test]
    fn test_extract_media_urls_photo() {
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
        
        let result = extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/image.jpg");
    }

    #[test]
    fn test_extract_media_urls_video() {
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
                                        "url": "https://example.com/video_low.mp4"
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
        
        let result = extract_media_urls(&tweet_obj).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "https://example.com/video_hd.mp4"); // 应该选择最高bitrate
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
        
        let (username, display_name) = extract_user_info(&tweet_obj).unwrap();
        assert_eq!(username, Some("test_user".to_string()));
        assert_eq!(display_name, Some("Test User".to_string()));
    }

    #[test]
    fn test_extract_tweet_timestamp_seconds() {
        let tweet_obj = json!({
            "legacy": {
                "created_at_ms": "1609459200000"
            }
        });
        
        let result = extract_tweet_timestamp_seconds(&tweet_obj).unwrap();
        assert_eq!(result, Some(1609459200));
    }

    #[test]
    fn test_extract_tweet_timestamp_datetime() {
        let tweet_obj = json!({
            "legacy": {
                "created_at_ms": "1609459200000"
            }
        });
        
        let result = extract_tweet_timestamp_datetime(&tweet_obj).unwrap();
        assert!(result.is_some());
    }
}

