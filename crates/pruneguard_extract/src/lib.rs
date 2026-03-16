use std::path::{Path, PathBuf};

use compact_str::CompactString;
use pruneguard_fs::FileRecord;
use pruneguard_resolver::ResolvedEdge;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

/// Extracted facts from a single JS/TS file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileFacts {
    /// Named exports (export name).
    pub exports: Vec<ExportInfo>,
    /// Import statements.
    pub imports: Vec<ImportInfo>,
    /// Re-export statements.
    pub reexports: Vec<ReexportInfo>,
    /// Whether this file has side effects at module level.
    pub has_side_effects: bool,
    /// Dynamic `import()` expressions.
    pub dynamic_imports: Vec<DynamicImportInfo>,
    /// CJS `require()` calls.
    pub requires: Vec<RequireInfo>,
    /// Non-standard dependency patterns (require.resolve, import.meta.glob, etc.).
    #[serde(default)]
    pub dependency_patterns: Vec<DependencyPattern>,
    /// Members of exported classes, enums, and namespaces.
    #[serde(default)]
    pub member_exports: Vec<MemberExportInfo>,
    /// References to exported symbols from within the same file.
    #[serde(default)]
    pub same_file_refs: Vec<SameFileRefInfo>,
    /// Member access patterns detected in the file (e.g., `Color.Red`).
    /// Used for tracking which enum/class/namespace members are referenced.
    #[serde(default)]
    pub member_accesses: Vec<MemberAccessInfo>,
}

/// A detected member access pattern (e.g., `Color.Red`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberAccessInfo {
    /// The object name (e.g., `Color`).
    pub object_name: String,
    /// The member name (e.g., `Red`).
    pub member_name: String,
    /// Source line number.
    pub line: u32,
    /// Whether this is a write-only access (assignment target, e.g. `obj.member = value`).
    #[serde(default)]
    pub is_write: bool,
}

/// The kind of an exported declaration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportKind {
    #[default]
    Value,
    Type,
    Class,
    Enum,
    Namespace,
    Reexport,
    Default,
}

/// An export from a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportInfo {
    /// The exported name (or "default").
    pub name: CompactString,
    /// Whether this is a type-only export.
    pub is_type: bool,
    /// The kind of the exported declaration.
    #[serde(default)]
    pub export_kind: ExportKind,
    /// Source line (1-indexed).
    pub line: u32,
}

/// Individual members of an exported class, enum, or namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberExportInfo {
    /// The name of the parent export (e.g. "`MyClass`", "`MyEnum`").
    pub parent_name: CompactString,
    /// The member name (e.g. method name, enum variant, namespace member).
    pub member_name: CompactString,
    /// The kind of member.
    pub member_kind: MemberKind,
    /// Line number where the member is defined.
    pub line: u32,
    /// Whether this member has a `@public` `JSDoc` tag in its leading comment.
    #[serde(default)]
    pub is_public_tagged: bool,
}

/// The kind of member within an exported class, enum, or namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberKind {
    Method,
    Property,
    EnumVariant,
    NamespaceMember,
    StaticMethod,
    StaticProperty,
    Getter,
    Setter,
}

/// A reference to an exported symbol from within the same file (not via import).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SameFileRefInfo {
    /// The export name being referenced.
    pub export_name: CompactString,
    /// Line number of the reference.
    pub line: u32,
}

/// An import into a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    /// The module specifier.
    pub specifier: CompactString,
    /// Imported names (empty for side-effect imports).
    pub names: Vec<ImportedName>,
    /// Whether this is a type-only import.
    pub is_type: bool,
    /// Whether this is a side-effect-only import (e.g. `import './setup'`).
    pub is_side_effect: bool,
    /// Source line (1-indexed).
    pub line: u32,
}

/// A single imported name binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedName {
    /// The name as imported (or "default", or "*").
    pub imported: CompactString,
    /// The local name bound.
    pub local: CompactString,
}

/// A re-export statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReexportInfo {
    /// The module specifier.
    pub specifier: CompactString,
    /// Re-exported names, if named. Empty for `export * from`.
    pub names: Vec<ReexportedName>,
    /// Whether this is `export * from`.
    pub is_star: bool,
    /// Whether this is a type-only re-export.
    pub is_type: bool,
    /// Source line (1-indexed).
    pub line: u32,
}

/// A single re-exported name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReexportedName {
    /// Original name in the source module.
    pub original: CompactString,
    /// Exported name (may differ if aliased).
    pub exported: CompactString,
}

/// A dynamic `import()` expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicImportInfo {
    /// The specifier, if it's a string literal.
    pub specifier: Option<CompactString>,
    /// Source line (1-indexed).
    pub line: u32,
}

/// A `require()` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequireInfo {
    /// The specifier, if it's a string literal.
    pub specifier: Option<CompactString>,
    /// Source line (1-indexed).
    pub line: u32,
}

/// Non-standard dependency patterns detected during extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DependencyPattern {
    /// `require.resolve('specifier')` — resolves but doesn't import.
    RequireResolve { specifier: String, line: u32 },
    /// `import.meta.glob('./dir/*.ts')` — Vite glob import.
    ImportMetaGlob { pattern: String, line: u32 },
    /// `/// <reference path="..." />` or `/// <reference types="..." />`.
    TripleSlashReference { path: String, is_types: bool, line: u32 },
    /// `JSDoc` `@typedef {import('specifier').Type}` or `@type {import('...')}`.
    JsDocImport { specifier: String, line: u32 },
    /// `import.meta.resolve('specifier')` — resolves a URL without importing.
    ImportMetaResolve { specifier: String, line: u32 },
    /// `require.context('./dir', true, /\.ts$/)` — webpack dynamic context.
    RequireContext {
        directory: String,
        recursive: bool,
        #[serde(default)]
        regex_filter: Option<String>,
        line: u32,
    },
    /// `new URL('./worker.js', import.meta.url)` — worker/asset URL pattern.
    UrlConstructor { specifier: String, line: u32 },
    /// `import foo = require('bar')` — TypeScript import-equals.
    ImportEquals { specifier: String, line: u32 },
    /// `import.meta.glob(['./a/*.ts', './b/*.ts'])` — array-form Vite glob.
    ImportMetaGlobArray { patterns: Vec<String>, line: u32 },
}

/// Output from a source adapter, including extracted facts and synthetic edges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdapterOutput {
    /// Core extracted facts.
    pub facts: FileFacts,
    /// Synthetic import edges generated by the adapter (e.g. template component references).
    #[serde(default)]
    pub synthetic_imports: Vec<SyntheticImport>,
    /// Synthetic re-export edges.
    #[serde(default)]
    pub synthetic_reexports: Vec<SyntheticReexport>,
    /// Member-level facts extracted from templates (e.g. component props).
    #[serde(default)]
    pub member_facts: Vec<MemberExportInfo>,
    /// Aliases discovered by the adapter (e.g. component auto-registration).
    /// Each tuple is `(alias, original)`.
    #[serde(default)]
    pub synthetic_aliases: Vec<(String, String)>,
    /// Confidence level for this adapter's output.
    #[serde(default)]
    pub confidence: AdapterConfidence,
    /// Structured diagnostic messages from the adapter.
    #[serde(default)]
    pub diagnostics: Vec<AdapterDiagnostic>,
}

/// A synthetic import generated by a source adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticImport {
    /// The specifier to resolve (e.g. `./MyComponent.vue`).
    pub specifier: String,
    /// Imported names (empty = side-effect import).
    pub names: Vec<CompactString>,
    /// Source line number (approximate, from template scanning).
    pub line: u32,
    /// Reason this synthetic import was generated.
    pub reason: String,
}

/// A synthetic re-export generated by a source adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticReexport {
    /// The specifier to resolve.
    pub specifier: String,
    /// Re-exported names.
    pub names: Vec<CompactString>,
    /// Source line number.
    pub line: u32,
    /// Reason this synthetic re-export was generated.
    pub reason: String,
}

/// Confidence level for an adapter's output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterConfidence {
    #[default]
    High,
    Medium,
    Low,
}

/// A structured diagnostic message from a source adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterDiagnostic {
    /// Severity level.
    pub level: DiagnosticLevel,
    /// Human-readable message.
    pub message: String,
    /// Optional source line (1-indexed).
    pub line: Option<u32>,
}

/// Severity level for adapter diagnostics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

impl AdapterOutput {
    /// Create an `AdapterOutput` wrapping plain `FileFacts` with no synthetic edges.
    pub fn from_facts(facts: FileFacts) -> Self {
        Self { facts, ..Default::default() }
    }
}

/// Extracted and resolved facts for a tracked repository file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedFile {
    pub file: FileRecord,
    pub facts: Option<FileFacts>,
    pub parse_diagnostics: Vec<String>,
    pub resolved_imports: Vec<ResolvedEdge>,
    pub resolved_reexports: Vec<ResolvedEdge>,
    pub external_dependencies: Vec<String>,
}

impl ExtractedFile {
    /// Create an empty extracted record for a tracked file.
    pub const fn new(file: FileRecord) -> Self {
        Self {
            file,
            facts: None,
            parse_diagnostics: Vec::new(),
            resolved_imports: Vec::new(),
            resolved_reexports: Vec::new(),
            external_dependencies: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Source adapter trait
// ---------------------------------------------------------------------------

/// Trait for adapting non-JS/TS source formats into extractable facts.
///
/// Each adapter knows how to detect its format and extract script content
/// that can be fed through the core JS/TS extractor.
pub trait SourceAdapter: Send + Sync {
    /// Name of this adapter (e.g. "vue", "svelte", "astro", "mdx").
    fn name(&self) -> &'static str;

    /// Whether this adapter handles the given path based on extension.
    fn matches(&self, path: &Path) -> bool;

    /// Extract facts (and optional synthetic edges) from the source file.
    fn extract(&self, path: &Path, source: &str) -> Result<AdapterOutput, ExtractError>;
}

/// Vue single-file component adapter.
pub struct VueAdapter;

impl SourceAdapter for VueAdapter {
    fn name(&self) -> &'static str {
        "vue"
    }
    fn matches(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("vue")
    }
    fn extract(&self, path: &Path, source: &str) -> Result<AdapterOutput, ExtractError> {
        let mut diagnostics = Vec::new();
        let blocks = extract_vue_script_blocks(source);

        if blocks.is_empty() {
            diagnostics.push(AdapterDiagnostic {
                level: DiagnosticLevel::Warning,
                message: "no <script> or <script setup> block found".into(),
                line: None,
            });
        }

        let has_setup = blocks.iter().any(|b| b.is_setup);
        let has_regular_script = blocks.iter().any(|b| !b.is_setup);

        if has_setup {
            diagnostics.push(AdapterDiagnostic {
                level: DiagnosticLevel::Info,
                message: "detected <script setup> block (Composition API)".into(),
                line: None,
            });
        }

        let facts = extract_from_script_blocks(path, &blocks);

        // Start with the existing template component ref detection (for
        // unresolved references not matched to script imports).
        let mut synthetic_imports = detect_template_component_refs(source, &facts, "vue");

        // Extract template content for enhanced Vue-specific analysis.
        let template_content = extract_vue_template(source);

        if let Some(ref template) = template_content {
            // --- <script setup> component resolution ---
            // In <script setup>, all imports are automatically available in the
            // template. Connect template component tags to their imports.
            if has_setup {
                let setup_refs = detect_vue_setup_component_refs(template, &facts, &blocks);
                if !setup_refs.is_empty() {
                    diagnostics.push(AdapterDiagnostic {
                        level: DiagnosticLevel::Info,
                        message: format!(
                            "resolved {} template component(s) to <script setup> imports",
                            setup_refs.len()
                        ),
                        line: None,
                    });
                }
                synthetic_imports.extend(setup_refs);
            }

            // --- Dynamic component detection ---
            // Detect `<component :is="...">` and `v-bind:is` patterns.
            let dynamic_refs = detect_vue_dynamic_components(template);
            for (ref_name, line) in &dynamic_refs {
                // Skip Vue built-ins.
                if is_vue_builtin(ref_name) {
                    continue;
                }
                synthetic_imports.push(SyntheticImport {
                    specifier: ref_name.clone(),
                    names: vec![CompactString::new(ref_name)],
                    line: *line,
                    reason: format!("dynamic component :is=\"{ref_name}\""),
                });
            }

            // --- Options API `components: {{ }}` detection ---
            // For regular (non-setup) script blocks, detect locally registered
            // components and generate synthetic imports so the dependency graph
            // knows these imports are used.
            if has_regular_script && !has_setup {
                for block in &blocks {
                    if block.is_setup {
                        continue;
                    }
                    let registered = detect_options_api_components(&block.content);
                    for comp_name in &registered {
                        synthetic_imports.push(SyntheticImport {
                            specifier: comp_name.clone(),
                            names: vec![CompactString::new(comp_name)],
                            line: 0,
                            reason: format!("Options API components registration: '{comp_name}'"),
                        });
                    }
                    if !registered.is_empty() {
                        diagnostics.push(AdapterDiagnostic {
                            level: DiagnosticLevel::Info,
                            message: format!(
                                "detected {} locally registered component(s) in Options API",
                                registered.len()
                            ),
                            line: None,
                        });
                    }
                }
            }
        }

        // Detect <style module> or <style scoped> blocks (informational).
        if detect_vue_style_blocks(source) {
            diagnostics.push(AdapterDiagnostic {
                level: DiagnosticLevel::Info,
                message: "detected <style module> or <style scoped> block (CSS, not extracted)"
                    .into(),
                line: None,
            });
        }

        // Build kebab-to-pascal aliases for template component references.
        let mut synthetic_aliases = Vec::new();
        for si in &synthetic_imports {
            if si.specifier.contains('-') {
                let pascal = kebab_to_pascal(&si.specifier);
                synthetic_aliases.push((si.specifier.clone(), pascal));
            }
        }

        // Deduplicate synthetic imports by specifier.
        let mut seen_specifiers: FxHashSet<String> = FxHashSet::default();
        synthetic_imports.retain(|si| seen_specifiers.insert(si.specifier.clone()));

        Ok(AdapterOutput {
            facts,
            synthetic_imports,
            synthetic_aliases,
            confidence: AdapterConfidence::High,
            diagnostics,
            ..Default::default()
        })
    }
}

/// Svelte component adapter.
pub struct SvelteAdapter;

impl SourceAdapter for SvelteAdapter {
    fn name(&self) -> &'static str {
        "svelte"
    }
    fn matches(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("svelte")
    }
    fn extract(&self, path: &Path, source: &str) -> Result<AdapterOutput, ExtractError> {
        let mut diagnostics = Vec::new();

        // Parse <script> and <script context="module"> blocks separately.
        let instance_blocks = extract_html_script_blocks(source, &["<script"]);
        let module_blocks = extract_html_script_blocks(source, &["<script context=\"module\""]);

        // Avoid double-counting: remove module blocks from instance blocks.
        let instance_only: Vec<_> = instance_blocks
            .into_iter()
            .filter(|b| !module_blocks.iter().any(|m| m.content == b.content))
            .collect();

        let mut facts = extract_from_script_blocks(path, &instance_only);
        let module_facts = extract_from_script_blocks(path, &module_blocks);

        // Module-level exports become re-exports from the component.
        let mut synthetic_reexports = Vec::new();
        for export in &module_facts.exports {
            synthetic_reexports.push(SyntheticReexport {
                specifier: path.display().to_string(),
                names: vec![export.name.clone()],
                line: export.line,
                reason: format!("svelte module context export '{}'", export.name),
            });
        }

        merge_facts(&mut facts, module_facts);

        let synthetic_imports = detect_template_component_refs(source, &facts, "svelte");

        if instance_only.is_empty() && module_blocks.is_empty() {
            diagnostics.push(AdapterDiagnostic {
                level: DiagnosticLevel::Info,
                message: "no <script> blocks found in Svelte component".into(),
                line: None,
            });
        }

        // Detect {#snippet ...} blocks and <slot> usage in the markup.
        // Imports used inside snippets should still be considered used, and
        // components that expose slots/snippets may have exports consumed by
        // parent components.
        if let Some(ref template) = extract_svelte_template(source) {
            let has_snippets = detect_svelte_snippets(template);
            let has_slots = template.contains("<slot") || template.contains("<Slot");
            if has_snippets {
                diagnostics.push(AdapterDiagnostic {
                    level: DiagnosticLevel::Info,
                    message: "detected {#snippet} blocks; exports may be consumed as snippet props"
                        .into(),
                    line: None,
                });
            }
            if has_slots {
                diagnostics.push(AdapterDiagnostic {
                    level: DiagnosticLevel::Info,
                    message: "detected <slot> usage; component may expose content insertion points"
                        .into(),
                    line: None,
                });
            }
        }

        Ok(AdapterOutput {
            facts,
            synthetic_imports,
            synthetic_reexports,
            confidence: AdapterConfidence::High,
            diagnostics,
            ..Default::default()
        })
    }
}

/// Astro component adapter.
pub struct AstroAdapter;

impl SourceAdapter for AstroAdapter {
    fn name(&self) -> &'static str {
        "astro"
    }
    fn matches(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("astro")
    }
    fn extract(&self, path: &Path, source: &str) -> Result<AdapterOutput, ExtractError> {
        let mut diagnostics = Vec::new();

        // Extract frontmatter (--- ... ---) as TypeScript.
        let frontmatter_facts = extract_astro_facts(path, source)?;

        // Also extract inline <script> tags from the template portion.
        let template_content = extract_astro_template(source);
        let inline_script_facts = {
            let inline_blocks = extract_html_script_blocks(&template_content, &["<script"]);
            if !inline_blocks.is_empty() {
                diagnostics.push(AdapterDiagnostic {
                    level: DiagnosticLevel::Info,
                    message: format!(
                        "extracted {} inline <script> block(s) from template",
                        inline_blocks.len()
                    ),
                    line: None,
                });
            }
            extract_from_script_blocks(path, &inline_blocks)
        };

        let mut facts = frontmatter_facts;
        merge_facts(&mut facts, inline_script_facts);

        let mut synthetic_imports = detect_template_component_refs(source, &facts, "astro");

        // Detect client:* hydration directives on components in the template.
        // Components with client:load, client:idle, client:visible, etc. are
        // hydration boundaries and are definitely used at runtime.
        {
            let hydrated = detect_astro_client_directives(&template_content);
            for (component_name, directive, line) in &hydrated {
                diagnostics.push(AdapterDiagnostic {
                    level: DiagnosticLevel::Info,
                    message: format!(
                        "component <{component_name}> uses {directive} hydration directive"
                    ),
                    line: Some(*line),
                });
                // If the hydrated component isn't already in the synthetic imports
                // (from detect_template_component_refs), add it.
                let already_present = synthetic_imports
                    .iter()
                    .any(|si| si.names.iter().any(|n| n.as_str() == component_name.as_str()));
                if !already_present {
                    synthetic_imports.push(SyntheticImport {
                        specifier: component_name.clone(),
                        names: vec![CompactString::new(component_name)],
                        line: *line,
                        reason: format!(
                            "Astro hydrated component <{component_name}> ({directive})"
                        ),
                    });
                }
            }
        }

        Ok(AdapterOutput {
            facts,
            synthetic_imports,
            confidence: AdapterConfidence::High,
            diagnostics,
            ..Default::default()
        })
    }
}

/// MDX adapter.
pub struct MdxAdapter;

impl SourceAdapter for MdxAdapter {
    fn name(&self) -> &'static str {
        "mdx"
    }
    fn matches(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()) == Some("mdx")
    }
    fn extract(&self, path: &Path, source: &str) -> Result<AdapterOutput, ExtractError> {
        let mut diagnostics = Vec::new();

        // Extract ESM imports/exports, excluding fenced code blocks and inline code.
        let facts = extract_mdx_facts(path, source)?;

        // Extract frontmatter layout as a synthetic import.
        let mut synthetic_imports = Vec::new();
        if let Some(layout_specifier) = extract_mdx_frontmatter_layout(source) {
            diagnostics.push(AdapterDiagnostic {
                level: DiagnosticLevel::Info,
                message: format!("frontmatter layout: {layout_specifier}"),
                line: Some(1),
            });
            synthetic_imports.push(SyntheticImport {
                specifier: layout_specifier.clone(),
                names: vec![CompactString::new("default")],
                line: 1,
                reason: format!("MDX frontmatter layout '{layout_specifier}'"),
            });
        }

        // Detect JSX component refs, skipping fenced code blocks.
        let jsx_refs = detect_mdx_component_refs(source, &facts);
        synthetic_imports.extend(jsx_refs);

        Ok(AdapterOutput {
            facts,
            synthetic_imports,
            confidence: AdapterConfidence::High,
            diagnostics,
            ..Default::default()
        })
    }
}

/// Return the built-in set of source adapters.
pub fn built_in_adapters() -> Vec<Box<dyn SourceAdapter>> {
    vec![
        Box::new(VueAdapter),
        Box::new(SvelteAdapter),
        Box::new(AstroAdapter),
        Box::new(MdxAdapter),
    ]
}

/// Extract all import/export facts from a tracked source file.
///
/// For JS/TS files this parses the full file. For framework SFCs (`.vue`,
/// `.svelte`, `.astro`, `.mdx`) this first extracts the embedded script
/// blocks and then feeds them through the JS/TS extractor, also scanning
/// templates for synthetic component references.
pub fn extract_file_facts(path: &Path, source: &str) -> Result<AdapterOutput, ExtractError> {
    extract_file_facts_with_adapters(path, source, &built_in_adapters())
}

/// Extract facts using a custom set of source adapters.
///
/// Tries each adapter in order; falls through to core JS/TS extraction
/// if no adapter matches.
pub fn extract_file_facts_with_adapters(
    path: &Path,
    source: &str,
    adapters: &[Box<dyn SourceAdapter>],
) -> Result<AdapterOutput, ExtractError> {
    for adapter in adapters {
        if adapter.matches(path) {
            return adapter.extract(path, source);
        }
    }
    extract_js_ts_facts(path, source).map(AdapterOutput::from_facts)
}

/// Core JS/TS extraction — parse the full source and walk the AST.
fn extract_js_ts_facts(path: &Path, source: &str) -> Result<FileFacts, ExtractError> {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = determine_source_type(path);
    let parser_ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if parser_ret.panicked {
        return Err(ExtractError::ParseFailed { path: path.to_path_buf() });
    }

    let program = &parser_ret.program;
    let mut facts = FileFacts::default();

    for stmt in &program.body {
        extract_from_statement(stmt, &mut facts);
    }

    refine_runtime_specifier_calls(source, &mut facts);
    refine_namespace_imports(source, &mut facts);

    let mut patterns = detect_require_resolve(source);
    patterns.extend(detect_import_meta_glob(source));
    patterns.extend(detect_triple_slash_references(source));
    patterns.extend(detect_jsdoc_imports(source));
    patterns.extend(detect_import_meta_resolve(source));
    patterns.extend(detect_require_context(source));
    patterns.extend(detect_url_constructor(source));
    patterns.extend(detect_import_equals(program));
    facts.dependency_patterns = patterns;

    // Build semantic model to detect same-file references to exported symbols.
    if !facts.exports.is_empty() {
        detect_same_file_refs(program, &mut facts);
    }

    // Detect member access patterns (e.g., Color.Red) for imported names.
    detect_member_accesses(source, &facts.imports, &mut facts.member_accesses);
    // Detect instance member accesses (e.g., const svc = new Service(); svc.method()).
    detect_instance_member_accesses(source, &facts.imports, &mut facts.member_accesses);

    Ok(facts)
}

/// Detect same-file references to exported symbols using the semantic model.
///
/// For each exported name, find the corresponding binding in the root scope
/// and collect all non-declaration references to it.
fn detect_same_file_refs(program: &oxc_ast::ast::Program<'_>, facts: &mut FileFacts) {
    let semantic_ret = oxc_semantic::SemanticBuilder::new().build(program);
    let semantic = &semantic_ret.semantic;
    let scoping = semantic.scoping();

    // Collect export names (excluding "default" which is harder to track by binding name).
    let export_names: FxHashSet<&str> = facts
        .exports
        .iter()
        .filter(|e| e.name.as_str() != "default")
        .map(|e| e.name.as_str())
        .collect();

    if export_names.is_empty() {
        return;
    }

    // Collect imported names so we can exclude them — they are not same-file symbols.
    let imported_locals: FxHashSet<&str> =
        facts.imports.iter().flat_map(|imp| imp.names.iter().map(|n| n.local.as_str())).collect();

    let root_scope_id = scoping.root_scope_id();

    for symbol_id in scoping.symbol_ids() {
        let name = scoping.symbol_name(symbol_id);

        // Only care about symbols that are exported and not imports.
        if !export_names.contains(name) || imported_locals.contains(name) {
            continue;
        }

        // Only look at symbols declared in the root scope (module-level).
        if scoping.symbol_scope_id(symbol_id) != root_scope_id {
            continue;
        }

        let decl_span = scoping.symbol_span(symbol_id);
        let export_name = CompactString::new(name);

        for reference in scoping.get_resolved_references(symbol_id) {
            let ref_span = semantic.reference_span(reference);

            // Skip the declaration site itself.
            if ref_span == decl_span {
                continue;
            }

            facts
                .same_file_refs
                .push(SameFileRefInfo { export_name: export_name.clone(), line: ref_span.start });
        }
    }
}

/// Determine the source type from a file path.
fn determine_source_type(path: &Path) -> oxc_span::SourceType {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("js");

    match ext {
        "ts" | "mts" | "cts" => oxc_span::SourceType::ts(),
        "tsx" => oxc_span::SourceType::tsx(),
        "jsx" => oxc_span::SourceType::jsx(),
        "cjs" => oxc_span::SourceType::cjs(),
        // mjs and all other extensions default to ESM
        _ => oxc_span::SourceType::mjs(),
    }
}

/// Extract facts from a single AST statement.
#[allow(clippy::too_many_lines)]
fn extract_from_statement(stmt: &oxc_ast::ast::Statement<'_>, facts: &mut FileFacts) {
    use oxc_ast::ast::Statement;

    match stmt {
        Statement::ImportDeclaration(import) => {
            let specifier = CompactString::new(import.source.value.as_str());
            let is_type = import.import_kind.is_type();
            let mut names = Vec::new();

            if let Some(specifiers) = &import.specifiers {
                for spec in specifiers {
                    use oxc_ast::ast::ImportDeclarationSpecifier;
                    match spec {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            names.push(ImportedName {
                                imported: CompactString::new(s.imported.name().as_str()),
                                local: CompactString::new(s.local.name.as_str()),
                            });
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            names.push(ImportedName {
                                imported: CompactString::new("default"),
                                local: CompactString::new(s.local.name.as_str()),
                            });
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                            names.push(ImportedName {
                                imported: CompactString::new("*"),
                                local: CompactString::new(s.local.name.as_str()),
                            });
                        }
                    }
                }
            }

            let is_side_effect = names.is_empty();
            facts.imports.push(ImportInfo {
                specifier,
                names,
                is_type,
                is_side_effect,
                line: import.span.start,
            });
        }

        Statement::ExportNamedDeclaration(export) => {
            if let Some(source) = &export.source {
                // This is a re-export: export { x } from 'y'
                let mut names = Vec::new();
                for spec in &export.specifiers {
                    names.push(ReexportedName {
                        original: CompactString::new(spec.local.name().as_str()),
                        exported: CompactString::new(spec.exported.name().as_str()),
                    });
                }
                facts.reexports.push(ReexportInfo {
                    specifier: CompactString::new(source.value.as_str()),
                    names,
                    is_star: false,
                    is_type: export.export_kind.is_type(),
                    line: export.span.start,
                });
            } else {
                // Named export from this file
                for spec in &export.specifiers {
                    facts.exports.push(ExportInfo {
                        name: CompactString::new(spec.exported.name().as_str()),
                        is_type: export.export_kind.is_type(),
                        export_kind: if export.export_kind.is_type() {
                            ExportKind::Type
                        } else {
                            ExportKind::Value
                        },
                        line: export.span.start,
                    });
                }
                if let Some(decl) = &export.declaration {
                    extract_exports_from_declaration(decl, export.export_kind.is_type(), facts);
                }
            }
        }

        Statement::ExportDefaultDeclaration(export) => {
            facts.exports.push(ExportInfo {
                name: CompactString::new("default"),
                is_type: false,
                export_kind: ExportKind::Default,
                line: export.span.start,
            });

            // Extract members from default-exported classes.
            if let oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) =
                &export.declaration
            {
                let parent = if let Some(id) = &class.id {
                    CompactString::new(id.name.as_str())
                } else {
                    CompactString::new("default")
                };
                extract_class_members(&parent, class, facts);
            }
        }

        Statement::ExportAllDeclaration(export) => {
            // `export * as Name from './mod'` — namespace re-export with alias.
            let names = if let Some(exported) = &export.exported {
                let name = exported.name();
                vec![ReexportedName {
                    original: CompactString::new("*"),
                    exported: CompactString::new(name.as_str()),
                }]
            } else {
                Vec::new()
            };
            facts.reexports.push(ReexportInfo {
                specifier: CompactString::new(export.source.value.as_str()),
                names,
                is_star: true,
                is_type: export.export_kind.is_type(),
                line: export.span.start,
            });
        }

        _ => {}
    }
}

fn refine_namespace_imports(source: &str, facts: &mut FileFacts) {
    let stripped = strip_comments(source);
    for import in &mut facts.imports {
        let Some(namespace_alias) = import
            .names
            .iter()
            .find(|name| name.imported == "*")
            .map(|name| name.local.to_string())
        else {
            continue;
        };

        let usage = collect_namespace_usage(&stripped, &namespace_alias);
        if usage.dynamic || usage.members.is_empty() {
            continue;
        }

        let mut members = usage.members.into_iter().collect::<Vec<_>>();
        members.sort();
        members.dedup();
        import.names = members
            .into_iter()
            .map(|member| ImportedName {
                imported: CompactString::new(&member),
                local: CompactString::new(format!("{namespace_alias}.{member}")),
            })
            .collect();
    }
}

/// Strip single-line (`//`) and block (`/* */`) comments from source text,
/// preserving line structure so that `is_in_import_context` still works.
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_string: Option<u8> = None;
    let mut escaped = false;

    while i < bytes.len() {
        if escaped {
            result.push(bytes[i]);
            escaped = false;
            i += 1;
            continue;
        }

        if let Some(quote) = in_string {
            if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == quote {
                in_string = None;
            }
            result.push(bytes[i]);
            i += 1;
            continue;
        }

        if bytes[i] == b'"' || bytes[i] == b'\'' || bytes[i] == b'`' {
            in_string = Some(bytes[i]);
            result.push(bytes[i]);
            i += 1;
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            // Single-line comment: skip until end of line, preserve newline
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                result.push(b' ');
                i += 1;
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Block comment: skip until */, preserve newlines
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    result.push(b'\n');
                } else {
                    result.push(b' ');
                }
                i += 1;
            }
            if i + 1 < bytes.len() {
                result.push(b' ');
                result.push(b' ');
                i += 2; // skip */
            }
            continue;
        }

        result.push(bytes[i]);
        i += 1;
    }

    String::from_utf8(result).unwrap_or_else(|_| source.to_string())
}

fn refine_runtime_specifier_calls(source: &str, facts: &mut FileFacts) {
    facts.dynamic_imports.extend(find_string_literal_calls(source, "import"));
    facts.requires.extend(find_string_literal_calls(source, "require"));
}

#[derive(Default)]
struct NamespaceUsage {
    members: FxHashSet<String>,
    dynamic: bool,
}

fn collect_namespace_usage(source: &str, alias: &str) -> NamespaceUsage {
    let bytes = source.as_bytes();
    let alias_bytes = alias.as_bytes();
    let mut usage = NamespaceUsage::default();
    let mut index = 0;

    while index + alias_bytes.len() < bytes.len() {
        if !bytes[index..].starts_with(alias_bytes) {
            index += 1;
            continue;
        }

        let before = index.checked_sub(1).and_then(|i| bytes.get(i).copied());
        let after = bytes.get(index + alias_bytes.len()).copied();
        let boundary_before = before.is_none_or(|byte| !is_identifier_byte(byte));
        if !boundary_before {
            index += 1;
            continue;
        }

        // Check for spread operator: `...alias` — keeps entire module live.
        if before == Some(b'.') {
            let dots_start = index.saturating_sub(3);
            let prefix = &bytes[dots_start..index];
            if prefix.ends_with(b"...") {
                usage.dynamic = true;
                index += alias_bytes.len();
                continue;
            }
        }

        match after {
            Some(b'.') => {
                let member_start = index + alias_bytes.len() + 1;
                let mut member_end = member_start;
                while member_end < bytes.len() && is_identifier_byte(bytes[member_end]) {
                    member_end += 1;
                }
                if member_end > member_start {
                    usage.members.insert(source[member_start..member_end].to_string());
                    index = member_end;
                    continue;
                }
            }
            Some(b'[') => {
                // Dynamic bracket access: `alias[key]` — keeps entire module live.
                usage.dynamic = true;
                index += alias_bytes.len();
                continue;
            }
            // The namespace passed as an argument: `fn(alias)` or `fn(alias,`
            // or assigned: `x = alias` — keeps entire module live.
            Some(b')' | b',') => {
                let after_boundary = after.is_none_or(|byte| !is_identifier_byte(byte));
                if after_boundary {
                    usage.dynamic = true;
                    index += alias_bytes.len();
                    continue;
                }
            }
            _ => {
                // If the alias appears as a standalone identifier (not followed by
                // `.` or `[`), it might be passed around, so mark as dynamic.
                let after_boundary =
                    after.is_none_or(|byte| !is_identifier_byte(byte) && byte != b'.');
                if after_boundary && after != Some(b':') && !matches!(before, Some(b'.' | b'=')) {
                    // Check if this is in an import/export context (which we already
                    // handle via the AST) by looking at the surrounding context.
                    // If not, it's a dynamic use.
                    if !is_in_import_context(bytes, index) {
                        usage.dynamic = true;
                        index += alias_bytes.len();
                        continue;
                    }
                }
            }
        }

        index += 1;
    }

    usage
}

/// Crude check: is this position likely inside an `import` or `export` statement?
/// We look backward for `import ` or `export ` on the same line.
fn is_in_import_context(bytes: &[u8], pos: usize) -> bool {
    let line_start = bytes[..pos].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let line_prefix = &bytes[line_start..pos];
    line_prefix.windows(7).any(|w| w == b"import ")
        || line_prefix.windows(7).any(|w| w == b"export ")
}

const fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// Count newlines in a byte slice to determine a 1-indexed line number.
fn count_newlines(bytes: &[u8]) -> usize {
    bytes.iter().fold(0, |acc, &b| acc + usize::from(b == b'\n'))
}

fn find_string_literal_calls<T>(source: &str, callee: &str) -> Vec<T>
where
    T: FromCallMatch,
{
    let bytes = source.as_bytes();
    let callee_bytes = callee.as_bytes();
    let mut matches = Vec::new();
    let mut index = 0;

    while index + callee_bytes.len() + 3 < bytes.len() {
        if !bytes[index..].starts_with(callee_bytes) {
            index += 1;
            continue;
        }

        let before = index.checked_sub(1).and_then(|i| bytes.get(i).copied());
        if before.is_some_and(is_identifier_byte) {
            index += 1;
            continue;
        }

        let open_paren = index + callee_bytes.len();
        if bytes.get(open_paren) != Some(&b'(') {
            index += 1;
            continue;
        }

        // Skip whitespace and block comments between `(` and the string literal.
        // This handles webpack magic comments: import(/* webpackChunkName: "x" */ './y')
        let mut after_paren = open_paren + 1;
        while after_paren < bytes.len() {
            let b = bytes[after_paren];
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                after_paren += 1;
            } else if b == b'/' && after_paren + 1 < bytes.len() && bytes[after_paren + 1] == b'*' {
                // Skip block comment /* ... */
                after_paren += 2;
                while after_paren + 1 < bytes.len()
                    && !(bytes[after_paren] == b'*' && bytes[after_paren + 1] == b'/')
                {
                    after_paren += 1;
                }
                if after_paren + 1 < bytes.len() {
                    after_paren += 2; // skip */
                }
            } else {
                break;
            }
        }

        let Some(quote) = bytes.get(after_paren).copied() else {
            break;
        };
        if !matches!(quote, b'"' | b'\'') {
            index += 1;
            continue;
        }

        let literal_start = after_paren + 1;
        let mut literal_end = literal_start;
        while literal_end < bytes.len() && bytes[literal_end] != quote {
            literal_end += 1;
        }
        if literal_end >= bytes.len() || bytes.get(literal_end + 1) != Some(&b')') {
            index += 1;
            continue;
        }

        let specifier = source[literal_start..literal_end].to_string();
        let line = 1 + count_newlines(&bytes[..index]);
        matches.push(T::from_call_match(specifier, u32::try_from(line).unwrap_or(u32::MAX)));
        index = literal_end + 2;
    }

    matches
}

trait FromCallMatch {
    fn from_call_match(specifier: String, line: u32) -> Self;
}

impl FromCallMatch for DynamicImportInfo {
    fn from_call_match(specifier: String, line: u32) -> Self {
        Self { specifier: Some(CompactString::new(specifier)), line }
    }
}

impl FromCallMatch for RequireInfo {
    fn from_call_match(specifier: String, line: u32) -> Self {
        Self { specifier: Some(CompactString::new(specifier)), line }
    }
}

/// Extract export names from a declaration.
fn extract_exports_from_declaration(
    decl: &oxc_ast::ast::Declaration<'_>,
    is_type: bool,
    facts: &mut FileFacts,
) {
    use oxc_ast::ast::Declaration;

    match decl {
        Declaration::VariableDeclaration(var_decl) => {
            for declarator in &var_decl.declarations {
                if let Some(name) = extract_binding_name(&declarator.id) {
                    facts.exports.push(ExportInfo {
                        name: CompactString::new(&name),
                        is_type,
                        export_kind: if is_type { ExportKind::Type } else { ExportKind::Value },
                        line: declarator.span.start,
                    });
                }
            }
        }
        Declaration::FunctionDeclaration(func) => {
            if let Some(id) = &func.id {
                facts.exports.push(ExportInfo {
                    name: CompactString::new(id.name.as_str()),
                    is_type,
                    export_kind: if is_type { ExportKind::Type } else { ExportKind::Value },
                    line: func.span.start,
                });
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                let class_name = CompactString::new(id.name.as_str());
                facts.exports.push(ExportInfo {
                    name: class_name.clone(),
                    is_type,
                    export_kind: if is_type { ExportKind::Type } else { ExportKind::Class },
                    line: class.span.start,
                });
                extract_class_members(&class_name, class, facts);
            }
        }
        Declaration::TSTypeAliasDeclaration(alias) => {
            facts.exports.push(ExportInfo {
                name: CompactString::new(alias.id.name.as_str()),
                is_type: true,
                export_kind: ExportKind::Type,
                line: alias.span.start,
            });
        }
        Declaration::TSInterfaceDeclaration(iface) => {
            facts.exports.push(ExportInfo {
                name: CompactString::new(iface.id.name.as_str()),
                is_type: true,
                export_kind: ExportKind::Type,
                line: iface.span.start,
            });
        }
        Declaration::TSEnumDeclaration(enum_decl) => {
            let enum_name = CompactString::new(enum_decl.id.name.as_str());
            facts.exports.push(ExportInfo {
                name: enum_name.clone(),
                is_type,
                export_kind: if is_type { ExportKind::Type } else { ExportKind::Enum },
                line: enum_decl.span.start,
            });
            extract_enum_members(&enum_name, enum_decl, facts);
        }
        Declaration::TSModuleDeclaration(module_decl) => {
            let ns_name = CompactString::new(module_decl.id.name().as_str());
            facts.exports.push(ExportInfo {
                name: ns_name.clone(),
                is_type,
                export_kind: if is_type { ExportKind::Type } else { ExportKind::Namespace },
                line: module_decl.span.start,
            });
            extract_namespace_members(&ns_name, module_decl, facts);
        }
        _ => {}
    }
}

/// Extract a simple binding name from a pattern.
fn extract_binding_name(pattern: &oxc_ast::ast::BindingPattern<'_>) -> Option<String> {
    use oxc_ast::ast::BindingPattern;
    match pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
        _ => None, // Destructuring patterns not handled yet
    }
}

/// Extract members from an exported class declaration.
fn extract_class_members(
    class_name: &CompactString,
    class: &oxc_ast::ast::Class<'_>,
    facts: &mut FileFacts,
) {
    use oxc_ast::ast::{ClassElement, MethodDefinitionKind};

    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method) => {
                // Skip constructors — they aren't individual "members" for dead-code purposes.
                if method.kind == MethodDefinitionKind::Constructor {
                    continue;
                }
                let Some(name) = method.key.static_name() else {
                    continue;
                };
                let member_kind = match (method.kind, method.r#static) {
                    (MethodDefinitionKind::Get, _) => MemberKind::Getter,
                    (MethodDefinitionKind::Set, _) => MemberKind::Setter,
                    (_, true) => MemberKind::StaticMethod,
                    _ => MemberKind::Method,
                };
                facts.member_exports.push(MemberExportInfo {
                    parent_name: class_name.clone(),
                    member_name: CompactString::new(&*name),
                    member_kind,
                    line: method.span.start,
                    is_public_tagged: false,
                });
            }
            ClassElement::PropertyDefinition(prop) => {
                // Skip private properties — they are internal implementation
                // details (e.g. backing fields for getters/setters) and should
                // not be tracked for dead-member analysis.
                if prop.accessibility.is_some_and(|a| a == oxc_ast::ast::TSAccessibility::Private) {
                    continue;
                }
                let Some(name) = prop.key.static_name() else {
                    continue;
                };
                let member_kind =
                    if prop.r#static { MemberKind::StaticProperty } else { MemberKind::Property };
                facts.member_exports.push(MemberExportInfo {
                    parent_name: class_name.clone(),
                    member_name: CompactString::new(&*name),
                    member_kind,
                    line: prop.span.start,
                    is_public_tagged: false,
                });
            }
            ClassElement::AccessorProperty(accessor) => {
                let Some(name) = accessor.key.static_name() else {
                    continue;
                };
                let member_kind = if accessor.r#static {
                    MemberKind::StaticProperty
                } else {
                    MemberKind::Property
                };
                facts.member_exports.push(MemberExportInfo {
                    parent_name: class_name.clone(),
                    member_name: CompactString::new(&*name),
                    member_kind,
                    line: accessor.span.start,
                    is_public_tagged: false,
                });
            }
            // StaticBlock and TSIndexSignature don't contribute named members.
            _ => {}
        }
    }
}

/// Extract members from an exported enum declaration.
fn extract_enum_members(
    enum_name: &CompactString,
    enum_decl: &oxc_ast::ast::TSEnumDeclaration<'_>,
    facts: &mut FileFacts,
) {
    for member in &enum_decl.body.members {
        let variant_name = member.id.static_name();
        facts.member_exports.push(MemberExportInfo {
            parent_name: enum_name.clone(),
            member_name: CompactString::new(variant_name.as_str()),
            member_kind: MemberKind::EnumVariant,
            line: member.span.start,
            is_public_tagged: false,
        });
    }
}

/// Extract members from an exported namespace / module declaration.
fn extract_namespace_members(
    ns_name: &CompactString,
    module_decl: &oxc_ast::ast::TSModuleDeclaration<'_>,
    facts: &mut FileFacts,
) {
    use oxc_ast::ast::{Statement, TSModuleDeclarationBody};

    let Some(body) = &module_decl.body else {
        return;
    };

    // Walk through nested TSModuleDeclarations to reach the block body.
    let block = match body {
        TSModuleDeclarationBody::TSModuleBlock(block) => block,
        TSModuleDeclarationBody::TSModuleDeclaration(_) => {
            // Nested namespace (e.g. `namespace A.B { ... }`). We only extract
            // top-level members here; the inner namespace becomes a member itself.
            return;
        }
    };

    for stmt in &block.body {
        // `export const foo = ...;` / `export function bar() {}` etc.
        if let Statement::ExportNamedDeclaration(export) = stmt {
            if let Some(decl) = &export.declaration {
                extract_namespace_decl_members(ns_name, decl, facts);
            }
            for spec in &export.specifiers {
                facts.member_exports.push(MemberExportInfo {
                    parent_name: ns_name.clone(),
                    member_name: CompactString::new(spec.exported.name().as_str()),
                    member_kind: MemberKind::NamespaceMember,
                    line: spec.span.start,
                    is_public_tagged: false,
                });
            }
        }
    }
}

/// Extract member info from a declaration inside a namespace body.
fn extract_namespace_decl_members(
    ns_name: &CompactString,
    decl: &oxc_ast::ast::Declaration<'_>,
    facts: &mut FileFacts,
) {
    use oxc_ast::ast::Declaration;

    match decl {
        Declaration::VariableDeclaration(var_decl) => {
            for declarator in &var_decl.declarations {
                if let Some(name) = extract_binding_name(&declarator.id) {
                    facts.member_exports.push(MemberExportInfo {
                        parent_name: ns_name.clone(),
                        member_name: CompactString::new(&name),
                        member_kind: MemberKind::NamespaceMember,
                        line: declarator.span.start,
                        is_public_tagged: false,
                    });
                }
            }
        }
        Declaration::FunctionDeclaration(func) => {
            if let Some(id) = &func.id {
                facts.member_exports.push(MemberExportInfo {
                    parent_name: ns_name.clone(),
                    member_name: CompactString::new(id.name.as_str()),
                    member_kind: MemberKind::NamespaceMember,
                    line: func.span.start,
                    is_public_tagged: false,
                });
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                facts.member_exports.push(MemberExportInfo {
                    parent_name: ns_name.clone(),
                    member_name: CompactString::new(id.name.as_str()),
                    member_kind: MemberKind::NamespaceMember,
                    line: class.span.start,
                    is_public_tagged: false,
                });
            }
        }
        Declaration::TSEnumDeclaration(enum_decl) => {
            facts.member_exports.push(MemberExportInfo {
                parent_name: ns_name.clone(),
                member_name: CompactString::new(enum_decl.id.name.as_str()),
                member_kind: MemberKind::NamespaceMember,
                line: enum_decl.span.start,
                is_public_tagged: false,
            });
        }
        Declaration::TSTypeAliasDeclaration(alias) => {
            facts.member_exports.push(MemberExportInfo {
                parent_name: ns_name.clone(),
                member_name: CompactString::new(alias.id.name.as_str()),
                member_kind: MemberKind::NamespaceMember,
                line: alias.span.start,
                is_public_tagged: false,
            });
        }
        Declaration::TSInterfaceDeclaration(iface) => {
            facts.member_exports.push(MemberExportInfo {
                parent_name: ns_name.clone(),
                member_name: CompactString::new(iface.id.name.as_str()),
                member_kind: MemberKind::NamespaceMember,
                line: iface.span.start,
                is_public_tagged: false,
            });
        }
        Declaration::TSModuleDeclaration(inner_ns) => {
            facts.member_exports.push(MemberExportInfo {
                parent_name: ns_name.clone(),
                member_name: CompactString::new(inner_ns.id.name().as_str()),
                member_kind: MemberKind::NamespaceMember,
                line: inner_ns.span.start,
                is_public_tagged: false,
            });
        }
        _ => {}
    }
}

/// Collect all specifiers referenced from a file's facts.
pub fn collect_specifiers(facts: &FileFacts) -> FxHashSet<CompactString> {
    let mut specifiers = FxHashSet::default();
    for import in &facts.imports {
        specifiers.insert(import.specifier.clone());
    }
    for reexport in &facts.reexports {
        specifiers.insert(reexport.specifier.clone());
    }
    for dynamic in &facts.dynamic_imports {
        if let Some(spec) = &dynamic.specifier {
            specifiers.insert(spec.clone());
        }
    }
    for require in &facts.requires {
        if let Some(spec) = &require.specifier {
            specifiers.insert(spec.clone());
        }
    }
    specifiers
}

// ---------------------------------------------------------------------------
// Framework SFC extractors
// ---------------------------------------------------------------------------

/// Shared helper: extract facts from a list of script blocks.
fn extract_from_script_blocks(path: &Path, blocks: &[ScriptBlock]) -> FileFacts {
    let mut combined = FileFacts::default();
    for block in blocks {
        let lang = block.lang.unwrap_or("js");
        let virtual_path = make_virtual_path(path, lang);
        if let Ok(facts) = extract_js_ts_facts(&virtual_path, &block.content) {
            merge_facts(&mut combined, facts);
        }
    }
    combined
}

/// Extract facts from an Astro component's frontmatter (`--- ... ---`).
fn extract_astro_facts(path: &Path, source: &str) -> Result<FileFacts, ExtractError> {
    let Some(frontmatter) = extract_astro_frontmatter(source) else {
        return Ok(FileFacts::default());
    };

    let virtual_path = make_virtual_path(path, "ts");
    extract_js_ts_facts(&virtual_path, &frontmatter)
}

/// Extract facts from an MDX file's top-level ESM imports/exports.
fn extract_mdx_facts(path: &Path, source: &str) -> Result<FileFacts, ExtractError> {
    let esm_block = extract_mdx_esm_lines(source);
    if esm_block.is_empty() {
        return Ok(FileFacts::default());
    }

    let virtual_path = make_virtual_path(path, "tsx");
    extract_js_ts_facts(&virtual_path, &esm_block)
}

// ---------------------------------------------------------------------------
// Script-block parsing helpers
// ---------------------------------------------------------------------------

struct ScriptBlock {
    content: String,
    lang: Option<&'static str>,
    /// Whether this is a `<script setup>` block (Vue 3 Composition API).
    is_setup: bool,
}

/// Extract `<script>` and `<script setup>` blocks from a Vue SFC.
fn extract_vue_script_blocks(source: &str) -> Vec<ScriptBlock> {
    extract_html_script_blocks(source, &["<script", "<script setup"])
}

/// Generic HTML-like `<script ...>...</script>` block extractor.
///
/// Handles `lang="ts"` / `lang="tsx"` attributes and detects `setup` attribute.
fn extract_html_script_blocks(source: &str, open_tags: &[&str]) -> Vec<ScriptBlock> {
    let mut blocks = Vec::new();
    let lower = source.to_ascii_lowercase();
    let bytes = lower.as_bytes();

    for &tag_prefix in open_tags {
        let mut search_start = 0;
        while let Some(tag_start) = find_substr(bytes, search_start, tag_prefix.as_bytes()) {
            // Find the closing `>` of the opening tag.
            let Some(open_end) = find_byte(bytes, tag_start + tag_prefix.len(), b'>') else {
                break;
            };
            let tag_attrs = &source[tag_start..=open_end];
            let lang = detect_lang_attr(tag_attrs);
            let is_setup = detect_setup_attr(tag_attrs);
            let content_start = open_end + 1;

            // Find the matching `</script>`.
            let Some(close_start) = find_substr(bytes, content_start, b"</script>") else {
                break;
            };

            blocks.push(ScriptBlock {
                content: source[content_start..close_start].to_string(),
                lang,
                is_setup,
            });

            search_start = close_start + b"</script>".len();
        }
    }

    blocks
}

/// Detect whether a `<script>` opening tag contains the `setup` attribute.
fn detect_setup_attr(tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    // Match `setup` as a standalone attribute (not part of another word).
    // Valid forms: `<script setup>`, `<script setup lang="ts">`, etc.
    lower.contains(" setup") || lower.contains("\tsetup") || lower.contains("\nsetup")
}

/// Extract Astro frontmatter delimited by `---`.
fn extract_astro_frontmatter(source: &str) -> Option<String> {
    let trimmed = source.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_first = &trimmed[3..];
    let close = after_first.find("---")?;
    Some(after_first[..close].to_string())
}

/// Extract top-level ESM `import` and `export` lines from an MDX file.
///
/// MDX allows standard ESM imports/exports at the top level, before any
/// markdown or JSX content. We collect contiguous import/export blocks,
/// skipping lines inside fenced code blocks (` ``` `) and YAML frontmatter.
fn extract_mdx_esm_lines(source: &str) -> String {
    let mut esm_lines = Vec::new();
    let mut in_fenced_code = false;
    let mut in_frontmatter = false;
    let mut is_first_line = true;

    for line in source.lines() {
        let trimmed = line.trim();

        // Handle YAML frontmatter (--- ... ---) at the start of the file.
        if is_first_line && trimmed == "---" {
            in_frontmatter = true;
            is_first_line = false;
            continue;
        }
        is_first_line = false;

        if in_frontmatter {
            if trimmed == "---" {
                in_frontmatter = false;
            }
            continue;
        }

        // Toggle fenced code blocks.
        if trimmed.starts_with("```") {
            in_fenced_code = !in_fenced_code;
            continue;
        }
        if in_fenced_code {
            continue;
        }

        // Strip inline code spans before checking for import/export.
        let stripped = strip_inline_code(trimmed);
        let check = stripped.trim();

        if check.starts_with("import ")
            || check.starts_with("import{")
            || check.starts_with("export ")
            || check.starts_with("export{")
            || check.starts_with("export default ")
        {
            esm_lines.push(line);
        }
    }
    esm_lines.join("\n")
}

/// Strip inline code spans (`` `...` ``) from a line, replacing them with spaces.
fn strip_inline_code(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'`' {
            // Find the closing backtick.
            let close = bytes[i + 1..].iter().position(|&b| b == b'`');
            if let Some(offset) = close {
                // Replace the inline code span with spaces.
                let span_len = offset + 2; // includes both backticks
                result.extend(std::iter::repeat_n(b' ', span_len));
                i += span_len;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }

    String::from_utf8(result).unwrap_or_else(|_| line.to_string())
}

/// Extract the `layout` value from MDX YAML frontmatter.
///
/// Looks for `layout: ./path/to/Layout.astro` or `layout: '../Layout'` in the
/// frontmatter block between `---` fences.
fn extract_mdx_frontmatter_layout(source: &str) -> Option<String> {
    let trimmed = source.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_first = &trimmed[3..];
    let close = after_first.find("---")?;
    let frontmatter = &after_first[..close];

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("layout:") {
            let value = rest.trim().trim_matches(|c| c == '\'' || c == '"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Detect `<style module>` or `<style scoped>` blocks in Vue SFCs.
fn detect_vue_style_blocks(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.contains("<style module") || lower.contains("<style scoped")
}

fn detect_lang_attr(tag: &str) -> Option<&'static str> {
    let lower = tag.to_ascii_lowercase();
    if lower.contains("lang=\"ts\"") || lower.contains("lang='ts'") {
        Some("ts")
    } else if lower.contains("lang=\"tsx\"") || lower.contains("lang='tsx'") {
        Some("tsx")
    } else {
        None
    }
}

fn make_virtual_path(original: &Path, lang: &str) -> PathBuf {
    original.with_extension(lang)
}

fn find_substr(haystack: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start + needle.len() > haystack.len() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|w| w.iter().zip(needle).all(|(a, b)| a.eq_ignore_ascii_case(b)))
        .map(|pos| pos + start)
}

fn find_byte(haystack: &[u8], start: usize, byte: u8) -> Option<usize> {
    haystack[start..].iter().position(|&b| b == byte).map(|pos| pos + start)
}

// ---------------------------------------------------------------------------
// Template component scanning (synthetic imports)
// ---------------------------------------------------------------------------

/// Standard HTML elements to ignore when scanning for component references.
const HTML_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

/// SVG elements to ignore.
const SVG_ELEMENTS: &[&str] = &[
    "svg",
    "path",
    "circle",
    "rect",
    "line",
    "polyline",
    "polygon",
    "ellipse",
    "g",
    "defs",
    "use",
    "text",
    "tspan",
    "image",
    "clippath",
    "mask",
    "filter",
    "lineargradient",
    "radialgradient",
    "stop",
    "pattern",
    "foreignobject",
    "animate",
    "animatetransform",
    "set",
];

/// Vue built-in components that should not generate synthetic imports.
const VUE_BUILTINS: &[&str] = &[
    "Component",
    "component",
    "Transition",
    "transition",
    "TransitionGroup",
    "transition-group",
    "KeepAlive",
    "keep-alive",
    "Teleport",
    "teleport",
    "Suspense",
    "suspense",
    "RouterView",
    "router-view",
    "RouterLink",
    "router-link",
    "Slot",
];

/// Check whether a tag name is a Vue built-in component.
fn is_vue_builtin(name: &str) -> bool {
    VUE_BUILTINS.contains(&name)
}

/// Detect component references in the template portion of an SFC.
///
/// Scans for `PascalCase` (e.g. `<MyComponent>`) and kebab-case component tags
/// (e.g. `<my-component>`) that aren't standard HTML/SVG elements and aren't
/// already imported in the script section. Returns synthetic imports for unresolved
/// component references.
fn detect_template_component_refs(
    source: &str,
    facts: &FileFacts,
    format: &str,
) -> Vec<SyntheticImport> {
    let template_content = match format {
        "vue" => extract_vue_template(source),
        "svelte" => extract_svelte_template(source),
        "astro" => Some(extract_astro_template(source)),
        _ => return Vec::new(),
    };
    let Some(template) = template_content else {
        return Vec::new();
    };

    // Collect names already imported in the script section.
    let imported_names: FxHashSet<&str> =
        facts.imports.iter().flat_map(|i| i.names.iter().map(|n| n.local.as_str())).collect();

    let component_tags = scan_component_tags(&template);
    let mut synthetic = Vec::new();

    for (tag_name, line_offset) in component_tags {
        // Skip Vue built-in components (Transition, KeepAlive, etc.).
        if format == "vue" && is_vue_builtin(&tag_name) {
            continue;
        }
        // Skip if already imported.
        if imported_names.contains(tag_name.as_str()) {
            continue;
        }
        // Also check kebab-to-pascal conversion.
        let pascal = kebab_to_pascal(&tag_name);
        if imported_names.contains(pascal.as_str()) {
            continue;
        }

        synthetic.push(SyntheticImport {
            specifier: tag_name.clone(),
            names: vec![CompactString::new(&tag_name)],
            line: line_offset,
            reason: format!("template component reference <{tag_name}>"),
        });
    }

    synthetic
}

/// Detect JSX component references in MDX content that aren't already imported.
///
/// Skips fenced code blocks (` ``` `) and inline code (`` ` ``).
fn detect_mdx_component_refs(source: &str, facts: &FileFacts) -> Vec<SyntheticImport> {
    let imported_names: FxHashSet<&str> =
        facts.imports.iter().flat_map(|i| i.names.iter().map(|n| n.local.as_str())).collect();

    // First, strip out fenced code blocks and inline code to avoid false positives.
    let cleaned = strip_mdx_code_regions(source);
    let bytes = cleaned.as_bytes();
    let mut synthetic = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    let mut i = 0;

    while i < bytes.len() {
        // Look for `<` followed by an uppercase letter (JSX component).
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_uppercase() {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let tag_name = &cleaned[start..end];
            if !imported_names.contains(tag_name)
                && !seen.contains(tag_name)
                && !is_html_element(tag_name)
            {
                let line = 1 + count_newlines(&bytes[..i]);
                synthetic.push(SyntheticImport {
                    specifier: tag_name.to_string(),
                    names: vec![CompactString::new(tag_name)],
                    line: u32::try_from(line).unwrap_or(u32::MAX),
                    reason: format!("MDX component reference <{tag_name}>"),
                });
                seen.insert(tag_name.to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }

    synthetic
}

/// Strip fenced code blocks and inline code from MDX source, preserving line
/// structure (newlines are kept, content is replaced with spaces).
fn strip_mdx_code_regions(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut in_fenced_code = false;
    let mut first = true;

    for line in source.lines() {
        if !first {
            result.push('\n');
        }
        first = false;
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            in_fenced_code = !in_fenced_code;
            // Replace the fence line itself with spaces.
            result.extend(std::iter::repeat_n(' ', line.len()));
            continue;
        }

        if in_fenced_code {
            // Replace fenced code content with spaces, preserving line structure.
            result.extend(std::iter::repeat_n(' ', line.len()));
            continue;
        }

        // Strip inline code spans.
        result.push_str(&strip_inline_code(line));
    }

    // Preserve trailing newline if the original had one.
    if source.ends_with('\n') {
        result.push('\n');
    }

    result
}

/// Extract the `<template>...</template>` section from a Vue SFC.
fn extract_vue_template(source: &str) -> Option<String> {
    let lower = source.to_ascii_lowercase();
    let start_tag = lower.find("<template")?;
    let content_start = source[start_tag..].find('>')? + start_tag + 1;
    let close_tag = lower[content_start..].find("</template>")?;
    Some(source[content_start..content_start + close_tag].to_string())
}

/// Extract template content from Svelte (everything outside `<script>` and `<style>`).
fn extract_svelte_template(source: &str) -> Option<String> {
    let mut result = source.to_string();
    // Remove script and style blocks.
    for tag in &["<script", "<style"] {
        loop {
            let current_lower = result.to_ascii_lowercase();
            let Some(start) = current_lower.find(tag) else { break };
            let close_tag = format!("</{}>", &tag[1..]);
            let Some(end_pos) = current_lower[start..].find(&close_tag) else { break };
            let end = start + end_pos + close_tag.len();
            result.replace_range(start..end, "");
        }
    }
    if result.trim().is_empty() { None } else { Some(result) }
}

/// Extract template content from Astro (everything after the frontmatter fence).
fn extract_astro_template(source: &str) -> String {
    let trimmed = source.trim_start();
    if let Some(after_first) = trimmed.strip_prefix("---")
        && let Some(close) = after_first.find("---")
    {
        let template = &after_first[close + 3..];
        if !template.trim().is_empty() {
            return template.to_string();
        }
    }
    // No frontmatter — entire file is template.
    source.to_string()
}

/// Scan template content for component tag names (`PascalCase` or kebab-case with hyphens).
fn scan_component_tags(template: &str) -> Vec<(String, u32)> {
    let bytes = template.as_bytes();
    let mut tags = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] != b'/' && bytes[i + 1] != b'!' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-' || bytes[end] == b'_')
            {
                end += 1;
            }
            if end > start {
                let tag_name = &template[start..end];
                let is_component =
                    tag_name.as_bytes()[0].is_ascii_uppercase() || tag_name.contains('-');
                if is_component && !is_html_element(tag_name) && !seen.contains(tag_name) {
                    let line = 1 + count_newlines(&bytes[..i]);
                    tags.push((tag_name.to_string(), u32::try_from(line).unwrap_or(u32::MAX)));
                    seen.insert(tag_name.to_string());
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }

    tags
}

fn is_html_element(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    HTML_ELEMENTS.contains(&lower.as_str()) || SVG_ELEMENTS.contains(&lower.as_str())
}

fn kebab_to_pascal(kebab: &str) -> String {
    kebab
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let mut s = c.to_uppercase().to_string();
                    s.extend(chars);
                    s
                }
            }
        })
        .collect()
}

/// Convert `PascalCase` to kebab-case (e.g. `MyComponent` -> `my-component`).
fn pascal_to_kebab(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Vue `<script setup>` and Options API component detection
// ---------------------------------------------------------------------------

/// For Vue `<script setup>` blocks, all top-level imports are automatically
/// exposed to the template. This function connects template component references
/// to their corresponding `<script setup>` imports.
///
/// In Vue 3's `<script setup>`:
///   - All top-level imports are available in the template
///   - `PascalCase` imports can be used as components directly (`<MyComponent>`)
///   - They can also be used in kebab-case form (`<my-component>`)
///   - Non-component imports (functions, constants) are also available
///
/// This function matches template component tags against script imports to produce
/// synthetic import entries that the dependency graph can follow.
fn detect_vue_setup_component_refs(
    template: &str,
    facts: &FileFacts,
    blocks: &[ScriptBlock],
) -> Vec<SyntheticImport> {
    // Only proceed if there is at least one `<script setup>` block.
    let has_setup = blocks.iter().any(|b| b.is_setup);
    if !has_setup {
        return Vec::new();
    }

    let component_tags = scan_component_tags(template);
    if component_tags.is_empty() {
        return Vec::new();
    }

    // Build a lookup of all imported names -> their import specifiers.
    let mut import_lookup: FxHashSet<String> = FxHashSet::default();
    for imp in &facts.imports {
        for name in &imp.names {
            import_lookup.insert(name.local.to_string());
        }
    }

    let mut synthetic = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    for (tag_name, line_offset) in &component_tags {
        // Skip Vue built-in components.
        if is_vue_builtin(tag_name) {
            continue;
        }

        // Determine the PascalCase form of the tag.
        let pascal =
            if tag_name.contains('-') { kebab_to_pascal(tag_name) } else { tag_name.clone() };

        if seen.contains(&pascal) {
            continue;
        }

        // Check if this component name (or its kebab form) matches a script import.
        let kebab = pascal_to_kebab(&pascal);
        let matches_import = import_lookup.contains(&pascal) || import_lookup.contains(&kebab);

        if matches_import {
            // This template component reference is backed by a <script setup> import.
            // Generate a synthetic import to mark it as "used by template".
            synthetic.push(SyntheticImport {
                specifier: pascal.clone(),
                names: vec![CompactString::new(&pascal)],
                line: *line_offset,
                reason: format!(
                    "<script setup> component: <{tag_name}> resolved to import '{pascal}'"
                ),
            });
            seen.insert(pascal);
        }
    }

    synthetic
}

/// Detect dynamic component patterns in Vue templates.
///
/// Handles:
///   - `<component :is="SomeName">` / `<component v-bind:is="SomeName">`
///   - `<component :is="someVariable">` (camelCase identifiers)
///
/// Returns identifier names found in dynamic `:is` bindings.
fn detect_vue_dynamic_components(template: &str) -> Vec<(String, u32)> {
    let bytes = template.as_bytes();
    let mut refs = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    let mut i = 0;

    while i < bytes.len() {
        // Look for `:is="..."` or `v-bind:is="..."`
        let is_attr = if i + 5 < bytes.len() && &bytes[i..i + 5] == b":is=\"" {
            Some(i + 5)
        } else if i + 12 < bytes.len() && &bytes[i..i + 12] == b"v-bind:is=\"" {
            Some(i + 12)
        } else if i + 5 < bytes.len() && &bytes[i..i + 5] == b":is='" {
            Some(i + 5)
        } else if i + 12 < bytes.len() && &bytes[i..i + 12] == b"v-bind:is='" {
            Some(i + 12)
        } else {
            None
        };

        if let Some(value_start) = is_attr {
            // Find the closing quote.
            let quote = bytes[value_start - 1]; // the opening quote character
            let mut value_end = value_start;
            while value_end < bytes.len() && bytes[value_end] != quote {
                value_end += 1;
            }
            if value_end > value_start {
                let value = template[value_start..value_end].trim();
                // Only consider simple identifiers (no expressions with dots, parens, etc.).
                if !value.is_empty()
                    && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && value.as_bytes()[0].is_ascii_alphabetic()
                    && !seen.contains(value)
                {
                    let line = 1 + count_newlines(&bytes[..i]);
                    refs.push((value.to_string(), u32::try_from(line).unwrap_or(u32::MAX)));
                    seen.insert(value.to_string());
                }
            }
            i = value_end + 1;
        } else {
            i += 1;
        }
    }

    refs
}

/// Detect Options API `components: { Foo, Bar }` registrations in a Vue `<script>` block.
///
/// In the Options API, components are registered like:
/// ```js
/// export default {
///   components: { Foo, Bar, BazComponent: Baz },
///   // ...
/// }
/// ```
///
/// This scans the script content for the `components:` property and extracts
/// the identifiers listed within its object literal.
fn detect_options_api_components(script_content: &str) -> Vec<String> {
    let bytes = script_content.as_bytes();

    // Find `components:` or `components :` followed by `{`.
    let pattern = "components";
    let mut components = Vec::new();
    let mut search_start = 0;

    while search_start < bytes.len() {
        let Some(pos) = script_content[search_start..].find(pattern) else {
            break;
        };
        let abs_pos = search_start + pos;
        let after_kw = abs_pos + pattern.len();

        // Skip whitespace/colon to find the opening brace.
        let mut j = after_kw;
        while j < bytes.len()
            && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n' || bytes[j] == b'\r')
        {
            j += 1;
        }
        // Expect a colon after `components`.
        if j < bytes.len() && bytes[j] == b':' {
            j += 1;
            // Skip whitespace after the colon.
            while j < bytes.len()
                && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n' || bytes[j] == b'\r')
            {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                j += 1; // skip opening brace
                // Find the matching closing brace, respecting nesting.
                let mut depth = 1;
                let obj_start = j;
                while j < bytes.len() && depth > 0 {
                    if bytes[j] == b'{' {
                        depth += 1;
                    } else if bytes[j] == b'}' {
                        depth -= 1;
                    }
                    if depth > 0 {
                        j += 1;
                    }
                }
                let obj_content = &script_content[obj_start..j];
                // Extract identifiers from the object. Handle both:
                //   - shorthand: `Foo, Bar`
                //   - aliased: `FooAlias: Foo, BarAlias: Bar`
                components.extend(extract_component_identifiers(obj_content));
            }
        }

        search_start = abs_pos + pattern.len();
    }

    components
}

/// Extract component identifiers from a `components: { ... }` object literal body.
///
/// Handles shorthand (`Foo, Bar`) and aliased (`AliasName: ImportedName`) forms.
/// Returns the value-side identifiers (the actual imports).
fn extract_component_identifiers(obj_body: &str) -> Vec<String> {
    let mut identifiers = Vec::new();

    for entry in obj_body.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }

        if let Some(colon_pos) = entry.find(':') {
            // Aliased form: `AliasName: ImportedName`
            let value = entry[colon_pos + 1..].trim();
            // Only take simple identifiers.
            if !value.is_empty()
                && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && value.as_bytes()[0].is_ascii_alphabetic()
            {
                identifiers.push(value.to_string());
            }
        } else {
            // Shorthand: `Foo`
            let name = entry.trim();
            if !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && name.as_bytes()[0].is_ascii_alphabetic()
            {
                identifiers.push(name.to_string());
            }
        }
    }

    identifiers
}

// ---------------------------------------------------------------------------
// Svelte snippet / slot detection
// ---------------------------------------------------------------------------

/// Detect `{#snippet ...}` blocks in Svelte markup.
///
/// Returns `true` if any `{#snippet` block is found. Snippets are Svelte 5's
/// replacement for slots that allow passing renderable content as props. Imports
/// used inside snippet blocks are genuinely used and should not be pruned.
fn detect_svelte_snippets(template: &str) -> bool {
    template.contains("{#snippet")
}

// ---------------------------------------------------------------------------
// Astro client:* directive detection
// ---------------------------------------------------------------------------

/// Detect Astro `client:*` hydration directives on component tags.
///
/// Scans for `PascalCase` component tags that have a `client:` attribute
/// (e.g. `client:load`, `client:idle`, `client:visible`, `client:media`,
/// `client:only`). Returns a vec of `(component_name, directive, line)`.
fn detect_astro_client_directives(template: &str) -> Vec<(String, String, u32)> {
    let bytes = template.as_bytes();
    let mut results = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    let mut i = 0;

    while i < bytes.len() {
        // Look for `<` followed by an uppercase letter (component tag).
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_uppercase() {
            let tag_open = i;
            let name_start = i + 1;
            let mut name_end = name_start;
            while name_end < bytes.len()
                && (bytes[name_end].is_ascii_alphanumeric()
                    || bytes[name_end] == b'_'
                    || bytes[name_end] == b'-')
            {
                name_end += 1;
            }
            if name_end > name_start {
                let tag_name = &template[name_start..name_end];
                // Scan the rest of the opening tag for client: directives.
                // Find the closing `>` or `/>` of the opening tag.
                let mut j = name_end;
                let mut tag_end = None;
                while j < bytes.len() {
                    if bytes[j] == b'>' {
                        tag_end = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(te) = tag_end {
                    let tag_content = &template[name_end..te];
                    // Look for client: directives.
                    if let Some(directive) = extract_client_directive(tag_content) {
                        let key = format!("{tag_name}:{directive}");
                        if seen.insert(key) {
                            let line = 1 + count_newlines(&bytes[..tag_open]);
                            results.push((
                                tag_name.to_string(),
                                directive,
                                u32::try_from(line).unwrap_or(u32::MAX),
                            ));
                        }
                    }
                    i = te + 1;
                } else {
                    i = name_end;
                }
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    results
}

/// Extract a `client:*` directive value from a tag's attribute string.
///
/// Returns the full directive (e.g. `"client:load"`, `"client:idle"`).
fn extract_client_directive(attrs: &str) -> Option<String> {
    // Look for `client:` followed by a directive name.
    let idx = attrs.find("client:")?;
    let after = &attrs[idx..];
    let mut end = 7; // len("client:")
    while end < after.len()
        && (after.as_bytes()[end].is_ascii_alphanumeric() || after.as_bytes()[end] == b'-')
    {
        end += 1;
    }
    if end > 7 { Some(after[..end].to_string()) } else { None }
}

// ---------------------------------------------------------------------------
// Member access detection
// ---------------------------------------------------------------------------

/// Detect `ImportedName.Member` patterns in source code.
///
/// For each named import, scans the source for `Name.identifier` patterns
/// and records them as member accesses. This is a lightweight heuristic
/// that covers the common case of enum/namespace member access.
#[allow(clippy::cast_possible_truncation)]
fn detect_member_accesses(
    source: &str,
    imports: &[ImportInfo],
    accesses: &mut Vec<MemberAccessInfo>,
) {
    // Collect all named import bindings.
    let imported_names: Vec<&str> =
        imports.iter().flat_map(|imp| imp.names.iter()).map(|name| name.local.as_str()).collect();

    if imported_names.is_empty() {
        return;
    }

    for (line_number, line) in source.lines().enumerate() {
        for &name in &imported_names {
            let name_bytes = name.as_bytes();
            let line_bytes = line.as_bytes();
            let mut pos = 0;
            while pos + name_bytes.len() + 1 < line_bytes.len() {
                let remaining = &line_bytes[pos..];
                let Some(found) = find_bytes(remaining, name_bytes) else {
                    break;
                };
                let abs_pos = pos + found;

                // Check that the name is not part of a larger identifier.
                let before_ok = abs_pos == 0 || !is_ident_char(line_bytes[abs_pos - 1]);
                let after_pos = abs_pos + name_bytes.len();
                let dot_follows = after_pos < line_bytes.len() && line_bytes[after_pos] == b'.';

                if before_ok && dot_follows {
                    // Extract the member name after the dot.
                    let member_start = after_pos + 1;
                    let mut member_end = member_start;
                    while member_end < line_bytes.len() && is_ident_char(line_bytes[member_end]) {
                        member_end += 1;
                    }
                    if member_end > member_start {
                        let member_name = &line[member_start..member_end];
                        // Check if this is an assignment target (write-only access).
                        let after_member = line[member_end..].trim_start();
                        let is_write =
                            after_member.starts_with('=') && !after_member.starts_with("==");
                        accesses.push(MemberAccessInfo {
                            object_name: name.to_string(),
                            member_name: member_name.to_string(),
                            line: (line_number + 1) as u32,
                            is_write,
                        });
                    }
                }

                pos = abs_pos + name_bytes.len();
            }
        }
    }
}

/// Detect instance member accesses: `const x = new Foo(); x.method()`.
///
/// Finds `new ImportedName(` assignments and tracks the variable, then
/// detects `variable.member` patterns.
#[allow(clippy::cast_possible_truncation)]
fn detect_instance_member_accesses(
    source: &str,
    imports: &[ImportInfo],
    accesses: &mut Vec<MemberAccessInfo>,
) {
    let imported_names: Vec<&str> =
        imports.iter().flat_map(|imp| imp.names.iter()).map(|name| name.local.as_str()).collect();

    if imported_names.is_empty() {
        return;
    }

    // Phase 1: Find `= new ClassName(` patterns and map variable → class name.
    let mut var_to_class: Vec<(&str, &str)> = Vec::new();
    for line in source.lines() {
        for &class_name in &imported_names {
            let new_pattern = format!("new {class_name}(");
            if let Some(new_pos) = line.find(&new_pattern) {
                // Look backwards from `new_pos` to find `= ` and then the variable name.
                let before = line[..new_pos].trim_end();
                if let Some(eq_pos) = before.rfind('=') {
                    let var_part = before[..eq_pos].trim();
                    // Extract the last word (variable name) from patterns like
                    // `const x`, `let x`, `var x`, or just `x`.
                    let var_name = var_part
                        .rsplit_once(char::is_whitespace)
                        .map_or(var_part, |(_, name)| name)
                        .trim();
                    if !var_name.is_empty() && var_name.bytes().all(is_ident_char) {
                        var_to_class.push((var_name, class_name));
                    }
                }
            }
        }
    }

    // Phase 2: Find `variable.member` patterns.
    for (line_number, line) in source.lines().enumerate() {
        for &(var_name, class_name) in &var_to_class {
            let var_bytes = var_name.as_bytes();
            let line_bytes = line.as_bytes();
            let mut pos = 0;
            while pos + var_bytes.len() + 1 < line_bytes.len() {
                let remaining = &line_bytes[pos..];
                let Some(found) = find_bytes(remaining, var_bytes) else {
                    break;
                };
                let abs_pos = pos + found;
                let before_ok = abs_pos == 0 || !is_ident_char(line_bytes[abs_pos - 1]);
                let after_pos = abs_pos + var_bytes.len();
                let dot_follows = after_pos < line_bytes.len() && line_bytes[after_pos] == b'.';

                if before_ok && dot_follows {
                    let member_start = after_pos + 1;
                    let mut member_end = member_start;
                    while member_end < line_bytes.len() && is_ident_char(line_bytes[member_end]) {
                        member_end += 1;
                    }
                    if member_end > member_start {
                        let member_name = &line[member_start..member_end];
                        // Check if this is an assignment target (write-only access).
                        let after_member = line[member_end..].trim_start();
                        let is_write =
                            after_member.starts_with('=') && !after_member.starts_with("==");
                        accesses.push(MemberAccessInfo {
                            object_name: class_name.to_string(),
                            member_name: member_name.to_string(),
                            line: (line_number + 1) as u32,
                            is_write,
                        });
                    }
                }
                pos = abs_pos + var_bytes.len();
            }
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

const fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

// ---------------------------------------------------------------------------
// Dynamic dependency pattern detectors
// ---------------------------------------------------------------------------

/// Detect `require.resolve('specifier')` calls via simple text scanning.
fn detect_require_resolve(source: &str) -> Vec<DependencyPattern> {
    let bytes = source.as_bytes();
    let needle = b"require.resolve(";
    let mut results = Vec::new();
    let mut index = 0;

    while index + needle.len() + 2 < bytes.len() {
        if !bytes[index..].starts_with(needle) {
            index += 1;
            continue;
        }

        // Check word boundary before `require`
        let before = index.checked_sub(1).and_then(|i| bytes.get(i).copied());
        if before.is_some_and(is_identifier_byte) {
            index += 1;
            continue;
        }

        let quote_pos = index + needle.len();
        let Some(quote) = bytes.get(quote_pos).copied() else {
            break;
        };
        if !matches!(quote, b'"' | b'\'') {
            index += 1;
            continue;
        }

        let literal_start = quote_pos + 1;
        let mut literal_end = literal_start;
        while literal_end < bytes.len() && bytes[literal_end] != quote {
            literal_end += 1;
        }
        if literal_end >= bytes.len() || bytes.get(literal_end + 1) != Some(&b')') {
            index += 1;
            continue;
        }

        let specifier = source[literal_start..literal_end].to_string();
        let line = 1 + count_newlines(&bytes[..index]);
        results.push(DependencyPattern::RequireResolve {
            specifier,
            line: u32::try_from(line).unwrap_or(u32::MAX),
        });
        index = literal_end + 2;
    }

    results
}

/// Detect `import.meta.glob('pattern')` and `import.meta.glob(['pat1', 'pat2'])` calls
/// via simple text scanning.
fn detect_import_meta_glob(source: &str) -> Vec<DependencyPattern> {
    let bytes = source.as_bytes();
    let needle = b"import.meta.glob(";
    let mut results = Vec::new();
    let mut index = 0;

    while index + needle.len() + 2 < bytes.len() {
        if !bytes[index..].starts_with(needle) {
            index += 1;
            continue;
        }

        let after_paren = index + needle.len();
        let Some(&first_char) = bytes.get(after_paren) else {
            break;
        };

        let line = 1 + count_newlines(&bytes[..index]);
        let line_u32 = u32::try_from(line).unwrap_or(u32::MAX);

        if first_char == b'[' {
            // Array form: import.meta.glob(['./a/*.ts', './b/*.ts'])
            if let Some(patterns) = extract_string_array(&source[after_paren..])
                && !patterns.is_empty()
            {
                results.push(DependencyPattern::ImportMetaGlobArray { patterns, line: line_u32 });
            }
            // Skip past the closing bracket.
            let mut skip = after_paren + 1;
            while skip < bytes.len() && bytes[skip] != b']' {
                skip += 1;
            }
            index = skip + 1;
        } else if matches!(first_char, b'"' | b'\'') {
            // Single string form: import.meta.glob('./modules/*.ts')
            let quote = first_char;
            let literal_start = after_paren + 1;
            let mut literal_end = literal_start;
            while literal_end < bytes.len() && bytes[literal_end] != quote {
                literal_end += 1;
            }
            if literal_end >= bytes.len() {
                index += 1;
                continue;
            }

            let pattern = source[literal_start..literal_end].to_string();
            results.push(DependencyPattern::ImportMetaGlob { pattern, line: line_u32 });
            index = literal_end + 1;
        } else {
            index += 1;
        }
    }

    results
}

/// Extract an array of string literals from a source fragment starting with `[`.
/// Returns `None` if the array can't be parsed.
fn extract_string_array(source: &str) -> Option<Vec<String>> {
    let bytes = source.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }

    let mut strings = Vec::new();
    let mut i = 1; // Skip the opening `[`.

    loop {
        // Skip whitespace and commas.
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b']' {
            break;
        }

        // Expect a quoted string.
        let quote = bytes[i];
        if !matches!(quote, b'"' | b'\'') {
            return None; // Non-string element, bail.
        }
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != quote {
            if bytes[i] == b'\\' {
                i += 1; // Skip escaped char.
            }
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        strings.push(source[start..i].to_string());
        i += 1; // Skip closing quote.
    }

    Some(strings)
}

/// Detect `/// <reference path="..." />` and `/// <reference types="..." />`
/// directives via line-by-line text scanning.
fn detect_triple_slash_references(source: &str) -> Vec<DependencyPattern> {
    let mut results = Vec::new();

    for (line_number, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("/// <reference ") {
            continue;
        }

        let line_u32 = u32::try_from(line_number + 1).unwrap_or(u32::MAX);

        if let Some(path_val) = extract_attr_value(trimmed, "path") {
            results.push(DependencyPattern::TripleSlashReference {
                path: path_val,
                is_types: false,
                line: line_u32,
            });
        } else if let Some(types_val) = extract_attr_value(trimmed, "types") {
            results.push(DependencyPattern::TripleSlashReference {
                path: types_val,
                is_types: true,
                line: line_u32,
            });
        }
    }

    results
}

/// Extract the value of an XML-like attribute, e.g. `path="foo"` -> `"foo"`.
fn extract_attr_value(text: &str, attr: &str) -> Option<String> {
    // Look for `attr="value"` or `attr='value'`
    for separator in ['"', '\''] {
        let pattern = format!("{attr}={separator}");
        if let Some(start) = text.find(&pattern) {
            let val_start = start + pattern.len();
            if let Some(val_end) = text[val_start..].find(separator) {
                return Some(text[val_start..val_start + val_end].to_string());
            }
        }
    }
    None
}

/// Detect `JSDoc` `@type {import('specifier')}` and `@typedef {import('specifier')...}`
/// patterns via simple text scanning.
fn detect_jsdoc_imports(source: &str) -> Vec<DependencyPattern> {
    let bytes = source.as_bytes();
    let needle = b"import(";
    let mut results = Vec::new();
    let mut index = 0;

    while index + needle.len() + 2 < bytes.len() {
        if !bytes[index..].starts_with(needle) {
            index += 1;
            continue;
        }

        // Only match inside JSDoc: look backward for `@type` or `@typedef` on the
        // same line (or preceding line within a `/** ... */` block).
        let line_start = bytes[..index].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let prefix = &source[line_start..index];
        let is_jsdoc =
            prefix.contains("@type") || prefix.contains("@param") || prefix.contains("@returns");
        if !is_jsdoc {
            index += 1;
            continue;
        }

        let quote_pos = index + needle.len();
        let Some(quote) = bytes.get(quote_pos).copied() else {
            break;
        };
        if !matches!(quote, b'"' | b'\'') {
            index += 1;
            continue;
        }

        let literal_start = quote_pos + 1;
        let mut literal_end = literal_start;
        while literal_end < bytes.len() && bytes[literal_end] != quote {
            literal_end += 1;
        }
        if literal_end >= bytes.len() || bytes.get(literal_end + 1) != Some(&b')') {
            index += 1;
            continue;
        }

        let specifier = source[literal_start..literal_end].to_string();
        let line = 1 + count_newlines(&bytes[..index]);
        results.push(DependencyPattern::JsDocImport {
            specifier,
            line: u32::try_from(line).unwrap_or(u32::MAX),
        });
        index = literal_end + 2;
    }

    results
}

/// Detect `import.meta.resolve('specifier')` calls via simple text scanning.
fn detect_import_meta_resolve(source: &str) -> Vec<DependencyPattern> {
    let bytes = source.as_bytes();
    let needle = b"import.meta.resolve(";
    let mut results = Vec::new();
    let mut index = 0;

    while index + needle.len() + 2 < bytes.len() {
        if !bytes[index..].starts_with(needle) {
            index += 1;
            continue;
        }

        let quote_pos = index + needle.len();
        let Some(quote) = bytes.get(quote_pos).copied() else {
            break;
        };
        if !matches!(quote, b'"' | b'\'') {
            index += 1;
            continue;
        }

        let literal_start = quote_pos + 1;
        let mut literal_end = literal_start;
        while literal_end < bytes.len() && bytes[literal_end] != quote {
            literal_end += 1;
        }
        if literal_end >= bytes.len() || bytes.get(literal_end + 1) != Some(&b')') {
            index += 1;
            continue;
        }

        let specifier = source[literal_start..literal_end].to_string();
        let line = 1 + count_newlines(&bytes[..index]);
        results.push(DependencyPattern::ImportMetaResolve {
            specifier,
            line: u32::try_from(line).unwrap_or(u32::MAX),
        });
        index = literal_end + 2;
    }

    results
}

/// Detect `require.context('./dir', ...)` calls via simple text scanning.
///
/// Extracts the directory argument. The `recursive` flag defaults to `true`
/// when the second argument is not a literal `false`.
fn detect_require_context(source: &str) -> Vec<DependencyPattern> {
    let bytes = source.as_bytes();
    let needle = b"require.context(";
    let mut results = Vec::new();
    let mut index = 0;

    while index + needle.len() + 2 < bytes.len() {
        if !bytes[index..].starts_with(needle) {
            index += 1;
            continue;
        }

        // Check word boundary before `require`
        let before = index.checked_sub(1).and_then(|i| bytes.get(i).copied());
        if before.is_some_and(is_identifier_byte) {
            index += 1;
            continue;
        }

        let quote_pos = index + needle.len();
        let Some(quote) = bytes.get(quote_pos).copied() else {
            break;
        };
        if !matches!(quote, b'"' | b'\'') {
            index += 1;
            continue;
        }

        let literal_start = quote_pos + 1;
        let mut literal_end = literal_start;
        while literal_end < bytes.len() && bytes[literal_end] != quote {
            literal_end += 1;
        }
        if literal_end >= bytes.len() {
            index += 1;
            continue;
        }

        let directory = source[literal_start..literal_end].to_string();
        let line = 1 + count_newlines(&bytes[..index]);

        // Check for `, false` after the closing quote to determine recursive flag
        // and extract optional regex filter (third argument).
        let after_quote = literal_end + 1;
        // Look further ahead to capture potential regex argument.
        let rest = &source[after_quote..source.len().min(after_quote + 200)];
        let rest_trimmed = rest.trim_start_matches([',', ' ']);
        let recursive = !rest_trimmed.starts_with("false");

        // Extract regex filter: look for /pattern/ as the third argument.
        let regex_filter = extract_require_context_regex(rest);

        results.push(DependencyPattern::RequireContext {
            directory,
            recursive,
            regex_filter,
            line: u32::try_from(line).unwrap_or(u32::MAX),
        });
        // Skip past the closing paren
        index = literal_end + 1;
    }

    results
}

/// Extract the regex filter from a `require.context` call's remaining arguments.
///
/// Given the text after the directory string literal (e.g. `, true, /\.json$/)`),
/// this searches for a `/pattern/` regex literal as the third argument.
fn extract_require_context_regex(rest: &str) -> Option<String> {
    // We need to find a regex literal `/pattern/` that appears after two commas
    // (the second comma separates the recursive flag from the regex).
    let bytes = rest.as_bytes();
    let mut comma_count = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b',' => {
                comma_count += 1;
            }
            b'/' if comma_count >= 2 => {
                // Start of regex literal.
                let regex_start = i + 1;
                let mut regex_end = regex_start;
                let mut escaped = false;
                while regex_end < bytes.len() {
                    if escaped {
                        escaped = false;
                        regex_end += 1;
                        continue;
                    }
                    if bytes[regex_end] == b'\\' {
                        escaped = true;
                        regex_end += 1;
                        continue;
                    }
                    if bytes[regex_end] == b'/' {
                        break;
                    }
                    regex_end += 1;
                }
                if regex_end < bytes.len() && regex_end > regex_start {
                    return Some(rest[regex_start..regex_end].to_string());
                }
                return None;
            }
            b')' => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Detect `new URL('./path', import.meta.url)` patterns via text scanning.
fn detect_url_constructor(source: &str) -> Vec<DependencyPattern> {
    let bytes = source.as_bytes();
    let needle = b"new URL(";
    let mut results = Vec::new();
    let mut index = 0;

    while index + needle.len() + 2 < bytes.len() {
        if !bytes[index..].starts_with(needle) {
            index += 1;
            continue;
        }

        let quote_pos = index + needle.len();
        let Some(quote) = bytes.get(quote_pos).copied() else {
            break;
        };
        if !matches!(quote, b'"' | b'\'') {
            index += 1;
            continue;
        }

        let literal_start = quote_pos + 1;
        let mut literal_end = literal_start;
        while literal_end < bytes.len() && bytes[literal_end] != quote {
            literal_end += 1;
        }
        if literal_end >= bytes.len() {
            index += 1;
            continue;
        }

        // Check that the second argument contains `import.meta.url`
        let after_first_arg = literal_end + 1;
        let close_paren = source[after_first_arg..].find(')').map(|i| i + after_first_arg);
        let Some(close) = close_paren else {
            index += 1;
            continue;
        };
        let between = &source[after_first_arg..close];
        if !between.contains("import.meta.url") {
            index += 1;
            continue;
        }

        let specifier = source[literal_start..literal_end].to_string();
        // Only track relative specifiers (not absolute URLs)
        if specifier.starts_with('.') || specifier.starts_with('/') {
            let line = 1 + count_newlines(&bytes[..index]);
            results.push(DependencyPattern::UrlConstructor {
                specifier,
                line: u32::try_from(line).unwrap_or(u32::MAX),
            });
        }
        index = close + 1;
    }

    results
}

/// Detect `import foo = require('bar')` from the parsed AST.
fn detect_import_equals(program: &oxc_ast::ast::Program<'_>) -> Vec<DependencyPattern> {
    let mut results = Vec::new();
    for stmt in &program.body {
        if let oxc_ast::ast::Statement::TSImportEqualsDeclaration(decl) = stmt
            && let oxc_ast::ast::TSModuleReference::ExternalModuleReference(ext) =
                &decl.module_reference
        {
            let specifier = ext.expression.value.to_string();
            let line = decl.span.start;
            results.push(DependencyPattern::ImportEquals { specifier, line });
        }
    }
    results
}

fn merge_facts(target: &mut FileFacts, source: FileFacts) {
    target.exports.extend(source.exports);
    target.imports.extend(source.imports);
    target.reexports.extend(source.reexports);
    target.has_side_effects = target.has_side_effects || source.has_side_effects;
    target.dynamic_imports.extend(source.dynamic_imports);
    target.requires.extend(source.requires);
    target.dependency_patterns.extend(source.dependency_patterns);
    target.member_exports.extend(source.member_exports);
    target.same_file_refs.extend(source.same_file_refs);
}

/// Errors from extraction.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("failed to parse {path}")]
    ParseFailed { path: std::path::PathBuf },
}
