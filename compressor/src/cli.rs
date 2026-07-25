//! CLI 模式：stdin → 压缩 → stdout

use std::io::{Read, Write};

use crate::pipeline::{compress, CompressOptions};

pub async fn run(args: crate::Cli) -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;

    if input.trim().is_empty() {
        eprintln!("error: empty input");
        std::process::exit(1);
    }

    let opts = CompressOptions {
        ratio: args.ratio,
        no_model: args.no_model,
        provider: args.provider.clone(),
        model: args.model.clone(),
        api_key: if args.api_key.is_empty() {
            std::env::var("DEEPSEEK_API_KEY").ok().filter(|s| !s.is_empty())
        } else {
            Some(args.api_key.clone())
        },
        base_url: None,
        reasoning_effort: None,
        custom_system: None,
        custom_user_template: None,
        verbose: args.verbose,
        preset: None,
        text_algo: None,
        target_chars_override: None,
        target_chars: None,
    };

    let result = compress(&input, &opts).await?;

    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(result.text.as_bytes())?;
    if !result.text.ends_with('\n') {
        lock.write_all(b"\n")?;
    }
    lock.flush()?;

    if args.verbose {
        eprintln!("---");
        eprintln!("original  : {} chars", result.original);
        eprintln!("compressed: {} chars", result.compressed);
        eprintln!("ratio     : {:.1}%", result.ratio * 100.0);
        eprintln!(
            "stages    : algo={}, model={}",
            result.stages.after_algo, result.stages.after_model
        );
    }

    Ok(())
}
