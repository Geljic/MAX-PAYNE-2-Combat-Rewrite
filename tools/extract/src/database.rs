//! Parser for Max Payne 2's text database (`data/database/**/*.txt` and `.h`).
//!
//! The files are a small C-preprocessor dialect (`#include`, `#define`, both
//! comment styles) wrapped around an INI-like tree: `[Section]` headers,
//! optional `{ ... }` blocks and `Key = Value;` statements, several per line.
//! This module turns that into a generic [`Node`] tree. It knows nothing about
//! skins or weapons; `generate.rs` interprets the tree.
//!
//! Format notes (in our own words) live in `docs/formats/database.md`.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A parse failure that names the file and line it happened at.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub file: String,
    pub line: u32,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.msg)
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Virtual file system: the merged archive tree
// ---------------------------------------------------------------------------

/// Where include directives and the parser read files from. A trait so unit
/// tests can feed in memory instead of the real game tree.
pub trait Source {
    /// Look up a virtual path (forward slashes, any case). Returns the
    /// real-cased virtual path and the file's text.
    fn read(&self, path: &str) -> Option<(String, String)>;
    /// True if the virtual path exists; returns its real-cased form.
    fn exists(&self, path: &str) -> Option<String>;
}

struct Entry {
    real: PathBuf,
    virt: String,
}

/// All extracted archives merged into one case-insensitive tree.
///
/// Every archive folder under the root holds a `data/` directory; the same
/// virtual path (`data/database/...`) can appear in several. The first archive
/// in scan order wins (Init, then Data, then the rest alphabetically), and a
/// differing duplicate is recorded in `conflicts`.
pub struct Vfs {
    files: HashMap<String, Entry>,
    pub conflicts: Vec<String>,
}

impl Vfs {
    pub fn scan(root: &Path) -> io::Result<Vfs> {
        let mut archives: Vec<PathBuf> = fs::read_dir(root)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        let rank = |p: &PathBuf| match p.file_name().and_then(|n| n.to_str()) {
            Some("MP2_Init") => 0,
            Some("MP2_Data") => 1,
            _ => 2,
        };
        archives.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
        let mut vfs = Vfs { files: HashMap::new(), conflicts: Vec::new() };
        for a in &archives {
            let data = a.join("data");
            if data.is_dir() {
                vfs.walk(&data, "data")?;
            }
        }
        Ok(vfs)
    }

    fn walk(&mut self, dir: &Path, virt: &str) -> io::Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let v = format!("{virt}/{name}");
            let p = e.path();
            if p.is_dir() {
                self.walk(&p, &v)?;
            } else {
                self.insert(p, v);
            }
        }
        Ok(())
    }

    fn insert(&mut self, real: PathBuf, virt: String) {
        let key = virt.to_ascii_lowercase();
        match self.files.get(&key) {
            None => {
                self.files.insert(key, Entry { real, virt });
            }
            Some(old) => {
                let same = match (fs::read(&old.real), fs::read(&real)) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => false,
                };
                if !same {
                    self.conflicts.push(format!(
                        "{} differs between {} and {}",
                        virt,
                        old.real.display(),
                        real.display()
                    ));
                }
            }
        }
    }

    /// Real-cased virtual paths directly inside `dir` (case-insensitive) whose
    /// names end with `ext` (e.g. ".txt"). Sorted.
    pub fn list(&self, dir: &str, ext: &str) -> Vec<String> {
        let prefix = format!("{}/", dir.to_ascii_lowercase());
        let ext = ext.to_ascii_lowercase();
        let mut v: Vec<String> = self
            .files
            .iter()
            .filter(|(k, _)| {
                k.starts_with(&prefix) && !k[prefix.len()..].contains('/') && k.ends_with(&ext)
            })
            .map(|(_, e)| e.virt.clone())
            .collect();
        v.sort();
        v
    }
}

/// Decode as Latin-1: the files are Windows-1252, and nothing we read cares
/// about the high half beyond not crashing.
fn latin1(bytes: &[u8]) -> String {
    let s: String = bytes.iter().map(|&b| b as char).collect();
    s.strip_prefix('\u{ef}')
        .and_then(|r| r.strip_prefix('\u{bb}'))
        .and_then(|r| r.strip_prefix('\u{bf}'))
        .map(str::to_string)
        .unwrap_or(s)
}

impl Source for Vfs {
    fn read(&self, path: &str) -> Option<(String, String)> {
        let e = self.files.get(&path.to_ascii_lowercase())?;
        let bytes = fs::read(&e.real).ok()?;
        Some((e.virt.clone(), latin1(&bytes)))
    }
    fn exists(&self, path: &str) -> Option<String> {
        self.files.get(&path.to_ascii_lowercase()).map(|e| e.virt.clone())
    }
}

/// Normalise a database path: backslashes to slashes, resolve `.` and `..`.
pub fn normalize_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in p.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// Directory part of a virtual path ("" if none).
pub fn dir_of(p: &str) -> &str {
    p.rfind('/').map(|i| &p[..i]).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Preprocessor
// ---------------------------------------------------------------------------

/// Preprocessed text plus enough bookkeeping to name file and line in errors.
#[derive(Debug, Default)]
pub struct Pre {
    pub text: String,
    /// (offset into `text`, index into `files`, line number) per source line.
    marks: Vec<(usize, usize, u32)>,
    pub files: Vec<String>,
    /// Macros, keys lowercased (the game treats names case-insensitively).
    pub macros: HashMap<String, String>,
    /// `#include`s that could not be found: "file:line: target".
    pub missing_includes: Vec<String>,
}

impl Pre {
    fn locate(&self, offset: usize) -> (String, u32) {
        let i = self.marks.partition_point(|m| m.0 <= offset).saturating_sub(1);
        match self.marks.get(i) {
            Some(&(_, f, l)) => (self.files[f].clone(), l),
            None => ("<empty>".into(), 0),
        }
    }
}

/// Strip `//` and `/* */` comments from `src`, keeping line structure.
/// Quotes protect comment markers; `in_block` carries across lines.
fn strip_comments(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in src.lines() {
        let c: Vec<char> = line.chars().collect();
        let mut s = String::new();
        let mut i = 0;
        let mut in_str = false;
        while i < c.len() {
            if in_block {
                if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    in_block = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if in_str {
                s.push(c[i]);
                if c[i] == '\\' && i + 1 < c.len() {
                    s.push(c[i + 1]);
                    i += 2;
                    continue;
                }
                if c[i] == '"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            match c[i] {
                '"' => {
                    in_str = true;
                    s.push('"');
                    i += 1;
                }
                '/' if c.get(i + 1) == Some(&'/') => break,
                '/' if c.get(i + 1) == Some(&'*') => {
                    in_block = true;
                    i += 2;
                }
                ch => {
                    s.push(ch);
                    i += 1;
                }
            }
        }
        out.push(s);
    }
    out
}

/// Run the preprocessor on `root` (a virtual path such as
/// `data/database/skins/MaxPayne.txt`).
pub fn preprocess(src: &dyn Source, root: &str) -> Result<Pre, ParseError> {
    let mut pre = Pre::default();
    let mut stack = Vec::new();
    pp_file(src, root, &mut pre, &mut stack)?;
    Ok(pre)
}

fn pp_file(
    src: &dyn Source,
    path: &str,
    pre: &mut Pre,
    stack: &mut Vec<String>,
) -> Result<(), ParseError> {
    let (virt, text) = src.read(path).ok_or_else(|| ParseError {
        file: path.to_string(),
        line: 0,
        msg: "file not found".into(),
    })?;
    if stack.len() > 32 {
        return Err(ParseError { file: virt, line: 0, msg: "include depth over 32 (cycle?)".into() });
    }
    stack.push(virt.clone());
    let file_idx = pre.files.len();
    pre.files.push(virt.clone());
    for (n, line) in strip_comments(&text).into_iter().enumerate() {
        let ln = n as u32 + 1;
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim_start();
            let (word, arg) = match rest.find(char::is_whitespace) {
                Some(i) => (&rest[..i], rest[i..].trim()),
                None => (rest, ""),
            };
            match word.to_ascii_lowercase().as_str() {
                "define" => {
                    let (name, val) = match arg.find(char::is_whitespace) {
                        Some(i) => (&arg[..i], arg[i..].trim()),
                        None => (arg, ""),
                    };
                    pre.macros.insert(name.to_ascii_lowercase(), val.to_string());
                }
                "include" => {
                    let target = arg
                        .strip_prefix('<')
                        .and_then(|a| a.split('>').next())
                        .or_else(|| arg.strip_prefix('"').and_then(|a| a.split('"').next()))
                        .ok_or_else(|| ParseError {
                            file: virt.clone(),
                            line: ln,
                            msg: format!("bad #include: {arg}"),
                        })?;
                    let rel = target.replace('\\', "/");
                    let cands = [
                        normalize_path(&format!("{}/{}", dir_of(&virt), rel)),
                        normalize_path(&format!("data/database/{rel}")),
                        normalize_path(&format!("data/{rel}")),
                        normalize_path(&rel),
                    ];
                    match cands.iter().find_map(|c| src.exists(c)) {
                        Some(found) => pp_file(src, &found, pre, stack)?,
                        None => pre.missing_includes.push(format!("{virt}:{ln}: {target}")),
                    }
                }
                other => {
                    return Err(ParseError {
                        file: virt,
                        line: ln,
                        msg: format!("unsupported directive #{other}"),
                    });
                }
            }
            continue;
        }
        pre.marks.push((pre.text.len(), file_idx, ln));
        pre.text.push_str(&line);
        pre.text.push('\n');
    }
    stack.pop();
    Ok(())
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A statement's value after macro expansion.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Value {
    /// Source text as written (macros not expanded), trimmed.
    pub raw: String,
    /// Expanded text: strings unquoted and unescaped, adjacent string
    /// literals and macros concatenated C-style, tuples as `a b c`.
    pub text: String,
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        parse_number(&self.text)
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self.text.to_ascii_lowercase().as_str() {
            "yes" | "true" => Some(true),
            "no" | "false" => Some(false),
            _ => None,
        }
    }
}

/// Numbers: decimal, hex (`0x7FFF`) and percentages (`7%` is 0.07).
pub fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        return p.trim().parse::<f64>().ok().map(|v| v / 100.0);
    }
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return i64::from_str_radix(h, 16).ok().map(|v| v as f64);
    }
    s.parse::<f64>().ok()
}

enum Piece {
    Str(String),
    Word(String),
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Names that look like constants (UPPER_SNAKE with an underscore) but are
/// not defined anywhere; reported so a missing header does not go unnoticed.
fn looks_like_constant(s: &str) -> bool {
    s.contains('_')
        && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !matches!(s, "TRUE" | "FALSE" | "YES" | "NO")
}

struct Expander<'a> {
    macros: &'a HashMap<String, String>,
    unresolved: &'a mut BTreeMap<String, u32>,
}

impl Expander<'_> {
    /// Expand one bare word: a macro's value, else the word itself.
    fn word(&mut self, w: &str, depth: u32) -> String {
        if is_ident(w) {
            if let Some(m) = self.macros.get(&w.to_ascii_lowercase()) {
                if depth < 16 {
                    let m = m.clone();
                    return self.text(&m, depth + 1);
                }
            } else if looks_like_constant(w) {
                *self.unresolved.entry(w.to_string()).or_insert(0) += 1;
            }
        }
        w.to_string()
    }

    /// Expand free-form value text (a macro body or a statement value).
    fn text(&mut self, s: &str, depth: u32) -> String {
        let pieces = scan_pieces(s).0;
        let any_str = pieces.iter().any(|p| matches!(p, Piece::Str(_)));
        let parts: Vec<String> = pieces
            .into_iter()
            .map(|p| match p {
                Piece::Str(s) => s,
                Piece::Word(w) if w.starts_with('(') => self.tuple(&w, depth),
                Piece::Word(w) => self.word(&w, depth),
            })
            .collect();
        parts.join(if any_str { "" } else { " " })
    }

    fn tuple(&mut self, t: &str, depth: u32) -> String {
        let inner = t.trim_start_matches('(').trim_end_matches(')');
        inner
            .split(',')
            .map(|e| self.text(e.trim(), depth))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Split value text into string literals and bare words/tuples. Scans bytes
/// (every delimiter is ASCII, so multi-byte characters stay inside pieces)
/// and stops at an unquoted `;` or `}`. Returns the pieces and the byte
/// length consumed, not counting the terminator. A string that does not close
/// on its line adds a marker word `"\0"` so the caller can report it.
fn scan_pieces(s: &str) -> (Vec<Piece>, usize, bool) {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let ch = b[i];
        if ch.is_ascii_whitespace() {
            i += 1;
        } else if ch == b';' || ch == b'}' {
            break;
        } else if ch == b'"' {
            let mut st: Vec<u8> = Vec::new();
            i += 1;
            let mut closed = false;
            while i < b.len() {
                match b[i] {
                    b'\\' if b.get(i + 1) == Some(&b'"') => {
                        st.push(b'"');
                        i += 2;
                    }
                    b'"' => {
                        closed = true;
                        i += 1;
                        break;
                    }
                    b'\n' => break,
                    x => {
                        st.push(x);
                        i += 1;
                    }
                }
            }
            out.push(Piece::Str(String::from_utf8_lossy(&st).into_owned()));
            if !closed {
                out.push(Piece::Word("\u{0}".into()));
            }
        } else if ch == b'(' {
            let start = i;
            while i < b.len() && b[i] != b')' && b[i] != b'\n' {
                i += 1;
            }
            if i < b.len() && b[i] == b')' {
                i += 1;
            }
            out.push(Piece::Word(s[start..i].to_string()));
        } else {
            let start = i;
            while i < b.len()
                && !b[i].is_ascii_whitespace()
                && !matches!(b[i], b';' | b'"' | b'(' | b'}')
            {
                i += 1;
            }
            out.push(Piece::Word(s[start..i].to_string()));
        }
    }
    (out, i, b.get(i) == Some(&b';'))
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// `[Name]`
    Section(String),
    /// `{`
    Open,
    /// `}`
    Close,
    /// `Key = Value;`
    Assign(String, Value),
}

/// A token and the byte offset in the preprocessed text where it began.
pub type Spanned = (Tok, usize);

fn tokenize(
    pre: &Pre,
    unresolved: &mut BTreeMap<String, u32>,
) -> Result<Vec<Spanned>, ParseError> {
    let text = &pre.text;
    let b = text.as_bytes();
    let err = |off: usize, msg: String| {
        let (file, line) = pre.locate(off);
        ParseError { file, line, msg }
    };
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let ch = b[i] as char;
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '{' {
            toks.push((Tok::Open, i));
            i += 1;
        } else if ch == '}' {
            toks.push((Tok::Close, i));
            i += 1;
        } else if ch == ';' {
            i += 1; // stray terminator
        } else if ch == '[' {
            let start = i;
            let close = text[i..]
                .find(['\n', ']'])
                .filter(|&k| b[i + k] == b']')
                .ok_or_else(|| err(i, "unclosed '[' section header".into()))?;
            toks.push((Tok::Section(text[i + 1..i + close].trim().to_string()), start));
            i += close + 1;
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            let start = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            let key = text[start..i].to_string();
            while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                i += 1;
            }
            if b.get(i) != Some(&b'=') {
                return Err(err(start, format!("expected '=' after '{key}'")));
            }
            i += 1;
            let (pieces, used, _semi) = scan_pieces(&text[i..]);
            let raw = text[i..i + used].trim().to_string();
            if matches!(pieces.last(), Some(Piece::Word(w)) if w == "\u{0}") {
                return Err(err(start, format!("unterminated string in value of '{key}'")));
            }
            let mut ex = Expander { macros: &pre.macros, unresolved };
            let val = ex.text(&raw, 0);
            // Raw strings keep their quotes; text is the expanded form.
            toks.push((Tok::Assign(key, Value { raw, text: val }), start));
            i += used;
            if b.get(i) == Some(&b';') {
                i += 1;
            }
        } else {
            return Err(err(i, format!("unexpected character '{ch}'")));
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// Tree
// ---------------------------------------------------------------------------

/// A `[Section]`: its own `Key = Value` pairs in order and its child sections.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub props: Vec<(String, Value)>,
    pub children: Vec<Node>,
    /// "file:line" of the header, for diagnostics.
    pub loc: String,
}

impl Node {
    /// First value for `key` (case-insensitive).
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.props.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.value(key).map(|v| v.text.as_str())
    }
    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Value> {
        self.props.iter().filter(move |(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }
    pub fn num(&self, key: &str) -> Option<f64> {
        self.value(key).and_then(Value::as_f64)
    }
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |c| c.name.eq_ignore_ascii_case(name))
    }
    /// Depth-first walk over this node and every descendant.
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for c in &self.children {
            c.walk(out);
        }
    }
}

/// Result of parsing one root file with its includes.
#[derive(Debug)]
pub struct Parsed {
    pub root: Node,
    pub missing_includes: Vec<String>,
    /// UPPER_CASE constants that no `#define` covered, with use counts.
    pub unresolved: BTreeMap<String, u32>,
    pub macros: HashMap<String, String>,
}

/// Preprocess, tokenize and build the tree for `root`.
pub fn parse(src: &dyn Source, root: &str) -> Result<Parsed, ParseError> {
    let pre = preprocess(src, root)?;
    let mut unresolved = BTreeMap::new();
    let toks = tokenize(&pre, &mut unresolved)?;
    let mut pos = 0;
    let mut tree = Node { name: String::new(), ..Default::default() };
    build(&pre, &toks, &mut pos, &mut tree, true)?;
    Ok(Parsed {
        root: tree,
        missing_includes: pre.missing_includes,
        unresolved,
        macros: pre.macros,
    })
}

/// Parse text already in memory (tests). Includes still resolve through `src`.
pub fn build_tree(
    pre: &Pre,
    unresolved: &mut BTreeMap<String, u32>,
) -> Result<Node, ParseError> {
    let toks = tokenize(pre, unresolved)?;
    let mut pos = 0;
    let mut tree = Node::default();
    build(pre, &toks, &mut pos, &mut tree, true)?;
    Ok(tree)
}

/// Fill `parent` from tokens until its closing brace (or EOF at top level).
///
/// A `[Name]` not followed by `{` stays "open": the `Key = Value;` statements
/// after it belong to it until the next header or the end of the block. That
/// is how `[Animation] Index = X; Filename = "y";` and `[Bone] Name = "z";`
/// lines work.
fn build(
    pre: &Pre,
    toks: &[Spanned],
    pos: &mut usize,
    parent: &mut Node,
    top: bool,
) -> Result<(), ParseError> {
    let mut open: Option<usize> = None; // index into parent.children
    let loc = |off: usize| {
        let (f, l) = pre.locate(off);
        format!("{f}:{l}")
    };
    while *pos < toks.len() {
        let (tok, off) = &toks[*pos];
        *pos += 1;
        match tok {
            Tok::Section(name) => {
                parent.children.push(Node { name: name.clone(), loc: loc(*off), ..Default::default() });
                let idx = parent.children.len() - 1;
                if matches!(toks.get(*pos), Some((Tok::Open, _))) {
                    *pos += 1;
                    build(pre, toks, pos, &mut parent.children[idx], false)?;
                    open = None;
                } else {
                    open = Some(idx);
                }
            }
            Tok::Assign(k, v) => {
                let target = match open {
                    Some(i) => &mut parent.children[i],
                    None => &mut *parent,
                };
                target.props.push((k.clone(), v.clone()));
            }
            Tok::Open => {
                // A bare block with no header: treat as part of the parent.
                build(pre, toks, pos, parent, false)?;
                open = None;
            }
            Tok::Close => {
                if top {
                    let (file, line) = pre.locate(*off);
                    return Err(ParseError { file, line, msg: "unmatched '}'".into() });
                }
                return Ok(());
            }
        }
    }
    if !top {
        let (file, line) = pre.locate(pre.text.len().saturating_sub(1));
        return Err(ParseError { file, line, msg: format!("missing '}}' for block '{}'", parent.name) });
    }
    Ok(())
}

/// A tiny in-memory [`Source`] for tests and tools.
#[derive(Default)]
pub struct MemSource(pub HashMap<String, String>);

impl MemSource {
    pub fn with(files: &[(&str, &str)]) -> Self {
        MemSource(files.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect())
    }
}

impl Source for MemSource {
    fn read(&self, path: &str) -> Option<(String, String)> {
        self.0.get(&path.to_ascii_lowercase()).map(|t| (path.to_string(), t.clone()))
    }
    fn exists(&self, path: &str) -> Option<String> {
        self.0.contains_key(&path.to_ascii_lowercase()).then(|| path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(text: &str) -> Vec<Tok> {
        let src = MemSource::with(&[("a.txt", text)]);
        let pre = preprocess(&src, "a.txt").unwrap();
        let mut u = BTreeMap::new();
        tokenize(&pre, &mut u).unwrap().into_iter().map(|t| t.0).collect()
    }

    fn val(text: &str) -> Value {
        match toks(text).remove(0) {
            Tok::Assign(_, v) => v,
            t => panic!("not an assign: {t:?}"),
        }
    }

    #[test]
    fn comments_are_stripped_but_not_inside_strings() {
        let t = toks("A = 1; // gone\n/* block\n still */ B = \"x//y\"; /* c */ C = 3;");
        assert_eq!(t.len(), 3);
        assert!(matches!(&t[1], Tok::Assign(k, v) if k == "B" && v.text == "x//y"));
    }

    #[test]
    fn define_and_include_resolve_relative_to_includer() {
        let src = MemSource::with(&[
            ("data/database/skins/a.txt", "#include <..\\shared.h>\n#include \"local.h\"\nX = FOO; Y = BAR;"),
            ("data/database/shared.h", "#define FOO 42"),
            ("data/database/skins/local.h", "#define BAR \"hi\""),
        ]);
        let p = parse(&src, "data/database/skins/a.txt").unwrap();
        assert_eq!(p.root.get("x"), Some("42"));
        assert_eq!(p.root.get("Y"), Some("hi"));
        assert_eq!(p.root.value("x").unwrap().raw, "FOO");
        assert!(p.missing_includes.is_empty());
    }

    #[test]
    fn missing_include_is_reported_not_fatal() {
        let src = MemSource::with(&[("a.txt", "#include <nope.h>\nA = 1;")]);
        let p = parse(&src, "a.txt").unwrap();
        assert_eq!(p.missing_includes.len(), 1);
    }

    #[test]
    fn several_statements_per_line_and_open_sections() {
        let src = MemSource::with(&[(
            "a.txt",
            "[Animation] Index = 3; Filename = \"x.kf2\";\n[Properties]\n{\n [Message] Frame = 1; String = \"hi\";\n}",
        )]);
        let p = parse(&src, "a.txt").unwrap();
        let a = &p.root.children[0];
        assert_eq!(a.name, "Animation");
        assert_eq!(a.get("filename"), Some("x.kf2"));
        let m = &p.root.children[1].children[0];
        assert_eq!(m.num("frame"), Some(1.0));
    }

    #[test]
    fn tuples_percent_and_bare_paths() {
        assert_eq!(val("A = ( 0, 0.08, 0.07 );").text, "0 0.08 0.07");
        assert_eq!(val("A = (0,0,1);").text, "0 0 1");
        assert_eq!(val("A = 7%;").as_f64(), Some(0.07));
        assert_eq!(val("A = 0x7FFF;").as_f64(), Some(32767.0));
        assert_eq!(val("A = anim\\Run.kf2;").text, "anim\\Run.kf2");
        assert_eq!(val("A = YES;").as_bool(), Some(true));
    }

    #[test]
    fn string_escapes_and_concatenation_with_macros() {
        let src = MemSource::with(&[(
            "a.txt",
            "#define N 2\n#define T TRUE\nA = \"say \\\"hi\\\"\";\nB = \"f(\" N \",\" N \")\";\nC = \"g( \"AI\", \"x\" );\";\nD = \"h( \" T \" );\";",
        )]);
        let p = parse(&src, "a.txt").unwrap();
        assert_eq!(p.root.get("A"), Some("say \"hi\""));
        assert_eq!(p.root.get("B"), Some("f(2,2)"));
        assert_eq!(p.root.get("C"), Some("g( AI, x );"));
        assert_eq!(p.root.get("D"), Some("h( TRUE );"));
    }

    #[test]
    fn errors_name_file_and_line() {
        let src = MemSource::with(&[("a.txt", "A = 1;\n\nB 2;")]);
        let e = parse(&src, "a.txt").unwrap_err();
        assert_eq!((e.file.as_str(), e.line), ("a.txt", 3));
        let src = MemSource::with(&[("a.txt", "[S]\n{\nA = 1;")]);
        assert!(parse(&src, "a.txt").is_err());
        let src = MemSource::with(&[("a.txt", "A = \"open;")]);
        assert!(parse(&src, "a.txt").unwrap_err().msg.contains("unterminated"));
    }

    #[test]
    fn unresolved_constants_are_collected() {
        let src = MemSource::with(&[("a.txt", "A = SOME_CONST;")]);
        let p = parse(&src, "a.txt").unwrap();
        assert_eq!(p.unresolved.get("SOME_CONST"), Some(&1));
    }

    #[test]
    fn path_normalising() {
        assert_eq!(normalize_path("data\\database\\skins\\..\\x.h"), "data/database/x.h");
    }
}
