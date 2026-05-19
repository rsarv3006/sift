use crate::embed::Embedder;
use crate::parser::{DefKind, ParsedFile};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type SymbolId = usize;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: DefKind,
    pub file: PathBuf,
    pub line: usize,
    pub end_line: usize,
    pub doc: Option<String>,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller_name: String,
    pub caller_file: PathBuf,
    pub caller_line: usize,
    pub callee_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEdge {
    pub file: PathBuf,
    pub symbol_name: String,
    pub resolved_to: Option<SymbolId>,
    pub resolved_file: Option<PathBuf>,
    pub resolved_line: Option<usize>,
    pub resolved_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIndex {
    pub symbols: Vec<Symbol>,
    pub calls: Vec<CallEdge>,
    pub imports: Vec<ImportEdge>,
    pub files: Vec<PathBuf>,
    pub root: PathBuf,

    // name -> symbol IDs (for fast lookup)
    by_name: HashMap<String, Vec<SymbolId>>,
    // file -> symbol IDs
    by_file: HashMap<PathBuf, Vec<SymbolId>>,
}

impl CodeIndex {
    pub fn build(
        parsed: Vec<ParsedFile>,
        root: &Path,
        embedder: Option<&dyn Embedder>,
    ) -> Self {
        let root = root.to_path_buf();
        let mut idx = CodeIndex {
            symbols: Vec::new(),
            calls: Vec::new(),
            imports: Vec::new(),
            files: Vec::new(),
            by_name: HashMap::new(),
            by_file: HashMap::new(),
            root,
        };

        for pf in &parsed {
            idx.add_file(pf);
        }

        if let Some(embedder) = embedder {
            idx.compute_embeddings(embedder);
        }

        idx.resolve_caller_names();
        idx.resolve_imports();
        idx
    }

    fn compute_embeddings(&mut self, embedder: &dyn Embedder) {
        let texts: Vec<String> = self
            .symbols
            .iter()
            .map(|s| {
                let mut t = format!("{}: {:?}", s.name, s.kind);
                if let Some(ref doc) = s.doc {
                    t.push('\n');
                    t.push_str(doc);
                }
                t
            })
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        if text_refs.is_empty() {
            return;
        }
        if let Ok(embeddings) = embedder.embed(&text_refs) {
            for (sym, emb) in self.symbols.iter_mut().zip(embeddings) {
                sym.embedding = Some(emb);
            }
        }
    }

    fn add_file(&mut self, pf: &ParsedFile) {
        if !self.files.contains(&pf.path) {
            self.files.push(pf.path.clone());
        }

        for def in &pf.definitions {
            let id = self.symbols.len();
            self.symbols.push(Symbol {
                id,
                name: def.name.clone(),
                kind: def.kind,
                file: pf.path.clone(),
                line: def.start_line,
                end_line: def.end_line,
                doc: def.doc.clone(),
                embedding: None,
            });
            self.by_name
                .entry(def.name.clone())
                .or_default()
                .push(id);
            self.by_file
                .entry(pf.path.clone())
                .or_default()
                .push(id);
        }

        for rf in &pf.references {
            self.calls.push(CallEdge {
                caller_name: String::new(),
                caller_file: pf.path.clone(),
                caller_line: rf.line,
                callee_name: rf.name.clone(),
            });
        }

        for imp in &pf.imports {
            self.imports.push(ImportEdge {
                file: pf.path.clone(),
                symbol_name: imp.name.clone(),
                resolved_to: None,
                resolved_file: None,
                resolved_line: None,
                resolved_kind: None,
            });
        }
    }

    fn resolve_caller_names(&mut self) {
        for call in &mut self.calls {
            let Some(sym_ids) = self.by_file.get(&call.caller_file) else {
                continue;
            };
            for &sym_id in sym_ids {
                let Some(sym) = self.symbols.get(sym_id) else {
                    continue;
                };
                if sym.line <= call.caller_line && call.caller_line <= sym.end_line {
                    call.caller_name = sym.name.clone();
                    break;
                }
            }
        }
    }

    fn resolve_imports(&mut self) {
        for imp in &mut self.imports {
            let Some(sym_ids) = self.by_name.get(&imp.symbol_name) else {
                continue;
            };
            // Prefer a definition from a different file than the import
            let resolved = sym_ids
                .iter()
                .filter_map(|id| self.symbols.get(*id))
                .find(|s| s.file != imp.file)
                .or_else(|| {
                    sym_ids
                        .iter()
                        .filter_map(|id| self.symbols.get(*id))
                        .next()
                });
            if let Some(sym) = resolved {
                imp.resolved_to = Some(sym.id);
                imp.resolved_file = Some(sym.file.clone());
                imp.resolved_line = Some(sym.line);
                imp.resolved_kind = Some(format!("{:?}", sym.kind).to_lowercase());
            }
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = bincode::serialize(self)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)?;
        let idx: CodeIndex = bincode::deserialize(&bytes)?;
        Ok(idx)
    }

    pub fn find_symbols_by_name(&self, name: &str) -> Vec<&Symbol> {
        self.by_name
            .get(name)
            .map(|ids| ids.iter().filter_map(|id| self.symbols.get(*id)).collect())
            .unwrap_or_default()
    }

    pub fn find_symbols_by_pattern(&self, pattern: &str) -> Vec<&Symbol> {
        let lower = pattern.to_lowercase();
        self.symbols
            .iter()
            .filter(|s| s.name.to_lowercase().contains(&lower))
            .collect()
    }

    pub fn find_calls_to(&self, name: &str) -> Vec<&CallEdge> {
        self.calls
            .iter()
            .filter(|c| c.callee_name == name)
            .collect()
    }

    pub fn find_calls_by(&self, name: &str) -> Vec<&CallEdge> {
        self.calls
            .iter()
            .filter(|c| c.caller_name == name)
            .collect()
    }

    pub fn find_implementations(&self, name: &str) -> Vec<&Symbol> {
        self.symbols
            .iter()
            .filter(|s| s.kind == DefKind::Impl && s.name == name)
            .collect()
    }

    pub fn find_symbols_in_file(&self, file: &Path) -> Vec<&Symbol> {
        self.by_file
            .get(file)
            .map(|ids| ids.iter().filter_map(|id| self.symbols.get(*id)).collect())
            .unwrap_or_default()
    }

    pub fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    }

    pub fn find_imports_in_file(&self, file: &Path) -> Vec<&ImportEdge> {
        self.imports
            .iter()
            .filter(|i| i.file == file)
            .collect()
    }

    pub fn find_importers_of(&self, name: &str) -> Vec<&ImportEdge> {
        self.imports
            .iter()
            .filter(|i| {
                i.resolved_to
                    .and_then(|id| self.symbols.get(id))
                    .is_some_and(|s| s.name == name)
            })
            .collect()
    }

    pub fn semantic_search(
        &self,
        query_embed: &[f32],
        k: usize,
    ) -> Vec<(f64, &Symbol)> {
        let mut scores: Vec<(f64, &Symbol)> = self
            .symbols
            .iter()
            .filter_map(|s| s.embedding.as_ref().map(|e| (cosine_similarity(query_embed, e), s)))
            .collect();
        scores.sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scores.truncate(k);
        scores
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let na: f64 = a.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{DefKind, ParsedDef, ParsedFile, ParsedImport, ParsedRef, RefKind};

    fn make_file(
        path: &str,
        defs: Vec<(&str, DefKind, usize, usize)>,
        refs: Vec<(&str, usize)>,
        imports: Vec<&str>,
    ) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(path),
            language: crate::parser::LanguageId::Rust,
            definitions: defs
                .into_iter()
                .map(|(name, kind, start_line, end_line)| ParsedDef {
                    name: name.to_string(),
                    kind,
                    start_line,
                    end_line,
                    doc: None,
                })
                .collect(),
            references: refs
                .into_iter()
                .map(|(name, line)| ParsedRef {
                    name: name.to_string(),
                    kind: RefKind::Call,
                    line,
                })
                .collect(),
            imports: imports
                .into_iter()
                .map(|name| ParsedImport {
                    name: name.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn test_build_empty_index() {
        let index = CodeIndex::build(vec![], Path::new("/root"), None);
        assert_eq!(index.symbols.len(), 0);
        assert_eq!(index.calls.len(), 0);
        assert_eq!(index.imports.len(), 0);
        assert_eq!(index.files.len(), 0);
    }

    #[test]
    fn test_build_index_with_symbols() {
        let files = vec![make_file(
            "src/main.rs",
            vec![("main", DefKind::Function, 1, 5)],
            vec![],
            vec![],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        assert_eq!(index.symbols.len(), 1);
        assert_eq!(index.symbols[0].name, "main");
        assert_eq!(index.symbols[0].kind, DefKind::Function);
        assert_eq!(index.symbols[0].line, 1);
        assert_eq!(index.symbols[0].end_line, 5);
    }

    #[test]
    fn test_find_symbols_by_name() {
        let files = vec![make_file(
            "src/lib.rs",
            vec![("foo", DefKind::Function, 1, 3), ("bar", DefKind::Function, 5, 7)],
            vec![],
            vec![],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        let found = index.find_symbols_by_name("foo");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "foo");
    }

    #[test]
    fn test_find_symbols_by_pattern() {
        let files = vec![make_file(
            "src/lib.rs",
            vec![
                ("calculate_revenue", DefKind::Function, 1, 3),
                ("calculate_expenses", DefKind::Function, 5, 7),
                ("print_report", DefKind::Function, 9, 11),
            ],
            vec![],
            vec![],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        let found = index.find_symbols_by_pattern("calculate");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn test_calls_are_recorded() {
        let files = vec![make_file(
            "src/main.rs",
            vec![("run", DefKind::Function, 1, 10)],
            vec![("helper", 3),("other", 5)],
            vec![],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        assert_eq!(index.calls.len(), 2);
    }

    #[test]
    fn test_imports_are_recorded() {
        let files = vec![make_file(
            "src/main.rs",
            vec![],
            vec![],
            vec!["HashMap", "Vec"],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        assert_eq!(index.imports.len(), 2);
        assert_eq!(index.imports[0].symbol_name, "HashMap");
        // No resolution since no symbols with those names exist
        assert!(index.imports[0].resolved_to.is_none());
    }

    #[test]
    fn test_import_resolution() {
        let files = vec![
            make_file(
                "src/lib.rs",
                vec![("HashMap", DefKind::Struct, 10, 30)],
                vec![],
                vec![],
            ),
            make_file(
                "src/main.rs",
                vec![("main", DefKind::Function, 1, 5)],
                vec![],
                vec!["HashMap"],
            ),
        ];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        let imports = index.find_imports_in_file(Path::new("src/main.rs"));
        assert_eq!(imports.len(), 1);
        let imp = imports[0];
        assert!(imp.resolved_to.is_some());
        assert_eq!(imp.resolved_file.as_deref(), Some(Path::new("src/lib.rs")));
        assert_eq!(imp.resolved_line, Some(10));
        assert_eq!(imp.resolved_kind.as_deref(), Some("struct"));
    }

    #[test]
    fn test_save_and_load_roundtrip() -> anyhow::Result<()> {
        let files = vec![make_file(
            "src/main.rs",
            vec![("main", DefKind::Function, 1, 10)],
            vec![("helper", 5)],
            vec!["std::fs"],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);

        let tmp = std::env::temp_dir().join("sift_test_index.bin");
        index.save(&tmp)?;
        let loaded = CodeIndex::load(&tmp)?;
        std::fs::remove_file(&tmp)?;

        assert_eq!(loaded.symbols.len(), 1);
        assert_eq!(loaded.symbols[0].name, "main");
        assert_eq!(loaded.calls.len(), 1);
        assert_eq!(loaded.imports.len(), 1);
        // Resolution persists (no match for "std::fs" so resolved_to is None)
        assert_eq!(loaded.imports[0].resolved_to, None);
        Ok(())
    }

    #[test]
    fn test_multiple_files_index() {
        let files = vec![
            make_file(
                "src/main.rs",
                vec![("main", DefKind::Function, 1, 10)],
                vec![("helper", 3)],
                vec![],
            ),
            make_file(
                "src/helper.rs",
                vec![("helper", DefKind::Function, 1, 5)],
                vec![],
                vec![],
            ),
        ];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        assert_eq!(index.symbols.len(), 2);
        assert_eq!(index.files.len(), 2);
    }

    #[test]
    fn test_find_implementations() {
        let files = vec![make_file(
            "src/main.rs",
            vec![
                ("Iterator", DefKind::Trait, 1, 3),
                ("Iterator", DefKind::Impl, 5, 20),
            ],
            vec![],
            vec![],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        let impls = index.find_implementations("Iterator");
        assert_eq!(impls.len(), 1);
        assert_eq!(impls[0].kind, DefKind::Impl);
    }

    #[test]
    fn test_relative_path() {
        let files = vec![make_file(
            "/root/src/main.rs",
            vec![],
            vec![],
            vec![],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);
        assert_eq!(index.relative_path(Path::new("/root/src/main.rs")), "src/main.rs");
    }
}
