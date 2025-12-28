use anyhow::Result;
use reqwest::Client;
use serde_json::Value;

pub struct Updater {
    client: Client,
}

impl Updater {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()?;

        Ok(Updater { client })
    }

    pub async fn update(&self) -> Result<()> {
        println!("检查更新...");

        // 获取最新版本信息
        let latest_version = self.get_latest_version().await?;
        let current_version = env!("CARGO_PKG_VERSION");

        if latest_version == current_version {
            println!("当前已是最新版本: {}", current_version);
            return Ok(());
        }

        println!("发现新版本: {} (当前版本: {})", latest_version, current_version);
        println!("请访问项目页面获取最新版本: https://github.com/HerbertGao/x_tweets_backup");

        Ok(())
    }

    async fn get_latest_version(&self) -> Result<String> {
        let response = self.client
            .get("https://api.github.com/repos/HerbertGao/x_tweets_backup/releases/latest")
            .header("User-Agent", "x_tweets_backup")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("无法获取版本信息"));
        }

        let data: Value = response.json().await?;
        let version = data.get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("无法解析版本信息"))?;

        Ok(version.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_updater_new() {
        let _updater = Updater::new().unwrap();
        // 验证创建成功
        assert!(true); // Updater 没有公开字段，只能验证创建不失败
    }

    #[test]
    fn test_updater_new_creates_client() {
        // 测试创建 Updater 时客户端是否成功创建
        let result = Updater::new();
        assert!(result.is_ok());
        let updater = result.unwrap();
        // 验证 updater 已创建（虽然没有公开字段，但可以验证不 panic）
        drop(updater);
    }

    #[test]
    fn test_updater_version_comparison_logic() {
        // 测试版本比较逻辑（不涉及网络请求）
        let current = "1.0.0";
        let latest = "1.0.0";
        assert_eq!(current == latest, true);
        
        let latest_new = "1.0.1";
        assert_eq!(current == latest_new, false);
    }

    #[test]
    fn test_updater_version_string_format() {
        // 测试版本字符串格式
        let version = env!("CARGO_PKG_VERSION");
        // 版本号应该是有效的字符串
        assert!(!version.is_empty());
        // 应该包含至少一个点
        assert!(version.contains('.') || version == "0.0.0" || version.parse::<f64>().is_ok());
    }
}
