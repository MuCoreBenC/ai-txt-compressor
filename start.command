#!/usr/bin/env bash
# macOS 双击启动脚本（.command 文件会被 Finder 用 Terminal.app 打开）
# 双击本文件即可在终端启动 aitxt 服务

# 切到脚本所在目录（必须 cd，否则双击时 cwd 是 $HOME）
cd "$(dirname "$0")" || {
  # 用 osascript 弹窗报错（终端可能没正确打开）
  osascript -e 'display dialog "无法切换到脚本目录，请手动在终端运行：cd /Users/wzy/projects/aitxt && bash start.sh" with title "AITXT 启动失败" buttons {"OK"}'
  exit 1
}

# 调用主启动脚本（前台运行，Ctrl+C 退出）
bash ./start.sh

# 如果 start.sh 退出（比如出错），保持终端窗口打开让用户看到错误
echo ""
echo "================================================"
echo "  服务已退出。按回车关闭终端窗口。"
echo "================================================"
read -r
