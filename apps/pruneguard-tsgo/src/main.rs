#![allow(clippy::print_stdout, clippy::print_stderr)]
//! `pruneguard-tsgo` — optional semantic helper for pruneguard.
//!
//! This binary communicates with the pruneguard Rust core over stdio using
//! length-prefixed binary framing. It provides semantic precision refinement
//! for dead-code analysis by leveraging TypeScript type information.
//!
//! Usage:
//!   pruneguard-tsgo headless          # Run in headless mode (stdin/stdout protocol)
//!   pruneguard-tsgo --version         # Print version
//!   pruneguard-tsgo --help            # Print help

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use pruneguard_semantic_protocol::{
    ErrorMessage, FoundReference, HEADER_SIZE, HandshakeRequest, MessageType, PROTOCOL_VERSION,
    QueryBatch, QueryKind, QueryResult, ReadyMessage, ResponseBatch, SemanticQuery, decode_header,
    encode_message,
};

// ---------------------------------------------------------------------------
// Semantic index data structures
// ---------------------------------------------------------------------------

/// Information about a single export in a file.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ExportEntry {
    /// The exported name (e.g., `MyClass`, "default").
    name: String,
    /// Whether this export is type-only (`export type`).
    is_type: bool,
    /// Byte offset of the export declaration in the source.
    offset: u32,
}

/// Information about a single import in a file.
#[derive(Debug, Clone)]
struct ImportEntry {
    /// The module specifier (e.g., `./utils`, `@scope/pkg`).
    source: String,
    /// Imported names: `(imported_name, local_alias, is_type_only_specifier)`.
    names: Vec<(String, String, bool)>,
    /// Whether the entire import statement is type-only (`import type { ... }`).
    is_type_import: bool,
    /// Line number (1-based) of the import declaration.
    line: u32,
    /// Column (0-based byte offset) of the import declaration.
    column: u32,
}

/// Information about a namespace alias (`import NS = Other.Sub`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct NamespaceAliasEntry {
    /// The local alias name (left side).
    alias: String,
    /// The qualified target (right side), e.g., `["Other", "Sub"]`.
    target_chain: Vec<String>,
    /// Byte offset in the source.
    offset: u32,
}

/// Information about an identifier usage site in a file.
#[derive(Debug, Clone)]
struct UsageEntry {
    /// The identifier name.
    name: String,
    /// Line number (1-based).
    line: u32,
    /// Column (0-based byte offset).
    column: u32,
    /// Whether this usage is in a type-only position.
    is_type_only: bool,
    /// Whether this is a write (assignment target).
    is_write: bool,
}

/// Per-file extracted semantic data.
#[derive(Debug, Clone, Default)]
struct FileData {
    exports: Vec<ExportEntry>,
    imports: Vec<ImportEntry>,
    namespace_aliases: Vec<NamespaceAliasEntry>,
    /// Identifier usages in the file (for same-file and cross-file analysis).
    usages: Vec<UsageEntry>,
}

/// The main semantic index holding extracted data for all indexed files.
#[allow(dead_code)]
struct SemanticIndex {
    /// Per-file semantic data, keyed by canonical file path.
    files: FxHashMap<String, FileData>,
    /// Reverse index: for a given module file path, which files import from it?
    /// Maps `canonical_target_path` -> `Vec<(importer_path, import_entry_index)>`.
    import_reverse: FxHashMap<String, Vec<(String, usize)>>,
    /// Project root for resolving relative paths.
    project_root: String,
}

impl SemanticIndex {
    /// Build the semantic index by discovering and parsing all files covered by
    /// the given tsconfig paths.
    fn build(tsconfig_paths: &[String], project_root: &str) -> (Self, usize, usize) {
        let file_paths = discover_files(tsconfig_paths, project_root);
        let projects_loaded = tsconfig_paths.len();

        tracing::info!(
            discovered_files = file_paths.len(),
            "discovered TypeScript/JavaScript files"
        );

        // Parse all files in parallel and extract semantic data.
        let parsed: Vec<(String, FileData)> = file_paths
            .par_iter()
            .filter_map(|path| {
                let source = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::debug!(path = %path.display(), error = %e, "failed to read file");
                        return None;
                    }
                };
                let path_str = path.to_string_lossy().to_string();
                let file_data = parse_and_extract(path, &source);
                Some((path_str, file_data))
            })
            .collect();

        let files_indexed = parsed.len();
        let mut files: FxHashMap<String, FileData> =
            FxHashMap::with_capacity_and_hasher(files_indexed, rustc_hash::FxBuildHasher);
        for (path, data) in parsed {
            files.insert(path, data);
        }

        // Build reverse import index.
        let mut import_reverse: FxHashMap<String, Vec<(String, usize)>> = FxHashMap::default();

        for (importer_path, file_data) in &files {
            let importer_dir = Path::new(importer_path).parent().unwrap_or_else(|| Path::new("."));
            for (idx, imp) in file_data.imports.iter().enumerate() {
                if let Some(resolved) = resolve_import_source(&imp.source, importer_dir) {
                    import_reverse.entry(resolved).or_default().push((importer_path.clone(), idx));
                }
            }
        }

        let index = Self { files, import_reverse, project_root: project_root.to_string() };

        (index, projects_loaded, files_indexed)
    }

    /// Process a single semantic query.
    fn process_query(&self, query: &SemanticQuery) -> QueryResult {
        match query.kind {
            QueryKind::FindExportReferences => self.find_export_references(query),
            QueryKind::FindMemberReferences => self.find_member_references(query),
            QueryKind::FindSameFileExportUsage => self.find_same_file_export_usage(query),
            QueryKind::ResolveNamespaceAliasChain => self.resolve_namespace_alias_chain(query),
            QueryKind::ClassifyTypeOnlyVsValueUsage => self.classify_type_only_vs_value(query),
        }
    }

    /// `FindExportReferences` -- scan all indexed files for imports that
    /// reference the given export from the given file.
    fn find_export_references(&self, query: &SemanticQuery) -> QueryResult {
        let Some(export_name) = &query.export_name else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some("export_name is required for FindExportReferences".into()),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };

        let canonical_target = canonicalize_path(&query.file_path);
        let mut refs = Vec::new();

        // Use the reverse import index to find files that import from this target.
        if let Some(importers) = self.import_reverse.get(&canonical_target) {
            for (importer_path, import_idx) in importers {
                if let Some(file_data) = self.files.get(importer_path) {
                    let imp = &file_data.imports[*import_idx];

                    for (imported_name, _local_alias, is_type_specifier) in &imp.names {
                        // Match: importing the exact name, or importing "*" (namespace).
                        let matches = imported_name == export_name.as_str()
                            || (imported_name == "default" && export_name == "default")
                            || imported_name == "*";
                        if matches {
                            refs.push(FoundReference {
                                file_path: importer_path.clone(),
                                line: imp.line,
                                column: imp.column,
                                is_type_only: imp.is_type_import || *is_type_specifier,
                                is_write: false,
                            });
                        }
                    }
                }
            }
        }

        let total = refs.len();
        QueryResult {
            id: query.id,
            success: true,
            error: None,
            references: refs,
            total_references: total,
            is_type_only: None,
            alias_chain: Vec::new(),
        }
    }

    /// `FindMemberReferences` -- scan files for references to
    /// `parent.member` patterns.
    fn find_member_references(&self, query: &SemanticQuery) -> QueryResult {
        let Some(parent_name) = &query.parent_name else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some("parent_name is required for FindMemberReferences".into()),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };
        let Some(member_name) = &query.member_name else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some("member_name is required for FindMemberReferences".into()),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };

        let canonical_target = canonicalize_path(&query.file_path);
        let mut refs = Vec::new();

        // Find all files that import the parent from the target file,
        // then check their usages for member access patterns.
        if let Some(importers) = self.import_reverse.get(&canonical_target) {
            for (importer_path, import_idx) in importers {
                if let Some(file_data) = self.files.get(importer_path) {
                    let imp = &file_data.imports[*import_idx];

                    // Check if any imported name matches the parent.
                    let has_parent_import = imp.names.iter().any(|(imported, _, _)| {
                        imported == parent_name.as_str()
                            || (imported == "default" && parent_name == "default")
                            || imported == "*"
                    });

                    if !has_parent_import {
                        continue;
                    }

                    // Scan usages in the importing file for the member name.
                    for usage in &file_data.usages {
                        if usage.name == member_name.as_str() {
                            refs.push(FoundReference {
                                file_path: importer_path.clone(),
                                line: usage.line,
                                column: usage.column,
                                is_type_only: usage.is_type_only,
                                is_write: usage.is_write,
                            });
                        }
                    }
                }
            }
        }

        // Also check the source file itself for member accesses.
        if let Some(file_data) = self.files.get(&canonical_target) {
            for usage in &file_data.usages {
                if usage.name == member_name.as_str() {
                    refs.push(FoundReference {
                        file_path: canonical_target.clone(),
                        line: usage.line,
                        column: usage.column,
                        is_type_only: usage.is_type_only,
                        is_write: usage.is_write,
                    });
                }
            }
        }

        let total = refs.len();
        QueryResult {
            id: query.id,
            success: true,
            error: None,
            references: refs,
            total_references: total,
            is_type_only: None,
            alias_chain: Vec::new(),
        }
    }

    /// `FindSameFileExportUsage` -- check if the exported symbol is also
    /// used within the same file (local references after the export).
    fn find_same_file_export_usage(&self, query: &SemanticQuery) -> QueryResult {
        let Some(export_name) = &query.export_name else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some("export_name is required for FindSameFileExportUsage".into()),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };

        let canonical_path = canonicalize_path(&query.file_path);
        let Some(file_data) = self.files.get(&canonical_path) else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some(format!("file not indexed: {}", query.file_path)),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };

        // Check if this export exists in the file.
        let has_export = file_data.exports.iter().any(|e| e.name == export_name.as_str());
        if !has_export {
            return QueryResult {
                id: query.id,
                success: true,
                error: None,
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        }

        // Find all usages of this identifier within the same file.
        let mut refs = Vec::new();
        for usage in &file_data.usages {
            if usage.name == export_name.as_str() {
                refs.push(FoundReference {
                    file_path: canonical_path.clone(),
                    line: usage.line,
                    column: usage.column,
                    is_type_only: usage.is_type_only,
                    is_write: usage.is_write,
                });
            }
        }

        let total = refs.len();
        QueryResult {
            id: query.id,
            success: true,
            error: None,
            references: refs,
            total_references: total,
            is_type_only: None,
            alias_chain: Vec::new(),
        }
    }

    /// `ResolveNamespaceAliasChain` -- follow `import NS = Other.Sub` chains.
    fn resolve_namespace_alias_chain(&self, query: &SemanticQuery) -> QueryResult {
        let Some(export_name) = &query.export_name else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some("export_name is required for ResolveNamespaceAliasChain".into()),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };

        let canonical_path = canonicalize_path(&query.file_path);
        let Some(file_data) = self.files.get(&canonical_path) else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some(format!("file not indexed: {}", query.file_path)),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };

        // Build a local alias map for the file.
        let mut alias_map: FxHashMap<&str, &[String]> = FxHashMap::default();
        for entry in &file_data.namespace_aliases {
            alias_map.insert(&entry.alias, &entry.target_chain);
        }

        // Follow the chain starting from export_name.
        let mut chain = vec![export_name.clone()];
        let mut current = export_name.as_str();
        let mut seen = FxHashSet::default();
        seen.insert(current.to_string());

        loop {
            if let Some(target) = alias_map.get(current) {
                let joined = target.join(".");
                if seen.contains(&joined) {
                    // Cycle detected, stop.
                    break;
                }
                chain.push(joined.clone());
                seen.insert(joined);
                // Try to follow further if the first segment is also an alias.
                if let Some(first) = target.first()
                    && alias_map.contains_key(first.as_str())
                {
                    current = first.as_str();
                    continue;
                }
                break;
            }
            break;
        }

        QueryResult {
            id: query.id,
            success: true,
            error: None,
            references: Vec::new(),
            total_references: 0,
            is_type_only: None,
            alias_chain: chain,
        }
    }

    /// `ClassifyTypeOnlyVsValueUsage` -- determine if the export is only
    /// used in type positions across all importing files.
    fn classify_type_only_vs_value(&self, query: &SemanticQuery) -> QueryResult {
        let Some(export_name) = &query.export_name else {
            return QueryResult {
                id: query.id,
                success: false,
                error: Some("export_name is required for ClassifyTypeOnlyVsValueUsage".into()),
                references: Vec::new(),
                total_references: 0,
                is_type_only: None,
                alias_chain: Vec::new(),
            };
        };

        let canonical_target = canonicalize_path(&query.file_path);

        // Check the export declaration itself.
        if let Some(file_data) = self.files.get(&canonical_target)
            && let Some(export_entry) =
                file_data.exports.iter().find(|e| e.name == export_name.as_str())
            && export_entry.is_type
        {
            // The export itself is type-only.
            return QueryResult {
                id: query.id,
                success: true,
                error: None,
                references: Vec::new(),
                total_references: 0,
                is_type_only: Some(true),
                alias_chain: Vec::new(),
            };
        }

        // Check all import sites: if every import of this export is type-only,
        // then the export is effectively type-only.
        let mut has_any_import = false;
        let mut all_type_only = true;

        if let Some(importers) = self.import_reverse.get(&canonical_target) {
            for (importer_path, import_idx) in importers {
                if let Some(file_data) = self.files.get(importer_path) {
                    let imp = &file_data.imports[*import_idx];

                    for (imported_name, _local_alias, is_type_specifier) in &imp.names {
                        let matches = imported_name == export_name.as_str()
                            || (imported_name == "default" && export_name == "default");
                        if matches {
                            has_any_import = true;
                            if !imp.is_type_import && !is_type_specifier {
                                all_type_only = false;
                                break;
                            }
                        }
                    }
                    if !all_type_only {
                        break;
                    }
                }
            }
        }

        let is_type_only = if has_any_import { Some(all_type_only) } else { None };

        QueryResult {
            id: query.id,
            success: true,
            error: None,
            references: Vec::new(),
            total_references: 0,
            is_type_only,
            alias_chain: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

/// Discover TypeScript/JavaScript files covered by the given tsconfig paths.
fn discover_files(tsconfig_paths: &[String], project_root: &str) -> Vec<PathBuf> {
    let mut all_files = Vec::new();
    let mut seen = FxHashSet::default();

    for tsconfig_path in tsconfig_paths {
        let tsconfig_dir =
            Path::new(tsconfig_path).parent().unwrap_or_else(|| Path::new(project_root));

        // Try to read and parse the tsconfig to get include/files patterns.
        let patterns = read_tsconfig_patterns(tsconfig_path, tsconfig_dir);

        for pattern in &patterns {
            let full_pattern = tsconfig_dir.join(pattern).to_string_lossy().to_string();
            match glob::glob(&full_pattern) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        if is_ts_js_file(&entry) && seen.insert(entry.clone()) {
                            all_files.push(entry);
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        pattern = %full_pattern,
                        error = %e,
                        "invalid glob pattern"
                    );
                }
            }
        }

        // If no patterns matched any files, fall back to walking the directory.
        if all_files.is_empty() {
            tracing::debug!(
                tsconfig = %tsconfig_path,
                "no files matched tsconfig patterns, falling back to directory walk"
            );
            walk_directory(tsconfig_dir, &mut all_files, &mut seen);
        }
    }

    // If no tsconfig produced files, walk the project root.
    if all_files.is_empty() {
        tracing::debug!("no tsconfig files produced results, walking project root");
        walk_directory(Path::new(project_root), &mut all_files, &mut seen);
    }

    all_files
}

/// Walk a directory tree, collecting TypeScript/JavaScript files.
fn walk_directory(root: &Path, files: &mut Vec<PathBuf>, seen: &mut FxHashSet<PathBuf>) {
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            // Skip common non-source directories.
            !matches!(
                name.as_ref(),
                "node_modules" | ".git" | "dist" | "build" | ".next" | "coverage"
            )
        })
        .flatten()
    {
        let path = entry.into_path();
        if path.is_file() && is_ts_js_file(&path) && seen.insert(path.clone()) {
            files.push(path);
        }
    }
}

/// Read tsconfig.json and extract include/files patterns.
fn read_tsconfig_patterns(tsconfig_path: &str, _tsconfig_dir: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(tsconfig_path) else {
        return default_ts_patterns();
    };

    // Strip JSONC comments for tsconfig parsing.
    let stripped = strip_jsonc_comments(&content);

    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return default_ts_patterns();
    };

    let mut patterns = Vec::new();

    // Extract "include" array.
    if let Some(include) = parsed.get("include").and_then(|v| v.as_array()) {
        for item in include {
            if let Some(s) = item.as_str() {
                patterns.push(s.to_string());
            }
        }
    }

    // Extract "files" array.
    if let Some(files) = parsed.get("files").and_then(|v| v.as_array()) {
        for item in files {
            if let Some(s) = item.as_str() {
                patterns.push(s.to_string());
            }
        }
    }

    if patterns.is_empty() {
        return default_ts_patterns();
    }

    patterns
}

fn default_ts_patterns() -> Vec<String> {
    vec![
        "**/*.ts".to_string(),
        "**/*.tsx".to_string(),
        "**/*.js".to_string(),
        "**/*.jsx".to_string(),
        "**/*.mts".to_string(),
        "**/*.cts".to_string(),
    ]
}

fn is_ts_js_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs")
    )
}

/// Strip single-line and block comments from JSONC content.
fn strip_jsonc_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if escaped {
            result.push(c);
            escaped = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            result.push(c);
            continue;
        }
        if c == '"' {
            in_string = true;
            result.push(c);
        } else if c == '/' {
            if chars.peek() == Some(&'/') {
                chars.next();
                for nc in chars.by_ref() {
                    if nc == '\n' {
                        result.push('\n');
                        break;
                    }
                }
            } else if chars.peek() == Some(&'*') {
                chars.next();
                loop {
                    match chars.next() {
                        Some('*') if chars.peek() == Some(&'/') => {
                            chars.next();
                            result.push(' ');
                            break;
                        }
                        Some('\n') => result.push('\n'),
                        Some(_) | None => {
                            if chars.peek().is_none() {
                                break;
                            }
                        }
                    }
                }
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Parsing and extraction
// ---------------------------------------------------------------------------

/// Determine the oxc `SourceType` from a file path extension.
fn determine_source_type(path: &Path) -> oxc_span::SourceType {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("js");
    match ext {
        "ts" | "mts" | "cts" => oxc_span::SourceType::ts(),
        "tsx" => oxc_span::SourceType::tsx(),
        "jsx" => oxc_span::SourceType::jsx(),
        "cjs" => oxc_span::SourceType::cjs(),
        _ => oxc_span::SourceType::mjs(),
    }
}

/// Parse a single file and extract semantic data (exports, imports, usages).
fn parse_and_extract(path: &Path, source: &str) -> FileData {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = determine_source_type(path);
    let parser_ret = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if parser_ret.panicked {
        tracing::debug!(path = %path.display(), "parse failed (panicked)");
        return FileData::default();
    }

    let program = &parser_ret.program;
    let mut data = FileData::default();

    // Walk top-level statements to extract imports, exports, namespace aliases.
    for stmt in &program.body {
        extract_statement(stmt, source, &mut data);
    }

    // Build semantic model to extract identifier usages with scope information.
    let semantic_ret = oxc_semantic::SemanticBuilder::new().build(program);
    let semantic = &semantic_ret.semantic;
    let scoping = semantic.scoping();

    for symbol_id in scoping.symbol_ids() {
        let name = scoping.symbol_name(symbol_id);

        for reference in scoping.get_resolved_references(symbol_id) {
            let ref_span = semantic.reference_span(reference);
            let (line, column) = offset_to_line_col(source, ref_span.start);

            // Determine if this reference is in a type-only position.
            // oxc_semantic tracks this: read references vs type references.
            let is_type_only = !reference.is_value();

            data.usages.push(UsageEntry {
                name: name.to_string(),
                line,
                column,
                is_type_only,
                is_write: reference.is_write(),
            });
        }
    }

    data
}

/// Extract imports, exports, and namespace aliases from a single statement.
fn extract_statement(stmt: &oxc_ast::ast::Statement<'_>, source: &str, data: &mut FileData) {
    use oxc_ast::ast::Statement;

    match stmt {
        Statement::ImportDeclaration(import) => {
            let source_str = import.source.value.to_string();
            let is_type = import.import_kind.is_type();
            let (line, column) = offset_to_line_col(source, import.span.start);
            let mut names = Vec::new();

            if let Some(specifiers) = &import.specifiers {
                for spec in specifiers {
                    use oxc_ast::ast::ImportDeclarationSpecifier;
                    match spec {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            let is_type_spec = s.import_kind.is_type();
                            names.push((
                                s.imported.name().to_string(),
                                s.local.name.to_string(),
                                is_type_spec,
                            ));
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            names.push(("default".to_string(), s.local.name.to_string(), false));
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                            names.push(("*".to_string(), s.local.name.to_string(), false));
                        }
                    }
                }
            }

            data.imports.push(ImportEntry {
                source: source_str,
                names,
                is_type_import: is_type,
                line,
                column,
            });
        }

        Statement::ExportNamedDeclaration(export) => {
            if export.source.is_some() {
                // Re-export: `export { x } from './y'` -- not a local export.
                return;
            }

            let is_type = export.export_kind.is_type();

            // Named specifiers: `export { Foo, Bar }`
            for spec in &export.specifiers {
                let name = spec.exported.name().to_string();
                data.exports.push(ExportEntry { name, is_type, offset: export.span.start });
            }

            // Declaration: `export const foo = ...`, `export class Foo { ... }`
            if let Some(decl) = &export.declaration {
                extract_exports_from_decl(decl, is_type, export.span.start, data);
            }
        }

        Statement::ExportDefaultDeclaration(export) => {
            data.exports.push(ExportEntry {
                name: "default".to_string(),
                is_type: false,
                offset: export.span.start,
            });
        }

        // Handle `import NS = Other.Sub` (`TSImportEqualsDeclaration`)
        Statement::TSImportEqualsDeclaration(decl) => {
            let alias = decl.id.name.to_string();

            if let oxc_ast::ast::TSModuleReference::QualifiedName(qname) = &decl.module_reference {
                let chain = collect_qualified_name(qname);
                data.namespace_aliases.push(NamespaceAliasEntry {
                    alias,
                    target_chain: chain,
                    offset: decl.span.start,
                });
            }
        }

        _ => {}
    }
}

/// Collect a dot-separated qualified name into a `Vec` of segments.
fn collect_qualified_name(name: &oxc_ast::ast::TSQualifiedName<'_>) -> Vec<String> {
    let mut parts = Vec::new();
    collect_qualified_name_inner(&name.left, &mut parts);
    parts.push(name.right.name.to_string());
    parts
}

fn collect_qualified_name_inner(name: &oxc_ast::ast::TSTypeName<'_>, parts: &mut Vec<String>) {
    match name {
        oxc_ast::ast::TSTypeName::IdentifierReference(ident) => {
            parts.push(ident.name.to_string());
        }
        oxc_ast::ast::TSTypeName::QualifiedName(qname) => {
            collect_qualified_name_inner(&qname.left, parts);
            parts.push(qname.right.name.to_string());
        }
        oxc_ast::ast::TSTypeName::ThisExpression(_) => {}
    }
}

/// Extract export names from a declaration.
fn extract_exports_from_decl(
    decl: &oxc_ast::ast::Declaration<'_>,
    is_type: bool,
    span_start: u32,
    data: &mut FileData,
) {
    use oxc_ast::ast::Declaration;

    match decl {
        Declaration::VariableDeclaration(var_decl) => {
            for declarator in &var_decl.declarations {
                if let Some(name) = extract_binding_name(&declarator.id) {
                    data.exports.push(ExportEntry { name, is_type, offset: span_start });
                }
            }
        }
        Declaration::FunctionDeclaration(func) => {
            if let Some(id) = &func.id {
                data.exports.push(ExportEntry {
                    name: id.name.to_string(),
                    is_type,
                    offset: span_start,
                });
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                data.exports.push(ExportEntry {
                    name: id.name.to_string(),
                    is_type,
                    offset: span_start,
                });
            }
        }
        Declaration::TSEnumDeclaration(enum_decl) => {
            data.exports.push(ExportEntry {
                name: enum_decl.id.name.to_string(),
                is_type,
                offset: span_start,
            });
        }
        Declaration::TSTypeAliasDeclaration(type_alias) => {
            data.exports.push(ExportEntry {
                name: type_alias.id.name.to_string(),
                is_type: true,
                offset: span_start,
            });
        }
        Declaration::TSInterfaceDeclaration(iface) => {
            data.exports.push(ExportEntry {
                name: iface.id.name.to_string(),
                is_type: true,
                offset: span_start,
            });
        }
        Declaration::TSModuleDeclaration(module) => {
            let name = match &module.id {
                oxc_ast::ast::TSModuleDeclarationName::Identifier(ident) => ident.name.to_string(),
                oxc_ast::ast::TSModuleDeclarationName::StringLiteral(lit) => lit.value.to_string(),
            };
            data.exports.push(ExportEntry { name, is_type, offset: span_start });
        }
        _ => {}
    }
}

/// Extract the binding name from a binding pattern (simple identifier case).
fn extract_binding_name(pattern: &oxc_ast::ast::BindingPattern<'_>) -> Option<String> {
    use oxc_ast::ast::BindingPattern;
    match pattern {
        BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Import resolution (simplified)
// ---------------------------------------------------------------------------

/// Resolve a module specifier relative to the importing file's directory.
/// Returns a canonical file path if the target can be found.
fn resolve_import_source(specifier: &str, importer_dir: &Path) -> Option<String> {
    // Only resolve relative imports (./ ../).
    // Bare specifiers (npm packages) cannot be resolved to local files.
    if !specifier.starts_with('.') {
        return None;
    }

    let base = importer_dir.join(specifier);

    // Try exact path first, then with common extensions.
    let extensions = [
        "",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        ".mts",
        ".cts",
        ".mjs",
        ".cjs",
        "/index.ts",
        "/index.tsx",
        "/index.js",
        "/index.jsx",
        "/index.mts",
        "/index.cts",
    ];

    for ext in &extensions {
        let candidate = if ext.is_empty() {
            base.clone()
        } else {
            PathBuf::from(format!("{}{ext}", base.display()))
        };

        if candidate.is_file() {
            return Some(canonicalize_path(&candidate.to_string_lossy()));
        }
    }

    // Fallback: try to find the file by normalizing the path without checking
    // the filesystem (for cases where the file is in the index but the path
    // format differs slightly).
    let normalized = normalize_path(&base);
    let normalized_str = normalized.to_string_lossy().to_string();

    // Try with extensions on the normalized path.
    for ext in &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts"] {
        let candidate = format!("{normalized_str}{ext}");
        let candidate_path = Path::new(&candidate);
        if candidate_path.is_file() {
            return Some(canonicalize_path(&candidate));
        }
    }

    None
}

/// Normalize a path by resolving `.` and `..` components without hitting the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

/// Canonicalize a file path string to a consistent form.
fn canonicalize_path(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => {
            // Fallback: normalize without filesystem.
            normalize_path(Path::new(path)).to_string_lossy().to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Convert a byte offset to (line, column) where line is 1-based and column is
/// 0-based.
fn offset_to_line_col(source: &str, offset: u32) -> (u32, u32) {
    let offset = offset as usize;
    let bytes = source.as_bytes();
    let mut line: u32 = 1;
    let mut col: u32 = 0;

    for (i, &byte) in bytes.iter().enumerate() {
        if i >= offset {
            break;
        }
        if byte == b'\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }

    (line, col)
}

// ---------------------------------------------------------------------------
// Main and protocol handling
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("pruneguard-tsgo: semantic helper for pruneguard");
        eprintln!("Usage: pruneguard-tsgo headless");
        eprintln!("       pruneguard-tsgo --version");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "headless" => {
            init_tracing();
            if let Err(e) = run_headless() {
                eprintln!("pruneguard-tsgo: fatal error: {e}");
                std::process::exit(1);
            }
        }
        "--version" | "-V" => {
            println!("pruneguard-tsgo {}", env!("CARGO_PKG_VERSION"));
        }
        "--help" | "-h" => {
            println!("pruneguard-tsgo — semantic helper for pruneguard dead-code precision");
            println!();
            println!("USAGE:");
            println!("  pruneguard-tsgo headless    Run in headless mode (stdio protocol)");
            println!("  pruneguard-tsgo --version   Print version");
            println!("  pruneguard-tsgo --help      Print this help");
        }
        other => {
            eprintln!("pruneguard-tsgo: unknown command: {other}");
            eprintln!("Usage: pruneguard-tsgo headless");
            std::process::exit(1);
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("PRUNEGUARD_TSGO_LOG")
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
}

fn run_headless() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    // Phase 1: Read handshake from Rust core
    let (msg_type, payload) = read_message(&mut stdin)?;
    if msg_type != MessageType::Query {
        return Err(format!("expected Query (handshake), got {msg_type:?}").into());
    }

    let handshake: HandshakeRequest = serde_json::from_slice(&payload)?;
    if handshake.version != PROTOCOL_VERSION {
        let err = ErrorMessage {
            error: format!(
                "protocol version mismatch: helper speaks v{}, core sent v{}",
                PROTOCOL_VERSION, handshake.version
            ),
            fatal: true,
        };
        send_message(&mut stdout, MessageType::Error, &serde_json::to_vec(&err)?)?;
        return Err("protocol version mismatch".into());
    }

    tracing::info!(
        project_root = %handshake.project_root,
        tsconfigs = handshake.tsconfig_paths.len(),
        "initializing semantic helper"
    );

    // Phase 1b: Initialize semantic index -- discover files, parse, and extract.
    let started = std::time::Instant::now();

    let (index, projects_loaded, files_indexed) =
        SemanticIndex::build(&handshake.tsconfig_paths, &handshake.project_root);

    #[allow(clippy::cast_possible_truncation)]
    let init_ms = started.elapsed().as_millis() as u64;
    let ready = ReadyMessage { version: PROTOCOL_VERSION, projects_loaded, files_indexed, init_ms };
    send_message(&mut stdout, MessageType::Ready, &serde_json::to_vec(&ready)?)?;

    tracing::info!(
        projects = projects_loaded,
        files = files_indexed,
        init_ms = started.elapsed().as_millis(),
        "semantic helper ready"
    );

    // Phase 2: Query loop
    loop {
        let (msg_type, payload) = match read_message(&mut stdin) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::debug!("stdin closed or error: {e}");
                break;
            }
        };

        match msg_type {
            MessageType::Shutdown => {
                tracing::info!("received shutdown signal");
                break;
            }
            MessageType::Query => {
                let batch: QueryBatch = serde_json::from_slice(&payload)?;
                let batch_started = std::time::Instant::now();

                tracing::debug!(
                    queries = batch.queries.len(),
                    tsconfig = %batch.tsconfig_path,
                    "processing query batch"
                );

                // Process each query against the semantic index.
                let results: Vec<QueryResult> =
                    batch.queries.iter().map(|q| index.process_query(q)).collect();

                #[allow(clippy::cast_possible_truncation)]
                let batch_ms = batch_started.elapsed().as_millis() as u64;
                let response = ResponseBatch { results, batch_ms };
                send_message(&mut stdout, MessageType::Response, &serde_json::to_vec(&response)?)?;
            }
            _ => {
                let err = ErrorMessage {
                    error: format!("unexpected message type: {msg_type:?}"),
                    fatal: false,
                };
                send_message(&mut stdout, MessageType::Error, &serde_json::to_vec(&err)?)?;
            }
        }
    }

    Ok(())
}

fn read_message(
    reader: &mut impl Read,
) -> Result<(MessageType, Vec<u8>), Box<dyn std::error::Error>> {
    let mut header = [0u8; HEADER_SIZE];
    reader.read_exact(&mut header)?;

    let (size, msg_type) =
        decode_header(header).ok_or_else(|| format!("invalid header: {header:?}"))?;

    let mut payload = vec![0u8; size as usize];
    reader.read_exact(&mut payload)?;

    Ok((msg_type, payload))
}

fn send_message(
    writer: &mut impl Write,
    msg_type: MessageType,
    payload: &[u8],
) -> Result<(), std::io::Error> {
    let msg = encode_message(msg_type, payload);
    writer.write_all(&msg)?;
    writer.flush()
}
