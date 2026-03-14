use compact_str::CompactString;
use globset::{Glob, GlobSet, GlobSetBuilder};
use rustc_hash::{FxHashMap, FxHashSet};

use pruneguard_config::AnalysisConfig;
use pruneguard_extract::ExportKind;
use pruneguard_graph::{FileId, GraphBuildResult, MemberAccessKind, MemberNodeKind};
use pruneguard_report::{Evidence, Finding, FindingCategory, FindingConfidence};

use crate::{make_finding, severity};

/// Detect exported classes/enums/namespaces whose individual members (methods,
/// properties, variants) are never referenced by any consumer.
///
/// Completely dead exports (those with no imports at all) are already reported
/// by the `unused_exports` analyzer; this analyzer only flags *partially* used
/// exports where some members are live and others are not.
#[allow(clippy::too_many_lines)]
pub fn analyze(build: &GraphBuildResult, config: &AnalysisConfig) -> Vec<Finding> {
    let Some(finding_severity) = severity(config.unused_members) else {
        return Vec::new();
    };

    let mut findings = Vec::new();

    // Compile ignore_members glob patterns.
    let ignore_members_matcher = compile_member_globs(&config.ignore_members);

    // Build a set of member references keyed by (source_file, export_name, member_name).
    // When member_write_only_is_unused is true, only count Read and ReadWrite accesses.
    let referenced_members: FxHashSet<(FileId, CompactString, CompactString)> = build
        .symbol_graph
        .member_refs
        .iter()
        .filter(|r| {
            if config.member_write_only_is_unused {
                // Only count Read and ReadWrite accesses as "used".
                matches!(r.access_kind, MemberAccessKind::Read | MemberAccessKind::ReadWrite)
            } else {
                true
            }
        })
        .map(|r| (r.source, r.export_name.clone(), r.member_name.clone()))
        .collect();

    // Build a set of live exports (exports that have at least one import).
    let imported_exports: FxHashSet<(FileId, CompactString)> =
        build.symbol_graph.import_edges.iter().map(|e| (e.source, e.export_name.clone())).collect();

    // Also count which unique member names exist per parent to identify partially-used exports.
    // Getter/setter pairs share the same name and should be counted once.
    let members_per_export: FxHashMap<(FileId, CompactString), Vec<CompactString>> = {
        let mut map: FxHashMap<(FileId, CompactString), FxHashSet<CompactString>> =
            FxHashMap::default();
        for member_node in &build.symbol_graph.member_exports {
            map.entry((member_node.file, member_node.parent_export.clone()))
                .or_default()
                .insert(member_node.member_name.clone());
        }
        map.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect()
    };

    // For each file, look at exports with members.
    for extracted_file in &build.files {
        let Some(facts) = &extracted_file.facts else {
            continue;
        };

        let Some(file_id) = build.module_graph.file_id(&extracted_file.file.path.to_string_lossy())
        else {
            continue;
        };

        for export in &facts.exports {
            // Only analyze class/enum/namespace exports that have tracked members.
            if !matches!(
                export.export_kind,
                ExportKind::Class | ExportKind::Enum | ExportKind::Namespace
            ) {
                continue;
            }

            // Skip exports that are completely unused — unused_exports handles those.
            if !imported_exports.contains(&(file_id, export.name.clone())) {
                // Also check if all file exports are live (entrypoint file).
                let is_live = build
                    .symbol_graph
                    .exports
                    .get(&(file_id, export.name.clone()))
                    .is_some_and(|node| node.is_live);
                if !is_live {
                    continue;
                }
            }

            let Some(members) = members_per_export.get(&(file_id, export.name.clone())) else {
                // No tracked members — nothing to analyze.
                continue;
            };

            // Find which members are referenced and which are dead.
            let mut dead_members = Vec::new();
            let mut live_count = 0;
            for member_name in members {
                let key = (file_id, export.name.clone(), member_name.clone());
                if referenced_members.contains(&key) {
                    live_count += 1;
                } else {
                    // Also check if the member is marked live in the symbol graph.
                    let is_member_live = build.symbol_graph.member_exports.iter().any(|m| {
                        m.file == file_id
                            && m.parent_export == export.name
                            && m.member_name == *member_name
                            && m.is_live
                    });
                    // When member_write_only_is_unused is enabled, a member that
                    // is marked live in the symbol graph but only has Write
                    // accesses should still be considered dead (the symbol graph's
                    // is_live flag doesn't distinguish access kinds).
                    let write_only_override = is_member_live
                        && config.member_write_only_is_unused
                        && build.symbol_graph.member_refs.iter().any(|r| {
                            r.source == file_id
                                && r.export_name == export.name
                                && r.member_name == *member_name
                        })
                        && !build.symbol_graph.member_refs.iter().any(|r| {
                            r.source == file_id
                                && r.export_name == export.name
                                && r.member_name == *member_name
                                && matches!(
                                    r.access_kind,
                                    MemberAccessKind::Read | MemberAccessKind::ReadWrite
                                )
                        });
                    if is_member_live && !write_only_override {
                        live_count += 1;
                    } else {
                        dead_members.push(member_name.clone());
                    }
                }
            }

            // Only report partially-used exports (some live, some dead).
            if dead_members.is_empty() || live_count == 0 {
                continue;
            }

            let relative_path = &extracted_file.file.relative_path;
            let kind_label = match export.export_kind {
                ExportKind::Class => "class",
                ExportKind::Enum => "enum",
                ExportKind::Namespace => "namespace",
                _ => "export",
            };

            // Demote confidence for files that are glob/context expansion targets.
            let is_glob_target = build.glob_expanded_targets.contains(&extracted_file.file.path);
            let confidence =
                if is_glob_target { FindingConfidence::Low } else { FindingConfidence::Medium };

            for dead_member in &dead_members {
                // --- Suppression checks ---

                // 1. Setter suppression: setters are typically paired with getters
                //    and don't need independent usage tracking.
                if should_suppress_member(
                    build,
                    file_id,
                    &export.name,
                    dead_member,
                    config,
                    ignore_members_matcher.as_ref(),
                ) {
                    continue;
                }

                let evidence = vec![Evidence {
                    kind: "unused-member".to_string(),
                    file: Some(relative_path.to_string_lossy().to_string()),
                    line: Some(export.line as usize),
                    description: format!(
                        "`{}.{}` is never referenced by any consumer ({} of {} members used)",
                        export.name,
                        dead_member,
                        live_count,
                        members.len(),
                    ),
                }];

                findings.push(make_finding(
                    "unused-member",
                    finding_severity,
                    FindingCategory::UnusedMember,
                    confidence,
                    format!(
                        "{}.{}",
                        relative_path.to_string_lossy(),
                        export.name,
                    ),
                    extracted_file.file.workspace.clone(),
                    extracted_file.file.package.clone(),
                    format!(
                        "{kind_label} `{}` member `{dead_member}` is exported but never used ({live_count}/{} members referenced)",
                        export.name,
                        members.len(),
                    ),
                    evidence,
                    Some(format!(
                        "Remove the unused member `{dead_member}` from `{}`",
                        export.name,
                    )),
                    None,
                ));
            }
        }
    }

    findings
}

/// Check whether a dead member should be suppressed from reporting.
fn should_suppress_member(
    build: &GraphBuildResult,
    file_id: FileId,
    parent_export: &CompactString,
    member_name: &CompactString,
    config: &AnalysisConfig,
    ignore_members_matcher: Option<&GlobSet>,
) -> bool {
    // Look up the member node to check kind and public tag.
    let member_node = build.symbol_graph.member_exports.iter().find(|m| {
        m.file == file_id && m.parent_export == *parent_export && m.member_name == *member_name
    });

    if let Some(node) = member_node {
        // 1. Setter suppression: setters are typically paired with getters
        //    and don't need independent usage tracking.
        if node.member_kind == MemberNodeKind::Setter {
            return true;
        }

        // 2. @public tag suppression: members marked as public API should not
        //    be reported as unused.
        if node.is_public_tagged {
            if config.public_tag_names.is_empty() {
                // Default: any @public tag suppresses the finding.
                return true;
            }
            // If specific tag names are configured, the extractor already
            // checked against those tags, so is_public_tagged being true
            // means it matched.
            return true;
        }
    }

    // 3. Ignore members matching glob patterns.
    if let Some(matcher) = ignore_members_matcher
        && matcher.is_match(member_name.as_str())
    {
        return true;
    }

    false
}

/// Compile a list of glob patterns into a `GlobSet`, returning `None` if the
/// list is empty or all patterns are invalid.
fn compile_member_globs(patterns: &[String]) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        match Glob::new(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(err) => {
                tracing::warn!(
                    pattern,
                    %err,
                    "invalid glob in ignoreMembers config, skipping pattern"
                );
            }
        }
    }
    match builder.build() {
        Ok(set) if !set.is_empty() => Some(set),
        _ => None,
    }
}
