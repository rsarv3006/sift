use crate::embed::Embedder;
use crate::index::CodeIndex;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct QueryResult {
    #[serde(rename = "type")]
    pub result_type: &'static str,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CallResult {
    #[serde(rename = "type")]
    pub result_type: &'static str,
    pub caller: String,
    pub callee: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct FileResult {
    #[serde(rename = "type")]
    pub result_type: &'static str,
    pub file: String,
    pub symbols: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    #[serde(rename = "type")]
    pub result_type: &'static str,
    pub file: String,
    pub symbol: String,
    pub resolved: bool,
    pub resolved_file: Option<String>,
    pub resolved_line: Option<usize>,
    pub resolved_kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImporterResult {
    #[serde(rename = "type")]
    pub result_type: &'static str,
    pub symbol: String,
    pub importer_file: String,
    pub import_name: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OutputRow {
    Query(QueryResult),
    Call(CallResult),
    File(FileResult),
    Simple(SimpleResult),
    Import(ImportResult),
    Importer(ImporterResult),
}

#[derive(Debug, Serialize)]
pub struct SimpleResult {
    #[serde(rename = "type")]
    pub result_type: String,
    pub value: String,
}

pub struct QueryEngine<'a> {
    index: &'a CodeIndex,
    embedder: Option<Box<dyn Embedder + 'a>>,
}

impl<'a> QueryEngine<'a> {
    pub fn new(index: &'a CodeIndex) -> Self {
        Self { index, embedder: None }
    }

    pub fn with_embedder(index: &'a CodeIndex, embedder: Box<dyn Embedder + 'a>) -> Self {
        Self { index, embedder: Some(embedder) }
    }

    pub fn execute(&self, query: &str) -> Vec<OutputRow> {
        let query = query.trim();
        let (cmd, arg) = query
            .split_once(' ')
            .map(|(c, a)| (c, a.trim()))
            .unwrap_or((query, ""));
        match (cmd, arg) {
            ("define", a) => self.cmd_define(a),
            ("calls", a) => self.cmd_calls(a),
            ("callees", a) => self.cmd_callees(a),
            ("implements", a) => self.cmd_implements(a),
            ("imports", a) => self.cmd_imports(a),
            ("importers", a) => self.cmd_importers(a),
            ("file", a) => self.cmd_file(a),
            ("symbols", a) if a.starts_with("matching ") => {
                self.cmd_symbols_matching(a.strip_prefix("matching ").unwrap_or("").trim())
            }
            ("semantic", a) => self.cmd_semantic(a),
            ("files", "") => self.cmd_files(),
            _ => self.cmd_define(query),
        }
    }

    fn rel(&self, path: &Path) -> String {
        self.index.relative_path(path)
    }

    fn cmd_define(&self, name: &str) -> Vec<OutputRow> {
        self.index
            .find_symbols_by_name(name)
            .into_iter()
            .map(|s| {
                OutputRow::Query(QueryResult {
                    result_type: "definition",
                    name: s.name.clone(),
                    kind: format!("{:?}", s.kind).to_lowercase(),
                    file: self.rel(&s.file),
                    line: s.line,
                    end_line: s.end_line,
                    score: None,
                    doc: s.doc.clone(),
                })
            })
            .collect()
    }

    fn cmd_calls(&self, name: &str) -> Vec<OutputRow> {
        self.index
            .find_calls_to(name)
            .into_iter()
            .map(|c| {
                OutputRow::Call(CallResult {
                    result_type: "call",
                    caller: c.caller_name.clone(),
                    callee: c.callee_name.clone(),
                    file: self.rel(&c.caller_file),
                    line: c.caller_line,
                })
            })
            .collect()
    }

    fn cmd_callees(&self, name: &str) -> Vec<OutputRow> {
        self.index
            .find_calls_by(name)
            .into_iter()
            .map(|c| {
                OutputRow::Call(CallResult {
                    result_type: "callee",
                    caller: c.caller_name.clone(),
                    callee: c.callee_name.clone(),
                    file: self.rel(&c.caller_file),
                    line: c.caller_line,
                })
            })
            .collect()
    }

    fn cmd_implements(&self, name: &str) -> Vec<OutputRow> {
        self.index
            .find_implementations(name)
            .into_iter()
            .map(|s| {
                OutputRow::Query(QueryResult {
                    result_type: "implementation",
                    name: s.name.clone(),
                    kind: format!("{:?}", s.kind).to_lowercase(),
                    file: self.rel(&s.file),
                    line: s.line,
                    end_line: s.end_line,
                    score: None,
                    doc: s.doc.clone(),
                })
            })
            .collect()
    }

    fn cmd_imports(&self, path: &str) -> Vec<OutputRow> {
        let query = Path::new(path);
        let matched: Vec<_> = self
            .index
            .files
            .iter()
            .filter(|f| self.rel(f) == path || f.ends_with(query))
            .collect();
        let mut rows = Vec::new();
        for f in matched {
            for imp in self.index.find_imports_in_file(f) {
                rows.push(OutputRow::Import(ImportResult {
                    result_type: "import",
                    file: self.rel(f),
                    symbol: imp.symbol_name.clone(),
                    resolved: imp.resolved_to.is_some(),
                    resolved_file: imp.resolved_file.as_ref().map(|p| self.rel(p)),
                    resolved_line: imp.resolved_line,
                    resolved_kind: imp.resolved_kind.clone(),
                }));
            }
        }
        rows
    }

    fn cmd_importers(&self, name: &str) -> Vec<OutputRow> {
        self.index
            .find_importers_of(name)
            .into_iter()
            .map(|imp| OutputRow::Importer(ImporterResult {
                result_type: "importer",
                symbol: name.to_string(),
                importer_file: self.rel(&imp.file),
                import_name: imp.symbol_name.clone(),
            }))
            .collect()
    }

    fn cmd_file(&self, path: &str) -> Vec<OutputRow> {
        let query = Path::new(path);
        let matched: Vec<_> = self
            .index
            .files
            .iter()
            .filter(|f| {
                self.rel(f) == path || f.ends_with(query)
            })
            .collect();

        if matched.is_empty() {
            return vec![];
        }

        let mut rows = Vec::new();
        for f in matched {
            let syms = self.index.find_symbols_in_file(f);
            rows.push(OutputRow::File(FileResult {
                result_type: "file",
                file: self.rel(f),
                symbols: syms.into_iter().map(|s| s.name.clone()).collect(),
            }));
        }
        rows
    }

    fn cmd_symbols_matching(&self, pattern: &str) -> Vec<OutputRow> {
        self.index
            .find_symbols_by_pattern(pattern)
            .into_iter()
            .map(|s| {
                OutputRow::Query(QueryResult {
                    result_type: "definition",
                    name: s.name.clone(),
                    kind: format!("{:?}", s.kind).to_lowercase(),
                    file: self.rel(&s.file),
                    line: s.line,
                    end_line: s.end_line,
                    score: None,
                    doc: s.doc.clone(),
                })
            })
            .collect()
    }

    fn cmd_semantic(&self, query_text: &str) -> Vec<OutputRow> {
        let Some(embedder) = &self.embedder else {
            return vec![];
        };
        let has_embeddings = self.index.symbols.iter().any(|s| s.embedding.is_some());
        if !has_embeddings {
            return vec![];
        }
        let Ok(embeddings) = embedder.embed(&[query_text]) else {
            return vec![];
        };
        let Some(query_embed) = embeddings.into_iter().next() else {
            return vec![];
        };
        self.index
            .semantic_search(&query_embed, 10)
            .into_iter()
            .map(|(score, s)| {
                OutputRow::Query(QueryResult {
                    result_type: "semantic",
                    name: s.name.clone(),
                    kind: format!("{:?}", s.kind).to_lowercase(),
                    file: self.rel(&s.file),
                    line: s.line,
                    end_line: s.end_line,
                    score: Some(score),
                    doc: s.doc.clone(),
                })
            })
            .collect()
    }

    fn cmd_files(&self) -> Vec<OutputRow> {
        self.index
            .files
            .iter()
            .map(|f| {
                OutputRow::Simple(SimpleResult {
                    result_type: "file".to_string(),
                    value: self.rel(f),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::CodeIndex;
    use crate::parser::{DefKind, ParsedDef, ParsedFile, ParsedImport, ParsedRef, RefKind};
    use std::path::PathBuf;

    fn make_index() -> CodeIndex {
        let files = vec![
            ParsedFile {
                path: PathBuf::from("/root/src/main.rs"),
                language: crate::parser::LanguageId::Rust,
                definitions: vec![
                    ParsedDef { name: "main".into(), kind: DefKind::Function, start_line: 1, end_line: 10, doc: None },
                    ParsedDef { name: "run".into(), kind: DefKind::Function, start_line: 12, end_line: 20, doc: None },
                ],
                references: vec![
                    ParsedRef { name: "run".into(), kind: RefKind::Call, line: 5 },
                    ParsedRef { name: "helper".into(), kind: RefKind::Call, line: 6 },
                ],
                imports: vec![
                    ParsedImport { name: "HashMap".into() },
                ],
            },
            ParsedFile {
                path: PathBuf::from("/root/src/helper.rs"),
                language: crate::parser::LanguageId::Rust,
                definitions: vec![
                    ParsedDef { name: "helper".into(), kind: DefKind::Function, start_line: 1, end_line: 3, doc: None },
                ],
                references: vec![],
                imports: vec![],
            },
            ParsedFile {
                path: PathBuf::from("/root/src/collections.rs"),
                language: crate::parser::LanguageId::Rust,
                definitions: vec![
                    ParsedDef { name: "HashMap".into(), kind: DefKind::Struct, start_line: 10, end_line: 50, doc: None },
                ],
                references: vec![],
                imports: vec![],
            },
        ];
        CodeIndex::build(files, Path::new("/root"), None)
    }

    #[test]
    fn test_define_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("define main");
        assert_eq!(results.len(), 1);
        if let OutputRow::Query(r) = &results[0] {
            assert_eq!(r.name, "main");
            assert_eq!(r.file, "src/main.rs");
        } else {
            panic!("expected Query result");
        }
    }

    #[test]
    fn test_define_missing() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("define nonexistent");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_calls_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("calls helper");
        assert_eq!(results.len(), 1);
        if let OutputRow::Call(r) = &results[0] {
            assert_eq!(r.callee, "helper");
            assert_eq!(r.caller, "main");
        } else {
            panic!("expected Call result");
        }
    }

    #[test]
    fn test_callees_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("callees main");
        assert_eq!(results.len(), 2);
        let callees: Vec<&str> = results.iter().map(|r| {
            if let OutputRow::Call(c) = r { c.callee.as_str() } else { "" }
        }).collect();
        assert!(callees.contains(&"run"));
        assert!(callees.contains(&"helper"));
    }

    #[test]
    fn test_files_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("files");
        assert_eq!(results.len(), 3);
        let files: Vec<&str> = results.iter().map(|r| {
            if let OutputRow::Simple(s) = r { s.value.as_str() } else { "" }
        }).collect();
        assert!(files.contains(&"src/main.rs"));
        assert!(files.contains(&"src/helper.rs"));
        assert!(files.contains(&"src/collections.rs"));
    }

    #[test]
    fn test_file_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("file src/main.rs");
        assert_eq!(results.len(), 1);
        if let OutputRow::File(r) = &results[0] {
            assert_eq!(r.file, "src/main.rs");
            assert!(r.symbols.contains(&"main".to_string()));
        } else {
            panic!("expected File result");
        }
    }

    #[test]
    fn test_file_query_partial_path() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("file main.rs");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_symbols_matching() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("symbols matching run");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_bare_name_fallback() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("main");
        assert_eq!(results.len(), 1);
        if let OutputRow::Query(r) = &results[0] {
            assert_eq!(r.name, "main");
        } else {
            panic!("expected Query result");
        }
    }

    #[test]
    fn test_implements_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("implements nonexistent");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_empty_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_imports_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("imports src/main.rs");
        assert_eq!(results.len(), 1);
        if let OutputRow::Import(r) = &results[0] {
            assert_eq!(r.symbol, "HashMap");
            assert!(r.resolved);
            assert_eq!(r.resolved_file.as_deref(), Some("src/collections.rs"));
            assert_eq!(r.resolved_kind.as_deref(), Some("struct"));
        } else {
            panic!("expected Import result");
        }
    }

    #[test]
    fn test_importers_query() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("importers HashMap");
        assert_eq!(results.len(), 1);
        if let OutputRow::Importer(r) = &results[0] {
            assert_eq!(r.symbol, "HashMap");
            assert_eq!(r.importer_file, "src/main.rs");
        } else {
            panic!("expected Importer result");
        }
    }

    #[test]
    fn test_imports_query_unresolved() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        // helper.rs has no imports
        let results = engine.execute("imports src/helper.rs");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_importers_query_no_results() {
        let index = make_index();
        let engine = QueryEngine::new(&index);
        let results = engine.execute("importers nonexistent");
        assert_eq!(results.len(), 0);
    }
}
