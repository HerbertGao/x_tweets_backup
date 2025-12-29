# X Tweets Backup

一个用于备份某人全部推文内容和媒体的Rust工具。

## 功能特性

- 🔐 支持X (Twitter) 认证
- 📱 自动获取用户推文
- 📝 保存推文内容为Markdown格式
- 🎥 下载推文中的媒体文件（图片、视频）
- ⚡ 断点续传，避免重复下载
- 🎯 支持批量处理和增量备份
- 📄 生成完整的推文备份文档

## 安装

### 从源码编译

```bash
git clone <repository-url>
cd x_tweets_backup
cargo build --release
```

### 使用预编译版本

从 [Releases](https://github.com/HerbertGao/x_tweets_backup/releases) 页面下载对应平台的预编译版本。

## 快速开始

### 1. 初始化配置

首先，你需要从浏览器获取认证信息。在X网站上打开开发者工具，找到任意一个API请求，复制其curl命令到 `curl_command.txt` 文件中。

然后运行：

```bash
./x_tweets_backup setup
```

这将自动解析curl命令并提取必要的认证信息。

> ⚠️ **安全警告**: 
> - `data/private_tokens.env` 文件包含敏感认证信息，请确保：
>   - 设置文件权限为 `chmod 600 data/private_tokens.env`
>   - 不要将此文件提交到版本控制系统
>   - 不要与他人分享此文件
> - `DEBUG_LOGS=true` 会输出敏感信息（包括认证令牌和用户数据），默认已关闭，仅在调试时启用

### 2. 配置环境变量

复制 `env.example` 为 `.env` 并根据需要修改配置：

```bash
cp env.example .env
```

主要配置项：

- `COUNT`: 每次获取的推文数量（默认20）
- `ALL`: 是否下载所有推文（默认true）
- `DOWNLOAD_DIR`: 下载目录
- `SAVE_MARKDOWN`: 是否保存推文内容为Markdown（默认true）
- `MARKDOWN_OUTPUT`: Markdown输出文件路径

### 3. 开始备份

```bash
# 方法1：通过命令行参数指定用户名
./x_tweets_backup backup <username>

# 方法2：通过配置文件指定用户名
# 在.env文件中设置TARGET_USER_ID=<username>
./x_tweets_backup backup
```

程序会：

1. 获取指定用户的所有推文
2. 提取推文内容和媒体文件
3. 下载媒体文件到指定目录
4. 保存推文内容为Markdown格式
5. 记录已下载的推文ID，避免重复下载

## 命令行选项

```bash
x_tweets_backup [COMMAND]

Commands:
  setup     初始化配置 [curl_file]
  backup    备份用户推文 [USERNAME]
  update    检查并更新到最新版本
```

## 配置说明

### 环境变量

| 变量名 | 说明 | 默认值 |
|--------|------|--------|
| `COUNT` | 每次获取的推文数量 | 20 |
| `ALL` | 是否下载所有推文 | true |
| `DOWNLOAD_DIR` | 下载目录 | data/downloads |
| `TARGET_USER_ID` | 目标用户名（必需） | 无 |
| `SAVE_MARKDOWN` | 保存推文内容为Markdown | true |
| `MARKDOWN_OUTPUT` | Markdown输出文件路径 | data/tweets_backup.md |
| `DEBUG_LOGS` | 启用详细调试日志 | false |


## 文件结构

```
x_tweets_backup/
├── data/
│   ├── downloads/          # 下载目录
│   ├── downloaded_tweet_ids.txt  # 下载记录
│   ├── tweets_backup.md   # Markdown备份文件
│   └── private_tokens.env        # 认证信息
├── src/                    # 源代码目录
│   ├── lib.rs             # 库入口（用于测试）
│   ├── main.rs            # 主程序入口
│   ├── config.rs          # 配置管理
│   ├── x_api.rs           # X API 客户端
│   ├── downloader.rs      # 媒体下载器
│   ├── markdown_generator.rs  # Markdown生成器
│   ├── setup.rs           # 初始化设置
│   └── updater.rs         # 更新检查
├── tests/                  # 集成测试目录
│   └── test_api_response.rs  # API响应测试工具
├── scripts/               # 工具脚本
│   ├── coverage.sh        # 覆盖率测试脚本
│   └── coverage-tarpaulin.sh  # Tarpaulin覆盖率脚本
├── Cargo.toml             # 项目配置
├── env.example            # 环境变量示例
└── README.md              # 项目说明
```

## 输出格式

程序会生成包含推文内容的Markdown文件，包括文本、媒体链接、发布时间等信息。媒体文件会下载到指定目录，并在Markdown中嵌入HTML标签以便直接预览。

## 注意事项

1. **认证信息**: 请妥善保管 `data/private_tokens.env` 文件，不要将其提交到版本控制系统。
2. **API限制**: X API有请求频率限制，大量推文可能需要较长时间。
3. **断点续传**: 程序会记录已下载的推文ID，重新运行时会跳过已下载的内容。
4. **调试日志**: 使用 `DEBUG_LOGS=true` 启用详细日志（包含敏感信息，请谨慎使用）。

## 测试

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行单元测试
cargo test --lib

# 运行集成测试
cargo test --test test_api_response
```

### 代码覆盖率

项目支持使用 `cargo-llvm-cov` 进行代码覆盖率测试，**覆盖率目标为 80%**。

```bash
# 安装工具
cargo install cargo-llvm-cov
rustup component add llvm-tools-preview

# 生成HTML报告
cargo llvm-cov --all-features --workspace --html

# 查看报告
open target/llvm-cov/html/index.html
```

或使用提供的脚本：

```bash
./scripts/coverage.sh
```

脚本会自动显示覆盖率摘要并与80%的目标进行对比。

## 贡献

欢迎提交Issue和Pull Request！提交前请确保代码通过所有测试。

## 许可证

MIT License

## 免责声明

本工具仅供学习和研究使用，请遵守X的使用条款和相关法律法规。使用者需自行承担使用风险。
