use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageId {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    C,
    Cpp,
    Java,
    Ruby,
    Zig,
}

impl LanguageId {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        match ext {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "js" | "jsx" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "go" => Some(Self::Go),
            "c" | "h" => Some(Self::C),
            "cpp" | "cxx" | "cc" | "hpp" | "hh" | "hxx" => Some(Self::Cpp),
            "java" => Some(Self::Java),
            "rb" => Some(Self::Ruby),
            "zig" => Some(Self::Zig),
            _ => None,
        }
    }

    fn grammar(&self) -> Language {
        match self {
            Self::Rust => Language::new(tree_sitter_rust::LANGUAGE),
            Self::Python => Language::new(tree_sitter_python::LANGUAGE),
            Self::JavaScript => Language::new(tree_sitter_javascript::LANGUAGE),
            Self::TypeScript => Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
            Self::Tsx => Language::new(tree_sitter_typescript::LANGUAGE_TSX),
            Self::Go => Language::new(tree_sitter_go::LANGUAGE),
            Self::C => Language::new(tree_sitter_c::LANGUAGE),
            Self::Cpp => Language::new(tree_sitter_cpp::LANGUAGE),
            Self::Java => Language::new(tree_sitter_java::LANGUAGE),
            Self::Ruby => Language::new(tree_sitter_ruby::LANGUAGE),
            Self::Zig => Language::new(tree_sitter_zig::LANGUAGE),
        }
    }

    /// Returns (capture_kind, query_pattern) pairs.
    /// Each pattern is tried independently; failed patterns are silently skipped.
    /// All definition patterns capture both @name (for the identifier) and @node (for the whole definition).
    fn patterns(&self) -> Vec<(CaptureKind, &'static str)> {
        match self {
            Self::Rust => vec![
                (CaptureKind::Def(DefKind::Function), "(function_item name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Struct), "(struct_item name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Trait), "(trait_item name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Impl), "(impl_item type: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Enum), "(enum_item name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::TypeAlias), "(type_item name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Constant), "(const_item name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Static), "(static_item name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Function), "(function_signature_item name: (identifier) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (identifier) @name)"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (field_expression field: (field_identifier) @name))"),
                (CaptureKind::Import, "(use_declaration (scoped_identifier name: (identifier) @name))"),
                (CaptureKind::Import, "(use_declaration (scoped_use_list list: (use_list (identifier) @name)))"),
                (CaptureKind::Import, "(use_declaration argument: (use_as_clause alias: (identifier) @name))"),
                (CaptureKind::Import, "(use_declaration argument: (identifier) @name)"),
            ],
            Self::Python => vec![
                (CaptureKind::Def(DefKind::Function), "(function_definition name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Class), "(class_definition name: (identifier) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call function: (identifier) @name)"),
                (CaptureKind::Import, "(import_statement name: (dotted_name) @name)"),
                (CaptureKind::Import, "(import_from_statement name: (dotted_name) @name)"),
            ],
            Self::JavaScript => vec![
                (CaptureKind::Def(DefKind::Function), "(function_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Class), "(class_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Method), "(method_definition name: (property_identifier) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (identifier) @name)"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (member_expression property: (property_identifier) @name))"),
                (CaptureKind::Import, "(import_statement source: (string) (import_clause name: (identifier) @name))"),
                (CaptureKind::Import, "(import_statement source: (string) (import_clause (named_imports (import_specifier name: (identifier) @name))))"),
            ],
            Self::TypeScript | Self::Tsx => vec![
                (CaptureKind::Def(DefKind::Function), "(function_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Class), "(class_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Method), "(method_definition name: (property_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Trait), "(interface_declaration name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::TypeAlias), "(type_alias_declaration name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Enum), "(enum_declaration name: (identifier) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (identifier) @name)"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (member_expression property: (property_identifier) @name))"),
                (CaptureKind::Import, "(import_statement source: (string) (import_clause name: (identifier) @name))"),
                (CaptureKind::Import, "(import_statement source: (string) (import_clause (named_imports (import_specifier name: (identifier) @name))))"),
            ],
            Self::Go => vec![
                (CaptureKind::Def(DefKind::Function), "(function_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Method), "(method_declaration name: (field_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Struct), "(type_declaration (type_spec name: (type_identifier) @name)) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (identifier) @name)"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (selector_expression field: (field_identifier) @name))"),
                (CaptureKind::Import, "(import_declaration (import_spec name: (package_identifier)? path: (interpreted_string_literal) @name))"),
            ],
            Self::C => vec![
                (CaptureKind::Def(DefKind::Function), "(function_definition declarator: (function_declarator declarator: (identifier) @name)) @node"),
                (CaptureKind::Def(DefKind::Struct), "(struct_specifier name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Struct), "(union_specifier name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Enum), "(enum_specifier name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::TypeAlias), "(type_definition declarator: (type_identifier) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (identifier) @name)"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (field_expression field: (field_identifier) @name))"),
                (CaptureKind::Import, "(preproc_include path: (string_literal) @name)"),
                (CaptureKind::Import, "(preproc_include path: (system_lib_string) @name)"),
            ],
            Self::Cpp => vec![
                (CaptureKind::Def(DefKind::Function), "(function_definition declarator: (function_declarator declarator: (identifier) @name)) @node"),
                (CaptureKind::Def(DefKind::Function), "(template_declaration declaration: (function_definition declarator: (function_declarator declarator: (identifier) @name))) @node"),
                (CaptureKind::Def(DefKind::Class), "(class_specifier name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Struct), "(struct_specifier name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Enum), "(enum_specifier name: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::TypeAlias), "(type_definition declarator: (type_identifier) @name) @node"),
                (CaptureKind::Def(DefKind::TypeAlias), "(alias_declaration name: (identifier) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (identifier) @name)"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (field_expression field: (field_identifier) @name))"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (qualified_identifier name: (identifier) @name))"),
                (CaptureKind::Import, "(preproc_include path: (string_literal) @name)"),
                (CaptureKind::Import, "(preproc_include path: (system_lib_string) @name)"),
            ],
            Self::Java => vec![
                (CaptureKind::Def(DefKind::Class), "(class_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Trait), "(interface_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Method), "(method_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Enum), "(enum_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Class), "(record_declaration name: (identifier) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(method_invocation name: (identifier) @name)"),
                (CaptureKind::Import, "(import_declaration name: (scoped_identifier name: (identifier) @name))"),
            ],
            Self::Ruby => vec![
                (CaptureKind::Def(DefKind::Method), "(method name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Method), "(singleton_method name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Class), "(class name: (constant) @name) @node"),
                (CaptureKind::Def(DefKind::Class), "(module name: (constant) @name) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call method: (identifier) @name)"),
                (CaptureKind::Import, "(require path: (string) @name)"),
            ],
            Self::Zig => vec![
                (CaptureKind::Def(DefKind::Function), "(function_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Struct), "(container_declaration name: (identifier) @name (container_kind struct)) @node"),
                (CaptureKind::Def(DefKind::Enum), "(container_declaration name: (identifier) @name (container_kind enum)) @node"),
                (CaptureKind::Def(DefKind::TypeAlias), "(type_declaration name: (identifier) @name) @node"),
                (CaptureKind::Def(DefKind::Constant), "(variable_declaration name: (identifier) @name (container_kind const)) @node"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (identifier) @name)"),
                (CaptureKind::Ref(RefKind::Call), "(call_expression function: (field_expression field: (identifier) @name))"),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureKind {
    Def(DefKind),
    Ref(RefKind),
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DefKind {
    Function,
    Struct,
    Trait,
    Impl,
    Enum,
    TypeAlias,
    Constant,
    Static,
    Class,
    Method,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RefKind {
    Call,
}

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub path: std::path::PathBuf,
    pub language: LanguageId,
    pub definitions: Vec<ParsedDef>,
    pub references: Vec<ParsedRef>,
    pub imports: Vec<ParsedImport>,
}

#[derive(Debug, Clone)]
pub struct ParsedDef {
    pub name: String,
    pub kind: DefKind,
    pub start_line: usize,
    pub end_line: usize,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedRef {
    pub name: String,
    pub kind: RefKind,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub name: String,
}

pub fn parse_file(path: &Path) -> Result<ParsedFile> {
    let language = LanguageId::from_path(path).context("unsupported language")?;
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    parse_source(path, language, &source)
}

pub fn parse_source(path: &Path, language: LanguageId, source: &str) -> Result<ParsedFile> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar())
        .context("setting language")?;

    let tree = parser.parse(source, None).context("parsing")?;
    let root = tree.root_node();

    let mut definitions: Vec<ParsedDef> = Vec::new();
    let mut references: Vec<ParsedRef> = Vec::new();
    let mut imports: Vec<ParsedImport> = Vec::new();

    let source_bytes = source.as_bytes();

    for (kind, pattern_str) in language.patterns() {
        let Ok(query) = Query::new(&language.grammar(), pattern_str) else {
            continue;
        };
        process_pattern(
            &query,
            &kind,
            root,
            source_bytes,
            &mut definitions,
            &mut references,
            &mut imports,
        );
    }

    // Extract doc comments for each definition
    for def in &mut definitions {
        if def.doc.is_none() {
            def.doc = extract_doc_comment(source, def.start_line);
        }
    }

    Ok(ParsedFile {
        path: path.to_path_buf(),
        language,
        definitions,
        references,
        imports,
    })
}

fn process_pattern(
    query: &Query,
    kind: &CaptureKind,
    root: tree_sitter::Node<'_>,
    source_bytes: &[u8],
    definitions: &mut Vec<ParsedDef>,
    references: &mut Vec<ParsedRef>,
    imports: &mut Vec<ParsedImport>,
) {
    let caps = query.capture_names();
    let name_idx = caps.iter().position(|n| *n == "name").map(|i| i as u32);
    let node_idx = caps.iter().position(|n| *n == "node").map(|i| i as u32);
    let Some(name_capture_idx) = name_idx else { return };

    let mut cursor = QueryCursor::new();
    let mut query_matches = cursor.matches(query, root, source_bytes);

    while let Some(match_) = query_matches.next() {
        let mut name_node = None;
        let mut span_node = None;
        for capture in match_.captures {
            if capture.index == node_idx.unwrap_or(u32::MAX) {
                span_node = Some(capture.node);
            } else if capture.index == name_capture_idx {
                name_node = Some(capture.node);
            }
        }

        let Some(name_node) = name_node else { continue };
        let Ok(name) = name_node.utf8_text(source_bytes) else { continue };
        let name = name.to_string();
        let line = name_node.start_position().row + 1;

        push_capture(kind, &name, line, span_node, definitions, references, imports);
    }
}

fn push_capture(
    kind: &CaptureKind,
    name: &str,
    line: usize,
    span_node: Option<tree_sitter::Node<'_>>,
    definitions: &mut Vec<ParsedDef>,
    references: &mut Vec<ParsedRef>,
    imports: &mut Vec<ParsedImport>,
) {
    match kind {
        CaptureKind::Def(def_kind) => {
            let (start_line, end_line) = if let Some(node) = span_node {
                (node.start_position().row + 1, node.end_position().row + 1)
            } else {
                (line, line)
            };
            definitions.push(ParsedDef {
                name: name.to_string(),
                kind: *def_kind,
                start_line,
                end_line,
                doc: None,
            });
        }
        CaptureKind::Ref(_) => {
            references.push(ParsedRef {
                name: name.to_string(),
                kind: RefKind::Call,
                line,
            });
        }
        CaptureKind::Import => {
            imports.push(ParsedImport {
                name: name.to_string(),
            });
        }
    }
}

fn is_doc_line(trimmed: &str) -> bool {
    trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("// ")
        || trimmed.starts_with("//\t")
        || trimmed.starts_with("# ")
        || trimmed.starts_with("##")
        || {
            let bytes = trimmed.as_bytes();
            bytes.len() > 1
                && bytes[0] == b'#'
                && bytes[1] != b'['
                && bytes[1] != b'!'
        }
}

fn is_block_start(trimmed: &str) -> bool {
    trimmed.starts_with("/**") || trimmed.starts_with("/*!")
}

/// Extract doc comment text preceding the given definition line (1-indexed).
/// Looks backward for consecutive doc comment lines/blocks.
fn extract_doc_comment(source: &str, def_line: usize) -> Option<String> {
    if def_line <= 1 {
        return None;
    }
    let lines: Vec<&str> = source.lines().collect();
    let mut collected: Vec<&str> = Vec::new();
    let mut cur = def_line.saturating_sub(2); // 0-indexed, line before def

    loop {
        let raw = lines[cur];
        let trimmed = raw.trim();

        if trimmed.is_empty() {
            if collected.is_empty() {
                if cur == 0 {
                    break;
                }
                cur -= 1;
                continue;
            }
            break;
        }

        if is_doc_line(trimmed) {
            collected.push(raw);
            if cur == 0 {
                break;
            }
            cur -= 1;
            continue;
        }

        // Block comment start+end on same line
        if is_block_start(trimmed) && trimmed.contains("*/") {
            collected.push(raw);
            break;
        }

        // Block comment end (contains */) — seek backward for start
        if trimmed.ends_with("*/") || trimmed == "*/" {
            collected.push(raw);
            if cur == 0 {
                break;
            }
            cur -= 1;
            loop {
                let inner = lines[cur];
                collected.push(inner);
                if inner.trim_start().starts_with("/*") {
                    break;
                }
                if cur == 0 {
                    break;
                }
                cur -= 1;
            }
            break;
        }

        // Block comment start without end on same line
        if is_block_start(trimmed) {
            collected.push(raw);
            break;
        }

        // Rust attributes between doc comment and definition
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            if cur == 0 {
                break;
            }
            cur -= 1;
            continue;
        }

        break;
    }

    if collected.is_empty() {
        None
    } else {
        collected.reverse();
        Some(collected.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> ParsedFile {
        let path = Path::new("test.rs");
        parse_source(path, LanguageId::Rust, source).unwrap()
    }

    #[test]
    fn test_parse_function_definition() {
        let pf = parse_rust("fn hello() {}");
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "hello");
        assert_eq!(pf.definitions[0].kind, DefKind::Function);
        assert_eq!(pf.definitions[0].start_line, 1);
        assert_eq!(pf.definitions[0].end_line, 1);
    }

    #[test]
    fn test_parse_struct_definition() {
        let pf = parse_rust("struct Point { x: i32, y: i32 }");
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "Point");
        assert_eq!(pf.definitions[0].kind, DefKind::Struct);
    }

    #[test]
    fn test_parse_trait_definition() {
        let pf = parse_rust("trait Foo { fn bar(&self); }");
        assert_eq!(pf.definitions.len(), 2); // trait Foo + method bar
        assert_eq!(pf.definitions[0].name, "Foo");
        assert_eq!(pf.definitions[0].kind, DefKind::Trait);
        assert_eq!(pf.definitions[1].name, "bar");
        assert_eq!(pf.definitions[1].kind, DefKind::Function);
    }

    #[test]
    fn test_parse_function_calls() {
        let pf = parse_rust("fn caller() { callee(); another() }");
        let calls: Vec<_> = pf.references.iter().map(|r| r.name.as_str()).collect();
        assert!(calls.contains(&"callee"));
        assert!(calls.contains(&"another"));
    }

    #[test]
    fn test_parse_method_calls() {
        let pf = parse_rust("fn caller() { foo.bar(); baz.qux() }");
        let calls: Vec<_> = pf.references.iter().map(|r| r.name.as_str()).collect();
        assert!(calls.contains(&"bar"), "method bar not found in calls: {:?}", calls);
        assert!(calls.contains(&"qux"), "method qux not found in calls: {:?}", calls);
    }

    #[test]
    fn test_parse_imports() {
        let pf = parse_rust("use std::collections::HashMap;");
        let imports: Vec<_> = pf.imports.iter().map(|i| i.name.as_str()).collect();
        assert!(imports.contains(&"HashMap"), "imports: {:?}", imports);
    }

    #[test]
    fn test_parse_import_from_list() {
        let pf = parse_rust("use std::io::{BufRead, Write};");
        let imports: Vec<_> = pf.imports.iter().map(|i| i.name.as_str()).collect();
        assert!(imports.contains(&"BufRead"), "imports: {:?}", imports);
        assert!(imports.contains(&"Write"), "imports: {:?}", imports);
    }

    #[test]
    fn test_parse_python_function() {
        let path = Path::new("test.py");
        let pf = parse_source(path, LanguageId::Python, "def hello():\n    pass\n").unwrap();
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "hello");
        assert_eq!(pf.definitions[0].kind, DefKind::Function);
    }

    #[test]
    fn test_parse_python_class() {
        let path = Path::new("test.py");
        let pf = parse_source(path, LanguageId::Python, "class MyClass:\n    pass\n").unwrap();
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "MyClass");
        assert_eq!(pf.definitions[0].kind, DefKind::Class);
    }

    #[test]
    fn test_parse_javascript_function() {
        let path = Path::new("test.js");
        let pf = parse_source(path, LanguageId::JavaScript, "function hello() {}\n").unwrap();
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "hello");
        assert_eq!(pf.definitions[0].kind, DefKind::Function);
    }

    #[test]
    fn test_parse_typescript_interface() {
        let path = Path::new("test.ts");
        let pf = parse_source(path, LanguageId::TypeScript, "interface Foo { bar(): void }\n").unwrap();
        let iface = pf.definitions.iter().find(|d| d.kind == DefKind::Trait);
        assert!(iface.is_some(), "no trait definition found");
        assert_eq!(iface.unwrap().name, "Foo");
    }

    #[test]
    fn test_parse_go_function() {
        let path = Path::new("test.go");
        let pf = parse_source(path, LanguageId::Go, "package main\nfunc hello() {}\n").unwrap();
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "hello");
        assert_eq!(pf.definitions[0].kind, DefKind::Function);
    }

    #[test]
    fn test_parse_go_struct() {
        let path = Path::new("test.go");
        let pf = parse_source(path, LanguageId::Go, "package main\ntype Point struct {\n  x int\n}\n").unwrap();
        let s = pf.definitions.iter().find(|d| d.kind == DefKind::Struct);
        assert!(s.is_some());
        assert_eq!(s.unwrap().name, "Point");
    }

    #[test]
    fn test_language_from_path() {
        assert_eq!(LanguageId::from_path(Path::new("foo.rs")), Some(LanguageId::Rust));
        assert_eq!(LanguageId::from_path(Path::new("foo.py")), Some(LanguageId::Python));
        assert_eq!(LanguageId::from_path(Path::new("foo.js")), Some(LanguageId::JavaScript));
        assert_eq!(LanguageId::from_path(Path::new("foo.ts")), Some(LanguageId::TypeScript));
        assert_eq!(LanguageId::from_path(Path::new("foo.tsx")), Some(LanguageId::Tsx));
        assert_eq!(LanguageId::from_path(Path::new("foo.go")), Some(LanguageId::Go));
        assert_eq!(LanguageId::from_path(Path::new("foo.c")), Some(LanguageId::C));
        assert_eq!(LanguageId::from_path(Path::new("foo.h")), Some(LanguageId::C));
        assert_eq!(LanguageId::from_path(Path::new("foo.cpp")), Some(LanguageId::Cpp));
        assert_eq!(LanguageId::from_path(Path::new("foo.hpp")), Some(LanguageId::Cpp));
        assert_eq!(LanguageId::from_path(Path::new("foo.java")), Some(LanguageId::Java));
        assert_eq!(LanguageId::from_path(Path::new("foo.rb")), Some(LanguageId::Ruby));
        assert_eq!(LanguageId::from_path(Path::new("foo.zig")), Some(LanguageId::Zig));
        assert_eq!(LanguageId::from_path(Path::new("foo.md")), None);
        assert_eq!(LanguageId::from_path(Path::new("foo")), None);
    }

    #[test]
    fn test_parse_c_function() {
        let path = Path::new("test.c");
        let pf = parse_source(path, LanguageId::C, "int add(int a, int b) { return a + b; }\n").unwrap();
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "add");
        assert_eq!(pf.definitions[0].kind, DefKind::Function);
    }

    #[test]
    fn test_parse_c_struct() {
        let path = Path::new("test.c");
        let pf = parse_source(path, LanguageId::C, "struct Point { int x; int y; };\n").unwrap();
        let s = pf.definitions.iter().find(|d| d.kind == DefKind::Struct);
        assert!(s.is_some());
        assert_eq!(s.unwrap().name, "Point");
    }

    #[test]
    fn test_parse_c_include() {
        let path = Path::new("test.c");
        let pf = parse_source(path, LanguageId::C, "#include <stdio.h>\n#include \"myheader.h\"\n").unwrap();
        assert_eq!(pf.imports.len(), 2);
        let names: Vec<_> = pf.imports.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"<stdio.h>"));
        assert!(names.contains(&"\"myheader.h\""));
    }

    #[test]
    fn test_parse_cpp_class() {
        let path = Path::new("test.cpp");
        let pf = parse_source(path, LanguageId::Cpp, "class MyClass {\npublic:\n  int getValue() { return 42; }\n};\n").unwrap();
        let cls = pf.definitions.iter().find(|d| d.kind == DefKind::Class);
        assert!(cls.is_some(), "no class found in {:?}", pf.definitions);
        assert_eq!(cls.unwrap().name, "MyClass");
    }

    #[test]
    fn test_parse_java_class() {
        let path = Path::new("Test.java");
        let pf = parse_source(path, LanguageId::Java, "public class Test {\n  public void hello() {}\n}\n").unwrap();
        let cls = pf.definitions.iter().find(|d| d.kind == DefKind::Class);
        assert!(cls.is_some());
        assert_eq!(cls.unwrap().name, "Test");
    }

    #[test]
    fn test_parse_ruby_method() {
        let path = Path::new("test.rb");
        let pf = parse_source(path, LanguageId::Ruby, "def hello(name)\n  puts name\nend\n").unwrap();
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "hello");
        assert_eq!(pf.definitions[0].kind, DefKind::Method);
    }

    #[test]
    fn test_parse_zig_function() {
        let path = Path::new("test.zig");
        let pf = parse_source(path, LanguageId::Zig, "fn hello() void {}\n").unwrap();
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "hello");
        assert_eq!(pf.definitions[0].kind, DefKind::Function);
    }

    #[test]
    fn test_parse_empty_file() {
        let pf = parse_rust("");
        assert_eq!(pf.definitions.len(), 0);
        assert_eq!(pf.references.len(), 0);
        assert_eq!(pf.imports.len(), 0);
    }

    #[test]
    fn test_parse_multiple_definitions() {
        let src = "fn a() {}\nfn b() {}\nstruct C {}";
        let pf = parse_rust(src);
        assert_eq!(pf.definitions.len(), 3);
    }

    #[test]
    fn test_call_line_numbers() {
        let pf = parse_rust("fn foo() {\n  bar()\n}\n");
        assert_eq!(pf.references.len(), 1);
        assert_eq!(pf.references[0].name, "bar");
        assert_eq!(pf.references[0].line, 2);
    }

    #[test]
    fn test_unsupported_language() {
        let result = parse_file(Path::new("foo.txt"));
        assert!(result.is_err());
    }

    // -- Doc comment extraction tests --

    #[test]
    fn test_extract_doc_line() {
        assert!(is_doc_line("/// docs"));
        assert!(is_doc_line("//! inner docs"));
        assert!(is_doc_line("// comment"));
        assert!(is_doc_line("# comment"));
        assert!(is_doc_line("## doc"));
        assert!(is_doc_line("//\t tabbed"));
        assert!(!is_doc_line("#[derive(Debug)]"));
        assert!(!is_doc_line("fn hello() {}"));
        assert!(!is_doc_line("pub struct Foo;"));
    }

    #[test]
    fn test_rust_doc_comment_three_slash() {
        let src = "/// Adds two numbers together\nfn add() {}";
        let pf = parse_rust(src);
        assert_eq!(pf.definitions.len(), 1);
        assert_eq!(pf.definitions[0].name, "add");
        assert_eq!(pf.definitions[0].doc.as_deref(), Some("/// Adds two numbers together"));
    }

    #[test]
    fn test_rust_doc_comment_multiple_lines() {
        let src = "/// Adds two numbers\n/// # Example\n/// ```\n/// let x = add(2, 3);\n/// ```\nfn add() {}";
        let pf = parse_rust(src);
        assert_eq!(pf.definitions[0].doc.as_deref(), Some(
            "/// Adds two numbers\n/// # Example\n/// ```\n/// let x = add(2, 3);\n/// ```"
        ));
    }

    #[test]
    fn test_rust_doc_with_attributes() {
        let src = "/// Doc comment\n#[inline]\nfn foo() {}";
        let pf = parse_rust(src);
        assert_eq!(pf.definitions[0].doc.as_deref(), Some("/// Doc comment"));
    }

    #[test]
    fn test_rust_block_doc_comment() {
        let src = "/** Documentation */\nfn foo() {}";
        let pf = parse_rust(src);
        assert_eq!(pf.definitions[0].doc.as_deref(), Some("/** Documentation */"));
    }

    #[test]
    fn test_rust_block_doc_multiline() {
        let src = "/**\n * Documentation\n */\nfn foo() {}";
        let pf = parse_rust(src);
        let doc = pf.definitions[0].doc.as_deref().unwrap();
        assert!(doc.contains("/**"));
        assert!(doc.contains("*/"));
        assert!(doc.contains("Documentation"));
    }

    #[test]
    fn test_no_doc_comment() {
        let pf = parse_rust("fn plain() {}");
        assert!(pf.definitions[0].doc.is_none());
    }

    #[test]
    fn test_def_on_line_one() {
        let pf = parse_rust("fn top() {}");
        assert!(pf.definitions[0].doc.is_none());
    }

    #[test]
    fn test_struct_with_doc() {
        let src = "/// A point in 2D space\nstruct Point { x: i32, y: i32 }";
        let pf = parse_rust(src);
        assert_eq!(pf.definitions[0].name, "Point");
        assert_eq!(pf.definitions[0].doc.as_deref(), Some("/// A point in 2D space"));
    }

    #[test]
    fn test_python_comment_doc() {
        let path = Path::new("test.py");
        let src = "# Add two numbers together\ndef add(a, b):\n    return a + b";
        let pf = parse_source(path, LanguageId::Python, src).unwrap();
        assert_eq!(pf.definitions[0].name, "add");
        assert_eq!(pf.definitions[0].doc.as_deref(), Some("# Add two numbers together"));
    }

    #[test]
    fn test_python_no_doc() {
        let path = Path::new("test.py");
        let pf = parse_source(path, LanguageId::Python, "def bare():\n    pass").unwrap();
        assert!(pf.definitions[0].doc.is_none());
    }

    #[test]
    fn test_jsdoc_block() {
        let path = Path::new("test.js");
        let src = "/** Calculate the total */\nfunction total() {}";
        let pf = parse_source(path, LanguageId::JavaScript, src).unwrap();
        assert_eq!(pf.definitions[0].doc.as_deref(), Some("/** Calculate the total */"));
    }

    #[test]
    fn test_doc_with_blank_line() {
        let pf = parse_rust("/// doc comment\n\nfn spaced() {}");
        assert!(pf.definitions[0].doc.is_some());
    }

    #[test]
    fn test_double_dash_comment_skipped() {
        let pf = parse_rust("x = 1;\nfn later() {}");
        assert!(pf.definitions[0].doc.is_none());
    }
}
