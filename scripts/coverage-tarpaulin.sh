#!/bin/bash
# 使用 cargo-tarpaulin 的覆盖率测试脚本

set -e

echo "🔍 使用 cargo-tarpaulin 运行代码覆盖率测试..."

# 检查是否安装了 cargo-tarpaulin
if ! command -v cargo-tarpaulin &> /dev/null; then
    echo "❌ cargo-tarpaulin 未安装"
    echo "📦 正在安装 cargo-tarpaulin..."
    cargo install cargo-tarpaulin
fi

# 创建覆盖率输出目录
mkdir -p coverage

# 运行覆盖率测试
echo "📊 生成覆盖率报告..."
cargo tarpaulin \
    --all-features \
    --workspace \
    --out Html \
    --output-dir coverage \
    --timeout 120

echo ""
echo "✅ 覆盖率测试完成！"
echo "📁 HTML报告位置: coverage/tarpaulin-report.html"
echo ""
echo "💡 提示: 使用 'open coverage/tarpaulin-report.html' 在浏览器中查看详细报告"

