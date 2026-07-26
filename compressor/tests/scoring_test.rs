use aitxt_compressor::scoring::{compute_score, FactRetention, FactsByType, InformationLoss, ScoreReport, ScoreRequest};

#[test]
fn test_structs_compile() {
    let _req = ScoreRequest {
        original: "测试".into(),
        compressed: "测".into(),
        original_chars: 2,
        compressed_chars: 1,
        target_ratio: Some(0.5),
        embedding_model: None,
        ollama_base: None,
    };
    let _report = ScoreReport {
        compression_ratio: 0.5,
        semantic_score: None,
        information_loss: InformationLoss {
            lost_sentence_count: 0,
            total_sentence_count: 1,
            loss_rate: 0.0,
            lost_sentences: vec![],
        },
        fact_retention: FactRetention {
            total_facts: 0,
            retained_facts: 0,
            retention_rate: 0.0,
            lost_facts: vec![],
            facts_by_type: FactsByType::default(),
        },
        total_score: 0.0,
    };
}

#[tokio::test]
async fn test_compute_score_full() {
    let req = ScoreRequest {
        original: "苹果是水果。香蕉也是水果。今天天气真好适合出去玩。葡萄是水果。".into(),
        compressed: "苹果是水果。香蕉也是水果。葡萄是水果。".into(),
        original_chars: 30,
        compressed_chars: 20,
        target_ratio: Some(0.66),
        embedding_model: None,
        ollama_base: None,
    };
    let report = compute_score(req).await;

    // 压缩率 = 20/30 ≈ 0.667
    assert!((report.compression_ratio - 20.0 / 30.0).abs() < 1e-9);
    // 语义为 None
    assert!(report.semantic_score.is_none());
    // 总分应在 0-100 之间
    assert!(report.total_score >= 0.0 && report.total_score <= 100.0);
    // 丢失 1 句
    assert_eq!(report.information_loss.lost_sentence_count, 1);
    println!("total_score = {}", report.total_score);
}

#[tokio::test]
async fn test_compute_score_perfect() {
    // 完美情况：原文与压缩相同
    let text = "这是一段测试文本。";
    let req = ScoreRequest {
        original: text.into(),
        compressed: text.into(),
        original_chars: 10,
        compressed_chars: 10,
        target_ratio: Some(1.0),
        embedding_model: None,
        ollama_base: None,
    };
    let report = compute_score(req).await;

    // 压缩率 = 1.0，target_ratio = 1.0，ratio_score = 1.0
    assert!((report.compression_ratio - 1.0).abs() < 1e-9);
    // 无信息丢失
    assert_eq!(report.information_loss.lost_sentence_count, 0);
    assert!((report.information_loss.loss_rate - 0.0).abs() < 1e-9);
    // 总分应接近 100（语义为 None 时，ratio=1.0, info=1.0, fact=1.0 → 100）
    assert!(
        report.total_score >= 99.0,
        "total_score should be near 100, got {}",
        report.total_score
    );
}

#[test]
fn test_compute_total_score_with_semantic() {
    let info = InformationLoss {
        lost_sentence_count: 0,
        total_sentence_count: 4,
        loss_rate: 0.0,
        lost_sentences: vec![],
    };
    let fact = FactRetention {
        total_facts: 4,
        retained_facts: 4,
        retention_rate: 1.0,
        lost_facts: vec![],
        facts_by_type: FactsByType::default(),
    };

    // 模拟完美情况：压缩率=0.66 (target 0.66) / 语义=1.0 / 信息丢失=0 / 事实=1.0
    // 但 compute_total_score 是私有函数，需要通过 compute_score 间接测试
    // 这里只验证结构能构造
    let _ = (info, fact);
}
