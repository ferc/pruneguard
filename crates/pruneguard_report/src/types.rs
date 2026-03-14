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
    /// External parity corpus score, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parity_score: Option<ExternalParityReport>,
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
    /// Trust-related notes for this finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_notes: Option<Vec<String>>,
    /// Framework context relevant to this finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework_context: Option<Vec<String>>,
    /// Source of precision for this finding's evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision_source: Option<PrecisionSource>,
    /// Human-readable reason for the confidence level assigned to this finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_reason: Option<String>,
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
    /// An exported class member, enum variant, or namespace member is unused.
    UnusedMember,
    /// The same symbol is exported from multiple paths (barrel re-export collision).
    DuplicateExport,
    /// A dependency is imported in source code but not declared in package.json.
    UnlistedDependency,
    /// An exported type (interface or type alias) is unused.
    UnusedType,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FindingConfidence {
    #[default]
    High,
    Medium,
    Low,
}

/// Source of the precision for a finding's evidence.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PrecisionSource {
    /// Rust static analysis only.
    #[default]
    RustStatic,
    /// Derived from framework-generated source maps or .d.ts files.
    GeneratedMap,
    /// Derived from framework config file extraction.
    ConfigDerived,
    /// Refined by the semantic helper binary.
    SemanticHelper,
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
    /// Unique identifier for this action.
    pub id: String,
    /// The kind of remediation to perform.
    pub kind: RemediationActionKind,
    /// Files or exports this action targets.
    pub targets: Vec<String>,
    /// Human-readable rationale explaining why this action is needed.
    pub why: String,
    /// Conditions that must be true before this action can be applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<String>,
    /// Ordered steps to execute.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<RemediationStep>,
    /// Verification commands to run after applying the action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification: Vec<String>,
    /// Risk level of this action.
    pub risk: RiskLevel,
    /// Confidence in this action's correctness.
    pub confidence: FindingConfidence,
    /// Ranking position within the plan (1-based, lower = do first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<usize>,
    /// Phase this action belongs to (dead-code, architecture, governance).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// IDs of findings this action addresses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finding_ids: Vec<String>,
}

/// Fix plan report for agent-driven remediation workflows.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FixPlanReport {
    /// The query targets that were searched for.
    pub query: Vec<String>,
    /// Findings matched by the query.
    pub matched_findings: Vec<Finding>,
    /// Ordered remediation actions (ranked: high confidence first, low blast radius first,
    /// dead-code before architecture churn).
    pub actions: Vec<RemediationAction>,
    /// Reasons why some findings could not produce actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Top-level verification steps to run after the entire plan is applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification_steps: Vec<String>,
    /// Overall risk level (the maximum across all actions).
    pub risk_level: RiskLevel,
    /// Overall confidence (the minimum across all matched findings).
    pub confidence: FindingConfidence,
    /// Total number of actions in the plan.
    pub total_actions: usize,
    /// Number of actions that are high confidence.
    pub high_confidence_actions: usize,
    /// Summary of actions by phase.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phase_summary: Vec<FixPlanPhase>,
}

/// A phase in a fix plan, grouping related actions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FixPlanPhase {
    /// Phase name.
    pub name: String,
    /// Phase ordering number (lower = earlier).
    pub order: usize,
    /// Number of actions in this phase.
    pub action_count: usize,
    /// Description of what this phase addresses.
    pub description: String,
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
    /// Framework that contributed this entrypoint, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    /// Reason this entrypoint was detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Whether this entrypoint was detected via heuristics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heuristic: Option<bool>,
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
    /// Frameworks detected during analysis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frameworks_detected: Vec<String>,
    /// Frameworks detected via heuristics (lower confidence).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heuristic_frameworks: Vec<String>,
    /// Number of entrypoints added by heuristic detection.
    #[serde(default)]
    pub heuristic_entrypoints: usize,
    /// Compatibility warnings from framework detection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compatibility_warnings: Vec<String>,
    /// Whether strict trust mode was applied.
    #[serde(default)]
    pub strict_trust_applied: bool,
    /// Framework confidence breakdown.
    #[serde(default)]
    pub framework_confidence_counts: FrameworkConfidenceCounts,
    /// Names of unsupported frameworks detected in the workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_frameworks: Vec<String>,
    /// External parity score percentage (0-100), if computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_parity_pct: Option<f64>,
    /// External parity score summary, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_parity: Option<ExternalParitySummary>,
    /// Semantic helper mode used for this analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_mode: Option<String>,
    /// Whether the semantic helper was actually invoked.
    #[serde(default)]
    pub semantic_used: bool,
    /// Wall-clock milliseconds spent in the semantic helper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_wall_ms: Option<u64>,
    /// Number of TypeScript projects loaded by the semantic helper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_projects: Option<usize>,
    /// Number of files sent to the semantic helper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_files: Option<usize>,
    /// Number of queries sent to the semantic helper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_queries: Option<usize>,
    /// Reason the semantic helper was skipped, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_skipped_reason: Option<String>,
    /// Weighted replacement score (0-100) against the external parity corpus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_score: Option<f64>,
    /// Per-family replacement scores.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replacement_family_scores: Vec<ReplacementFamilyScore>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfidenceCounts {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

/// Breakdown of framework detection confidence levels.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkConfidenceCounts {
    pub exact: usize,
    pub heuristic: usize,
    pub unsupported: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedByReasonStats {
    pub missing_file: usize,
    pub unsupported_specifier: usize,
    pub tsconfig_path_miss: usize,
    pub exports_condition_miss: usize,
    pub externalized: usize,
    /// Subpath not declared in a workspace package's `exports` map.
    #[serde(default)]
    pub workspace_exports_miss: usize,
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
    /// Machine-readable recommended next actions for agents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommended_actions: Vec<RecommendedAction>,
    /// Proposed remediation actions for blocking findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposed_actions: Vec<RemediationAction>,
    /// Execution mode used for this review.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
    /// Wall-clock latency in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Compatibility warnings from framework detection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compatibility_warnings: Vec<String>,
    /// Whether strict trust mode was applied.
    #[serde(default)]
    pub strict_trust_applied: bool,
}

/// A machine-readable recommended next action for an AI agent or CI system.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedAction {
    /// Machine-readable action kind.
    pub kind: RecommendedActionKind,
    /// Human-readable description of what to do.
    pub description: String,
    /// Priority rank (1 = most important).
    pub priority: usize,
    /// The pruneguard command to run, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Targets this action applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
}

/// Kind of recommended next action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RecommendedActionKind {
    /// Run safe-delete on identified targets.
    RunSafeDelete,
    /// Run fix-plan for specific findings.
    RunFixPlan,
    /// Resolve blocking findings before merge.
    ResolveBlocking,
    /// Investigate unresolved specifier pressure.
    FixResolverConfig,
    /// Review advisory findings.
    ReviewAdvisory,
    /// Run a full-scope scan for higher confidence.
    RunFullScope,
    /// Branch is clean; no action required.
    None,
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
    /// Execution mode used for this analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
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
    /// Recommended deletion order (safe targets only, dependency-aware).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deletion_order: Vec<DeletionOrderEntry>,
    /// Supporting evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

/// An entry in the dependency-aware deletion order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeletionOrderEntry {
    /// The target to delete.
    pub target: String,
    /// Position in the deletion sequence (1-based).
    pub step: usize,
    /// Why this target is at this position in the order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Explicit classification for a safe-delete candidate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SafeDeleteClassification {
    /// Target is safe to delete without further review.
    Safe,
    /// Target needs manual review before deletion.
    NeedsReview,
    /// Target must not be deleted.
    Blocked,
}

/// A candidate in a safe-delete evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SafeDeleteCandidate {
    /// The target file or export.
    pub target: String,
    /// Explicit classification for this candidate.
    pub classification: SafeDeleteClassification,
    /// Confidence in the safety assessment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<FindingConfidence>,
    /// Reasons for the classification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// Per-candidate evidence supporting the classification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
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
    /// Recommended governance actions ordered by priority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub governance_actions: Vec<GovernanceAction>,
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
    /// Rationale explaining why this specific rule was suggested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// Kind of suggested rule.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SuggestedRuleKind {
    Forbidden,
    Required,
    TagAssignment,
    OwnershipBoundary,
    ReachabilityFence,
    LayerEnforcement,
}

/// A suggested tag for directory grouping.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedTag {
    /// Tag name.
    pub name: String,
    /// Glob pattern for the tag.
    pub glob: String,
    /// Source of the tag inference (e.g. "directory-cluster", "workspace", "package", "ownership").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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
    /// Packages touched by cross-team edges from this area.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched_packages: Vec<String>,
    /// Rationale for the suggestion.
    pub rationale: String,
}

/// A hotspot file with high edge traffic.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hotspot {
    /// File path.
    pub file: String,
    /// Workspace containing this file, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Package containing this file, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Cross-package import count.
    pub cross_package_imports: usize,
    /// Cross-owner import count.
    pub cross_owner_imports: usize,
    /// Incoming edge count.
    pub incoming_edges: usize,
    /// Outgoing edge count.
    pub outgoing_edges: usize,
    /// Hotspot rank (1 = highest traffic).
    pub rank: usize,
    /// Distinct teams that touch this file via imports (source or target).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams_involved: Vec<String>,
    /// Suggestion for addressing this hotspot.
    pub suggestion: String,
}

/// A recommended governance action with priority ordering.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceAction {
    /// Priority rank (1 = most impactful).
    pub priority: usize,
    /// Kind of governance action.
    pub kind: GovernanceActionKind,
    /// Human-readable description of what to do.
    pub description: String,
    /// Estimated effort level.
    pub effort: EffortLevel,
    /// Expected impact on governance coverage.
    pub impact: ImpactLevel,
    /// Concrete configuration fragment or steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_fragment: Option<serde_json::Value>,
}

/// Kind of governance action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GovernanceActionKind {
    AddBoundaryRule,
    AssignOwnership,
    IntroduceTags,
    SplitHotspot,
    AddReachabilityFence,
    EnforceLayering,
}

/// Estimated effort level.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    Low,
    Medium,
    High,
}

/// Expected impact level.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ImpactLevel {
    Low,
    Medium,
    High,
}

impl SuggestRulesReport {
    /// Generate the JSON Schema for the suggest-rules report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}

/// External parity report from the fixture-derived corpus.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalParityReport {
    /// Total number of parity test cases.
    pub total_cases: usize,
    /// Number of fully passing cases.
    pub passed_cases: usize,
    /// Total number of individual checks across all cases.
    pub total_checks: usize,
    /// Number of individual checks that passed.
    pub passed_checks: usize,
    /// Overall parity percentage (0-100).
    pub overall_pct: f64,
    /// Per-family parity breakdown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_family: Vec<ExternalParityFamilyScore>,
    /// Per-reference-tool parity breakdown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_reference_tool: Vec<ExternalParityToolScore>,
    /// Individual case results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_results: Vec<ExternalParityCaseResult>,
    /// Stale matrix deltas (features where the hand-authored tracker disagrees with reality).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale_deltas: Vec<ParityStaleDelta>,
}

/// A stale delta where the hand-authored parity matrix disagrees with actual test results.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ParityStaleDelta {
    /// Feature or test case name.
    pub feature: String,
    /// Level claimed in the parity matrix.
    pub matrix_level: String,
    /// Actual level observed from test results.
    pub actual_level: String,
}

/// Per-family score in the external parity report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalParityFamilyScore {
    pub family: String,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub pct: f64,
}

/// Per-tool score in the external parity report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalParityToolScore {
    pub tool: String,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub pct: f64,
}

/// Result of a single parity case in the external report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalParityCaseResult {
    pub family: String,
    pub name: String,
    pub reference_tool: String,
    pub passed: bool,
    pub total_checks: usize,
    pub passed_checks: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

/// Summary of external parity score for the stats section.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExternalParitySummary {
    /// Overall parity percentage (0-100).
    pub overall_pct: f64,
    /// Total cases evaluated.
    pub total_cases: usize,
    /// Cases that fully passed.
    pub passed_cases: usize,
    /// Total individual checks.
    pub total_checks: usize,
    /// Individual checks that passed.
    pub passed_checks: usize,
}

/// Per-family replacement score for the weighted replacement metric.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementFamilyScore {
    /// Family name (e.g. "vite", "next", "jest").
    pub family: String,
    /// Replacement score for this family (0-100).
    pub score: f64,
    /// Tier of this family (1 or 2).
    pub tier: u8,
    /// Total test cases in this family.
    pub total_cases: usize,
    /// Passing test cases in this family.
    pub passed_cases: usize,
}

/// Daemon status report for querying daemon health from JS or CLI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatusReport {
    /// Whether a daemon process is currently running.
    pub running: bool,
    /// Process ID of the running daemon, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// TCP port the daemon is listening on, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Version of the running daemon binary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// ISO-8601 timestamp when the daemon started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Absolute path to the project root the daemon is serving.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    /// Whether the hot index has been warmed (initial build complete).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_warm: Option<bool>,
    /// Milliseconds since the last graph update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update_ms: Option<u64>,
    /// Number of nodes in the module graph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_nodes: Option<usize>,
    /// Number of edges in the module graph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_edges: Option<usize>,
    /// Number of files being watched for changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched_files: Option<usize>,
    /// Current generation (rebuild counter) of the index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// Milliseconds of watcher lag (time since last fs event was processed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher_lag_ms: Option<u64>,
    /// Number of files pending invalidation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_invalidations: Option<usize>,
    /// Uptime of the daemon in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    /// Absolute path to the daemon binary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    /// Milliseconds the initial graph build took.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_build_ms: Option<u64>,
    /// Milliseconds the last incremental rebuild took.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_rebuild_ms: Option<u64>,
    /// Number of incremental rebuilds since daemon start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental_rebuilds: Option<u64>,
    /// Total number of files invalidated since daemon start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_invalidations: Option<u64>,
    /// Whether a config-level change is pending that requires full rebuild.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_change_pending: Option<bool>,
}

impl DaemonStatusReport {
    /// Generate the JSON Schema for the daemon status report format.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}
