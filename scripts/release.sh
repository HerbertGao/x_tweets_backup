#!/bin/bash

# 发布脚本
# 用法: ./scripts/release.sh [major|minor|patch|build|版本号]
# 示例: ./scripts/release.sh 1.2.3 或 ./scripts/release.sh patch

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # 无色

# 检查 git 仓库
check_git_repo() {
    if ! git rev-parse --git-dir > /dev/null 2>&1; then
        echo -e "${RED}错误: 当前目录不是 git 仓库${NC}"
        exit 1
    fi
}

# 检查未提交更改
check_uncommitted_changes() {
    if ! git diff-index --quiet HEAD --; then
        echo -e "${YELLOW}警告: 有未提交的更改${NC}"
        git status --porcelain
        echo -e "${YELLOW}是否继续？(y/N): ${NC}"
        read -r response
        if [[ ! "$response" =~ ^[Yy]$ ]]; then
            echo "取消发布"
            exit 0
        fi
    fi
}

# 运行测试
run_tests() {
    echo -e "${BLUE}运行测试...${NC}"
    cargo check
    echo -e "${GREEN}✓ 代码检查通过${NC}"
}

# 编译发布版本
build_release() {
    echo -e "${BLUE}编译发布版本...${NC}"
    cargo build --release
    echo -e "${GREEN}✓ 编译完成${NC}"
}

# 创建 release 目录并复制可执行文件
create_release_package() {
    local version=$1
    local release_dir="release"
    mkdir -p "$release_dir"
    local target_name="x_tweets_backup"
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" ]]; then
        target_name="${target_name}.exe"
    fi
    cp "target/release/$target_name" "$release_dir/${target_name}_$(uname -s)_$(uname -m)"
    echo -e "${GREEN}✓ 发布包已创建: $release_dir/${target_name}_$(uname -s)_$(uname -m)${NC}"
}

# 提交更改
commit_changes() {
    local version=$1
    local commit_message="chore: bump version to ${version}"
    echo -e "${BLUE}提交更改...${NC}"
    git add .
    git commit -m "$commit_message"
    echo -e "${GREEN}✓ 更改已提交${NC}"
}

# 创建标签
git_tag() {
    local version=$1
    local tag_name="v${version}"
    echo -e "${BLUE}创建标签 ${tag_name}...${NC}"
    git tag "$tag_name"
    echo -e "${GREEN}✓ 标签已创建${NC}"
}

# 推送到远程
push_to_remote() {
    local version=$1
    local tag_name="v${version}"
    echo -e "${BLUE}推送到远程仓库...${NC}"
    git push origin master
    git push origin "$tag_name"
    echo -e "${GREEN}✓ 已推送到远程仓库${NC}"
}

# 显示发布信息
show_release_info() {
    local version=$1
    local tag_name="v${version}"
    echo -e "${GREEN}🎉 发布完成！${NC}"
    echo -e "${YELLOW}版本: ${version}${NC}"
    echo -e "${YELLOW}标签: ${tag_name}${NC}"
    echo -e "${YELLOW}GitHub Actions 将自动构建多平台版本${NC}"
    echo -e "${BLUE}查看构建状态: https://github.com/HerbertGao/x_tweets_backup/actions${NC}"
}

# 主流程
main() {
    if [ $# -eq 0 ]; then
        echo -e "${RED}错误: 请指定版本类型或版本号${NC}"
        echo "用法: $0 [major|minor|patch|build|版本号]"
        echo "  major     - 主版本号 (1.0.0 -> 2.0.0)"
        echo "  minor     - 次版本号 (1.0.0 -> 1.1.0)"
        echo "  patch     - 补丁版本 (1.0.0 -> 1.0.1)"
        echo "  build     - 构建版本 (1.0.0 -> 1.0.0.1)"
        echo "  版本号    - 自定义版本号 (如: 1.2.3 或 1.2.3.4)"
        exit 1
    fi
    local version_type=$1
    echo -e "${BLUE}开始发布流程...${NC}"
    check_git_repo
    check_uncommitted_changes
    echo -e "${BLUE}更新版本...${NC}"
    ./scripts/version.sh "$version_type"
    local new_version=$(grep '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
    run_tests
    build_release
    commit_changes "$new_version"
    git_tag "$new_version"
    push_to_remote "$new_version"
    show_release_info "$new_version"
}

main "$@"

