use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let package_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../npm/pruneguard");
    std::fs::create_dir_all(&package_root)?;

    let configuration_schema = pruneguard_config::PruneguardConfig::json_schema();
    let report_schema = pruneguard_report::AnalysisReport::json_schema();

    std::fs::write(
        package_root.join("configuration_schema.json"),
        serde_json::to_string_pretty(&configuration_schema)?,
    )?;
    std::fs::write(
        package_root.join("report_schema.json"),
        serde_json::to_string_pretty(&report_schema)?,
    )?;
    Ok(())
}
