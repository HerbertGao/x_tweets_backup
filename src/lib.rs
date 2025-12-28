//! X 推文备份库
//!
//! 本库提供从 X（原 Twitter）备份推文的功能，
//! 包括下载媒体文件和生成 Markdown 文档。
//!
//! # 示例
//!
//! ```no_run
//! use x_tweets_backup::{Config, XApi, Downloader, MarkdownGenerator};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Config::load()?;
//! let api = XApi::new(config.clone())?;
//! let downloader = Downloader::new(config.clone())?;
//! let markdown_gen = MarkdownGenerator::new(config);
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod downloader;
pub mod setup;
pub mod updater;
pub mod x_api;
pub mod markdown_generator;
pub mod tweet_parser;

// 重新导出常用类型
pub use config::Config;
pub use downloader::Downloader;
pub use markdown_generator::MarkdownGenerator;
pub use x_api::XApi;

