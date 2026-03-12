use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use oxgraph_config::AnalysisSeverity;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_fs::is_docs_path;
use oxgraph_graph::{FileId, GraphBuildResult, ModuleNode};
use oxgraph_report::{Evidence, Finding, FindingCategory};

use crate::{make_finding, severity};

/// Find exports that are never consumed by reachable imports or re-export chains.
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let reachable_files = build.module_graph.reachable_file_ids(profile);
    let active_entrypoints = active_entrypoint_files(build, profile);

    let mut live = LiveDemand::default();
    for file_id in active_entrypoints {
        live.mark_all(file_id, false);
        live.mark_all(file_id, true);
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
                if live.is_all_live(reexport_edge.reexporter, is_type) {
                    changed |= live.mark_all(reexport_edge.source, is_type);
                }

                let demanded_names = live
                    .live_names(reexport_edge.reexporter, is_type)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                for name in demanded_names {
                    changed |= live.mark_named(reexport_edge.source, &name, is_type);
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

    let mut findings = Vec::new();
    for export in build.symbol_graph.exports.values() {
        if !reachable_files.contains(&export.file) {
            continue;
        }

        let Some((_, ModuleNode::File { relative_path, workspace, package, role, .. })) = build
            .module_graph
            .file_node_by_id(export.file)
        else {
            continue;
        };

        if role.excluded_from_dead_code_by_default() || is_docs_path(std::path::Path::new(relative_path)) {
            continue;
        }

        if live.is_export_live(export.file, &export.name, export.is_type) {
            continue;
        }

        let subject = format!("{relative_path}#{}", export.name);
        findings.push(make_finding(
            "unused-export",
            finding_severity,
            FindingCategory::UnusedExport,
            &subject,
            workspace.clone(),
            package.clone(),
            format!("Export `{}` from `{relative_path}` is never consumed.", export.name),
            vec![Evidence {
                kind: if export.is_type { "path" } else { "reachability" }.to_string(),
                file: Some(relative_path.clone()),
                line: None,
                description:
                    "No reachable import or re-export demand reaches this export.".to_string(),
            }],
            Some("Remove the export or reference it from a reachable module.".to_string()),
            None,
        ));
    }

    findings
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
                seed.profile == EntrypointProfile::Production || seed.profile == EntrypointProfile::Both
            }
            EntrypointProfile::Development => {
                seed.profile == EntrypointProfile::Development || seed.profile == EntrypointProfile::Both
            }
        };

        if !active {
            continue;
        }

        if let Some(file_id) = build
            .module_graph
            .file_id(&seed.path.to_string_lossy())
        {
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
        if is_type {
            self.type_all.insert(file)
        } else {
            self.value_all.insert(file)
        }
    }

    fn mark_named(&mut self, file: FileId, name: &str, is_type: bool) -> bool {
        if is_type {
            self.type_names
                .entry(file)
                .or_default()
                .insert(CompactString::new(name))
        } else {
            self.value_names
                .entry(file)
                .or_default()
                .insert(CompactString::new(name))
        }
    }

    fn is_all_live(&self, file: FileId, is_type: bool) -> bool {
        if is_type {
            self.type_all.contains(&file)
        } else {
            self.value_all.contains(&file)
        }
    }

    fn is_named_live(&self, file: FileId, name: &str, is_type: bool) -> bool {
        if self.is_all_live(file, is_type) {
            return true;
        }

        let names = if is_type {
            self.type_names.get(&file)
        } else {
            self.value_names.get(&file)
        };
        names.is_some_and(|names: &FxHashSet<CompactString>| names.contains(name))
    }

    fn is_export_live(&self, file: FileId, name: &str, is_type: bool) -> bool {
        self.is_named_live(file, name, is_type)
    }

    fn live_names<'a>(&'a self, file: FileId, is_type: bool) -> impl Iterator<Item = &'a str> {
        let names = if is_type {
            self.type_names.get(&file)
        } else {
            self.value_names.get(&file)
        };
        names
            .into_iter()
            .flat_map(|names: &'a FxHashSet<CompactString>| names.iter().map(CompactString::as_str))
    }
}
