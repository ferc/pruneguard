use std::path::Path;

pub use pruneguard_frameworks::{
    DetectionConfidence, FrameworkDetection, FrameworkTrustNote, TrustNoteScope,
};
use pruneguard_manifest::PackageManifest;
use pruneguard_report::{Finding, FindingConfidence};
use serde::Serialize;

/// Result of compatibility analysis for a workspace.
#[derive(Debug, Clone, Serialize)]
pub struct CompatibilityReport {
    /// Frameworks detected with full support.
    pub supported_frameworks: Vec<String>,
    /// Frameworks detected heuristically (partial support).
    pub heuristic_frameworks: Vec<String>,
    /// Framework signals seen but not supported by any pack.
    pub unsupported_signals: Vec<UnsupportedSignal>,
    /// Warnings that may lower dead-code confidence.
    pub warnings: Vec<CompatibilityWarning>,
    /// Trust downgrade signals for downstream consumers.
    pub trust_downgrades: Vec<TrustDowngrade>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnsupportedSignal {
    pub signal: String,
    pub source: String, // e.g. "dependency", "config-file", "directory"
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompatibilityWarning {
    pub code: String,
    pub message: String,
    pub affected_scope: Option<String>,
    pub severity: WarningSeverity,
}

#[derive(Debug, Clone, Serialize)]
pub enum WarningSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustDowngrade {
    pub reason: String,
    pub scope: TrustDowngradeScope,
    pub severity: WarningSeverity,
}

#[derive(Debug, Clone, Serialize)]
pub enum TrustDowngradeScope {
    Global,
    Workspace(String),
    Path(String),
}

impl CompatibilityReport {
    /// Compute compatibility report from framework detections and trust notes.
    ///
    /// The `workspace_root` parameter enables config-file-based detection of
    /// unsupported frameworks (e.g. `redwood.toml`, `ionic.config.json`).
    pub fn compute(
        detections: &[FrameworkDetection],
        trust_notes: &[FrameworkTrustNote],
        manifest: &PackageManifest,
        workspace_root: Option<&Path>,
    ) -> Self {
        let mut supported = Vec::new();
        let mut heuristic = Vec::new();
        let mut warnings = Vec::new();
        let mut trust_downgrades = Vec::new();

        for det in detections {
            match det.confidence {
                DetectionConfidence::Exact => supported.push(det.name.to_string()),
                DetectionConfidence::Heuristic => {
                    heuristic.push(det.name.to_string());
                    warnings.push(CompatibilityWarning {
                        code: format!("heuristic-detection-{}", det.name),
                        message: format!(
                            "Framework '{}' detected heuristically: {}",
                            det.name,
                            det.reasons.join("; ")
                        ),
                        affected_scope: None,
                        severity: WarningSeverity::Medium,
                    });
                    trust_downgrades.push(TrustDowngrade {
                        reason: format!("Heuristic framework detection: {}", det.name),
                        scope: TrustDowngradeScope::Global,
                        severity: WarningSeverity::Medium,
                    });
                }
            }
        }

        for note in trust_notes {
            warnings.push(CompatibilityWarning {
                code: "framework-trust-note".to_string(),
                message: note.message.clone(),
                affected_scope: match &note.affects {
                    TrustNoteScope::AllFindings => None,
                    TrustNoteScope::EntrypointsOnly => Some("entrypoints".to_string()),
                    TrustNoteScope::Workspace(w) => Some(format!("workspace:{w}")),
                    TrustNoteScope::Path(p) => Some(format!("path:{p}")),
                },
                severity: WarningSeverity::Medium,
            });
        }

        // Check for unsupported signals in manifest
        let mut unsupported_signals =
            Self::detect_unsupported_signals(manifest, &supported, &heuristic);

        // Check for unsupported signals via config files in the workspace root
        if let Some(root) = workspace_root {
            unsupported_signals.extend(Self::detect_unsupported_directory_signals(root));
        }

        // Add trust downgrades for each unsupported signal
        for signal in &unsupported_signals {
            let framework_name = Self::framework_name_for_signal(&signal.signal);
            let note = framework_name
                .and_then(trust_note_for_unsupported)
                .unwrap_or_else(|| signal.suggestion.clone().unwrap_or_default());

            if !note.is_empty() {
                trust_downgrades.push(TrustDowngrade {
                    reason: note,
                    scope: TrustDowngradeScope::Global,
                    severity: WarningSeverity::High,
                });
            }
        }

        Self {
            supported_frameworks: supported,
            heuristic_frameworks: heuristic,
            unsupported_signals,
            warnings,
            trust_downgrades,
        }
    }

    fn detect_unsupported_signals(
        manifest: &PackageManifest,
        supported: &[String],
        heuristic: &[String],
    ) -> Vec<UnsupportedSignal> {
        // Known framework-like packages that might indicate unsupported tooling
        let known_framework_deps = [
            ("gatsby", "Gatsby"),
            ("@redwoodjs/core", "RedwoodJS"),
            ("@redwoodjs/web", "RedwoodJS"),
            ("@ember/core", "Ember"),
            ("ember-cli", "Ember"),
            ("ember-source", "Ember"),
            ("@glimmer/component", "Ember"),
            ("@capacitor/core", "Capacitor"),
            ("@capacitor/cli", "Capacitor"),
            ("@ionic/angular", "Ionic"),
            ("@ionic/react", "Ionic"),
            ("@ionic/vue", "Ionic"),
            ("@ionic/core", "Ionic"),
            ("electron", "Electron"),
            ("electron-builder", "Electron"),
            ("tauri", "Tauri"),
            ("@tauri-apps/api", "Tauri"),
            ("@tauri-apps/cli", "Tauri"),
            // Partially supported (have adapters but limited)
            ("solid-js", "SolidJS"),
            ("@solidjs/router", "SolidJS"),
            ("preact", "Preact"),
        ];

        let all_known: Vec<&str> =
            supported.iter().chain(heuristic.iter()).map(String::as_str).collect();
        let mut signals = Vec::new();

        for (dep, name) in &known_framework_deps {
            let has_dep = manifest.dependencies.as_ref().is_some_and(|d| d.contains_key(*dep))
                || manifest.dev_dependencies.as_ref().is_some_and(|d| d.contains_key(*dep));
            if has_dep && !all_known.iter().any(|k| k.eq_ignore_ascii_case(name)) {
                signals.push(UnsupportedSignal {
                    signal: (*dep).to_string(),
                    source: "dependency".to_string(),
                    suggestion: Some(format!(
                        "{name} is not yet supported; findings may be less accurate"
                    )),
                });
            }
        }

        signals
    }

    /// Detect unsupported framework signals from config files in the workspace root.
    fn detect_unsupported_directory_signals(workspace_root: &Path) -> Vec<UnsupportedSignal> {
        let mut signals = Vec::new();
        let known_files = [
            ("redwood.toml", "RedwoodJS"),
            ("ionic.config.json", "Ionic"),
            ("capacitor.config.ts", "Capacitor"),
            ("capacitor.config.json", "Capacitor"),
            ("electron-builder.yml", "Electron"),
            ("electron-builder.json", "Electron"),
        ];
        for (file, name) in &known_files {
            if workspace_root.join(file).exists() {
                signals.push(UnsupportedSignal {
                    signal: (*file).to_string(),
                    source: "config-file".to_string(),
                    suggestion: Some(format!(
                        "{name} detected via config file; findings may be less accurate"
                    )),
                });
            }
        }
        signals
    }

    /// Map a signal (dependency name or config file) back to its framework name.
    fn framework_name_for_signal(signal: &str) -> Option<&'static str> {
        match signal {
            "gatsby" => Some("Gatsby"),
            "@redwoodjs/core" | "@redwoodjs/web" | "redwood.toml" => Some("RedwoodJS"),
            "@ember/core" | "ember-cli" | "ember-source" | "@glimmer/component" => Some("Ember"),
            "@capacitor/core"
            | "@capacitor/cli"
            | "capacitor.config.ts"
            | "capacitor.config.json" => Some("Capacitor"),
            "@ionic/angular" | "@ionic/react" | "@ionic/vue" | "@ionic/core"
            | "ionic.config.json" => Some("Ionic"),
            "electron" | "electron-builder" | "electron-builder.yml" | "electron-builder.json" => {
                Some("Electron")
            }
            "tauri" | "@tauri-apps/api" | "@tauri-apps/cli" => Some("Tauri"),
            "solid-js" | "@solidjs/router" => Some("SolidJS"),
            "preact" => Some("Preact"),
            _ => None,
        }
    }

    /// Whether strict trust should be applied based on compatibility state.
    pub fn should_apply_strict_trust(&self) -> bool {
        !self.heuristic_frameworks.is_empty()
            || !self.unsupported_signals.is_empty()
            || self.warnings.iter().any(|w| matches!(w.severity, WarningSeverity::High))
    }

    /// Check if a specific finding path is affected by trust downgrades.
    pub fn is_path_affected(&self, path: &str) -> bool {
        self.trust_downgrades.iter().any(|td| match &td.scope {
            TrustDowngradeScope::Global => true,
            TrustDowngradeScope::Path(p) => path.starts_with(p.as_str()),
            TrustDowngradeScope::Workspace(_) => false,
        })
    }

    /// Get trust notes for a specific finding.
    pub fn trust_notes_for_path(&self, path: &str) -> Vec<String> {
        self.trust_downgrades
            .iter()
            .filter(|td| match &td.scope {
                TrustDowngradeScope::Global => true,
                TrustDowngradeScope::Path(p) => path.starts_with(p.as_str()),
                TrustDowngradeScope::Workspace(_) => false,
            })
            .map(|td| td.reason.clone())
            .collect()
    }

    /// Apply trust downgrades to a list of findings.
    ///
    /// For each finding affected by trust downgrades, this lowers the confidence
    /// by one level and attaches trust notes explaining why.
    pub fn apply_trust_downgrades(&self, findings: &mut [Finding]) {
        if self.trust_downgrades.is_empty() {
            return;
        }

        for finding in findings.iter_mut() {
            let notes = self.trust_notes_for_path(&finding.subject);
            if notes.is_empty() {
                continue;
            }

            // Lower confidence by one level
            finding.confidence = match finding.confidence {
                FindingConfidence::High => FindingConfidence::Medium,
                FindingConfidence::Medium => FindingConfidence::Low,
                FindingConfidence::Low => FindingConfidence::Low,
            };

            // Attach trust notes
            let existing = finding.trust_notes.get_or_insert_with(Vec::new);
            existing.extend(notes);
        }
    }

    /// Attach framework context to findings affected by heuristic or unsupported
    /// frameworks.
    ///
    /// This does not change confidence — it annotates findings so that downstream
    /// consumers understand which frameworks were in play.
    pub fn attach_framework_context(&self, findings: &mut [Finding]) {
        let mut contexts = Vec::new();

        for fw in &self.heuristic_frameworks {
            contexts.push(format!("Framework '{fw}' detected heuristically"));
        }

        for signal in &self.unsupported_signals {
            if let Some(suggestion) = &signal.suggestion {
                contexts.push(suggestion.clone());
            }
        }

        if contexts.is_empty() {
            return;
        }

        for finding in findings.iter_mut() {
            let existing = finding.framework_context.get_or_insert_with(Vec::new);
            existing.extend(contexts.clone());
        }
    }

    /// Names of unsupported frameworks detected.
    pub fn unsupported_framework_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .unsupported_signals
            .iter()
            .filter_map(|s| Self::framework_name_for_signal(&s.signal).map(String::from))
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

/// Return a framework-specific trust note explaining what might be missed
/// for an unsupported framework.
fn trust_note_for_unsupported(name: &str) -> Option<String> {
    match name {
        "Gatsby" => Some(
            "Gatsby page/template queries and programmatic page creation may not be detected"
                .to_string(),
        ),
        "RedwoodJS" => Some(
            "RedwoodJS cells and services may have implicit entrypoints not detected".to_string(),
        ),
        "Ember" => {
            Some("Ember route conventions and service injection may not be detected".to_string())
        }
        "Ionic" => {
            Some("Ionic page routing and lazy-loaded modules may not be detected".to_string())
        }
        "Capacitor" => {
            Some("Capacitor plugin auto-registration may create implicit dependencies".to_string())
        }
        "Electron" => {
            Some("Electron main/renderer process separation may not be modeled".to_string())
        }
        "Tauri" => Some("Tauri command handlers and IPC bridges may not be detected".to_string()),
        "SolidJS" => Some(
            "SolidJS has partial support; some reactive primitives may not be tracked".to_string(),
        ),
        "Preact" => Some(
            "Preact has partial support; compat layer aliases may not be fully resolved"
                .to_string(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashMap;

    fn make_manifest_with_deps(deps: &[(&str, &str)]) -> PackageManifest {
        let mut dep_map = FxHashMap::default();
        for (name, version) in deps {
            dep_map.insert((*name).to_string(), (*version).to_string());
        }
        PackageManifest { dependencies: Some(dep_map), ..PackageManifest::default() }
    }

    fn make_finding(subject: &str, confidence: FindingConfidence) -> Finding {
        Finding {
            id: "test-id".to_string(),
            code: "unused-file".to_string(),
            severity: pruneguard_report::FindingSeverity::Warn,
            category: pruneguard_report::FindingCategory::UnusedFile,
            subject: subject.to_string(),
            workspace: None,
            package: None,
            message: "test finding".to_string(),
            evidence: Vec::new(),
            suggestion: None,
            rule_name: None,
            confidence,
            primary_action_kind: None,
            action_kinds: Vec::new(),
            trust_notes: None,
            framework_context: None,
        }
    }

    #[test]
    fn detects_redwoodjs_unsupported_signal() {
        let manifest = make_manifest_with_deps(&[("@redwoodjs/core", "^5.0.0")]);
        let report = CompatibilityReport::compute(&[], &[], &manifest, None);
        assert!(!report.unsupported_signals.is_empty());
        assert!(report.unsupported_signals.iter().any(|s| s.signal == "@redwoodjs/core"));
    }

    #[test]
    fn detects_multiple_unsupported_signals() {
        let manifest =
            make_manifest_with_deps(&[("electron", "^28.0.0"), ("@redwoodjs/core", "^5.0.0")]);
        let report = CompatibilityReport::compute(&[], &[], &manifest, None);
        assert!(report.unsupported_signals.len() >= 2);
    }

    #[test]
    fn does_not_flag_supported_frameworks() {
        let manifest = make_manifest_with_deps(&[("next", "^14.0.0")]);
        let detections = vec![FrameworkDetection {
            name: "Next.js",
            confidence: DetectionConfidence::Exact,
            signals: vec!["next".to_string()],
            reasons: vec!["next dependency".to_string()],
        }];
        let report = CompatibilityReport::compute(&detections, &[], &manifest, None);
        // "next" is not in the unsupported list, so should be empty
        assert!(report.unsupported_signals.is_empty());
    }

    #[test]
    fn apply_trust_downgrades_lowers_confidence() {
        let manifest = make_manifest_with_deps(&[("gatsby", "^5.0.0")]);
        let report = CompatibilityReport::compute(&[], &[], &manifest, None);
        assert!(report.should_apply_strict_trust());

        let mut findings = vec![make_finding("src/unused.ts", FindingConfidence::High)];
        report.apply_trust_downgrades(&mut findings);

        assert_eq!(findings[0].confidence, FindingConfidence::Medium);
        assert!(findings[0].trust_notes.is_some());
        let notes = findings[0].trust_notes.as_ref().unwrap();
        assert!(!notes.is_empty());
    }

    #[test]
    fn apply_trust_downgrades_does_not_go_below_low() {
        let manifest = make_manifest_with_deps(&[("gatsby", "^5.0.0")]);
        let report = CompatibilityReport::compute(&[], &[], &manifest, None);

        let mut findings = vec![make_finding("src/unused.ts", FindingConfidence::Low)];
        report.apply_trust_downgrades(&mut findings);

        assert_eq!(findings[0].confidence, FindingConfidence::Low);
    }

    #[test]
    fn attach_framework_context_adds_annotations() {
        let manifest = make_manifest_with_deps(&[("@ionic/react", "^7.0.0")]);
        let report = CompatibilityReport::compute(&[], &[], &manifest, None);

        let mut findings = vec![make_finding("src/page.tsx", FindingConfidence::High)];
        report.attach_framework_context(&mut findings);

        assert!(findings[0].framework_context.is_some());
        let ctx = findings[0].framework_context.as_ref().unwrap();
        assert!(ctx.iter().any(|c| c.contains("Ionic")));
    }

    #[test]
    fn no_downgrades_when_no_unsupported() {
        let manifest = make_manifest_with_deps(&[("react", "^18.0.0")]);
        let report = CompatibilityReport::compute(&[], &[], &manifest, None);

        let mut findings = vec![make_finding("src/unused.ts", FindingConfidence::High)];
        report.apply_trust_downgrades(&mut findings);

        assert_eq!(findings[0].confidence, FindingConfidence::High);
        assert!(findings[0].trust_notes.is_none());
    }

    #[test]
    fn unsupported_framework_names_deduplicates() {
        let manifest =
            make_manifest_with_deps(&[("@redwoodjs/core", "^5.0.0"), ("@redwoodjs/web", "^5.0.0")]);
        let report = CompatibilityReport::compute(&[], &[], &manifest, None);
        let names = report.unsupported_framework_names();
        assert_eq!(names, vec!["RedwoodJS"]);
    }
}
