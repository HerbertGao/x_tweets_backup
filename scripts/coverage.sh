#!/bin/bash
# 代码覆盖率测试脚本
# 覆盖率目标: 80%

set -e

echo "🔍 运行代码覆盖率测试..."
echo "📊 覆盖率目标: 80%"

# 检查是否安装了 cargo-llvm-cov
if ! command -v cargo-llvm-cov &> /dev/null; then
    echo "❌ cargo-llvm-cov 未安装"
    echo "📦 正在安装 cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

# 设置环境变量（如果需要）
if [ -z "$LLVM_COV" ]; then
    export LLVM_COV=$(find ~/.rustup -name "llvm-cov" 2>/dev/null | head -1)
    export LLVM_PROFDATA=$(find ~/.rustup -name "llvm-profdata" 2>/dev/null | head -1)
fi

# 运行覆盖率测试
echo "📊 生成覆盖率报告..."
cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info

# 生成HTML报告
echo "📄 生成HTML报告..."
cargo llvm-cov --all-features --workspace --html

# 获取覆盖率摘要
echo ""
echo "📈 覆盖率摘要:"
cargo llvm-cov --all-features --workspace --summary-only 2>&1 | grep -E "(lines|functions|regions)" || true

echo ""
echo "✅ 覆盖率测试完成！"
echo "📁 HTML报告位置: target/llvm-cov/html/index.html"
echo "📁 LCOV报告位置: lcov.info"
echo "🎯 覆盖率目标: 80%"
echo ""
echo "💡 提示: 使用 'open target/llvm-cov/html/index.html' 在浏览器中查看详细报告"

