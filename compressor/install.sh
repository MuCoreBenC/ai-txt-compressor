#!/usr/bin/env bash
# 一键安装脚本：Rust + Ollama + qwen2.5:0.5b 模型 + 编译本工具
# 适用 macOS（Apple Silicon / Intel），其他平台需手动调整

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "============================================"
echo "  aitxt-compressor · 一键安装"
echo "============================================"

# ---------- 1. Homebrew ----------
if ! command -v brew >/dev/null 2>&1; then
    echo "[1/5] 安装 Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
else
    echo "[1/5] Homebrew 已安装 ✓"
fi

# ---------- 2. Rust toolchain ----------
if ! command -v cargo >/dev/null 2>&1; then
    echo "[2/5] 安装 Rust 工具链..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
    source "$HOME/.cargo/env"
else
    echo "[2/5] Rust 已安装 ✓ ($(cargo --version))"
    source "$HOME/.cargo/env" 2>/dev/null || true
fi

# ---------- 3. Ollama ----------
if ! command -v ollama >/dev/null 2>&1; then
    echo "[3/5] 安装 Ollama..."
    brew install ollama
else
    echo "[3/5] Ollama 已安装 ✓ ($(ollama --version 2>&1 || echo 'version unknown'))"
fi

# ---------- 4. 启动 Ollama 服务 + 拉模型 ----------
echo "[4/5] 启动 Ollama 后台服务..."
if ! curl -s http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
    if command -v brew >/dev/null 2>&1; then
        brew services start ollama 2>/dev/null || (ollama serve >/tmp/ollama.log 2>&1 &)
    else
        ollama serve >/tmp/ollama.log 2>&1 &
    fi
    # 等待服务就绪
    for i in $(seq 1 30); do
        if curl -s http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
            break
        fi
        sleep 1
    done
fi

if curl -s http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
    echo "  Ollama 服务就绪 ✓"
else
    echo "  ⚠️  Ollama 服务未就绪，请手动运行: ollama serve"
fi

echo "  拉取 qwen2.5:1.5b 模型 (约 986MB，首次较慢，质量更好)..."
if ! ollama list 2>/dev/null | grep -q "qwen2.5:1.5b"; then
    ollama pull qwen2.5:1.5b
else
    echo "  qwen2.5:1.5b 已存在 ✓"
fi

echo "  拉取 qwen2.5:0.5b 模型 (约 397MB，备选快速模式)..."
if ! ollama list 2>/dev/null | grep -q "qwen2.5:0.5b"; then
    ollama pull qwen2.5:0.5b
else
    echo "  qwen2.5:0.5b 已存在 ✓"
fi

# ---------- 5. 编译本项目 ----------
echo "[5/5] 编译 compressor..."
cargo build --release

echo ""
echo "============================================"
echo "  安装完成 ✓"
echo "============================================"
echo ""
echo "使用方式："
echo ""
echo "  # CLI 模式（默认）"
echo "  echo '测试文本' | ./target/release/compressor --verbose"
echo ""
echo "  # 文件模式"
echo "  ./target/release/compressor < input.txt > output.txt --ratio 0.5"
echo ""
echo "  # 仅算法（不调用模型，离线快速）"
echo "  echo '测试' | ./target/release/compressor --no-model"
echo ""
echo "  # HTTP 服务模式"
echo "  ./target/release/compressor --serve --port 8787"
echo "  curl -X POST http://127.0.0.1:8787/compress \\"
echo "    -H 'Content-Type: application/json' \\"
echo "    -d '{\"text\":\"测试内容\",\"ratio\":0.5}'"
echo ""
echo "参数："
echo "  --ratio <0.0-1.0>  目标压缩比，默认 0.5"
echo "  --no-model         只跑算法，不调 Ollama"
echo "  --model <NAME>     指定 Ollama 模型，默认 qwen2.5:1.5b（可用 qwen2.5:0.5b 更快）"
echo "  --serve            启动 HTTP 服务"
echo "  --port <PORT>      HTTP 端口，默认 8787"
echo "  --verbose          打印每阶段字数与耗时"
