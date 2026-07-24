//! 模型 prompt 模板

pub fn build_compress_prompt(text: &str, target_chars: usize) -> String {
    let original_chars = text.chars().count();
    format!(
        "任务：把下面这段{orig}字的文字压缩到不超过{target}字。\n\
         \n\
         要求：\n\
         1. 必须大幅压缩，目标长度不超过{target}字（当前{orig}字，需压缩{cut}%）\n\
         2. 保留所有关键信息、数据、专有名词和结论\n\
         3. 删除所有示例、铺垫、重复、过渡句\n\
         4. 合并相似内容，用更精炼的表达\n\
         5. 不添加任何新内容，不改变原意\n\
         6. 只输出压缩后的文字，不要解释、不要前后缀、不要引号\n\
         \n\
         原文：\n\
         {text}",
        orig = original_chars,
        target = target_chars,
        cut = ((1.0 - target_chars as f32 / original_chars as f32) * 100.0) as u32,
        text = text
    )
}

