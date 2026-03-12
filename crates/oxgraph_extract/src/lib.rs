use std::path::Path;

use compact_str::CompactString;
use oxgraph_fs::FileRecord;
use oxgraph_resolver::ResolvedEdge;
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
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(file: FileRecord) -> Self {
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

/// Extract all import/export facts from a single JS/TS file.
pub fn extract_file_facts(path: &Path, source: &str) -> Result<FileFacts, ExtractError> {
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

/// Errors from extraction.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("failed to parse {path}")]
    ParseFailed { path: std::path::PathBuf },
}
