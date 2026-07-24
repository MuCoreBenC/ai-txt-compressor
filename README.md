# ai-txt-compressor

> 把长文本压缩成精简提示词的本地工具 —— 规则清洗 + TextRank 抽取 + 本地小模型抽象压缩，三段式混合管线，全程离线可跑。

## 简介

`ai-txt-compressor` 是一个**面向 LLM 提示词中转场景**的文本压缩工具。当原始文案太长、塞进 prompt 会爆上下文或拖慢推理时，先用它把文本压到目标字数（默认压到原文 50%），再喂给下游模型。

核心思路是**算法 + 模型混合**：

1. **规则清洗**：剥离 Markdown 符号、合并断句、同义词替换、删填充词
2. **TextRank 抽取**：jieba 分词 + TextRank 选最重要句子拼到目标字数
3. **模型压缩（可选）**：本地 Ollama 调 qwen2.5 小模型做抽象式精简

不调云 API、不传数据出本地，适合处理敏感或隐私文本。

## 特性

- **三段式混合管线**：规则 → TextRank → 本地模型，逐级压缩，任一阶段达标即停
- **离线可用**：`--no-model` 模式纯算法运行，无需 Ollama
- **双入口**：CLI（stdin/stdout，方便管道串联）+ HTTP 服务（axum，方便前端调用）
- **中文优化**：jieba 分词、中文停用词、中文填充词 / 同义词表
- **自带前端**：单文件 HTML，深色 / 浅色双主题，实时显示每阶段字数与压缩比
- **一键启停**：`start.sh` 拉起后端 + 前端 + 自动开浏览器

## 工作原理

```
原文 ──► [规则清洗] ──► [TextRank 抽取] ──► [Ollama 模型] ──► 压缩结果
              │              │                   │
              │              │                   │ 可选（--no-model 跳过）
              │              │                   │
              │              │ 算法阶段目标 = 最终目标 × 1.5
              │              │ 给模型留进一步压缩空间
              │              │
              │ 剥离 MD 符号、合并断句
              │ 同义词替换、删填充词
              │
              └── 若清洗后已达标，直接返回，跳过后续阶段
```

模型阶段有**回退保护**：若模型输出未比算法输出短至少 5%，自动回退用算法结果，避免模型"写了反而更长"。

## 快速开始

### 一键安装（macOS）

```bash
cd compressor
bash install.sh
```

`install.sh` 会自动安装：Homebrew → Rust 工具链 → Ollama → 拉取 `qwen2.5:1.5b` 与 `qwen2.5:0.5b` 模型 → 编译本工具。

### 一键启动

```bash
bash start.sh
```

会同时拉起：

- 后端 HTTP 服务 `http://127.0.0.1:8787`
- 前端页面 `http://127.0.0.1:8765/compress.html`（自动打开浏览器）

停止服务：`bash start.sh stop`

## 使用方式

### CLI 模式

```bash
# 从 stdin 读取，压缩后输出到 stdout
echo "要压缩的文本" | ./compressor/target/release/compressor --verbose

# 文件模式 + 指定压缩比
./compressor/target/release/compressor --ratio 0.3 < input.txt > output.txt

# 仅算法，不调模型（离线快速）
echo "测试" | ./compressor/target/release/compressor --no-model
```

### HTTP 服务模式

```bash
./compressor/target/release/compressor --serve --port 8787

curl -X POST http://127.0.0.1:8787/compress \
  -H 'Content-Type: application/json' \
  -d '{"text":"要压缩的内容","ratio":0.5}'
```

健康检查：`GET http://127.0.0.1:8787/health`

### 前端页面

浏览器打开 `compress.html`（或通过 `start.sh` 启动后访问 `http://127.0.0.1:8765/compress.html`），粘贴文本即可压缩，支持深色 / 浅色主题切换、实时字数与压缩比展示。

## 参数说明

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--serve` | false | 启动 HTTP 服务模式（默认走 stdin/stdout CLI） |
| `--port` | 8787 | HTTP 服务监听端口 |
| `--ratio` | 0.5 | 目标压缩比 (0.0-1.0)，0.5 = 压到原文一半 |
| `--no-model` | false | 只跑算法，不调 Ollama（离线 / 快速模式） |
| `--model` | qwen2.5:1.5b | Ollama 模型名（1.5b 质量更好，0.5b 更快） |
| `--verbose` | false | 打印每阶段字数与耗时到 stderr |

## 项目结构

```
ai-txt-compressor/
├── compress.html              # 前端单页应用（拆分 + 深度压缩）
├── index.html                 # 早期版本（纯文本拆分器）
├── start.sh                   # 一键启动后端 + 前端
├── compressor/
│   ├── install.sh             # 一键安装环境 + 编译
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs            # 入口 + CLI 参数定义
│       ├── cli.rs             # CLI 模式（stdin → stdout）
│       ├── server.rs          # HTTP 服务（axum + CORS）
│       ├── pipeline.rs        # 三段式混合管线编排
│       ├── prompt.rs          # Ollama 模型 prompt 模板
│       ├── algo/
│       │   ├── mod.rs         # 算法入口
│       │   ├── rules.rs       # 规则清洗（MD 符号 / 同义词 / 填充词）
│       │   ├── tokenize.rs    # 句子切分 + jieba 分词
│       │   ├── textrank.rs    # TextRank 句子重要度排序
│       │   └── stopwords.rs   # 中文停用词表
│       └── model/
│           └── ollama.rs      # Ollama HTTP 客户端
```

## 技术栈

- **后端**：Rust 2021 + axum 0.7 + tokio + reqwest
- **算法**：jieba-rs（中文分词）+ TextRank（抽取式摘要）+ regex（规则清洗）
- **模型**：Ollama 本地推理，默认 `qwen2.5:1.5b`
- **前端**：原生 HTML / CSS / JS，单文件，无构建步骤

## 开发

```bash
cd compressor
cargo build              # 调试构建
cargo build --release    # 发布构建
cargo run -- --verbose   # 直接运行
```

## License

MIT
