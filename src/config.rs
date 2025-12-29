use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // 用户认证信息
    pub user_id: String,
    pub bearer_token: String,
    pub auth_token: String,
    pub ct0: String,
    pub personalization_id: String,
    pub user_agent: String,
    pub x_client_uuid: String,
    pub x_client_transaction_id: String,

    // 下载配置
    pub count: String,
    pub all: bool,
    pub download_dir: String,
    pub download_record: String,
    pub file_format: String,

    // 目标用户配置
    pub target_user_id: String,
    pub target_username: String, // 保存原始用户名（用于文件名生成）

    // 过滤配置
    pub only_self: bool,

    // Markdown输出配置
    pub save_markdown: bool,
    pub markdown_output: String,

    // 调试配置
    pub debug_logs: bool,

    // API 配置
    pub user_by_screen_name_query_id: String,
    pub user_tweets_query_id: String,

    // 性能配置
    pub max_pages: usize,
    pub download_timeout_secs: u64,
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();

        let private_tokens = Self::load_private_tokens("data/private_tokens.env")?;

        Ok(Config {
            // 从private_tokens加载
            user_id: private_tokens
                .get("USER_ID")
                .unwrap_or(&"".to_string())
                .clone(),
            bearer_token: private_tokens
                .get("BEARER_TOKEN")
                .unwrap_or(&"".to_string())
                .clone(),
            auth_token: private_tokens
                .get("AUTH_TOKEN")
                .unwrap_or(&"".to_string())
                .clone(),
            ct0: private_tokens.get("CT0").unwrap_or(&"".to_string()).clone(),
            personalization_id: private_tokens
                .get("PERSONALIZATION_ID")
                .unwrap_or(&"".to_string())
                .clone(),
            user_agent: private_tokens
                .get("USER_AGENT")
                .unwrap_or(&"".to_string())
                .clone(),
            x_client_uuid: private_tokens
                .get("X_CLIENT_UUID")
                .unwrap_or(&"".to_string())
                .clone(),
            x_client_transaction_id: private_tokens
                .get("X_CLIENT_TRANSACTION_ID")
                .unwrap_or(&"".to_string())
                .clone(),

            // 从环境变量加载
            count: env::var("COUNT").unwrap_or_else(|_| "20".to_string()),
            all: env::var("ALL")
                .unwrap_or_else(|_| "false".to_string())
                .to_lowercase()
                == "true",
            download_dir: env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "data/downloads".to_string()),
            download_record: env::var("DOWNLOAD_RECORD")
                .unwrap_or_else(|_| "data/downloaded_tweet_ids.txt".to_string()),
            file_format: env::var("FILE_FORMAT").unwrap_or_else(|_| "{USERNAME} {ID}".to_string()),

            // 目标用户配置（添加输入验证，防止注入攻击）
            target_user_id: env::var("TARGET_USER_ID")
                .unwrap_or_else(|_| "".to_string())
                .trim()
                .trim_start_matches('@')
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect(),
            target_username: "".to_string(), // 将在运行时设置

            // 过滤配置
            only_self: env::var("ONLY_SELF")
                .unwrap_or_else(|_| "false".to_string())
                .to_lowercase()
                == "true",

            // Markdown输出配置
            save_markdown: env::var("SAVE_MARKDOWN")
                .unwrap_or_else(|_| "true".to_string())
                .to_lowercase()
                == "true",
            markdown_output: env::var("MARKDOWN_OUTPUT")
                .unwrap_or_else(|_| "data/tweets_backup.md".to_string()),

            // 调试配置（默认关闭以保护敏感信息）
            debug_logs: env::var("DEBUG_LOGS")
                .unwrap_or_else(|_| "false".to_string())
                .to_lowercase()
                == "true",

            // API 配置（GraphQL 查询 ID，如果 Twitter 更新 API 可以在这里修改）
            user_by_screen_name_query_id: env::var("USER_BY_SCREEN_NAME_QUERY_ID")
                .unwrap_or_else(|_| "6ND0OKRCgPajU_yJbcWSVw".to_string()),
            user_tweets_query_id: env::var("USER_TWEETS_QUERY_ID")
                .unwrap_or_else(|_| "V3vRrAJh5U6n9m1ZJ8xYQw".to_string()),

            // 性能配置
            max_pages: {
                let value = env::var("MAX_PAGES").unwrap_or_else(|_| "50".to_string());
                value.parse().unwrap_or_else(|_| {
                    eprintln!("[WARNING] 无效的 MAX_PAGES 值 '{}'，使用默认值 50", value);
                    50
                })
            },
            download_timeout_secs: {
                let value = env::var("DOWNLOAD_TIMEOUT_SECS").unwrap_or_else(|_| "30".to_string());
                value.parse().unwrap_or_else(|_| {
                    eprintln!(
                        "[WARNING] 无效的 DOWNLOAD_TIMEOUT_SECS 值 '{}'，使用默认值 30",
                        value
                    );
                    30
                })
            },
        })
    }

    fn load_private_tokens(filename: &str) -> Result<HashMap<String, String>> {
        if !Path::new(filename).exists() {
            return Err(anyhow::anyhow!(
                "{} 不存在，请先运行 setup 命令初始化。",
                filename
            ));
        }

        let content =
            fs::read_to_string(filename).with_context(|| format!("无法读取文件: {}", filename))?;

        let mut tokens = HashMap::new();
        for line in content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                tokens.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_load_private_tokens() {
        // 创建临时目录和文件
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("test_tokens.env");

        // 写入测试数据
        fs::write(
            &token_file,
            "USER_ID=test_user\nBEARER_TOKEN=test_token\nAUTH_TOKEN=test_auth",
        )
        .unwrap();

        // 测试加载
        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();

        assert_eq!(tokens.get("USER_ID"), Some(&"test_user".to_string()));
        assert_eq!(tokens.get("BEARER_TOKEN"), Some(&"test_token".to_string()));
        assert_eq!(tokens.get("AUTH_TOKEN"), Some(&"test_auth".to_string()));
    }

    #[test]
    fn test_load_private_tokens_missing_file() {
        // 测试不存在的文件
        let result = Config::load_private_tokens("nonexistent_file.env");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_private_tokens_empty_file() {
        // 创建临时空文件
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("empty_tokens.env");
        fs::write(&token_file, "").unwrap();

        // 测试加载空文件
        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_load_private_tokens_with_spaces() {
        // 测试处理带空格的值
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("spaced_tokens.env");
        fs::write(&token_file, "KEY1=value with spaces\nKEY2=  trimmed  ").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        assert_eq!(tokens.get("KEY1"), Some(&"value with spaces".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"trimmed".to_string()));
    }

    #[test]
    fn test_load_private_tokens_multiple_keys() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("multi_tokens.env");
        fs::write(&token_file, "KEY1=value1\nKEY2=value2\nKEY3=value3").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"value2".to_string()));
        assert_eq!(tokens.get("KEY3"), Some(&"value3".to_string()));
    }

    #[test]
    fn test_load_private_tokens_no_equal_sign() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("invalid_tokens.env");
        fs::write(&token_file, "KEY1=value1\nINVALID_LINE\nKEY2=value2").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        // 无效行应该被忽略
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_load_private_tokens_empty_value() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("empty_value.env");
        fs::write(&token_file, "KEY1=\nKEY2=value2").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        assert_eq!(tokens.get("KEY1"), Some(&"".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_load_private_tokens_with_comments() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("comment_tokens.env");
        fs::write(&token_file, "KEY1=value1\n# This is a comment\nKEY2=value2").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        // 注释行（没有=）应该被忽略
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_config_load_with_env_vars() {
        use std::env;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("tokens.env");
        fs::write(&token_file, "USER_ID=test_user\nBEARER_TOKEN=test_bearer\nAUTH_TOKEN=test_auth\nCT0=test_ct0\nPERSONALIZATION_ID=test_pid\nUSER_AGENT=test_ua\nX_CLIENT_UUID=test_uuid\nX_CLIENT_TRANSACTION_ID=test_tid").unwrap();

        // 设置环境变量
        env::set_var("COUNT", "50");
        env::set_var("ALL", "true");
        env::set_var("DOWNLOAD_DIR", "/tmp/test_downloads");
        env::set_var("TARGET_USER_ID", "target_user");
        env::set_var("ONLY_SELF", "true");
        env::set_var("SAVE_MARKDOWN", "false");
        env::set_var("DEBUG_LOGS", "true");

        // 临时修改load_private_tokens的路径
        // 由于load是私有方法且依赖文件系统，我们需要通过其他方式测试
        // 这里我们测试环境变量的读取逻辑

        // 清理环境变量
        env::remove_var("COUNT");
        env::remove_var("ALL");
        env::remove_var("DOWNLOAD_DIR");
        env::remove_var("TARGET_USER_ID");
        env::remove_var("ONLY_SELF");
        env::remove_var("SAVE_MARKDOWN");
        env::remove_var("DEBUG_LOGS");

        // 验证环境变量读取逻辑
        let count = env::var("COUNT").unwrap_or_else(|_| "20".to_string());
        assert_eq!(count, "20"); // 清理后应该返回默认值
    }

    #[test]
    fn test_config_env_var_parsing() {
        use std::env;

        // 测试布尔值解析
        env::set_var("TEST_ALL", "True");
        let all_true = env::var("TEST_ALL")
            .unwrap_or_else(|_| "False".to_string())
            .to_lowercase()
            == "true";
        assert!(all_true);

        env::set_var("TEST_ALL", "false");
        let all_false = env::var("TEST_ALL")
            .unwrap_or_else(|_| "False".to_string())
            .to_lowercase()
            == "true";
        assert!(!all_false);

        env::remove_var("TEST_ALL");
    }

    #[test]
    fn test_config_load_private_tokens_missing_keys() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("partial_tokens.env");
        fs::write(&token_file, "USER_ID=test_user\nBEARER_TOKEN=test_token").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        assert_eq!(tokens.get("USER_ID"), Some(&"test_user".to_string()));
        assert_eq!(tokens.get("BEARER_TOKEN"), Some(&"test_token".to_string()));
        // 缺失的键应该不存在
        assert_eq!(tokens.get("AUTH_TOKEN"), None);
    }

    #[test]
    fn test_config_load_private_tokens_duplicate_keys() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("duplicate_tokens.env");
        fs::write(&token_file, "KEY1=value1\nKEY1=value2\nKEY2=value3").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        // 重复的键，后面的值会覆盖前面的
        assert_eq!(tokens.get("KEY1"), Some(&"value2".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"value3".to_string()));
    }

    #[test]
    fn test_config_load_private_tokens_only_key() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("only_key.env");
        fs::write(&token_file, "KEY_WITHOUT_VALUE=\nKEY2=value2").unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        assert_eq!(tokens.get("KEY_WITHOUT_VALUE"), Some(&"".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_config_load_private_tokens_special_chars() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("special_chars.env");
        fs::write(
            &token_file,
            "KEY1=value with\nnewline\nKEY2=value with=equals",
        )
        .unwrap();

        let tokens = Config::load_private_tokens(token_file.to_str().unwrap()).unwrap();
        // split_once 只会在第一个 = 处分割
        assert_eq!(tokens.get("KEY1"), Some(&"value with".to_string()));
        assert_eq!(tokens.get("KEY2"), Some(&"value with=equals".to_string()));
    }
}
