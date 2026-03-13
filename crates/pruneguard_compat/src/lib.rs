use pruneguard_manifest::PackageManifest;
use serde::Serialize;

// These types represent framework detection metadata that feeds into compatibility
// analysis. They are defined here because the upstream `pruneguard_frameworks`
// crate currently exposes a trait-based `FrameworkPack` API rather than these
// concrete signal structs. When the frameworks crate grows richer detection
// metadata, these can be re-exported from there instead.

/// A concrete framework detection result with confidence metadata.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkDetection {
    pub name: String,
    pub confidence: DetectionConfidence,
    pub reasons: Vec<String>,
}

/// How confident the detection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DetectionConfidence {
    /// Detected via an authoritative signal (e.g. dependency + config file).
    Exact,
    /// Detected via weaker heuristics (e.g. directory conventions only).
    Heuristic,
}

/// A trust note emitted by a framework pack that may lower confidence in
/// specific findings.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkTrustNote {
    pub message: String,
    pub affects: TrustNoteScope,
}

/// The scope a trust note applies to.
#[derive(Debug, Clone, Serialize)]
pub enum TrustNoteScope {
    AllFindings,
    EntrypointsOnly,
    Workspace(String),
    Path(String),
}

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
    pub fn compute(
        detections: &[FrameworkDetection],
        trust_notes: &[FrameworkTrustNote],
        manifest: &PackageManifest,
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
        let unsupported_signals =
            Self::detect_unsupported_signals(manifest, &supported, &heuristic);

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
            ("@ember/core", "Ember"),
            ("ember-cli", "Ember"),
            ("@capacitor/core", "Capacitor"),
            ("@ionic/angular", "Ionic"),
            ("@ionic/react", "Ionic"),
            ("@ionic/vue", "Ionic"),
            ("electron", "Electron"),
            ("tauri", "Tauri"),
            ("@tauri-apps/api", "Tauri"),
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
}
