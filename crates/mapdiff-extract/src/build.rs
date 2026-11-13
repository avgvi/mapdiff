//! Turns what a language extractor found (`Raw`) into finished scopes: IDs,
//! ranges, signatures and hashes. Everything here is shared by all languages.

use std::collections::HashMap;

use mapdiff_core::{
    Access, Hashes, Language, Modifier, Position, Range, Scope, ScopeKind, ScopePath,
};
use tree_sitter::Node;
use xxhash_rust::xxh3::Xxh3;

/// How a scope names itself inside its parent.
#[derive(Debug, Clone)]
pub enum Segment {
    /// A fixed segment, for example `total_area()` or `impl Circle`.
    Name(String),
    /// Numbered in source order among sibling imports: `{import.N}`.
    Import,
    /// Numbered in source order among sibling blocks: `{block.N}`.
    Block,
}

/// A scope as a language extractor sees it: which syntax nodes it covers and
/// what it is. Positions, IDs and hashes are filled in by `Builder`.
pub struct Raw<'t> {
    pub kind: ScopeKind,
    pub lang_kind: &'static str,
    pub name: Option<String>,
    pub segment: Segment,
    pub access: Access,
    pub modifiers: Vec<Modifier>,
    /// First syntax node of the scope.
    pub node: Node<'t>,
    /// Last syntax node. Differs from `node` only for runs of statements.
    pub last: Node<'t>,
    pub name_node: Option<Node<'t>>,
    pub body: Option<Node<'t>>,
    pub children: Vec<Raw<'t>>,
}

impl<'t> Raw<'t> {
    pub fn new(kind: ScopeKind, lang_kind: &'static str, node: Node<'t>) -> Self {
        Self {
            kind,
            lang_kind,
            name: None,
            segment: Segment::Block,
            access: Access::Private,
            modifiers: Vec::new(),
            node,
            last: node,
            name_node: None,
            body: None,
            children: Vec::new(),
        }
    }

    /// Sets the name from a node and uses it as the segment.
    pub fn named(mut self, name_node: Node<'t>, source: &[u8]) -> Self {
        let name = text(name_node, source).to_owned();
        self.segment = Segment::Name(name.clone());
        self.name = Some(name);
        self.name_node = Some(name_node);
        self
    }

    pub fn segment(mut self, segment: impl Into<String>) -> Self {
        self.segment = Segment::Name(segment.into());
        self
    }

    pub fn access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    pub fn modifier(mut self, modifier: Modifier) -> Self {
        if !self.modifiers.contains(&modifier) {
            self.modifiers.push(modifier);
        }
        self
    }

    pub fn modifier_if(self, condition: bool, modifier: Modifier) -> Self {
        if condition {
            self.modifier(modifier)
        } else {
            self
        }
    }

    pub fn body(mut self, body: Option<Node<'t>>) -> Self {
        self.body = body;
        self
    }

    /// True when any syntax node of this scope holds an unparsed or
    /// incomplete region.
    pub fn has_error(&self) -> bool {
        let mut node = self.node;
        loop {
            if node.has_error() {
                return true;
            }
            if node.id() == self.last.id() {
                return false;
            }
            match node.next_sibling() {
                Some(next) => node = next,
                None => return false,
            }
        }
    }

    pub fn children(mut self, children: Vec<Raw<'t>>) -> Self {
        self.children = children;
        self
    }
}

/// What a language extractor decides about one child node of a container.
///
/// Values live only while one container is classified, so the size gap
/// between the variants costs nothing worth a `Box`.
#[allow(clippy::large_enum_variant)]
pub enum Item<'t> {
    /// A comment, attribute or decorator. Attaches to whatever follows it.
    Trivia,
    /// Belongs to the parent itself: a docstring, a `'use strict'` line.
    Owned,
    /// A scope of its own.
    Scope(Raw<'t>),
    /// A plain statement. Adjacent statements are merged into one block.
    Statement(&'static str),
    /// A statement that is a unit by itself (an `if`, a `try`, an immediately
    /// invoked function). Always a block of its own.
    Standalone(&'static str),
}

/// Groups classified children into the final list for a container, merging
/// runs of statements into `block` scopes. `nested` finds named definitions
/// inside a statement, which become children of its block.
pub fn assemble<'t>(
    items: Vec<(Node<'t>, Item<'t>)>,
    nested: &dyn Fn(Node<'t>) -> Vec<Raw<'t>>,
) -> Vec<Raw<'t>> {
    let mut out = Vec::new();
    let mut run: Option<Raw<'t>> = None;

    for (node, item) in items {
        let is_error = node.is_error();
        match item {
            Item::Trivia => {}
            Item::Statement(lang_kind) => {
                let children = nested(node);
                match run.as_mut() {
                    // An unparsed region is always a block of its own.
                    Some(block) if !is_error && !block.modifiers.contains(&Modifier::Error) => {
                        block.last = node;
                        block.children.extend(children);
                    }
                    _ => {
                        out.extend(run.take());
                        let mut block = Raw::new(ScopeKind::Block, lang_kind, node)
                            .modifier_if(is_error, Modifier::Error);
                        block.children = children;
                        run = Some(block);
                    }
                }
            }
            Item::Standalone(lang_kind) => {
                out.extend(run.take());
                let mut block = Raw::new(ScopeKind::Block, lang_kind, node)
                    .modifier_if(is_error, Modifier::Error);
                block.children = nested(node);
                out.push(block);
            }
            Item::Owned => out.extend(run.take()),
            Item::Scope(raw) => {
                out.extend(run.take());
                out.push(raw);
            }
        }
    }
    out.extend(run);
    out
}

pub fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.byte_range()]).unwrap_or("")
}

/// The last line a node touches. Nodes that end right after a newline (doc
/// comments do) end on the line before.
pub fn last_line(node: Node<'_>) -> usize {
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

/// Collapses runs of whitespace to single spaces.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Token {
    start: u32,
    end: u32,
    comment: bool,
}

/// Per-file state for building scopes.
pub struct Builder<'s> {
    source: &'s [u8],
    language: Language,
    file: String,
    is_attachable: fn(Node<'_>) -> bool,
    line_starts: Vec<u32>,
    tokens: Vec<Token>,
}

const SIGNATURE_LIMIT: usize = 200;

impl<'s> Builder<'s> {
    pub fn new(
        file: &str,
        source: &'s [u8],
        language: Language,
        root: Option<Node<'_>>,
        is_comment: fn(Node<'_>) -> bool,
        is_attachable: fn(Node<'_>) -> bool,
    ) -> Self {
        let mut line_starts = vec![0u32];
        for (offset, byte) in source.iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(offset as u32 + 1);
            }
        }
        let mut tokens = Vec::new();
        match root {
            Some(root) => collect_tokens(root, source, is_comment, &mut tokens),
            // An unparsed file is one token, so its hashes still track edits.
            None if !source.is_empty() => tokens.push(Token {
                start: 0,
                end: source.len() as u32,
                comment: false,
            }),
            None => {}
        }
        Self {
            source,
            language,
            file: file.to_owned(),
            is_attachable,
            line_starts,
            tokens,
        }
    }

    fn position(&self, byte: u32) -> Position {
        let line = match self.line_starts.binary_search(&byte) {
            Ok(line) => line,
            Err(next) => next - 1,
        };
        Position {
            line: line as u32,
            column: byte - self.line_starts[line],
        }
    }

    pub fn range(&self, start: usize, end: usize) -> Range {
        let (start, end) = (start as u32, end.max(start) as u32);
        Range {
            start_byte: start,
            end_byte: end,
            start: self.position(start),
            end: self.position(end),
        }
    }

    fn hash(&self, start: u32, end: u32, with_comments: bool) -> String {
        let first = self.tokens.partition_point(|token| token.start < start);
        let mut hasher = Xxh3::new();
        for token in &self.tokens[first..] {
            if token.end > end {
                break;
            }
            if token.comment && !with_comments {
                continue;
            }
            hasher.update(&self.source[token.start as usize..token.end as usize]);
            hasher.update(&[0x1f]);
        }
        format!("{:016x}", hasher.digest())
    }

    fn hashes(&self, range: &Range, header: &Range, body: Option<&Range>) -> Hashes {
        let mut text = Xxh3::new();
        text.update(&self.source[range.start_byte as usize..range.end_byte as usize]);
        // Without a body, the body hash is the hash of nothing.
        let (body_start, body_end) = body.map_or((0, 0), |body| (body.start_byte, body.end_byte));
        Hashes {
            text: format!("{:016x}", text.digest()),
            tokens: self.hash(range.start_byte, range.end_byte, true),
            code: self.hash(range.start_byte, range.end_byte, false),
            signature: self.hash(header.start_byte, header.end_byte, false),
            body: self.hash(body_start, body_end, false),
        }
    }

    /// The source between two offsets as one line, without comments.
    fn signature(&self, start: u32, end: u32) -> Option<String> {
        let first = self.tokens.partition_point(|token| token.start < start);
        let mut out = String::new();
        let mut previous_end = start;
        for token in &self.tokens[first..] {
            if token.end > end {
                break;
            }
            if token.comment {
                continue;
            }
            if token.start > previous_end && !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&String::from_utf8_lossy(
                &self.source[token.start as usize..token.end as usize],
            ));
            previous_end = token.end;
        }
        let mut line = one_line(&out);
        if line.is_empty() {
            return None;
        }
        if line.chars().count() > SIGNATURE_LIMIT {
            line = line.chars().take(SIGNATURE_LIMIT).collect::<String>() + "…";
        }
        Some(line)
    }

    /// Where a scope starts once comments, attributes and decorators that sit
    /// directly above it are attached. Never goes above `floor`.
    fn attached_start(&self, node: Node<'_>, floor: usize) -> usize {
        let mut first = node;
        loop {
            let previous = match first.prev_sibling() {
                Some(previous) => Some(previous),
                // A comment above the first statement of an indented block is
                // a sibling of the block, not of the statement.
                None => first
                    .parent()
                    .filter(|parent| parent.start_byte() == first.start_byte())
                    .and_then(|parent| parent.prev_sibling()),
            };
            let Some(previous) = previous else { break };
            if !(self.is_attachable)(previous)
                || previous.start_byte() < floor
                || first.start_position().row > last_line(previous) + 1
            {
                break;
            }
            // A comment that shares a line with earlier code trails that code.
            if let Some(before) = previous.prev_sibling()
                && last_line(before) == previous.start_position().row
            {
                break;
            }
            first = previous;
        }
        first.start_byte()
    }

    /// Where a scope ends once a comment trailing on its last line is added.
    fn attached_end(&self, node: Node<'_>, ceiling: usize) -> usize {
        match node.next_sibling() {
            Some(next)
                if (self.is_attachable)(next)
                    && next.kind().contains("comment")
                    && next.start_position().row == last_line(node)
                    && next.end_byte() <= ceiling =>
            {
                // Doc-style comment nodes include their newline; leave it out.
                let mut end = next.end_byte();
                while end > next.start_byte() && matches!(self.source[end - 1], b'\n' | b'\r') {
                    end -= 1;
                }
                end
            }
            _ => node.end_byte(),
        }
    }

    /// Builds the file scope and everything under it, in pre-order.
    pub fn file(
        &self,
        kind: ScopeKind,
        mut modifiers: Vec<Modifier>,
        root_has_error: bool,
        children: Vec<Raw<'_>>,
    ) -> Vec<Scope> {
        // A syntax error marks the innermost scope that contains it. One
        // that no child contains belongs to the file itself.
        if root_has_error && !children.iter().any(Raw::has_error) {
            modifiers.push(Modifier::Error);
        }
        let whole = self.range(0, self.source.len());
        let header = self.range(0, 0);
        let path = ScopePath {
            file: self.file.clone(),
            segments: Vec::new(),
        };
        let name = self
            .file
            .rsplit('/')
            .next()
            .unwrap_or(&self.file)
            .to_owned();
        let mut file = Scope {
            id: path.id(),
            path,
            kind,
            lang_kind: String::from(if kind == ScopeKind::File {
                "file"
            } else {
                "document"
            }),
            language: self.language,
            name: Some(name),
            access: Access::Public,
            modifiers,
            range: whole,
            name_range: None,
            header_range: header,
            body_range: Some(whole),
            signature: None,
            hashes: self.hashes(&whole, &header, Some(&whole)),
            parent: None,
            children: Vec::new(),
        };
        file.modifiers.sort();

        let mut out = Vec::new();
        let child_ids = self.emit_children(children, &file, &mut out);
        file.children = child_ids;
        out.insert(0, file);
        out
    }

    fn emit_children(
        &self,
        mut children: Vec<Raw<'_>>,
        parent: &Scope,
        out: &mut Vec<Scope>,
    ) -> Vec<String> {
        children.sort_by_key(|child| child.node.start_byte());
        let segments = segments_for(&children);
        let mut ids = Vec::new();
        let mut floor = parent.range.start_byte as usize;
        let ceiling = parent.range.end_byte as usize;

        for (raw, segment) in children.into_iter().zip(segments) {
            let start = self.attached_start(raw.node, floor);
            let end = self.attached_end(raw.last, ceiling);
            // A child that would overlap its neighbour or leave its parent is
            // dropped rather than breaking the coverage rule.
            if start < floor || end > ceiling || end <= start {
                continue;
            }
            floor = end;
            ids.push(self.emit(raw, segment, start, end, parent, out));
        }
        ids
    }

    fn emit(
        &self,
        raw: Raw<'_>,
        segment: String,
        start: usize,
        end: usize,
        parent: &Scope,
        out: &mut Vec<Scope>,
    ) -> String {
        let range = self.range(start, end);

        let (header_range, body_range) = match raw.body {
            Some(body) if body.end_byte() <= end && body.start_byte() >= start => {
                let mut before = body.prev_sibling();
                while let Some(node) = before
                    && node.is_extra()
                {
                    before = node.prev_sibling();
                }
                let header_end = before
                    .map(|node| node.end_byte())
                    .filter(|offset| *offset >= start)
                    .unwrap_or(body.start_byte());
                let (open, close) = (
                    self.source[body.start_byte()],
                    self.source[body.end_byte() - 1],
                );
                let inner = if matches!((open, close), (b'{', b'}') | (b'(', b')') | (b'[', b']')) {
                    self.range(body.start_byte() + 1, body.end_byte() - 1)
                } else {
                    self.range(header_end, body.end_byte())
                };
                (self.range(start, header_end), Some(inner))
            }
            _ => (range, None),
        };

        let signature = match raw.kind {
            ScopeKind::Block => None,
            _ => self.signature(
                raw.node.start_byte() as u32,
                header_range.end_byte.max(raw.node.start_byte() as u32),
            ),
        };

        let mut path = parent.path.clone();
        path.segments.push(segment);

        let own_error = raw.has_error() && !raw.children.iter().any(Raw::has_error);
        let mut modifiers = raw.modifiers;
        if own_error {
            modifiers.push(Modifier::Error);
        }
        if parent.has(Modifier::Test) && !modifiers.contains(&Modifier::Test) {
            modifiers.push(Modifier::Test);
        }
        modifiers.sort();
        modifiers.dedup();

        let mut scope = Scope {
            id: path.id(),
            path,
            kind: raw.kind,
            lang_kind: raw.lang_kind.to_owned(),
            language: self.language,
            name: raw.name,
            access: raw.access,
            modifiers,
            range,
            name_range: raw
                .name_node
                .filter(|node| node.start_byte() >= start && node.end_byte() <= end)
                .map(|node| self.range(node.start_byte(), node.end_byte())),
            header_range,
            body_range,
            signature,
            hashes: self.hashes(&range, &header_range, body_range.as_ref()),
            parent: Some(parent.id.clone()),
            children: Vec::new(),
        };

        let position = out.len();
        let id = scope.id.clone();
        let mut below = Vec::new();
        scope.children = self.emit_children(raw.children, &scope, &mut below);
        out.insert(position, scope);
        out.extend(below);
        id
    }
}

/// Final segment text for each sibling: numbers imports and blocks, then
/// adds `#N` to repeats.
fn segments_for(children: &[Raw<'_>]) -> Vec<String> {
    let (mut imports, mut blocks) = (0, 0);
    let mut seen: HashMap<String, u32> = HashMap::new();
    children
        .iter()
        .map(|child| {
            let base = match &child.segment {
                Segment::Name(name) => name.clone(),
                Segment::Import => {
                    imports += 1;
                    format!("{{import.{imports}}}")
                }
                Segment::Block => {
                    blocks += 1;
                    format!("{{block.{blocks}}}")
                }
            };
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{base}#{count}")
            }
        })
        .collect()
}

/// Lists the file's tokens in order. A comment is one token. Text between
/// leaves that is not whitespace (the inside of some string literals) is kept
/// as a token too, so a change there always changes a hash.
fn collect_tokens(
    root: Node<'_>,
    source: &[u8],
    is_comment: fn(Node<'_>) -> bool,
    out: &mut Vec<Token>,
) {
    let mut cursor = root.walk();
    let mut previous_end = 0usize;
    let push = |start: usize,
                end: usize,
                comment: bool,
                out: &mut Vec<Token>,
                previous_end: &mut usize| {
        if start > *previous_end
            && source[*previous_end..start]
                .iter()
                .any(|byte| !byte.is_ascii_whitespace())
        {
            out.push(Token {
                start: *previous_end as u32,
                end: start as u32,
                comment: false,
            });
        }
        if end > start {
            out.push(Token {
                start: start as u32,
                end: end as u32,
                comment,
            });
        }
        *previous_end = (*previous_end).max(end);
    };

    'walk: loop {
        let node = cursor.node();
        let comment = is_comment(node);
        let leaf = comment || node.child_count() == 0;
        if leaf {
            if node.start_byte() >= previous_end {
                push(
                    node.start_byte(),
                    node.end_byte(),
                    comment,
                    out,
                    &mut previous_end,
                );
            }
        } else if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    push(source.len(), source.len(), false, out, &mut previous_end);
}

/// A scope for a file that is not parsed: one `document` covering all of it.
pub fn document(
    file: &str,
    source: &[u8],
    language: Language,
    modifiers: Vec<Modifier>,
) -> Vec<Scope> {
    fn never(_: Node<'_>) -> bool {
        false
    }
    Builder::new(file, source, language, None, never, never).file(
        ScopeKind::Document,
        modifiers,
        false,
        Vec::new(),
    )
}
