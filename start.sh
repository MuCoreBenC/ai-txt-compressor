#!/usr/bin/env bash
# 一键启动：后端 compressor 服务 + 前端静态服务器 + 自动打开浏览器
# 用法：bash start.sh
# 停止：bash start.sh stop

set -e

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMPRESSOR_DIR="$ROOT_DIR/compressor"
BIN="$COMPRESSOR_DIR/target/release/compressor"
PID_FILE="$ROOT_DIR/.start.pids"
FRONTEND_PORT=8765
BACKEND_PORT=8787

# ===== stop 子命令 =====
if [ "$1" = "stop" ]; then
  echo "[stop] 正在停止服务…"
  if [ -f "$PID_FILE" ]; then
    while read -r pid; do
      if kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null && echo "  已停止 PID $pid"
      fi
    done < "$PID_FILE"
    rm -f "$PID_FILE"
  fi
  # 兜底：按端口杀进程
  for port in $FRONTEND_PORT $BACKEND_PORT; do
    pids=$(lsof -ti tcp:$port 2>/dev/null || true)
    if [ -n "$pids" ]; then
      echo "$pids" | xargs kill 2>/dev/null && echo "  已释放端口 $port"
    fi
  done
  echo "[stop] 完成"
  exit 0
fi

# ===== 检查 compressor 二进制 =====
if [ ! -f "$BIN" ]; then
  echo "[build] compressor 二进制不存在，开始编译…"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "[error] cargo 未安装。请先运行：cd compressor && bash install.sh"
    exit 1
  fi
  (cd "$COMPRESSOR_DIR" && cargo build --release)
fi

# ===== 清理旧进程 =====
if [ -f "$PID_FILE" ]; then
  echo "[cleanup] 清理旧进程…"
  while read -r pid; do
    kill "$pid" 2>/dev/null || true
  done < "$PID_FILE"
  rm -f "$PID_FILE"
fi
for port in $FRONTEND_PORT $BACKEND_PORT; do
  pids=$(lsof -ti tcp:$port 2>/dev/null || true)
  [ -n "$pids" ] && echo "$pids" | xargs kill 2>/dev/null || true
done

# ===== 启动后端 compressor =====
echo "[backend] 启动 compressor --serve (端口 $BACKEND_PORT)…"
nohup "$BIN" --serve --port "$BACKEND_PORT" > "$ROOT_DIR/.backend.log" 2>&1 &
BACKEND_PID=$!
echo "$BACKEND_PID" > "$PID_FILE"

# 等待后端就绪（最多 10 秒）
echo "[backend] 等待服务就绪…"
for i in $(seq 1 20); do
  if curl -sf "http://127.0.0.1:$BACKEND_PORT/health" >/dev/null 2>&1; then
    echo "[backend] ✓ 就绪 (PID $BACKEND_PID)"
    break
  fi
  sleep 0.5
  if [ $i -eq 20 ]; then
    echo "[backend] ✗ 启动超时，查看 .backend.log"
    exit 1
  fi
done

# ===== 启动前端静态服务器 =====
echo "[frontend] 启动静态服务器 (端口 $FRONTEND_PORT)…"
nohup python3 -m http.server "$FRONTEND_PORT" --bind 127.0.0.1 --directory "$ROOT_DIR" > "$ROOT_DIR/.frontend.log" 2>&1 &
FRONTEND_PID=$!
echo "$FRONTEND_PID" >> "$PID_FILE"

# 等待前端就绪
for i in $(seq 1 10); do
  if curl -sf "http://127.0.0.1:$FRONTEND_PORT/" >/dev/null 2>&1; then
    echo "[frontend] ✓ 就绪 (PID $FRONTEND_PID)"
    break
  fi
  sleep 0.3
done

# ===== 打开浏览器 =====
URL="http://127.0.0.1:$FRONTEND_PORT/compress.html"
echo ""
echo "================================================"
echo "  ✓ 服务已启动"
echo "  前端页面: $URL"
echo "  后端 API: http://127.0.0.1:$BACKEND_PORT"
echo "  健康检查: http://127.0.0.1:$BACKEND_PORT/health"
echo ""
echo "  停止服务: bash start.sh stop"
echo "  后端日志: tail -f $ROOT_DIR/.backend.log"
echo "================================================"
echo ""
echo "[browser] 正在打开浏览器…"
if command -v open >/dev/null 2>&1; then
  open "$URL"
elif command -v xdg-open >/dev/null 2>&1; then
  xdg-open "$URL"
fi

echo "[done] 启动完成。按 Ctrl+C 不会停止后台服务，请用 'bash start.sh stop' 停止。"
