use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let package_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../npm/pruneguard");
    std::fs::create_dir_all(&package_root)?;

    let schemas: &[(&str, serde_json::Value)] = &[
        (
            "configuration_schema.json",
            serde_json::to_value(pruneguard_config::PruneguardConfig::json_schema())?,
        ),
        (
            "report_schema.json",
            serde_json::to_value(pruneguard_report::AnalysisReport::json_schema())?,
        ),
        (
            "review_report_schema.json",
            serde_json::to_value(pruneguard_report::ReviewReport::json_schema())?,
        ),
        (
            "safe_delete_report_schema.json",
            serde_json::to_value(pruneguard_report::SafeDeleteReport::json_schema())?,
        ),
        (
            "fix_plan_report_schema.json",
            serde_json::to_value(pruneguard_report::FixPlanReport::json_schema())?,
        ),
        (
            "suggest_rules_report_schema.json",
            serde_json::to_value(pruneguard_report::SuggestRulesReport::json_schema())?,
        ),
        (
            "daemon_status_report_schema.json",
            serde_json::to_value(pruneguard_report::DaemonStatusReport::json_schema())?,
        ),
    ];

    for (name, schema) in schemas {
        std::fs::write(package_root.join(name), serde_json::to_string_pretty(schema)?)?;
    }
    Ok(())
}
