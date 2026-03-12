use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Full analysis report from a scan.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisReport {
    /// Schema version for the report format.
    pub version: u32,
    /// Version of the oxgraph tool that produced this report.
    pub tool_version: String,
    /// Working directory the analysis was run from.
    pub cwd: String,
    /// The profile used for this analysis.
    pub profile: String,
    /// Summary counts.
    pub summary: Summary,
    /// Inventories of discovered entities.
    pub inventories: Inventories,
    /// Detected findings (violations, unused items, etc.).
    pub findings: Vec<Finding>,
    /// All detected entrypoints.
    pub entrypoints: Vec<EntrypointInfo>,
    /// Performance statistics.
    pub stats: Stats,
}

/// Summary counts for a report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub total_files: usize,
    pub total_packages: usize,
    pub total_workspaces: usize,
    pub total_exports: usize,
    pub total_findings: usize,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

/// Inventories of discovered entities.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Inventories {
    pub files: Vec<FileInfo>,
    pub packages: Vec<PackageInfo>,
    pub workspaces: Vec<WorkspaceInfo>,
}

/// A single finding from analysis.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable deterministic ID for this finding.
    pub id: String,
    /// Machine-readable code (e.g. `unused-export`, `cycle`, `boundary-violation`).
    pub code: String,
    /// Severity level.
    pub severity: FindingSeverity,
    /// Category of the finding.
    pub category: FindingCategory,
    /// The subject of the finding (file path, export name, etc.).
    pub subject: String,
    /// Workspace this finding belongs to, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Package this finding belongs to, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Human-readable message.
    pub message: String,
    /// Evidence supporting the finding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Suggested fix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// Name of the rule that produced this finding, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    Error,
    Warn,
    Info,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FindingCategory {
    UnusedExport,
    UnusedFile,
    UnusedPackage,
    UnusedDependency,
    Cycle,
    BoundaryViolation,
    OwnershipViolation,
    Impact,
}

/// Evidence supporting a finding.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// Type of evidence.
    pub kind: String,
    /// File path involved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Line number, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Description of this evidence.
    pub description: String,
}

/// Impact report for blast-radius analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImpactReport {
    /// The target that was analyzed.
    pub target: String,
    /// Entrypoints affected by changes to the target.
    pub affected_entrypoints: Vec<String>,
    /// Packages affected.
    pub affected_packages: Vec<String>,
    /// Files affected.
    pub affected_files: Vec<String>,
    /// Evidence chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

/// Explain report for proof output.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplainReport {
    /// The query that was explained.
    pub query: String,
    /// The matched graph node, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_node: Option<String>,
    /// Proof trees showing why-used or why-unused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proofs: Vec<ProofNode>,
    /// Related findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_findings: Vec<Finding>,
}

/// A node in a proof tree.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProofNode {
    /// The node in the graph.
    pub node: String,
    /// Relationship to the parent.
    pub relationship: String,
    /// Children in the proof tree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Self>,
}

/// Info about a discovered file.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub workspace: Option<String>,
    pub kind: FileKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<FileRole>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Source,
    Test,
    Story,
    Config,
    Generated,
    BuildOutput,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum FileRole {
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "test")]
    Test,
    #[serde(rename = "story")]
    Story,
    #[serde(rename = "fixture")]
    Fixture,
    #[serde(rename = "example")]
    Example,
    #[serde(rename = "template")]
    Template,
    #[serde(rename = "benchmark")]
    Benchmark,
    #[serde(rename = "config")]
    Config,
    #[serde(rename = "generated")]
    Generated,
    #[serde(rename = "buildOutput")]
    BuildOutput,
}

/// Info about a discovered package.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub name: String,
    pub version: Option<String>,
    pub workspace: String,
    pub path: String,
}

/// Info about a discovered workspace.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub name: String,
    pub path: String,
    pub package_count: usize,
}

/// Info about an entrypoint.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EntrypointInfo {
    pub path: String,
    pub kind: String,
    pub profile: String,
    pub workspace: Option<String>,
    pub source: String,
}

/// Performance statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub duration_ms: u64,
    pub files_parsed: usize,
    pub files_cached: usize,
    pub files_discovered: usize,
    pub files_resolved: usize,
    pub unresolved_specifiers: usize,
    pub entrypoints_detected: usize,
    pub graph_nodes: usize,
    pub graph_edges: usize,
    pub changed_files: usize,
    pub affected_files: usize,
    pub affected_packages: usize,
    pub affected_entrypoints: usize,
    pub baseline_applied: bool,
    pub baseline_profile_mismatch: bool,
    pub suppressed_findings: usize,
    pub new_findings: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub cache_entries_read: usize,
    pub cache_entries_written: usize,
    pub affected_scope_incomplete: bool,
}

impl AnalysisReport {
    /// Generate the JSON Schema for the report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}
