#!/usr/bin/env bash
# 一键启动：单文件二进制（页面 + API 同端口）前台运行
# 用法：
#   bash start.sh                        # 默认本地 Ollama，端口 8787
#   DEEPSEEK_API_KEY=sk-xxx bash start.sh# 启用 DeepSeek（也可在页面下方填写）
#   bash start.sh --rebuild              # 强制重新编译
#   bash start.sh stop                   # 清理残留进程
#
# 端口固定 8787（如需修改改本文件 PORT 变量）
# 自动 rebuild：检测 compress.html 或 compressor/src/**/*.rs 比 target/release/compressor 新时自动重编译
# 停止：直接 Ctrl+C，或关闭终端

set -e

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMPRESSOR_DIR="$ROOT_DIR/compressor"
BIN="$COMPRESSOR_DIR/target/release/compressor"
PID_FILE="$ROOT_DIR/.start.pids"
# 固定端口 8787（如需换端口改这里，所有 URL 会自动同步）
PORT=8787

# 参数解析：仅支持 --rebuild / stop，不再接受端口参数（避免端口漂移）
FORCE_REBUILD=0
for arg in "$@"; do
  case "$arg" in
    --rebuild) FORCE_REBUILD=1 ;;
    stop) PORT_CMD=stop ;;
    *) echo "[warn] 忽略未知参数：$arg（端口已固定为 $PORT，不再支持参数指定）" ;;
  esac
done

# ===== stop 子命令（兜底清理旧的后台进程） =====
if [ "${PORT_CMD:-}" = "stop" ]; then
  echo "[stop] 正在清理残留进程…"
  if [ -f "$PID_FILE" ]; then
    while read -r pid; do
      if kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null && echo "  已停止 PID $pid"
      fi
    done < "$PID_FILE"
    rm -f "$PID_FILE"
  fi
  pids=$(lsof -ti tcp:"$PORT" 2>/dev/null || true)
  [ -n "$pids" ] && echo "$pids" | xargs kill 2>/dev/null && echo "  已释放端口 $PORT"
  echo "[stop] 完成"
  exit 0
fi

# ===== 检测是否需要重新编译 =====
# 单文件模式：compress.html 通过 include_str! 嵌入二进制，
# 所以 compress.html 或 src/**/*.rs 改动后必须重新 cargo build
NEED_BUILD=0
if [ ! -f "$BIN" ]; then
  echo "[build] 二进制不存在，开始编译…"
  NEED_BUILD=1
elif [ "$FORCE_REBUILD" = "1" ]; then
  echo "[build] --rebuild 强制重新编译…"
  NEED_BUILD=1
else
  # 取二进制 mtime，与 compress.html + 所有 src/**/*.rs 比对
  BIN_MT=$(stat -f %m "$BIN" 2>/dev/null || stat -c %Y "$BIN" 2>/dev/null || echo 0)
  newest_src=0
  # 检查 compress.html
  if [ -f "$ROOT_DIR/compress.html" ]; then
    t=$(stat -f %m "$ROOT_DIR/compress.html" 2>/dev/null || stat -c %Y "$ROOT_DIR/compress.html" 2>/dev/null || echo 0)
    [ "$t" -gt "$newest_src" ] && newest_src=$t
  fi
  # 检查 compressor/src 下所有 .rs
  while IFS= read -r -d '' f; do
    t=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || echo 0)
    [ "$t" -gt "$newest_src" ] && newest_src=$t
  done < <(find "$COMPRESSOR_DIR/src" -name '*.rs' -print0 2>/dev/null)
  # 检查 Cargo.toml
  if [ -f "$COMPRESSOR_DIR/Cargo.toml" ]; then
    t=$(stat -f %m "$COMPRESSOR_DIR/Cargo.toml" 2>/dev/null || stat -c %Y "$COMPRESSOR_DIR/Cargo.toml" 2>/dev/null || echo 0)
    [ "$t" -gt "$newest_src" ] && newest_src=$t
  fi

  if [ "$newest_src" -gt "$BIN_MT" ]; then
    echo "[build] 检测到源文件比二进制新（compress.html / src/*.rs / Cargo.toml 改动），自动重新编译…"
    NEED_BUILD=1
  else
    echo "[build] 二进制已是最新，跳过编译（强制重编译：bash start.sh --rebuild）"
  fi
fi

if [ "$NEED_BUILD" = "1" ]; then
  if ! command -v cargo >/dev/null 2>&1 && ! [ -x "$HOME/.cargo/bin/cargo" ]; then
    echo "[error] cargo 未安装。请先运行：cd compressor && bash install.sh"
    exit 1
  fi
  # 兼容 PATH 中无 cargo 的情况
  CARGO_BIN=$(command -v cargo 2>/dev/null || echo "$HOME/.cargo/bin/cargo")
  (cd "$COMPRESSOR_DIR" && "$CARGO_BIN" build --release)
  echo "[build] ✓ 编译完成"
fi

# ===== 清理同端口的旧进程 =====
pids=$(lsof -ti tcp:"$PORT" 2>/dev/null || true)
[ -n "$pids" ] && echo "$pids" | xargs kill 2>/dev/null && echo "[cleanup] 已清理占用端口 $PORT 的旧进程" || true

# ===== DeepSeek API key 状态提示 =====
if [ -n "$DEEPSEEK_API_KEY" ]; then
  echo "[deepseek] ✓ DEEPSEEK_API_KEY 已配置（长度 ${#DEEPSEEK_API_KEY}）"
else
  echo "[deepseek] 未配置 DEEPSEEK_API_KEY（仅本地 Ollama 可用）"
  echo "          启用 DeepSeek：DEEPSEEK_API_KEY=sk-xxx bash start.sh"
fi

# ===== 前台启动单文件服务 =====
URL="http://127.0.0.1:$PORT/"
echo ""
echo "================================================"
echo "  单文件模式：页面 + API 同端口"
echo "  页面地址: $URL"
echo "  健康检查: ${URL}health"
echo ""
echo "  停止方式: Ctrl+C  /  直接关闭终端"
echo "  （如残留进程可执行: bash start.sh stop）"
echo "================================================"
echo ""

# 后台 fork 二进制以便就绪后打开浏览器；随后 wait 让其占据前台
"$BIN" --serve --port "$PORT" &
SERVER_PID=$!

# 捕获信号：Ctrl+C 或终端关闭时一并杀掉子进程
cleanup() {
  kill "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
  echo ""
  echo "[exit] 已停止服务 (PID $SERVER_PID)"
  exit 0
}
trap cleanup INT TERM HUP

# 等待服务就绪（最多 10 秒）
for i in $(seq 1 20); do
  if curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    echo "[ready] ✓ 服务就绪 (PID $SERVER_PID)"
    break
  fi
  sleep 0.5
  if [ $i -eq 20 ]; then
    echo "[ready] ✗ 启动超时"
    exit 1
  fi
done

# 打开浏览器
if command -v open >/dev/null 2>&1; then
  open "$URL"
elif command -v xdg-open >/dev/null 2>&1; then
  xdg-open "$URL"
fi

echo "[running] 服务运行中。按 Ctrl+C 停止，或直接关闭终端。"
# 阻塞前台：等待子进程退出（Ctrl+C 由 trap 处理）
wait "$SERVER_PID"
