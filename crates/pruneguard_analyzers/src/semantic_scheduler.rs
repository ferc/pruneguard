//! Hybrid auto mode scheduler for the semantic precision layer.
//!
//! This module decides whether to invoke the `pruneguard-tsgo` helper
//! based on uncertainty scoring of candidate findings. The helper is
//! only triggered when ALL of these conditions are met:
//!
//! 1. The repo contains TS/TSX files and a resolvable tsconfig
//! 2. Candidate findings are in semantic-sensitive categories
//! 3. The candidate batch size is below configured thresholds
//! 4. The predicted overhead fits within the configured budget
//!
//! In one-shot mode, the Rust scan runs first, then the helper refines
//! the narrowed candidate slice. In daemon mode, Rust results are returned
//! immediately and helper refinement runs in the background.

use std::path::{Path, PathBuf};

use pruneguard_config::SemanticConfig;
use pruneguard_report::Finding;

/// Result of evaluating whether the semantic helper should be invoked.
#[derive(Debug, Clone)]
pub enum SemanticDecision {
    /// The helper should be invoked with these candidates.
    Invoke {
        /// Findings that should be refined by the helper.
        candidates: Vec<SemanticCandidate>,
        /// Tsconfig paths to load.
        tsconfig_paths: Vec<String>,
        /// Estimated overhead in milliseconds.
        estimated_overhead_ms: u64,
    },
    /// The helper should be skipped.
    Skip {
        /// Reason the helper was skipped.
        reason: SkipReason,
    },
}

/// A finding candidate for semantic refinement.
#[derive(Debug, Clone)]
pub struct SemanticCandidate {
    /// Index of the finding in the original findings list.
    pub finding_index: usize,
    /// The finding to refine.
    pub finding: Finding,
    /// Uncertainty score (0-100). Higher = more uncertain = more value from refinement.
    pub uncertainty_score: u8,
    /// File path of the export/member being checked.
    pub file_path: PathBuf,
    /// Export name, if applicable.
    pub export_name: Option<String>,
    /// Parent name, if applicable (for member findings).
    pub parent_name: Option<String>,
    /// Member name, if applicable.
    pub member_name: Option<String>,
}

/// Reason the semantic helper was skipped.
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// Semantic mode is set to "off".
    ModeOff,
    /// No TypeScript/TSX files found in the project.
    NoTypeScriptFiles,
    /// No tsconfig.json found.
    NoTsconfig,
    /// No findings are in semantic-sensitive categories.
    NoCandidates,
    /// Candidate batch exceeds the configured file limit.
    BatchTooLarge { count: usize, limit: usize },
    /// Predicted overhead exceeds the configured budget.
    OverheadExceeded { predicted_ms: u64, budget_ms: u64 },
    /// Project reference count exceeds the configured limit.
    TooManyProjects { count: usize, limit: usize },
    /// Helper binary not found.
    BinaryNotFound(String),
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModeOff => write!(f, "semantic mode is off"),
            Self::NoTypeScriptFiles => write!(f, "no TypeScript files in project"),
            Self::NoTsconfig => write!(f, "no tsconfig.json found"),
            Self::NoCandidates => write!(f, "no semantic-sensitive findings to refine"),
            Self::BatchTooLarge { count, limit } => {
                write!(f, "candidate batch ({count} files) exceeds limit ({limit})")
            }
            Self::OverheadExceeded { predicted_ms, budget_ms } => {
                write!(f, "predicted overhead ({predicted_ms}ms) exceeds budget ({budget_ms}ms)")
            }
            Self::TooManyProjects { count, limit } => {
                write!(f, "project count ({count}) exceeds limit ({limit})")
            }
            Self::BinaryNotFound(reason) => write!(f, "helper binary not found: {reason}"),
        }
    }
}

/// Categories of findings that benefit from semantic refinement.
const SEMANTIC_SENSITIVE_CODES: &[&str] = &["unused-export", "unused-member", "duplicate-export"];

/// Evaluate whether the semantic helper should be invoked.
pub fn evaluate_semantic_decision(
    config: &SemanticConfig,
    findings: &[Finding],
    project_root: &Path,
    scan_duration_ms: u64,
) -> SemanticDecision {
    use pruneguard_config::SemanticMode;

    // Check mode
    if config.mode == SemanticMode::Off {
        return SemanticDecision::Skip { reason: SkipReason::ModeOff };
    }

    // Check for TypeScript files
    let has_ts_files = has_typescript_files(project_root);
    if !has_ts_files {
        return SemanticDecision::Skip { reason: SkipReason::NoTypeScriptFiles };
    }

    // Find tsconfig files
    let tsconfig_paths = find_tsconfig_paths(project_root);
    if tsconfig_paths.is_empty() {
        return SemanticDecision::Skip { reason: SkipReason::NoTsconfig };
    }

    // Check project reference count
    let project_count = tsconfig_paths.len();
    if project_count > config.max_project_refs {
        return SemanticDecision::Skip {
            reason: SkipReason::TooManyProjects {
                count: project_count,
                limit: config.max_project_refs,
            },
        };
    }

    // Score findings for semantic sensitivity
    let candidates: Vec<SemanticCandidate> = findings
        .iter()
        .enumerate()
        .filter(|(_, f)| is_semantic_sensitive(f))
        .filter_map(|(idx, f)| score_candidate(idx, f, config.min_uncertainty_score))
        .collect();

    if candidates.is_empty() {
        return SemanticDecision::Skip { reason: SkipReason::NoCandidates };
    }

    // Check batch size
    let unique_files: std::collections::HashSet<&Path> =
        candidates.iter().map(|c| c.file_path.as_path()).collect();
    if unique_files.len() > config.max_files_per_query_batch {
        return SemanticDecision::Skip {
            reason: SkipReason::BatchTooLarge {
                count: unique_files.len(),
                limit: config.max_files_per_query_batch,
            },
        };
    }

    // Estimate overhead
    let estimated_overhead_ms = estimate_overhead(candidates.len(), project_count);
    let budget_ms = u64::from(config.max_cold_overhead_pct) * scan_duration_ms / 100;
    let effective_budget = budget_ms.max(config.max_helper_wall_ms);

    if config.mode == SemanticMode::Auto && estimated_overhead_ms > effective_budget {
        return SemanticDecision::Skip {
            reason: SkipReason::OverheadExceeded {
                predicted_ms: estimated_overhead_ms,
                budget_ms: effective_budget,
            },
        };
    }

    SemanticDecision::Invoke { candidates, tsconfig_paths, estimated_overhead_ms }
}

/// Check if a finding is in a semantic-sensitive category.
fn is_semantic_sensitive(finding: &Finding) -> bool {
    SEMANTIC_SENSITIVE_CODES.iter().any(|code| finding.code == *code)
}

/// Score a candidate finding for uncertainty. Returns `Some` if the score
/// meets the minimum threshold.
fn score_candidate(index: usize, finding: &Finding, min_score: u8) -> Option<SemanticCandidate> {
    let mut score: u8 = 0;

    // Medium confidence findings benefit most from refinement
    if finding.confidence == pruneguard_report::FindingConfidence::Medium {
        score += 40;
    } else if finding.confidence == pruneguard_report::FindingConfidence::Low {
        score += 30;
    }

    // Unused exports in files with many exports are more uncertain
    if finding.code == "unused-export" {
        score += 20;
    }

    // Unused members are inherently uncertain without type info
    if finding.code == "unused-member" {
        score += 30;
    }

    // Findings with framework context may be false positives
    if finding.framework_context.is_some() {
        score += 10;
    }

    if score < min_score {
        return None;
    }

    // Extract file path and names from the finding subject
    let file_path = PathBuf::from(&finding.subject);
    let export_name =
        finding.evidence.iter().find(|e| e.kind == "export-name").map(|e| e.description.clone());
    let parent_name =
        finding.evidence.iter().find(|e| e.kind == "parent-name").map(|e| e.description.clone());
    let member_name =
        finding.evidence.iter().find(|e| e.kind == "member-name").map(|e| e.description.clone());

    Some(SemanticCandidate {
        finding_index: index,
        finding: finding.clone(),
        uncertainty_score: score,
        file_path,
        export_name,
        parent_name,
        member_name,
    })
}

/// Check if the project contains TypeScript files.
fn has_typescript_files(project_root: &Path) -> bool {
    // Quick heuristic: check for tsconfig.json or common TS entry files
    if project_root.join("tsconfig.json").exists() {
        return true;
    }
    // Check src/ for .ts/.tsx files
    let src_dir = project_root.join("src");
    if src_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&src_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "ts" || ext == "tsx" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Find tsconfig.json paths in the project.
fn find_tsconfig_paths(project_root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let tsconfig = project_root.join("tsconfig.json");
    if tsconfig.exists() {
        paths.push(tsconfig.to_string_lossy().to_string());
    }
    // Also check for project references in packages/
    let packages_dir = project_root.join("packages");
    if packages_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&packages_dir) {
            for entry in entries.flatten() {
                let tsconfig = entry.path().join("tsconfig.json");
                if tsconfig.exists() {
                    paths.push(tsconfig.to_string_lossy().to_string());
                }
            }
        }
    }
    paths
}

/// Estimate the overhead of running the semantic helper in milliseconds.
fn estimate_overhead(candidate_count: usize, project_count: usize) -> u64 {
    // Rough heuristic: base cost + per-project init + per-candidate query
    let base_ms: u64 = 200;
    let per_project_ms: u64 = 300;
    let per_candidate_ms: u64 = 5;
    base_ms + (project_count as u64 * per_project_ms) + (candidate_count as u64 * per_candidate_ms)
}

/// Cache key for semantic results.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SemanticCacheKey {
    /// Hash of the tsconfig content.
    pub tsconfig_hash: u64,
    /// Hash of the project reference graph.
    pub project_graph_hash: u64,
    /// Hash of the file content.
    pub file_content_hash: u64,
    /// Hash of the candidate query parameters.
    pub query_hash: u64,
}
