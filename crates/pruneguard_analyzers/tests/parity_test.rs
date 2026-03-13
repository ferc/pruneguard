use pruneguard_analyzers::parity::{
    SupportLevel, compute_parity_stats, format_parity_table, parity_matrix,
};

#[test]
fn parity_matrix_is_complete() {
    let matrix = parity_matrix();
    assert!(!matrix.is_empty(), "parity matrix should not be empty");
    // Every feature should have a non-empty family and name.
    for feature in &matrix {
        assert!(!feature.family.is_empty());
        assert!(!feature.name.is_empty());
        assert!(!feature.reference_tool.is_empty());
    }
}

#[test]
fn parity_stats_are_reasonable() {
    let stats = compute_parity_stats();
    assert!(stats.total > 30, "expected at least 30 tracked features");
    assert!(
        stats.completion_pct > 70.0,
        "expected >70% completion, got {:.1}%",
        stats.completion_pct
    );
    eprintln!("\n{}", format_parity_table());
    eprintln!(
        "\nOverall parity: {:.1}% ({} supported, {} partial, {} unsupported of {})",
        stats.completion_pct, stats.supported, stats.partial, stats.unsupported, stats.total
    );
}

#[test]
fn no_unsupported_features_in_dynamic_patterns() {
    let matrix = parity_matrix();
    let unsupported: Vec<_> = matrix
        .iter()
        .filter(|f| f.family == "dynamic-patterns" && f.level == SupportLevel::Unsupported)
        .collect();
    assert!(
        unsupported.is_empty(),
        "dynamic-patterns should have no unsupported features: {:?}",
        unsupported.iter().map(|f| f.name).collect::<Vec<_>>()
    );
}
