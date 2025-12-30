#!/bin/bash

# 版本管理脚本
# 用法: ./scripts/version.sh [major|minor|patch|build|版本号]
# 示例: ./scripts/version.sh 1.2.3 或 ./scripts/version.sh patch

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # 无色

# 获取当前版本
get_current_version() {
    grep '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/'
}

# 验证版本号格式
is_valid_version() {
    local version=$1
    # 匹配格式: x.y.z 或 x.y.z.w (x, y, z, w 都是数字)
    if [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
        return 0
    else
        return 1
    fi
}

# 更新版本号
update_version() {
    local version_input=$1
    local current_version=$(get_current_version)
    echo -e "${YELLOW}当前版本: ${current_version}${NC}"
    
    local new_version
    
    # 检查是否是有效的版本号格式
    if is_valid_version "$version_input"; then
        # 直接使用提供的版本号
        new_version="$version_input"
        echo -e "${YELLOW}使用自定义版本号: ${new_version}${NC}"
    else
        # 使用递增逻辑
        IFS='.' read -ra VERSION_PARTS <<< "$current_version"
        local major=${VERSION_PARTS[0]}
        local minor=${VERSION_PARTS[1]}
        local patch=${VERSION_PARTS[2]}
        local build=${VERSION_PARTS[3]:-0}
        case $version_input in
            "major")
                major=$((major + 1)); minor=0; patch=0; build=0;;
            "minor")
                minor=$((minor + 1)); patch=0; build=0;;
            "patch")
                patch=$((patch + 1)); build=0;;
            "build")
                build=$((build + 1));;
            *)
                echo -e "${RED}错误: 无效的版本类型或版本号格式${NC}"
                echo "使用: major|minor|patch|build 或版本号格式 (如: 1.2.3)"
                exit 1;;
        esac
        new_version="${major}.${minor}.${patch}"
        if [ $build -gt 0 ]; then
            new_version="${new_version}.${build}"
        fi
    fi
    echo -e "${YELLOW}新版本: ${new_version}${NC}"
    
    # 检查版本是否相同
    if [ "$current_version" == "$new_version" ]; then
        echo -e "${YELLOW}提示: 新版本号与当前版本号相同，无需更新文件${NC}"
        echo -e "${YELLOW}Git 状态:${NC}"
        git status --porcelain
        echo -e "${GREEN}版本检查完成！${NC}"
        echo -e "${YELLOW}下一步操作建议:${NC}"
        echo "1. 测试代码: cargo check && cargo test"
        echo "2. 如需创建标签: git tag v${new_version}"
        echo "3. 如需推送标签: git push origin v${new_version}"
        return 0
    fi
    
    # 更新 Cargo.toml
    if [[ "$OSTYPE" == "darwin"* ]]; then
        sed -i '' "s/^version = \".*\"/version = \"${new_version}\"/" Cargo.toml
    else
        sed -i "s/^version = \".*\"/version = \"${new_version}\"/" Cargo.toml
    fi
    echo -e "${GREEN}✓ 已更新 Cargo.toml 版本为 ${new_version}${NC}"
    # 更新 README.md 版本
    if [ -f "README.md" ]; then
        if [[ "$OSTYPE" == "darwin"* ]]; then
            sed -i '' "s/版本.*[0-9]\+\.[0-9]\+\.[0-9]\+/版本 ${new_version}/g" README.md
        else
            sed -i "s/版本.*[0-9]\+\.[0-9]\+\.[0-9]\+/版本 ${new_version}/g" README.md
        fi
        echo -e "${GREEN}✓ 已更新 README.md 版本信息${NC}"
    fi
    echo -e "${YELLOW}Git 状态:${NC}"
    git status --porcelain
    echo -e "${GREEN}版本更新完成！${NC}"
    echo -e "${YELLOW}下一步操作建议:${NC}"
    echo "1. 测试代码: cargo check && cargo test"
    echo "2. 提交更改: git add . && git commit -m \"chore: bump version to ${new_version}\""
    echo "3. 创建标签: git tag v${new_version}"
    echo "4. 推送到远程: git push origin master && git push origin v${new_version}"
}

# 显示版本信息
show_version() {
    local current_version=$(get_current_version)
    echo -e "${GREEN}当前版本: ${current_version}${NC}"
    echo -e "${YELLOW}最近的标签:${NC}"
    git tag --sort=-version:refname | head -5
}

# 主流程
main() {
    if [ $# -eq 0 ]; then
        show_version
        echo -e "${YELLOW}用法: $0 [major|minor|patch|build|版本号]${NC}"
        echo "  major     - 主版本号 (1.0.0 -> 2.0.0)"
        echo "  minor     - 次版本号 (1.0.0 -> 1.1.0)"
        echo "  patch     - 补丁版本 (1.0.0 -> 1.0.1)"
        echo "  build     - 构建版本 (1.0.0 -> 1.0.0.1)"
        echo "  版本号    - 自定义版本号 (如: 1.2.3 或 1.2.3.4)"
        exit 0
    fi
    update_version $1
}

main "$@"

