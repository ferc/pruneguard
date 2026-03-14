use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use pruneguard_config::AnalysisSeverity;
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_fs::is_docs_path;
use pruneguard_graph::{FileId, GraphBuildResult, ModuleNode};
use pruneguard_report::{Evidence, Finding, FindingCategory, FindingConfidence};

use crate::{make_finding, severity};

/// Find exports that are never consumed by reachable imports or re-export chains.
#[allow(clippy::too_many_lines, clippy::implicit_hasher)]
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
    ignore_exports_used_in_file: bool,
    include_entry_exports: bool,
    reachable_files: &FxHashSet<pruneguard_graph::FileId>,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };
    let active_entrypoints = active_entrypoint_files(build, profile);

    let mut live = LiveDemand::default();
    // When `include_entry_exports` is true, do NOT blanket-mark all entrypoint
    // exports as live.  Instead, let each export's liveness be determined solely
    // by actual import/re-export demand across the graph.
    if !include_entry_exports {
        for file_id in active_entrypoints {
            live.mark_all(file_id, false);
            live.mark_all(file_id, true);
        }
    }

    for import_edge in &build.symbol_graph.import_edges {
        if !reachable_files.contains(&import_edge.importer) {
            continue;
        }

        if import_edge.export_name == "*" {
            live.mark_all(import_edge.source, import_edge.is_type);
        } else {
            live.mark_named(import_edge.source, &import_edge.export_name, import_edge.is_type);
        }
    }

    let mut changed = true;
    while changed {
        changed = false;

        for reexport_edge in &build.symbol_graph.reexport_edges {
            if !reachable_files.contains(&reexport_edge.reexporter) {
                continue;
            }

            let is_type = reexport_edge.is_type;
            if reexport_edge.is_star {
                // Distinguish true star (`export * from`) where original_name
                // AND exported_name are both "*", from namespace re-exports
                // (`export * as Name from`) where original_name is "*" but
                // exported_name is a named alias.
                let is_true_star =
                    reexport_edge.original_name == "*" && reexport_edge.exported_name == "*";

                if is_true_star {
                    // True star: if the reexporter has all exports live,
                    // blanket-live every export in the source.
                    if live.is_all_live(reexport_edge.reexporter, is_type) {
                        changed |= live.mark_all(reexport_edge.source, is_type);
                    }

                    // Also propagate individually demanded names from the
                    // reexporter to the source (the star makes the names
                    // transparent).
                    let demanded_names = live
                        .live_names(reexport_edge.reexporter, is_type)
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    for name in demanded_names {
                        changed |= live.mark_named(reexport_edge.source, &name, is_type);
                    }
                } else {
                    // Namespace re-export (`export * as Name from`): do NOT
                    // blanket-live all source exports. Instead, look at member
                    // refs on the namespace alias to determine which specific
                    // source exports are consumed.
                    if live.is_all_live(reexport_edge.reexporter, is_type)
                        || live.is_named_live(
                            reexport_edge.reexporter,
                            &reexport_edge.exported_name,
                            is_type,
                        )
                    {
                        for member_ref in &build.symbol_graph.member_refs {
                            if member_ref.source == reexport_edge.reexporter
                                && member_ref.export_name == reexport_edge.exported_name
                            {
                                changed |= live.mark_named(
                                    reexport_edge.source,
                                    &member_ref.member_name,
                                    is_type,
                                );
                            }
                        }
                    }
                }
                continue;
            }

            if live.is_all_live(reexport_edge.reexporter, is_type)
                || live.is_named_live(
                    reexport_edge.reexporter,
                    &reexport_edge.exported_name,
                    is_type,
                )
            {
                changed |=
                    live.mark_named(reexport_edge.source, &reexport_edge.original_name, is_type);
            }
        }
    }

    // When `ignore_exports_used_in_file` is enabled, collect exports that are
    // consumed within the same file.  This includes both:
    //  1. import edges where importer == source (self-imports), and
    //  2. same-file references (direct calls/references to an exported symbol
    //     without going through an import statement).
    let same_file_used: FxHashSet<(FileId, CompactString)> = if ignore_exports_used_in_file {
        let mut set: FxHashSet<(FileId, CompactString)> = build
            .symbol_graph
            .import_edges
            .iter()
            .filter(|edge| edge.importer == edge.source)
            .map(|edge| (edge.source, edge.export_name.clone()))
            .collect();
        for same_ref in &build.symbol_graph.same_file_refs {
            set.insert((same_ref.file, same_ref.export_name.clone()));
        }
        set
    } else {
        FxHashSet::default()
    };

    // Compute global unresolved pressure for confidence demotion (integer percentage).
    let total_specifiers = build.stats.files_resolved + build.stats.unresolved_specifiers;
    let global_pressure_pct = if total_specifiers > 0 {
        build.stats.unresolved_specifiers * 100 / total_specifiers
    } else {
        0
    };

    let mut findings = Vec::new();
    for export in build.symbol_graph.exports.values() {
        if !reachable_files.contains(&export.file) {
            continue;
        }

        let Some((
            _,
            ModuleNode::File { path: abs_path, relative_path, workspace, package, role, .. },
        )) = build.module_graph.file_node_by_id(export.file)
        else {
            continue;
        };

        if role.excluded_from_dead_code_by_default()
            || is_docs_path(std::path::Path::new(relative_path))
        {
            continue;
        }

        // Skip ambient declaration files — their exports augment the global scope.
        if is_ambient_declaration(relative_path) {
            continue;
        }

        if live.is_export_live(export.file, &export.name, export.is_type) {
            continue;
        }

        // Skip exports consumed within the same file when the option is enabled.
        if ignore_exports_used_in_file
            && same_file_used.contains(&(export.file, export.name.clone()))
        {
            continue;
        }

        // If the export is a value export whose name is "default" in a file that has
        // an `export * from` re-export targeting it, the name might be consumed through
        // the star re-export rather than directly.  (We already handle this in the
        // LiveDemand propagation, but this is a safety check.)
        if export.name == "default" && live.is_all_live(export.file, false) {
            continue;
        }

        let subject = format!("{relative_path}#{}", export.name);
        let (unresolved_count, benign_unresolved) = file_unresolved_counts(build, export.file);
        let effective_unresolved = unresolved_count.saturating_sub(benign_unresolved);
        let neighbor_pressure = neighbor_unresolved_pressure(build, export.file);

        // Demote confidence when the file is a target of glob/context expansion —
        // its liveness depends on heuristic pattern matching.
        let is_glob_target = build.glob_expanded_targets.contains(std::path::Path::new(abs_path));

        // Type exports (interfaces, type aliases) default to Low confidence
        // because they are often intentionally part of the public API surface
        // for library consumers.
        let confidence = if export.is_type
            || effective_unresolved >= 5
            || global_pressure_pct > 15
            || neighbor_pressure >= 8
        {
            FindingConfidence::Low
        } else if is_glob_target {
            // Glob/context expansion targets get at most Medium confidence.
            FindingConfidence::Medium
        } else if effective_unresolved == 0
            && !live.has_any_demand(export.file)
            && neighbor_pressure == 0
        {
            // Truly isolated: no unresolved, no demand, no neighbor pressure.
            if global_pressure_pct > 5 {
                FindingConfidence::Medium
            } else {
                FindingConfidence::High
            }
        } else {
            FindingConfidence::Medium
        };

        // Distinguish type-only exports from value exports.
        let (finding_code, finding_category) = if export.is_type {
            ("unused-type", FindingCategory::UnusedType)
        } else {
            ("unused-export", FindingCategory::UnusedExport)
        };

        let mut evidence = vec![Evidence {
            kind: if export.is_type { "path" } else { "reachability" }.to_string(),
            file: Some(relative_path.clone()),
            line: None,
            description: "No reachable import or re-export demand reaches this export.".to_string(),
        }];
        if effective_unresolved >= 3 {
            evidence.push(Evidence {
                kind: "unresolved-pressure".to_string(),
                file: Some(relative_path.clone()),
                line: None,
                description: format!(
                    "{effective_unresolved} unresolved specifiers may affect accuracy of this finding"
                ),
            });
        }
        findings.push(make_finding(
            finding_code,
            finding_severity,
            finding_category,
            confidence,
            &subject,
            workspace.clone(),
            package.clone(),
            format!("Export `{}` from `{relative_path}` is never consumed.", export.name),
            evidence,
            Some("Remove the export or reference it from a reachable module.".to_string()),
            None,
        ));
    }

    findings
}

/// Return (`total_unresolved`, `benign_unresolved`) from an `ExtractedFile`.
fn file_unresolved_counts_raw(file: &pruneguard_extract::ExtractedFile) -> (usize, usize) {
    let mut total = 0;
    let mut benign = 0;
    for edge in file.resolved_imports.iter().chain(&file.resolved_reexports) {
        if matches!(edge.outcome, pruneguard_resolver::ResolutionOutcome::Unresolved) {
            total += 1;
            if edge.unresolved_reason.is_some_and(pruneguard_resolver::UnresolvedReason::is_benign)
            {
                benign += 1;
            }
        }
    }
    (total, benign)
}

/// Return (`total_unresolved`, `benign_unresolved`) for a file by ID.
fn file_unresolved_counts(build: &GraphBuildResult, file_id: FileId) -> (usize, usize) {
    let Some((_, ModuleNode::File { path, .. })) = build.module_graph.file_node_by_id(file_id)
    else {
        return (1, 0);
    };
    let Some(file) = build.find_file(path) else {
        return (1, 0);
    };
    file_unresolved_counts_raw(file)
}

/// Count effective (non-benign) unresolved specifiers across files that are
/// connected to the given file by import/re-export edges.
fn neighbor_unresolved_pressure(build: &GraphBuildResult, file_id: FileId) -> usize {
    use petgraph::visit::EdgeRef;
    let Some((node_index, _)) = build.module_graph.file_node_by_id(file_id) else {
        return 0;
    };
    let mut total = 0usize;
    let mut visited = FxHashSet::default();
    for edge in build.module_graph.graph.edges_directed(node_index, petgraph::Direction::Incoming) {
        if let pruneguard_graph::ModuleNode::File { path, .. } =
            &build.module_graph.graph[edge.source()]
            && visited.insert(path.clone())
            && let Some(file) = build.find_file(path)
        {
            let (unresolved, benign) = file_unresolved_counts_raw(file);
            total = total.saturating_add(unresolved.saturating_sub(benign));
        }
    }
    for edge in build.module_graph.graph.edges(node_index) {
        if let pruneguard_graph::ModuleNode::File { path, .. } =
            &build.module_graph.graph[edge.target()]
            && visited.insert(path.clone())
            && let Some(file) = build.find_file(path)
        {
            let (unresolved, benign) = file_unresolved_counts_raw(file);
            total = total.saturating_add(unresolved.saturating_sub(benign));
        }
    }
    total
}

fn is_ambient_declaration(relative_path: &str) -> bool {
    relative_path.ends_with(".d.ts")
        || relative_path.ends_with(".d.mts")
        || relative_path.ends_with(".d.cts")
}

fn active_entrypoint_files(
    build: &GraphBuildResult,
    profile: EntrypointProfile,
) -> FxHashSet<FileId> {
    let mut files = FxHashSet::default();
    for seed in &build.entrypoint_seeds {
        let active = match profile {
            EntrypointProfile::Both => true,
            EntrypointProfile::Production => {
                seed.profile == EntrypointProfile::Production
                    || seed.profile == EntrypointProfile::Both
            }
            EntrypointProfile::Development => {
                seed.profile == EntrypointProfile::Development
                    || seed.profile == EntrypointProfile::Both
            }
        };

        if !active {
            continue;
        }

        // Normalize the seed path to remove `.` segments (e.g. `/a/./b` → `/a/b`)
        // before converting to a string for module graph lookup. PathBuf equality
        // handles this automatically, but string-based lookup does not.
        let normalized: std::path::PathBuf = seed.path.components().collect();
        if let Some(file_id) = build.module_graph.file_id(&normalized.to_string_lossy()) {
            files.insert(file_id);
        }
    }
    files
}

#[derive(Default)]
struct LiveDemand {
    value_all: FxHashSet<FileId>,
    type_all: FxHashSet<FileId>,
    value_names: FxHashMap<FileId, FxHashSet<CompactString>>,
    type_names: FxHashMap<FileId, FxHashSet<CompactString>>,
}

impl LiveDemand {
    fn mark_all(&mut self, file: FileId, is_type: bool) -> bool {
        if is_type { self.type_all.insert(file) } else { self.value_all.insert(file) }
    }

    fn mark_named(&mut self, file: FileId, name: &str, is_type: bool) -> bool {
        if is_type {
            self.type_names.entry(file).or_default().insert(CompactString::new(name))
        } else {
            self.value_names.entry(file).or_default().insert(CompactString::new(name))
        }
    }

    fn is_all_live(&self, file: FileId, is_type: bool) -> bool {
        if is_type { self.type_all.contains(&file) } else { self.value_all.contains(&file) }
    }

    fn is_named_live(&self, file: FileId, name: &str, is_type: bool) -> bool {
        if self.is_all_live(file, is_type) {
            return true;
        }

        let names = if is_type { self.type_names.get(&file) } else { self.value_names.get(&file) };
        names.is_some_and(|names: &FxHashSet<CompactString>| names.contains(name))
    }

    fn is_export_live(&self, file: FileId, name: &str, is_type: bool) -> bool {
        // A value export is also considered live if there is type-only demand for it
        // (e.g. `import type { MyEnum } from './enums'` keeps the value alive because
        // TypeScript emits the value at runtime).
        // A type export is live if there is value demand (the consumer may be re-exporting
        // the type under a value import specifier).
        self.is_named_live(file, name, is_type) || self.is_named_live(file, name, !is_type)
    }

    fn has_any_demand(&self, file: FileId) -> bool {
        self.value_all.contains(&file)
            || self.type_all.contains(&file)
            || self.value_names.get(&file).is_some_and(|n| !n.is_empty())
            || self.type_names.get(&file).is_some_and(|n| !n.is_empty())
    }

    fn live_names<'a>(&'a self, file: FileId, is_type: bool) -> impl Iterator<Item = &'a str> {
        let names = if is_type { self.type_names.get(&file) } else { self.value_names.get(&file) };
        names
            .into_iter()
            .flat_map(|names: &'a FxHashSet<CompactString>| names.iter().map(CompactString::as_str))
    }
}
