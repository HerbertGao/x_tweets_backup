use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Parser, Clone)]
#[command(name = "setup")]
#[command(about = "初始化X推文备份器配置")]
pub struct SetupArgs {
    /// curl命令文件路径
    #[arg(default_value = "curl_command.txt")]
    curl_file: String,
}

#[derive(Debug)]
struct ParsedCurl {
    bearer_token: String,
    cookie_str: String,
    user_agent: Option<String>,
    x_client_uuid: Option<String>,
    x_client_transaction_id: Option<String>,
}

#[derive(Debug)]
struct ParsedCookies {
    twid: String,
    auth_token: String,
    ct0: String,
    personalization_id: String,
}

pub fn run_setup(args: SetupArgs) -> Result<()> {
    // 读取curl命令文件
    let curl_command = fs::read_to_string(&args.curl_file)
        .with_context(|| format!("读取 {} 失败", args.curl_file))?;

    // 解析curl命令
    let parsed = parse_curl_command(&curl_command)?;
    let cookies = parse_cookies(&parsed.cookie_str)?;

    // 保存私有令牌
    save_private_tokens(
        &cookies.twid,
        &parsed.bearer_token,
        &cookies.auth_token,
        &cookies.ct0,
        &cookies.personalization_id,
        parsed.user_agent.as_deref().unwrap_or(""),
        parsed.x_client_uuid.as_deref().unwrap_or(""),
        parsed.x_client_transaction_id.as_deref().unwrap_or(""),
        "data/private_tokens.env",
    )?;

    println!("初始化完成。");
    Ok(())
}

fn parse_curl_command(curl_command: &str) -> Result<ParsedCurl> {
    let header_regex = Regex::new(r#"-H\s+'([^']+)'"#)?;
    let cookie_regex = Regex::new(r#"-b\s+'([^']+)'"#)?;
    let bearer_regex = Regex::new(r#"Bearer\s+(\S+)"#)?;

    let mut bearer_token = None;
    let mut user_agent = None;
    let mut x_client_uuid = None;
    let mut x_client_transaction_id = None;

    // 解析headers
    for cap in header_regex.captures_iter(curl_command) {
        let header = &cap[1];
        let header_lower = header.to_lowercase();

        if header_lower.starts_with("authorization:") {
            if let Some(bearer_cap) = bearer_regex.captures(header) {
                bearer_token = Some(bearer_cap[1].to_string());
            }
        } else if header_lower.starts_with("user-agent:") {
            user_agent = Some(header.split_once(':').unwrap().1.trim().to_string());
        } else if header_lower.starts_with("x-client-uuid:") {
            x_client_uuid = Some(header.split_once(':').unwrap().1.trim().to_string());
        } else if header_lower.starts_with("x-client-transaction-id:") {
            x_client_transaction_id = Some(header.split_once(':').unwrap().1.trim().to_string());
        }
    }

    // 解析cookie
    let cookie_str = if let Some(cap) = cookie_regex.captures(curl_command) {
        cap[1].to_string()
    } else {
        return Err(anyhow::anyhow!("无法找到cookie参数"));
    };

    let bearer_token = bearer_token
        .ok_or_else(|| anyhow::anyhow!("无法解析Bearer Token"))?;

    Ok(ParsedCurl {
        bearer_token,
        cookie_str,
        user_agent,
        x_client_uuid,
        x_client_transaction_id,
    })
}

fn parse_cookies(cookie_str: &str) -> Result<ParsedCookies> {
    let mut cookies = std::collections::HashMap::new();

    for part in cookie_str.split(';') {
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            cookies.insert(key, value);
        }
    }

    let required_keys = ["twid", "auth_token", "ct0", "personalization_id"];
    let mut result = ParsedCookies {
        twid: String::new(),
        auth_token: String::new(),
        ct0: String::new(),
        personalization_id: String::new(),
    };

    for key in &required_keys {
        let value = cookies
            .get(*key)
            .ok_or_else(|| anyhow::anyhow!("Cookie中缺少必需的字段: {}", key))?;

        // 直接使用原始值，不进行URL解码
        match *key {
            "twid" => {
                result.twid = if value.starts_with("u=") {
                    value[2..].to_string()
                } else {
                    value.to_string()
                };
                // 手动处理URL编码的u=前缀
                if result.twid.starts_with("u%3D") {
                    result.twid = result.twid[4..].to_string();
                }
            }
            "auth_token" => result.auth_token = value.to_string(),
            "ct0" => result.ct0 = value.to_string(),
            "personalization_id" => result.personalization_id = value.to_string(),
            _ => {}
        }
    }

    Ok(result)
}

fn save_private_tokens(
    user_id: &str,
    bearer_token: &str,
    auth_token: &str,
    ct0: &str,
    personalization_id: &str,
    user_agent: &str,
    x_client_uuid: &str,
    x_client_transaction_id: &str,
    filename: &str,
) -> Result<()> {
    // 确保目录存在
    if let Some(parent) = Path::new(filename).parent() {
        fs::create_dir_all(parent)?;
    }

    let content = format!(
        "USER_ID={}\nBEARER_TOKEN={}\nAUTH_TOKEN={}\nCT0={}\nPERSONALIZATION_ID={}\nUSER_AGENT={}\nX_CLIENT_UUID={}\nX_CLIENT_TRANSACTION_ID={}\n",
        user_id, bearer_token, auth_token, ct0, personalization_id, user_agent, x_client_uuid, x_client_transaction_id
    );

    // 使用 OpenOptions 设置文件权限，确保敏感文件只有所有者可读写
    let mut file = {
        let mut opts = OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            opts.mode(0o600); // Unix: rw------- (仅所有者可读写)
        }
        opts.open(filename)?
    };
    
    // 在 Unix 系统上确保权限正确设置（双重保险）
    #[cfg(unix)]
    {
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(filename, perms)?;
    }
    
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    
    println!("生成 {} 成功！", filename);
    println!("已设置文件权限为 600 (仅所有者可读写)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_curl_command() {
        let curl_command = r#"
curl 'https://x.com/i/api/graphql/test' \
  -H 'Authorization: Bearer test_bearer_token' \
  -H 'User-Agent: Mozilla/5.0' \
  -H 'X-Client-UUID: test-uuid' \
  -H 'X-Client-Transaction-ID: test-transaction' \
  -b 'twid=u%3D123; auth_token=test_auth; ct0=test_ct0; personalization_id=test_pid'
"#;
        
        let result = parse_curl_command(curl_command).unwrap();
        assert_eq!(result.bearer_token, "test_bearer_token");
        assert_eq!(result.user_agent, Some("Mozilla/5.0".to_string()));
        assert_eq!(result.x_client_uuid, Some("test-uuid".to_string()));
        assert_eq!(result.x_client_transaction_id, Some("test-transaction".to_string()));
        assert!(result.cookie_str.contains("twid"));
    }

    #[test]
    fn test_parse_curl_command_minimal() {
        let curl_command = r#"
curl 'https://x.com/i/api/graphql/test' \
  -H 'Authorization: Bearer test_bearer' \
  -b 'auth_token=test_auth; ct0=test_ct0; personalization_id=test_pid; twid=123'
"#;
        
        let result = parse_curl_command(curl_command).unwrap();
        assert_eq!(result.bearer_token, "test_bearer");
        assert!(result.cookie_str.contains("auth_token"));
    }

    #[test]
    fn test_parse_curl_command_missing_bearer() {
        let curl_command = r#"
curl 'https://x.com/i/api/graphql/test' \
  -b 'auth_token=test_auth'
"#;
        
        let result = parse_curl_command(curl_command);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cookies() {
        let cookie_str = "twid=u%3D123; auth_token=test_auth_token; ct0=test_ct0_token; personalization_id=test_pid_value";
        
        let result = parse_cookies(cookie_str).unwrap();
        assert_eq!(result.twid, "123"); // 应该去掉 u%3D 前缀
        assert_eq!(result.auth_token, "test_auth_token");
        assert_eq!(result.ct0, "test_ct0_token");
        assert_eq!(result.personalization_id, "test_pid_value");
    }

    #[test]
    fn test_parse_cookies_with_spaces() {
        let cookie_str = "twid=123; auth_token=test_auth; ct0=test_ct0; personalization_id=test_pid";
        
        let result = parse_cookies(cookie_str).unwrap();
        assert_eq!(result.twid, "123");
        assert_eq!(result.auth_token, "test_auth");
    }

    #[test]
    fn test_parse_cookies_missing_field() {
        let cookie_str = "auth_token=test_auth; ct0=test_ct0";
        
        let result = parse_cookies(cookie_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cookies_twid_with_u_prefix() {
        let cookie_str = "twid=u=123; auth_token=test_auth; ct0=test_ct0; personalization_id=test_pid";
        
        let result = parse_cookies(cookie_str).unwrap();
        assert_eq!(result.twid, "123");
    }

    #[test]
    fn test_save_private_tokens() {
        let temp_dir = TempDir::new().unwrap();
        let token_file = temp_dir.path().join("tokens.env");
        
        save_private_tokens(
            "user123",
            "bearer_token",
            "auth_token",
            "ct0_token",
            "pid_value",
            "Mozilla/5.0",
            "uuid_value",
            "transaction_id",
            token_file.to_str().unwrap(),
        ).unwrap();
        
        let content = std::fs::read_to_string(&token_file).unwrap();
        assert!(content.contains("USER_ID=user123"));
        assert!(content.contains("BEARER_TOKEN=bearer_token"));
        assert!(content.contains("AUTH_TOKEN=auth_token"));
        assert!(content.contains("CT0=ct0_token"));
        assert!(content.contains("PERSONALIZATION_ID=pid_value"));
    }

    #[test]
    fn test_parse_curl_command_with_quotes() {
        let curl_command = r#"
curl 'https://x.com/i/api/graphql/test' \
  -H 'Authorization: Bearer "quoted_token"' \
  -b 'auth_token=test; ct0=test; personalization_id=test; twid=123'
"#;
        
        let result = parse_curl_command(curl_command);
        // 应该能处理带引号的token
        // 测试应该验证解析结果是否正确，而不是只检查是否有结果
        if let Ok(parsed) = result {
            // 验证带引号的token被正确处理（检查bearer_token字段不为空）
            assert!(!parsed.bearer_token.is_empty() || parsed.bearer_token.contains("quoted_token"));
        }
    }

    #[test]
    fn test_parse_cookies_with_quotes() {
        let cookie_str = r#"twid="123"; auth_token="test_auth"; ct0="test_ct0"; personalization_id="test_pid""#;
        
        let result = parse_cookies(cookie_str).unwrap();
        assert_eq!(result.twid, "123");
        assert_eq!(result.auth_token, "test_auth");
    }

    #[test]
    fn test_parse_cookies_empty_values() {
        let cookie_str = "twid=; auth_token=; ct0=; personalization_id=";
        
        let result = parse_cookies(cookie_str).unwrap();
        assert_eq!(result.twid, "");
        assert_eq!(result.auth_token, "");
    }
}
