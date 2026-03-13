use std::path::{Path, PathBuf};

use compact_str::CompactString;
use pruneguard_fs::{FileRecord, SourceKind};
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
}

/// An export from a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportInfo {
    /// The exported name (or "default").
    pub name: CompactString,
    /// Whether this is a type-only export.
    pub is_type: bool,
    /// Source line (1-indexed).
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

/// Extract all import/export facts from a tracked source file.
///
/// For JS/TS files this parses the full file. For framework SFCs (`.vue`,
/// `.svelte`, `.astro`, `.mdx`) this first extracts the embedded script
/// blocks and then feeds them through the JS/TS extractor.
pub fn extract_file_facts(path: &Path, source: &str) -> Result<FileFacts, ExtractError> {
    let source_kind = SourceKind::from_path(path);
    match source_kind {
        Some(SourceKind::Vue) => Ok(extract_vue_facts(path, source)),
        Some(SourceKind::Svelte) => Ok(extract_svelte_facts(path, source)),
        Some(SourceKind::Astro) => extract_astro_facts(path, source),
        Some(SourceKind::Mdx) => extract_mdx_facts(path, source),
        _ => extract_js_ts_facts(path, source),
    }
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

    Ok(facts)
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
                line: export.span.start,
            });
        }

        Statement::ExportAllDeclaration(export) => {
            facts.reexports.push(ReexportInfo {
                specifier: CompactString::new(export.source.value.as_str()),
                names: Vec::new(),
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

#[allow(clippy::naive_bytecount)]
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

        let Some(quote) = bytes.get(open_paren + 1).copied() else {
            break;
        };
        if !matches!(quote, b'"' | b'\'') {
            index += 1;
            continue;
        }

        let literal_start = open_paren + 2;
        let mut literal_end = literal_start;
        while literal_end < bytes.len() && bytes[literal_end] != quote {
            literal_end += 1;
        }
        if literal_end >= bytes.len() || bytes.get(literal_end + 1) != Some(&b')') {
            index += 1;
            continue;
        }

        let specifier = source[literal_start..literal_end].to_string();
        let line = 1 + bytes[..index].iter().filter(|byte| **byte == b'\n').count();
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
                    line: func.span.start,
                });
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                facts.exports.push(ExportInfo {
                    name: CompactString::new(id.name.as_str()),
                    is_type,
                    line: class.span.start,
                });
            }
        }
        Declaration::TSTypeAliasDeclaration(alias) => {
            facts.exports.push(ExportInfo {
                name: CompactString::new(alias.id.name.as_str()),
                is_type: true,
                line: alias.span.start,
            });
        }
        Declaration::TSInterfaceDeclaration(iface) => {
            facts.exports.push(ExportInfo {
                name: CompactString::new(iface.id.name.as_str()),
                is_type: true,
                line: iface.span.start,
            });
        }
        Declaration::TSEnumDeclaration(enum_decl) => {
            facts.exports.push(ExportInfo {
                name: CompactString::new(enum_decl.id.name.as_str()),
                is_type,
                line: enum_decl.span.start,
            });
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

/// Extract facts from a Vue single-file component.
///
/// Parses `<script>` and `<script setup>` blocks and feeds their content
/// through the JS/TS extractor. The file itself remains the node of record.
fn extract_vue_facts(path: &Path, source: &str) -> FileFacts {
    let blocks = extract_vue_script_blocks(source);
    extract_from_script_blocks(path, &blocks)
}

/// Extract facts from a Svelte component.
///
/// Parses `<script>` and `<script context="module">` blocks.
fn extract_svelte_facts(path: &Path, source: &str) -> FileFacts {
    let blocks = extract_svelte_script_blocks(source);
    extract_from_script_blocks(path, &blocks)
}

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
}

/// Extract `<script>` and `<script setup>` blocks from a Vue SFC.
fn extract_vue_script_blocks(source: &str) -> Vec<ScriptBlock> {
    extract_html_script_blocks(source, &["<script", "<script setup"])
}

/// Extract `<script>` and `<script context="module">` blocks from Svelte.
fn extract_svelte_script_blocks(source: &str) -> Vec<ScriptBlock> {
    extract_html_script_blocks(source, &["<script", "<script context=\"module\""])
}

/// Generic HTML-like `<script ...>...</script>` block extractor.
///
/// Handles `lang="ts"` / `lang="tsx"` attributes.
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
            let content_start = open_end + 1;

            // Find the matching `</script>`.
            let Some(close_start) = find_substr(bytes, content_start, b"</script>") else {
                break;
            };

            blocks.push(ScriptBlock {
                content: source[content_start..close_start].to_string(),
                lang,
            });

            search_start = close_start + b"</script>".len();
        }
    }

    blocks
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
/// markdown or JSX content. We collect contiguous import/export blocks.
fn extract_mdx_esm_lines(source: &str) -> String {
    let mut esm_lines = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ")
            || trimmed.starts_with("import{")
            || trimmed.starts_with("export ")
            || trimmed.starts_with("export{")
            || trimmed.starts_with("export default ")
        {
            esm_lines.push(line);
        }
    }
    esm_lines.join("\n")
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

fn merge_facts(target: &mut FileFacts, source: FileFacts) {
    target.exports.extend(source.exports);
    target.imports.extend(source.imports);
    target.reexports.extend(source.reexports);
    target.has_side_effects = target.has_side_effects || source.has_side_effects;
    target.dynamic_imports.extend(source.dynamic_imports);
    target.requires.extend(source.requires);
}

/// Errors from extraction.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("failed to parse {path}")]
    ParseFailed { path: std::path::PathBuf },
}
