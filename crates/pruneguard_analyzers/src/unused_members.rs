use pruneguard_config::AnalysisSeverity;
use pruneguard_graph::GraphBuildResult;
use pruneguard_report::Finding;

use crate::severity;

/// Detect exported classes/enums whose individual members (methods, properties,
/// variants) are never referenced by any consumer.
///
/// # Current status
///
/// Member-level liveness analysis requires two capabilities that are not yet
/// present in the extraction pipeline:
///
/// 1. **Member extraction**: `FileFacts` does not yet capture the individual
///    members (methods, properties, enum variants) of exported classes and
///    enums.  The `ExportInfo` struct records the export name and kind, but not
///    its constituent members.
///
/// 2. **Property-access tracking**: The symbol graph records named imports and
///    re-exports, but does not track which *members* of an imported binding are
///    accessed (e.g. `MyEnum.Variant` or `instance.method()`).  Without this
///    information, we cannot distinguish "the enum is imported and all variants
///    are used" from "the enum is imported but only one variant is used."
///
/// Once these extraction capabilities are added, this analyzer should:
///
/// 1. Iterate over exports whose `export_kind` is `Enum` or `Class`.
/// 2. For each such export, enumerate its members from the extraction data.
/// 3. Cross-reference the members against property-access edges in the symbol
///    graph to determine which members are live.
/// 4. Emit a `FindingCategory::UnusedMember` finding for each member that has
///    no live reference, using `FindingConfidence::Medium` (since property
///    access tracking may miss dynamic access patterns).
///
/// Note: Completely dead enums/classes (those with no imports at all) are
/// already reported by the `unused_exports` analyzer, so this analyzer should
/// only flag *partially* used exports where some members are live and others
/// are not.
pub fn analyze(
    _build: &GraphBuildResult,
    level: AnalysisSeverity,
) -> Vec<Finding> {
    let Some(_finding_severity) = severity(level) else {
        return Vec::new();
    };

    // Awaiting member-level extraction and property-access tracking in the
    // symbol graph before findings can be emitted.
    Vec::new()
}
