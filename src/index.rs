use crate::embed::Embedder;
use crate::parser::{DefKind, LanguageId, ParsedDef, ParsedFile, ParsedImport, ParsedRef, RefKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Filter notify events to only those that should trigger a re-index.
/// Skips `.sift/` paths, non-source files, and Access/Other event kinds.
pub fn is_relevant_source_event(event: &notify::Event) -> bool {
    use notify::EventKind::*;
    if matches!(event.kind, Access(_) | Other) {
        return false;
    }
    for path in &event.paths {
        if path.components().any(|c| c.as_os_str() == ".sift") {
            continue;
        }
        if LanguageId::from_path(path).is_some() {
            return true;
        }
    }
    false
}

/// Magic bytes prefix for V2+ index files, to distinguish from V1 (no prefix).
const V2_MAGIC: &[u8; 4] = b"siV2";

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

    /// File -> mtime in millis since UNIX_EPOCH. Used for incremental re-index.
    pub file_mtimes: HashMap<PathBuf, u64>,

    /// name -> symbol IDs (for fast lookup). Rebuilt on load; not serialized.
    #[serde(skip)]
    by_name: HashMap<String, Vec<SymbolId>>,
    /// file -> symbol IDs. Rebuilt on load; not serialized.
    #[serde(skip)]
    by_file: HashMap<PathBuf, Vec<SymbolId>>,
}

// Old format without file_mtimes, for backward-compatible deserialization
#[derive(Serialize, Deserialize)]
struct CodeIndexV1 {
    pub symbols: Vec<Symbol>,
    pub calls: Vec<CallEdge>,
    pub imports: Vec<ImportEdge>,
    pub files: Vec<PathBuf>,
    pub root: PathBuf,
    by_name: HashMap<String, Vec<SymbolId>>,
    by_file: HashMap<PathBuf, Vec<SymbolId>>,
}

impl From<CodeIndexV1> for CodeIndex {
    fn from(old: CodeIndexV1) -> Self {
        CodeIndex {
            symbols: old.symbols,
            calls: old.calls,
            imports: old.imports,
            files: old.files,
            root: old.root,
            file_mtimes: HashMap::new(),
            by_name: old.by_name,
            by_file: old.by_file,
        }
    }
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
            file_mtimes: HashMap::new(),
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
        match embedder.embed(&text_refs) {
            Ok(embeddings) => {
                for (sym, emb) in self.symbols.iter_mut().zip(embeddings) {
                    sym.embedding = Some(emb);
                }
            }
            Err(e) => {
                eprintln!("warn: embedding computation failed: {:#}", e);
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

    /// Collect current modification times for all source files under root.
    /// Returns a map from absolute file path → millis since UNIX_EPOCH.
    pub fn collect_mtimes(root: &Path) -> HashMap<PathBuf, u64> {
        let mut mtimes = HashMap::new();
        let walk = ignore::WalkBuilder::new(root)
            .standard_filters(true)
            .build();
        for entry in walk {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            if path.components().any(|c| c.as_os_str() == "target") {
                continue;
            }
            if LanguageId::from_path(path).is_none() {
                continue;
            }
            if let Ok(meta) = path.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if let Ok(dur) = mtime.duration_since(UNIX_EPOCH) {
                        mtimes.insert(path.to_path_buf(), dur.as_millis() as u64);
                    }
                }
            }
        }
        mtimes
    }

    /// Given current mtimes, return (unchanged_files, changed_or_new_files)
    /// by comparing against stored `file_mtimes`.
    pub fn classify_files(&self, current: &HashMap<PathBuf, u64>) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut unchanged = Vec::new();
        let mut changed = Vec::new();

        // Check all currently-known files
        let mut seen = std::collections::HashSet::new();
        for (path, mtime) in current {
            seen.insert(path.clone());
            match self.file_mtimes.get(path) {
                Some(stored) if *stored == *mtime => unchanged.push(path.clone()),
                _ => changed.push(path.clone()),
            }
        }
        // Files that were in stored mtimes but no longer exist → changed (will be cleaned up)
        for path in self.file_mtimes.keys() {
            if !seen.contains(path) {
                changed.push(path.clone());
            }
        }

        (unchanged, changed)
    }

    /// Reconstruct a `ParsedFile` from stored index data for a given file.
    /// Used to avoid re-parsing unchanged files during incremental re-index.
    pub fn reconstruct_parsed_file(&self, path: &Path) -> ParsedFile {
        let language = LanguageId::from_path(path).unwrap_or(LanguageId::Rust);
        let definitions: Vec<ParsedDef> = self
            .by_file
            .get(path)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.symbols.get(*id))
                    .map(|s| ParsedDef {
                        name: s.name.clone(),
                        kind: s.kind,
                        start_line: s.line,
                        end_line: s.end_line,
                        doc: s.doc.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let references: Vec<ParsedRef> = self
            .calls
            .iter()
            .filter(|c| c.caller_file == path)
            .map(|c| ParsedRef {
                name: c.callee_name.clone(),
                kind: RefKind::Call,
                line: c.caller_line,
            })
            .collect();
        let imports: Vec<ParsedImport> = self
            .imports
            .iter()
            .filter(|i| i.file == path)
            .map(|i| ParsedImport {
                name: i.symbol_name.clone(),
            })
            .collect();
        ParsedFile {
            path: path.to_path_buf(),
            language,
            definitions,
            references,
            imports,
        }
    }

    /// Insert `id` into a name→ids map, cloning the key only when first seen.
    fn map_push<K: std::hash::Hash + Eq + Clone>(
        map: &mut HashMap<K, Vec<SymbolId>>,
        key: &K,
        id: SymbolId,
    ) {
        match map.get_mut(key) {
            Some(v) => v.push(id),
            None => {
                map.insert(key.clone(), vec![id]);
            }
        }
    }

    /// Incrementally update the index: remove old data for `changed` files,
    /// add `new_parsed` data, rebuild lookup maps, and re-resolve cross-file refs.
    /// Embeddings are NOT handled here — the caller should call
    /// `preserve_embeddings()` or `compute_missing_embeddings()` afterward.
    pub fn apply_changes(&mut self, changed: &[PathBuf], new_parsed: &[ParsedFile]) {
        let changed_set: std::collections::HashSet<&Path> =
            changed.iter().map(|p| p.as_path()).collect();

        let changed_names = self.rebuild_symbols_and_maps(&changed_set, new_parsed);
        self.rebuild_calls_from(&changed_set, new_parsed);
        self.rebuild_imports_from(&changed_set, new_parsed);
        self.rebuild_files_from(&changed_set, new_parsed);

        self.resolve_caller_names_incremental(&changed_set);
        self.resolve_imports_incremental(&changed_set, &changed_names);
    }

    /// Rebuild `symbols`, `by_name`, and `by_file` in a single pass.
    /// Returns the set of symbol names that were in changed files (old or new).
    fn rebuild_symbols_and_maps(
        &mut self,
        changed_set: &std::collections::HashSet<&Path>,
        new_parsed: &[ParsedFile],
    ) -> std::collections::HashSet<String> {
        let old_symbols = std::mem::take(&mut self.symbols);
        let n = old_symbols.len();

        // Phase 1: Build offset array (number of removals before each position)
        // and removed bitmap. Simultaneously compact survivors into new_symbols.
        let mut offset = vec![0usize; n];
        let mut removed = vec![false; n];
        let mut removed_count = 0;
        let mut new_symbols = Vec::with_capacity(n);
        let mut changed_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (i, mut sym) in old_symbols.into_iter().enumerate() {
            offset[i] = removed_count;
            if changed_set.contains(sym.file.as_path()) {
                changed_names.insert(sym.name);
                removed[i] = true;
                removed_count += 1;
            } else {
                let id = new_symbols.len();
                sym.id = id;
                new_symbols.push(sym);
            }
        }

        // Phase 2: Transform old lookup maps using the offset array.
        // This avoids HashMap insertions for every survivor symbol.
        let old_by_name = std::mem::take(&mut self.by_name);
        let mut new_by_name: HashMap<String, Vec<SymbolId>> = old_by_name
            .into_iter()
            .map(|(name, ids)| {
                let ids: Vec<SymbolId> = ids
                    .into_iter()
                    .filter_map(|old_id| {
                        if removed[old_id] {
                            None
                        } else {
                            Some(old_id - offset[old_id])
                        }
                    })
                    .collect();
                (name, ids)
            })
            .filter(|(_, ids)| !ids.is_empty())
            .collect();

        let old_by_file = std::mem::take(&mut self.by_file);
        let mut new_by_file: HashMap<PathBuf, Vec<SymbolId>> = old_by_file
            .into_iter()
            .map(|(file, ids)| {
                let ids: Vec<SymbolId> = ids
                    .into_iter()
                    .filter_map(|old_id| {
                        if removed[old_id] {
                            None
                        } else {
                            Some(old_id - offset[old_id])
                        }
                    })
                    .collect();
                (file, ids)
            })
            .filter(|(_, ids)| !ids.is_empty())
            .collect();

        // Phase 3: Add new parsed symbols to the maps
        for pf in new_parsed {
            for def in &pf.definitions {
                changed_names.insert(def.name.clone());
                let id = new_symbols.len();
                new_symbols.push(Symbol {
                    id,
                    name: def.name.clone(),
                    kind: def.kind,
                    file: pf.path.clone(),
                    line: def.start_line,
                    end_line: def.end_line,
                    doc: def.doc.clone(),
                    embedding: None,
                });
                Self::map_push(&mut new_by_name, &def.name, id);
                Self::map_push(&mut new_by_file, &pf.path, id);
            }
        }

        self.symbols = new_symbols;
        self.by_name = new_by_name;
        self.by_file = new_by_file;
        changed_names
    }

    /// Rebuild `calls`, keeping only survivors + new parsed references.
    fn rebuild_calls_from(
        &mut self,
        changed_set: &std::collections::HashSet<&Path>,
        new_parsed: &[ParsedFile],
    ) {
        let old_calls = std::mem::take(&mut self.calls);
        let mut new_calls = Vec::with_capacity(old_calls.len());
        for call in old_calls {
            if !changed_set.contains(call.caller_file.as_path()) {
                new_calls.push(call);
            }
        }
        for pf in new_parsed {
            for rf in &pf.references {
                new_calls.push(CallEdge {
                    caller_name: String::new(),
                    caller_file: pf.path.clone(),
                    caller_line: rf.line,
                    callee_name: rf.name.clone(),
                });
            }
        }
        self.calls = new_calls;
    }

    /// Rebuild `imports`, keeping only survivors + new parsed imports.
    fn rebuild_imports_from(
        &mut self,
        changed_set: &std::collections::HashSet<&Path>,
        new_parsed: &[ParsedFile],
    ) {
        let old_imports = std::mem::take(&mut self.imports);
        let mut new_imports = Vec::with_capacity(old_imports.len());
        for imp in old_imports {
            if !changed_set.contains(imp.file.as_path()) {
                new_imports.push(imp);
            }
        }
        for pf in new_parsed {
            for imp in &pf.imports {
                new_imports.push(ImportEdge {
                    file: pf.path.clone(),
                    symbol_name: imp.name.clone(),
                    resolved_to: None,
                    resolved_file: None,
                    resolved_line: None,
                    resolved_kind: None,
                });
            }
        }
        self.imports = new_imports;
    }

    /// Rebuild `files`, keeping only survivors + new parsed files.
    fn rebuild_files_from(
        &mut self,
        changed_set: &std::collections::HashSet<&Path>,
        new_parsed: &[ParsedFile],
    ) {
        let old_files = std::mem::take(&mut self.files);
        let mut new_files = Vec::with_capacity(old_files.len());
        for f in old_files {
            if !changed_set.contains(f.as_path()) {
                new_files.push(f);
            }
        }
        for pf in new_parsed {
            if !new_files.contains(&pf.path) {
                new_files.push(pf.path.clone());
            }
        }
        self.files = new_files;
    }

    /// Re-resolve caller names only for calls from changed files.
    fn resolve_caller_names_incremental(&mut self, changed: &std::collections::HashSet<&Path>) {
        for call in &mut self.calls {
            if !changed.contains(call.caller_file.as_path()) {
                continue;
            }
            call.caller_name = String::new();
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

    /// Re-resolve imports that are in changed files or reference names
    /// that were defined in changed files.
    fn resolve_imports_incremental(
        &mut self,
        changed: &std::collections::HashSet<&Path>,
        changed_names: &std::collections::HashSet<String>,
    ) {
        for imp in &mut self.imports {
            if !changed.contains(imp.file.as_path()) && !changed_names.contains(&imp.symbol_name)
            {
                continue;
            }
            imp.resolved_to = None;
            imp.resolved_file = None;
            imp.resolved_line = None;
            imp.resolved_kind = None;
            let Some(sym_ids) = self.by_name.get(&imp.symbol_name) else {
                continue;
            };
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

    /// Compute embeddings only for symbols that don't already have one.
    pub fn compute_missing_embeddings(&mut self, embedder: &dyn Embedder) {
        let new_ids: Vec<usize> = self
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.embedding.is_none())
            .map(|(i, _)| i)
            .collect();
        if new_ids.is_empty() {
            return;
        }
        let texts: Vec<String> = new_ids
            .iter()
            .map(|&id| {
                let s = &self.symbols[id];
                let mut t = format!("{}: {:?}", s.name, s.kind);
                if let Some(ref doc) = s.doc {
                    t.push('\n');
                    t.push_str(doc);
                }
                t
            })
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        if let Ok(embeddings) = embedder.embed(&text_refs) {
            for (&id, emb) in new_ids.iter().zip(embeddings) {
                self.symbols[id].embedding = Some(emb);
            }
        }
    }

    /// Copy embeddings from an older index for symbols that still exist
    /// in this index, matched by (name, file, line).
    pub fn preserve_embeddings(&mut self, old: &CodeIndex) {
        let old_lookup: std::collections::HashMap<(&str, &Path, usize), &[f32]> = old
            .symbols
            .iter()
            .filter_map(|s| s.embedding.as_deref().map(|emb| ((s.name.as_str(), &*s.file, s.line), emb)))
            .collect();
        for sym in &mut self.symbols {
            if sym.embedding.is_none() {
                if let Some(emb) = old_lookup.get(&(sym.name.as_str(), &*sym.file, sym.line)) {
                    sym.embedding = Some(emb.to_vec());
                }
            }
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buf = V2_MAGIC.to_vec();
        buf.extend(bincode::serialize(self)?);
        // Atomic write: write to temp file, then rename (atomic on POSIX).
        // This prevents readers from seeing a partially-written index.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &buf)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Rebuild `by_name` and `by_file` maps from `symbols`.
    /// Needed after deserialization when maps are skipped.
    fn rebuild_maps(&mut self) {
        for (i, sym) in self.symbols.iter().enumerate() {
            Self::map_push(&mut self.by_name, &sym.name, i);
            Self::map_push(&mut self.by_file, &sym.file, i);
        }
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)?;
        if bytes.len() >= 4 && &bytes[..4] == V2_MAGIC {
            let mut idx: CodeIndex = bincode::deserialize(&bytes[4..])?;
            idx.rebuild_maps();
            return Ok(idx);
        }
        // V1 format — no file_mtimes field
        let old: CodeIndexV1 = bincode::deserialize(&bytes)?;
        Ok(old.into())
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
        // Round-trip preserves file_mtimes
        assert!(loaded.file_mtimes.is_empty());
        Ok(())
    }

    #[test]
    fn test_save_and_load_with_mtimes() -> anyhow::Result<()> {
        let files = vec![make_file(
            "src/main.rs",
            vec![("main", DefKind::Function, 1, 10)],
            vec![],
            vec![],
        )];
        let mut index = CodeIndex::build(files, Path::new("/root"), None);
        index.file_mtimes =
            vec![(PathBuf::from("src/main.rs"), 42)].into_iter().collect();

        let tmp = std::env::temp_dir().join("sift_test_mtimes.bin");
        index.save(&tmp)?;
        // Verify V2 magic is written before loading
        let raw = std::fs::read(&tmp)?;
        assert_eq!(&raw[..4], V2_MAGIC);
        // No .tmp file left behind
        assert!(!tmp.with_extension("tmp").exists());
        // Round-trip load
        let loaded = CodeIndex::load(&tmp)?;
        std::fs::remove_file(&tmp)?;

        assert_eq!(loaded.file_mtimes.len(), 1);
        assert_eq!(loaded.file_mtimes.get(Path::new("src/main.rs")), Some(&42));
        Ok(())
    }

    #[test]
    fn test_load_old_format_without_mtimes() -> anyhow::Result<()> {
        // Serialize a CodeIndexV1 (no file_mtimes), then load as CodeIndex (with file_mtimes)
        let pf = make_file(
            "src/main.rs",
            vec![("main", DefKind::Function, 1, 10)],
            vec![("helper", 5)],
            vec!["std::fs"],
        );
        let old = CodeIndexV1 {
            symbols: pf.definitions.iter().map(|d| Symbol {
                id: 0,
                name: d.name.clone(),
                kind: d.kind,
                file: pf.path.clone(),
                line: d.start_line,
                end_line: d.end_line,
                doc: d.doc.clone(),
                embedding: None,
            }).collect(),
            calls: pf.references.iter().map(|r| CallEdge {
                caller_name: String::new(),
                caller_file: pf.path.clone(),
                caller_line: r.line,
                callee_name: r.name.clone(),
            }).collect(),
            imports: pf.imports.iter().map(|i| ImportEdge {
                file: pf.path.clone(),
                symbol_name: i.name.clone(),
                resolved_to: None,
                resolved_file: None,
                resolved_line: None,
                resolved_kind: None,
            }).collect(),
            files: vec![pf.path.clone()],
            root: PathBuf::from("/root"),
            by_name: HashMap::from([("main".into(), vec![0])]),
            by_file: HashMap::from([(pf.path, vec![0])]),
        };

        let tmp = std::env::temp_dir().join("sift_test_v1_index.bin");
        let bytes = bincode::serialize(&old)?;
        std::fs::write(&tmp, bytes)?;  // raw bincode, no V2 magic prefix

        let loaded = CodeIndex::load(&tmp)?;
        std::fs::remove_file(&tmp)?;

        assert_eq!(loaded.symbols.len(), 1);
        assert_eq!(loaded.symbols[0].name, "main");
        assert!(loaded.file_mtimes.is_empty(), "V1 load should produce empty file_mtimes");
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

    #[test]
    fn test_classify_files_unchanged() {
        let files = vec![make_file(
            "src/main.rs",
            vec![("foo", DefKind::Function, 1, 10)],
            vec![],
            vec![],
        )];
        let mut index = CodeIndex::build(files, Path::new("/root"), None);
        index.file_mtimes = vec![(PathBuf::from("src/main.rs"), 1000)].into_iter().collect();

        let current = vec![(PathBuf::from("src/main.rs"), 1000)].into_iter().collect();
        let (unchanged, changed) = index.classify_files(&current);
        assert_eq!(unchanged.len(), 1);
        assert_eq!(changed.len(), 0);
    }

    #[test]
    fn test_classify_files_changed_mtime() {
        let files = vec![make_file(
            "src/main.rs",
            vec![("foo", DefKind::Function, 1, 10)],
            vec![],
            vec![],
        )];
        let mut index = CodeIndex::build(files, Path::new("/root"), None);
        index.file_mtimes = vec![(PathBuf::from("src/main.rs"), 1000)].into_iter().collect();

        let current = vec![(PathBuf::from("src/main.rs"), 2000)].into_iter().collect();
        let (unchanged, changed) = index.classify_files(&current);
        assert_eq!(unchanged.len(), 0);
        assert_eq!(changed.len(), 1);
    }

    #[test]
    fn test_classify_files_new_and_deleted() {
        let mut index = CodeIndex::build(vec![], Path::new("/root"), None);
        index.file_mtimes = vec![(PathBuf::from("deleted.rs"), 1000)].into_iter().collect();

        let current = vec![(PathBuf::from("new.rs"), 2000)].into_iter().collect();
        let (unchanged, changed) = index.classify_files(&current);
        assert_eq!(unchanged.len(), 0);
        assert!(changed.contains(&PathBuf::from("new.rs")));
        assert!(changed.contains(&PathBuf::from("deleted.rs")));
    }

    #[test]
    fn test_reconstruct_parsed_file() {
        let files = vec![make_file(
            "src/main.rs",
            vec![
                ("foo", DefKind::Function, 1, 10),
                ("Bar", DefKind::Struct, 15, 25),
            ],
            vec![("helper", 5)],
            vec!["std::collections::HashMap"],
        )];
        let index = CodeIndex::build(files, Path::new("/root"), None);

        let pf = index.reconstruct_parsed_file(Path::new("src/main.rs"));
        assert_eq!(pf.path, PathBuf::from("src/main.rs"));
        assert_eq!(pf.definitions.len(), 2);
        assert_eq!(pf.definitions[0].name, "foo");
        assert_eq!(pf.definitions[0].kind, DefKind::Function);
        assert_eq!(pf.definitions[1].name, "Bar");
        assert_eq!(pf.definitions[1].kind, DefKind::Struct);
        assert_eq!(pf.references.len(), 1);
        assert_eq!(pf.references[0].name, "helper");
        assert_eq!(pf.imports.len(), 1);
        assert_eq!(pf.imports[0].name, "std::collections::HashMap");
    }

    #[test]
    fn test_reconstruct_parsed_file_empty() {
        let index = CodeIndex::build(vec![], Path::new("/root"), None);
        let pf = index.reconstruct_parsed_file(Path::new("nonexistent.rs"));
        assert_eq!(pf.definitions.len(), 0);
        assert_eq!(pf.references.len(), 0);
        assert_eq!(pf.imports.len(), 0);
    }

    #[test]
    fn test_preserve_embeddings() {
        let mut new_index = CodeIndex::build(vec![], Path::new("/root"), None);
        new_index.symbols.push(Symbol {
            id: 0,
            name: "foo".into(),
            kind: DefKind::Function,
            file: PathBuf::from("src/lib.rs"),
            line: 1,
            end_line: 10,
            doc: None,
            embedding: None,
        });

        let mut old_index = CodeIndex::build(vec![], Path::new("/root"), None);
        old_index.symbols.push(Symbol {
            id: 0,
            name: "foo".into(),
            kind: DefKind::Function,
            file: PathBuf::from("src/lib.rs"),
            line: 1,
            end_line: 10,
            doc: None,
            embedding: Some(vec![0.1, 0.2, 0.3]),
        });

        new_index.preserve_embeddings(&old_index);
        assert_eq!(
            new_index.symbols[0].embedding,
            Some(vec![0.1, 0.2, 0.3]),
        );
    }

    #[test]
    fn test_preserve_embeddings_no_match() {
        let mut new_index = CodeIndex::build(vec![], Path::new("/root"), None);
        new_index.symbols.push(Symbol {
            id: 0,
            name: "bar".into(),
            kind: DefKind::Function,
            file: PathBuf::from("src/lib.rs"),
            line: 1,
            end_line: 10,
            doc: None,
            embedding: None,
        });

        let mut old_index = CodeIndex::build(vec![], Path::new("/root"), None);
        old_index.symbols.push(Symbol {
            id: 0,
            name: "foo".into(),
            kind: DefKind::Function,
            file: PathBuf::from("src/lib.rs"),
            line: 1,
            end_line: 10,
            doc: None,
            embedding: Some(vec![0.1, 0.2, 0.3]),
        });

        new_index.preserve_embeddings(&old_index);
        assert!(new_index.symbols[0].embedding.is_none());
    }

    // -----------------------------------------------------------------------
    // is_relevant_source_event tests
    // -----------------------------------------------------------------------

    fn make_event(kind: notify::EventKind, paths: Vec<PathBuf>) -> notify::Event {
        notify::Event { kind, paths, attrs: notify::event::EventAttributes::default() }
    }

    #[test]
    fn test_is_relevant_source_event_skips_access_and_other() {
        let skip_kinds = [
            notify::EventKind::Access(notify::event::AccessKind::Any),
            notify::EventKind::Other,
        ];
        for kind in &skip_kinds {
            let ev = make_event(kind.clone(), vec![PathBuf::from("src/main.rs")]);
            assert!(!is_relevant_source_event(&ev), "should skip {:?}", kind);
        }
    }

    #[test]
    fn test_is_relevant_source_event_allows_modify() {
        let ev = make_event(
            notify::EventKind::Modify(notify::event::ModifyKind::Data(notify::event::DataChange::Content)),
            vec![PathBuf::from("src/main.rs")],
        );
        assert!(is_relevant_source_event(&ev));
    }

    #[test]
    fn test_is_relevant_source_event_allows_create() {
        let ev = make_event(
            notify::EventKind::Create(notify::event::CreateKind::File),
            vec![PathBuf::from("src/lib.rs")],
        );
        assert!(is_relevant_source_event(&ev));
    }

    #[test]
    fn test_is_relevant_source_event_skips_dot_sift() {
        let ev = make_event(
            notify::EventKind::Create(notify::event::CreateKind::File),
            vec![PathBuf::from(".sift/index.bin")],
        );
        assert!(!is_relevant_source_event(&ev));
    }

    #[test]
    fn test_is_relevant_source_event_skips_non_source() {
        let ev = make_event(
            notify::EventKind::Create(notify::event::CreateKind::File),
            vec![PathBuf::from("Makefile")],
        );
        assert!(!is_relevant_source_event(&ev));
    }

    #[test]
    fn test_is_relevant_source_event_multiple_paths() {
        let ev = make_event(
            notify::EventKind::Modify(notify::event::ModifyKind::Data(notify::event::DataChange::Content)),
            vec![PathBuf::from("README.md"), PathBuf::from("src/main.rs")],
        );
        assert!(is_relevant_source_event(&ev));
    }

    // -----------------------------------------------------------------------
    // collect_mtimes tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_collect_mtimes_finds_source_files() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join("sift_test_mtimes_find");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("main.rs"), "fn main() {}")?;
        std::fs::write(dir.join("lib.py"), "def foo(): pass")?;
        std::fs::write(dir.join("README.md"), "# docs")?;
        std::fs::create_dir(dir.join("target"))?;
        std::fs::write(dir.join("target").join("out.rs"), "fn build() {}")?;

        let mtimes = CodeIndex::collect_mtimes(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        // Should find main.rs and lib.py, but NOT README.md or target/out.rs
        assert!(mtimes.contains_key(&dir.join("main.rs")), "should find main.rs");
        assert!(mtimes.contains_key(&dir.join("lib.py")), "should find lib.py");
        assert!(!mtimes.contains_key(&dir.join("README.md")), "should skip non-source");
        assert!(!mtimes.contains_key(&dir.join("target").join("out.rs")), "should skip target/");
        assert_eq!(mtimes.len(), 2);
        Ok(())
    }

    #[test]
    fn test_collect_mtimes_returns_valid_timestamps() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join("sift_test_mtimes_ts");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("a.rs"), "fn a() {}")?;
        // Sleep 1ms to ensure mtime changes
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(dir.join("b.rs"), "fn b() {}")?;

        let mtimes = CodeIndex::collect_mtimes(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(mtimes.len(), 2);
        let mtime_a = mtimes.get(&dir.join("a.rs")).expect("a.rs should exist");
        let mtime_b = mtimes.get(&dir.join("b.rs")).expect("b.rs should exist");
        assert!(*mtime_b >= *mtime_a, "b.rs (written later) should have >= mtime than a.rs");
        Ok(())
    }

    #[test]
    fn test_collect_mtimes_empty_dir() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join("sift_test_mtimes_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        let mtimes = CodeIndex::collect_mtimes(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(mtimes.is_empty());
        Ok(())
    }
}
