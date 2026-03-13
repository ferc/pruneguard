//! Parity tracking against knip and dependency-cruiser.
//!
//! Records which dead-code-relevant features are supported, partial, or
//! unsupported, and computes a completion percentage.

use std::fmt::Write;

/// Support level for a parity feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportLevel {
    Supported,
    Partial,
    Unsupported,
}

impl SupportLevel {
    fn symbol(self) -> &'static str {
        match self {
            SupportLevel::Supported => "OK",
            SupportLevel::Partial => "PARTIAL",
            SupportLevel::Unsupported => "MISSING",
        }
    }
}

/// A single feature in the parity matrix.
#[derive(Debug, Clone)]
pub struct ParityFeature {
    pub family: &'static str,
    pub name: &'static str,
    pub reference_tool: &'static str,
    pub level: SupportLevel,
    pub notes: &'static str,
}

/// Build the full parity matrix.
pub fn parity_matrix() -> Vec<ParityFeature> {
    vec![
        // ── Dynamic patterns ────────────────────────────────────────────
        ParityFeature {
            family: "dynamic-patterns",
            name: "require.resolve",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Resolved as RequireResolve edge",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "import.meta.resolve",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Resolved as ImportMetaResolve edge",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "import.meta.glob-literal",
            reference_tool: "knip",
            level: SupportLevel::Partial,
            notes: "Literal-only, glob expansion pending",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "import.meta.glob-wildcard",
            reference_tool: "knip",
            level: SupportLevel::Partial,
            notes: "Wildcard expansion implemented",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "import.meta.glob-array",
            reference_tool: "knip",
            level: SupportLevel::Partial,
            notes: "Array form extraction exists",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "import.meta.glob-negation",
            reference_tool: "knip",
            level: SupportLevel::Partial,
            notes: "Negation patterns supported in expansion",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "require.context",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Partial,
            notes: "Directory edge only, file expansion implemented",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "new URL(_, import.meta.url)",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Resolved as UrlConstructor edge",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "import = require",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Resolved as ImportEquals edge",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "JSDoc @import",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Resolved as JsDocImport edge",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "triple-slash file",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Resolved as TripleSlashFile edge",
        },
        ParityFeature {
            family: "dynamic-patterns",
            name: "triple-slash types",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Resolved as TripleSlashTypes edge",
        },
        // ── Export semantics ────────────────────────────────────────────
        ParityFeature {
            family: "export-semantics",
            name: "includeEntryExports",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "Config flag wired to symbol graph seeding",
        },
        ParityFeature {
            family: "export-semantics",
            name: "ignoreExportsUsedInFile",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "Same-file ref tracking implemented",
        },
        ParityFeature {
            family: "export-semantics",
            name: "namespace-member-refs",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "Member ref tracking in symbol graph",
        },
        ParityFeature {
            family: "export-semantics",
            name: "enum-member-tracking",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "Member extraction + unused_members analyzer",
        },
        ParityFeature {
            family: "export-semantics",
            name: "class-member-tracking",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "Member extraction + unused_members analyzer",
        },
        ParityFeature {
            family: "export-semantics",
            name: "duplicate-re-export-paths",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "duplicate_exports analyzer",
        },
        // ── Manifest / resolver ─────────────────────────────────────────
        ParityFeature {
            family: "manifest-resolver",
            name: "wildcard-exports",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "expand_wildcard_exports consumed in graph build",
        },
        ParityFeature {
            family: "manifest-resolver",
            name: "subpath-imports",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "resolve_import_alias in manifest",
        },
        ParityFeature {
            family: "manifest-resolver",
            name: "browser-field",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "BrowserField handling in manifest",
        },
        ParityFeature {
            family: "manifest-resolver",
            name: "typings-field",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "typings in entrypoint_files",
        },
        ParityFeature {
            family: "manifest-resolver",
            name: "config-aliases",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "Config adapter aliases fed to resolver",
        },
        // ── Config-driven liveness ──────────────────────────────────────
        ParityFeature {
            family: "config-liveness",
            name: "vite-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "ViteAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "vitest-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "VitestAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "webpack-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "WebpackAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "jest-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "JestAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "storybook-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "StorybookAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "playwright-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "PlaywrightAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "next-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "NextAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "nuxt-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "NuxtAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "astro-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "AstroAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "sveltekit-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "SvelteKitAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "remix-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "RemixAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "angular-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "AngularAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "nx-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "NxAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "turborepo-config",
            reference_tool: "knip",
            level: SupportLevel::Partial,
            notes: "TurborepoAdapter, limited static extraction",
        },
        ParityFeature {
            family: "config-liveness",
            name: "vitepress-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "VitePressAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "docusaurus-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "DocusaurusAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "rollup-config",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "RollupAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "rspack-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "RspackAdapter",
        },
        ParityFeature {
            family: "config-liveness",
            name: "gatsby-config",
            reference_tool: "knip",
            level: SupportLevel::Supported,
            notes: "GatsbyAdapter",
        },
        // ── Framework source adapters ───────────────────────────────────
        ParityFeature {
            family: "source-adapters",
            name: "vue-sfc-script",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Partial,
            notes: "Script block extraction exists",
        },
        ParityFeature {
            family: "source-adapters",
            name: "svelte-script",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Partial,
            notes: "Script block extraction exists",
        },
        ParityFeature {
            family: "source-adapters",
            name: "nuxt-auto-imports",
            reference_tool: "knip",
            level: SupportLevel::Partial,
            notes: "Auto-import roots tracked, synthetic imports pending",
        },
        ParityFeature {
            family: "source-adapters",
            name: "astro-frontmatter",
            reference_tool: "knip",
            level: SupportLevel::Partial,
            notes: "Frontmatter extraction pending",
        },
        // ── Dependency typing (dependency-cruiser) ──────────────────────
        ParityFeature {
            family: "dependency-typing",
            name: "dependency-kind-attribution",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "ResolvedEdgeKind covers all dependency types",
        },
        ParityFeature {
            family: "dependency-typing",
            name: "reachable-semantics",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "ModuleGraph reachable_nodes",
        },
        ParityFeature {
            family: "dependency-typing",
            name: "orphan-detection",
            reference_tool: "dependency-cruiser",
            level: SupportLevel::Supported,
            notes: "unused_files analyzer",
        },
    ]
}

/// Aggregate parity statistics.
pub struct ParityStats {
    pub total: usize,
    pub supported: usize,
    pub partial: usize,
    pub unsupported: usize,
    pub completion_pct: f64,
    pub by_family: Vec<FamilyStats>,
}

pub struct FamilyStats {
    pub family: String,
    pub total: usize,
    pub supported: usize,
    pub partial: usize,
    pub unsupported: usize,
    pub completion_pct: f64,
}

/// Compute parity statistics from the full matrix.
pub fn compute_parity_stats() -> ParityStats {
    let matrix = parity_matrix();

    let mut supported = 0usize;
    let mut partial = 0usize;
    let mut unsupported = 0usize;

    for f in &matrix {
        match f.level {
            SupportLevel::Supported => supported += 1,
            SupportLevel::Partial => partial += 1,
            SupportLevel::Unsupported => unsupported += 1,
        }
    }

    let total = matrix.len();
    let completion_pct = if total == 0 {
        0.0
    } else {
        ((supported as f64 + partial as f64 * 0.5) / total as f64) * 100.0
    };

    // Group by family.
    let mut families: Vec<String> = matrix.iter().map(|f| f.family.to_string()).collect();
    families.sort();
    families.dedup();

    let by_family = families
        .into_iter()
        .map(|family| {
            let members: Vec<_> = matrix.iter().filter(|f| f.family == family).collect();
            let f_supported = members.iter().filter(|f| f.level == SupportLevel::Supported).count();
            let f_partial = members.iter().filter(|f| f.level == SupportLevel::Partial).count();
            let f_unsupported =
                members.iter().filter(|f| f.level == SupportLevel::Unsupported).count();
            let f_total = members.len();
            let f_pct = if f_total == 0 {
                0.0
            } else {
                ((f_supported as f64 + f_partial as f64 * 0.5) / f_total as f64) * 100.0
            };
            FamilyStats {
                family,
                total: f_total,
                supported: f_supported,
                partial: f_partial,
                unsupported: f_unsupported,
                completion_pct: f_pct,
            }
        })
        .collect();

    ParityStats { total, supported, partial, unsupported, completion_pct, by_family }
}

/// Format the parity matrix as a human-readable table.
pub fn format_parity_table() -> String {
    let matrix = parity_matrix();
    let stats = compute_parity_stats();

    // Column widths.
    let w_family = matrix.iter().map(|f| f.family.len()).max().unwrap_or(10);
    let w_name = matrix.iter().map(|f| f.name.len()).max().unwrap_or(10);
    let w_ref = matrix.iter().map(|f| f.reference_tool.len()).max().unwrap_or(10);
    let w_level = 7; // "PARTIAL" is the longest symbol
    let w_notes = 50;

    let mut out = String::new();

    // Header.
    let _ = writeln!(
        out,
        "{:<w_family$}  {:<w_name$}  {:<w_ref$}  {:<w_level$}  {}",
        "FAMILY", "FEATURE", "REF TOOL", "STATUS", "NOTES"
    );
    let _ = writeln!(
        out,
        "{:-<w_family$}  {:-<w_name$}  {:-<w_ref$}  {:-<w_level$}  {:-<w_notes$}",
        "", "", "", "", ""
    );

    let mut last_family = "";
    for f in &matrix {
        let family_display = if f.family != last_family {
            last_family = f.family;
            f.family
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{:<w_family$}  {:<w_name$}  {:<w_ref$}  {:<w_level$}  {}",
            family_display,
            f.name,
            f.reference_tool,
            f.level.symbol(),
            f.notes,
        );
    }

    // Summary per family.
    let _ = writeln!(out);
    let _ = writeln!(out, "Per-family summary:");
    for fs in &stats.by_family {
        let _ = writeln!(
            out,
            "  {:<20} {:>2}/{:<2} supported  {:>2} partial  {:>2} unsupported  ({:.1}%)",
            fs.family, fs.supported, fs.total, fs.partial, fs.unsupported, fs.completion_pct,
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Overall: {}/{} supported, {} partial, {} unsupported => {:.1}% parity",
        stats.supported, stats.total, stats.partial, stats.unsupported, stats.completion_pct,
    );

    out
}

/// Format the parity table with an optional external corpus score appended.
pub fn format_parity_table_with_external(
    external_score: Option<&crate::external_parity::ExternalParityScore>,
) -> String {
    let mut out = format_parity_table();

    if let Some(score) = external_score {
        let _ = writeln!(out);
        let _ = writeln!(out, "--- External Parity Corpus ---");
        out.push_str(&crate::external_parity::format_external_parity_report(score));
    }

    out
}

/// A delta entry comparing the hand-authored matrix against external corpus results.
#[derive(Debug)]
pub struct StaleDelta {
    pub family: String,
    pub name: String,
    pub matrix_level: SupportLevel,
    pub corpus_passed: bool,
    pub is_stale: bool,
}

/// Compare the hand-authored parity matrix against external corpus results.
///
/// Returns entries where the matrix claims `Supported` but the corpus case
/// failed, or where the matrix claims `Unsupported`/`Partial` but the corpus
/// case passed. These indicate potentially stale hand-authored entries.
pub fn stale_delta(corpus_results: &[crate::external_parity::ParityCaseResult]) -> Vec<StaleDelta> {
    let matrix = parity_matrix();
    let mut deltas = Vec::new();

    for result in corpus_results {
        // Try to find a matching matrix entry by family+name (fuzzy match).
        let matching = matrix.iter().find(|f| {
            f.family == result.family
                && (f.name == result.name
                    || f.name.replace('.', "-").replace(' ', "-").to_lowercase()
                        == result.name.replace('.', "-").replace(' ', "-").to_lowercase())
        });

        if let Some(feature) = matching {
            let is_stale = match feature.level {
                // Matrix says supported but corpus says it fails.
                SupportLevel::Supported => !result.passed,
                // Matrix says unsupported but corpus says it passes.
                SupportLevel::Unsupported => result.passed,
                // Matrix says partial -- stale if corpus fully passes.
                SupportLevel::Partial => result.passed,
            };

            deltas.push(StaleDelta {
                family: result.family.clone(),
                name: result.name.clone(),
                matrix_level: feature.level,
                corpus_passed: result.passed,
                is_stale,
            });
        }
    }

    deltas
}
