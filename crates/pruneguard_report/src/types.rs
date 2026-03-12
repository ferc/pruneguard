use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Full analysis report from a scan.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisReport {
    /// Schema version for the report format.
    pub version: u32,
    /// Version of the pruneguard tool that produced this report.
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
    /// Confidence level for this finding.
    pub confidence: FindingConfidence,
    /// Primary remediation action kind for this finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_action_kind: Option<RemediationActionKind>,
    /// All applicable remediation action kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_kinds: Vec<RemediationActionKind>,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FindingConfidence {
    #[default]
    High,
    Medium,
    Low,
}

/// Risk level for a remediation action or fix plan.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
}

/// The kind of remediation action to take.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RemediationActionKind {
    DeleteFile,
    DeleteExport,
    RemoveDependency,
    BreakCycle,
    MoveImport,
    TightenEntrypoint,
    UpdateBoundaryRule,
    AssignOwner,
    SplitPackage,
    AcknowledgeBaseline,
}

/// A single step in a remediation action.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemediationStep {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// A remediation action describing how to fix one or more findings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemediationAction {
    pub id: String,
    pub kind: RemediationActionKind,
    pub targets: Vec<String>,
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<RemediationStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification: Vec<String>,
    pub risk: RiskLevel,
    pub confidence: FindingConfidence,
}

/// Fix plan report for agent-driven remediation workflows.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FixPlanReport {
    pub query: Vec<String>,
    pub matched_findings: Vec<Finding>,
    pub actions: Vec<RemediationAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification_steps: Vec<String>,
    pub risk_level: RiskLevel,
    pub confidence: FindingConfidence,
}

/// Execution mode for daemon/oneshot distinction.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    Oneshot,
    Daemon,
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
    /// Whether focus filtering removed any affected nodes or proof edges.
    #[serde(default)]
    pub focus_filtered: bool,
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
    /// How the query was interpreted.
    pub query_kind: ExplainQueryKind,
    /// Proof trees showing why-used or why-unused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proofs: Vec<ProofNode>,
    /// Related findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_findings: Vec<Finding>,
    /// Whether focus filtering removed any related findings or proof edges.
    #[serde(default)]
    pub focus_filtered: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExplainQueryKind {
    Finding,
    #[default]
    File,
    Export,
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
#[allow(clippy::struct_excessive_bools)]
pub struct Stats {
    pub duration_ms: u64,
    pub files_parsed: usize,
    pub files_cached: usize,
    pub files_discovered: usize,
    pub files_resolved: usize,
    pub unresolved_specifiers: usize,
    pub unresolved_by_reason: UnresolvedByReasonStats,
    pub resolved_via_exports: usize,
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
    pub focus_applied: bool,
    pub focused_files: usize,
    pub focused_findings: usize,
    pub full_scope_required: bool,
    pub partial_scope: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_scope_reason: Option<String>,
    pub confidence_counts: ConfidenceCounts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parity_warnings: Vec<String>,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub cache_entries_read: usize,
    pub cache_entries_written: usize,
    pub affected_scope_incomplete: bool,
    /// Execution mode used for this analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
    /// Whether the graph index was warm (reused from a previous run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_warm: Option<bool>,
    /// Age of the reused graph index in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_age_ms: Option<u64>,
    /// Number of graph nodes reused from a warm index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused_graph_nodes: Option<usize>,
    /// Number of graph edges reused from a warm index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused_graph_edges: Option<usize>,
    /// Lag of the file-system watcher in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher_lag_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfidenceCounts {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedByReasonStats {
    pub missing_file: usize,
    pub unsupported_specifier: usize,
    pub tsconfig_path_miss: usize,
    pub exports_condition_miss: usize,
    pub externalized: usize,
}

/// Branch review report for CI/agent branch gating.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReport {
    /// Base ref used for comparison.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Files changed on this branch.
    pub changed_files: Vec<String>,
    /// All new findings introduced on this branch.
    pub new_findings: Vec<Finding>,
    /// Findings that should block merge (high confidence errors/warnings).
    pub blocking_findings: Vec<Finding>,
    /// Findings that are advisory (medium/low confidence, or info severity).
    pub advisory_findings: Vec<Finding>,
    /// Trust summary for this review.
    pub trust: ReviewTrust,
    /// Concise recommendations for the branch author.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommendations: Vec<String>,
    /// Proposed remediation actions for blocking findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposed_actions: Vec<RemediationAction>,
    /// Execution mode used for this review.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
    /// Wall-clock latency in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

/// Trust summary within a review report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTrust {
    /// Whether full-scope analysis was performed.
    pub full_scope: bool,
    /// Whether a baseline was applied.
    pub baseline_applied: bool,
    /// Unresolved specifier pressure ratio.
    pub unresolved_pressure: f64,
    /// Confidence counts for new findings.
    pub confidence_counts: ConfidenceCounts,
}

/// Safe-delete report for deletion approval workflows.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SafeDeleteReport {
    /// Targets that were evaluated.
    pub targets: Vec<String>,
    /// Targets safe to delete.
    pub safe: Vec<SafeDeleteCandidate>,
    /// Targets that need manual review before deletion.
    pub needs_review: Vec<SafeDeleteCandidate>,
    /// Targets that should not be deleted.
    pub blocked: Vec<SafeDeleteCandidate>,
    /// Recommended deletion order (safe targets only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deletion_order: Vec<String>,
    /// Supporting evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

/// A candidate in a safe-delete evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SafeDeleteCandidate {
    /// The target file or export.
    pub target: String,
    /// Confidence in the safety assessment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<FindingConfidence>,
    /// Reasons for the classification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

impl AnalysisReport {
    /// Generate the JSON Schema for the report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}

impl ReviewReport {
    /// Generate the JSON Schema for the review report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}

impl SafeDeleteReport {
    /// Generate the JSON Schema for the safe-delete report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}

impl FixPlanReport {
    /// Generate the JSON Schema for the fix-plan report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}

/// Suggested governance rules report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestRulesReport {
    /// Suggested rules inferred from graph analysis.
    pub suggested_rules: Vec<SuggestedRule>,
    /// Suggested tags for directory grouping.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<SuggestedTag>,
    /// Ownership hints from cross-boundary analysis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ownership_hints: Vec<OwnershipHint>,
    /// Hotspot files with high edge traffic.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hotspots: Vec<Hotspot>,
    /// Rationale explaining the suggestions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rationale: Vec<String>,
}

/// A single suggested rule.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedRule {
    /// Kind of rule suggestion.
    pub kind: SuggestedRuleKind,
    /// Rule name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Configuration fragment that implements this rule.
    pub config_fragment: serde_json::Value,
    /// Confidence in the suggestion.
    pub confidence: FindingConfidence,
    /// Evidence supporting the suggestion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// Kind of suggested rule.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SuggestedRuleKind {
    Forbidden,
    Required,
    TagAssignment,
    OwnershipBoundary,
}

/// A suggested tag for directory grouping.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedTag {
    /// Tag name.
    pub name: String,
    /// Glob pattern for the tag.
    pub glob: String,
    /// Why this tag is suggested.
    pub rationale: String,
}

/// Ownership assignment hint.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipHint {
    /// Path glob to match.
    pub path_glob: String,
    /// Suggested owner identifier.
    pub suggested_owner: String,
    /// Number of cross-team edges.
    pub cross_team_edges: usize,
    /// Rationale for the suggestion.
    pub rationale: String,
}

/// A hotspot file with high edge traffic.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hotspot {
    /// File path.
    pub file: String,
    /// Cross-package import count.
    pub cross_package_imports: usize,
    /// Cross-owner import count.
    pub cross_owner_imports: usize,
    /// Incoming edge count.
    pub incoming_edges: usize,
    /// Outgoing edge count.
    pub outgoing_edges: usize,
    /// Suggestion for addressing this hotspot.
    pub suggestion: String,
}

impl SuggestRulesReport {
    /// Generate the JSON Schema for the suggest-rules report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}
