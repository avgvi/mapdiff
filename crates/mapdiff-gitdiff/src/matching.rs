//! Decides which scope on the base side is the same scope on the head side.
//!
//! Three passes, each only looking at what the one before left unmatched:
//!
//! 1. Same parent, same name.
//! 2. Same parent, different name: a rename, found by similar content.
//! 3. Different parent or file: a move, found by equal or similar content.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use mapdiff_core::{Scope, ScopeKind};
use xxhash_rust::xxh3::xxh3_64;

/// Every scope of every changed file on one side of the comparison.
#[derive(Default)]
pub struct Side {
    pub scopes: Vec<Scope>,
    pub parent: Vec<Option<usize>>,
    pub children: Vec<Vec<usize>>,
    /// Which entry of `sources` each scope's bytes are in.
    pub source_of: Vec<usize>,
    pub sources: Vec<Vec<u8>>,
}

impl Side {
    /// Adds one file's scopes (file scope first, as the extractor returns
    /// them) and returns the position of its file scope.
    pub fn add_file(&mut self, scopes: Vec<Scope>, source: Vec<u8>) -> usize {
        let offset = self.scopes.len();
        let source_index = self.sources.len();
        self.sources.push(source);

        let positions: HashMap<&str, usize> = scopes
            .iter()
            .enumerate()
            .map(|(position, scope)| (scope.id.as_str(), offset + position))
            .collect();
        for scope in &scopes {
            self.parent.push(
                scope
                    .parent
                    .as_deref()
                    .and_then(|id| positions.get(id).copied()),
            );
            self.children.push(
                scope
                    .children
                    .iter()
                    .filter_map(|id| positions.get(id.as_str()).copied())
                    .collect(),
            );
            self.source_of.push(source_index);
        }
        drop(positions);
        self.scopes.extend(scopes);
        offset
    }

    pub fn text(&self, scope: usize) -> &[u8] {
        let range = &self.scopes[scope].range;
        let source = &self.sources[self.source_of[scope]];
        source
            .get(range.start_byte as usize..range.end_byte as usize)
            .unwrap_or(&[])
    }

    fn segment(&self, scope: usize) -> &str {
        let scope = &self.scopes[scope];
        scope
            .path
            .segments
            .last()
            .map(String::as_str)
            .unwrap_or(scope.path.file.as_str())
    }
}

/// Renames need this much shared content (0 to 1).
const RENAME_THRESHOLD: f64 = 0.6;
/// Moves have no shared parent to vouch for them, so they need more.
const MOVE_THRESHOLD: f64 = 0.7;
/// Numbered siblings (imports, blocks, overloads) pair up at this level.
const SIBLING_THRESHOLD: f64 = 0.5;
/// Scopes with fewer tokens are too small to recognise by content alone.
const MIN_RENAME_TOKENS: usize = 8;
const MIN_MOVE_TOKENS: usize = 12;
/// Above this many candidate pairs the similarity pass for moves is skipped;
/// identical moved code is still found.
const MAX_MOVE_PAIRS: usize = 250_000;

pub struct Matcher<'a> {
    pub old: &'a Side,
    pub new: &'a Side,
    pub old_to_new: Vec<Option<usize>>,
    pub new_to_old: Vec<Option<usize>>,
    old_bags: RefCell<HashMap<usize, Vec<u64>>>,
    new_bags: RefCell<HashMap<usize, Vec<u64>>>,
}

impl<'a> Matcher<'a> {
    pub fn new(old: &'a Side, new: &'a Side) -> Self {
        Self {
            old,
            new,
            old_to_new: vec![None; old.scopes.len()],
            new_to_old: vec![None; new.scopes.len()],
            old_bags: RefCell::default(),
            new_bags: RefCell::default(),
        }
    }

    /// Declares two scopes to be the same one, then matches what is inside.
    pub fn pair(&mut self, old: usize, new: usize) {
        self.old_to_new[old] = Some(new);
        self.new_to_old[new] = Some(old);
        self.match_children(old, new);
    }

    /// True when the pair sits under parents that are themselves a pair.
    pub fn same_place(&self, old: usize, new: usize) -> bool {
        match (self.old.parent[old], self.new.parent[new]) {
            (Some(old_parent), Some(new_parent)) => self.old_to_new[old_parent] == Some(new_parent),
            (None, None) => true,
            _ => false,
        }
    }

    fn match_children(&mut self, old_parent: usize, new_parent: usize) {
        // Group siblings by the name they would have without numbering.
        let mut groups: BTreeMap<String, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
        for &child in &self.old.children[old_parent] {
            if self.old_to_new[child].is_none() {
                groups
                    .entry(group_key(self.old, child))
                    .or_default()
                    .0
                    .push(child);
            }
        }
        for &child in &self.new.children[new_parent] {
            if self.new_to_old[child].is_none() {
                groups
                    .entry(group_key(self.new, child))
                    .or_default()
                    .1
                    .push(child);
            }
        }

        let mut pairs = Vec::new();
        for (key, (olds, news)) in &groups {
            let numbered = key.starts_with('{');
            if !numbered && olds.len() == 1 && news.len() == 1 {
                pairs.push((olds[0], news[0]));
                continue;
            }
            // Several scopes share this name (or are imports or blocks):
            // pair identical ones first, then the most alike.
            let mut free_old: Vec<usize> = olds.clone();
            let mut free_new: Vec<usize> = news.clone();
            for hash in [Hash::Tokens, Hash::Code] {
                self.pair_equal(&mut free_old, &mut free_new, hash, &mut pairs);
            }
            self.pair_similar(
                &mut free_old,
                &mut free_new,
                SIBLING_THRESHOLD,
                0,
                &mut pairs,
            );
            if !numbered {
                // Same name, nothing else to go on: take them in order.
                for (old, new) in free_old.iter().zip(&free_new) {
                    pairs.push((*old, *new));
                }
            }
        }

        // Pass 2: what is left under this parent may have been renamed.
        let taken_old: Vec<usize> = pairs.iter().map(|pair| pair.0).collect();
        let taken_new: Vec<usize> = pairs.iter().map(|pair| pair.1).collect();
        let renameable = |side: &Side, scope: usize| !group_key(side, scope).starts_with('{');
        let mut free_old: Vec<usize> = self.old.children[old_parent]
            .iter()
            .copied()
            .filter(|child| self.old_to_new[*child].is_none() && !taken_old.contains(child))
            .filter(|child| renameable(self.old, *child))
            .collect();
        let mut free_new: Vec<usize> = self.new.children[new_parent]
            .iter()
            .copied()
            .filter(|child| self.new_to_old[*child].is_none() && !taken_new.contains(child))
            .filter(|child| renameable(self.new, *child))
            .collect();
        self.pair_similar(
            &mut free_old,
            &mut free_new,
            RENAME_THRESHOLD,
            MIN_RENAME_TOKENS,
            &mut pairs,
        );

        for (old, new) in pairs {
            self.pair(old, new);
        }
    }

    /// Pairs scopes whose chosen hash is equal, in source order.
    fn pair_equal(
        &self,
        free_old: &mut Vec<usize>,
        free_new: &mut Vec<usize>,
        hash: Hash,
        pairs: &mut Vec<(usize, usize)>,
    ) {
        let key = |side: &Side, scope: usize| match hash {
            Hash::Tokens => side.scopes[scope].hashes.tokens.clone(),
            Hash::Code => side.scopes[scope].hashes.code.clone(),
        };
        free_old.retain(|&old| {
            let wanted = key(self.old, old);
            match free_new
                .iter()
                .position(|&new| key(self.new, new) == wanted)
            {
                Some(position) => {
                    pairs.push((old, free_new.remove(position)));
                    false
                }
                None => true,
            }
        });
    }

    /// Pairs the most alike scopes first, down to `threshold`. Ties go to
    /// the earlier scope, so the result never depends on hash order.
    fn pair_similar(
        &self,
        free_old: &mut Vec<usize>,
        free_new: &mut Vec<usize>,
        threshold: f64,
        min_tokens: usize,
        pairs: &mut Vec<(usize, usize)>,
    ) {
        if free_old.is_empty() || free_new.is_empty() {
            return;
        }
        let mut scored = Vec::new();
        for &old in free_old.iter() {
            for &new in free_new.iter() {
                if family(self.old.scopes[old].kind) != family(self.new.scopes[new].kind) {
                    continue;
                }
                let score = self.similarity(old, new, min_tokens);
                if score >= threshold {
                    scored.push((score, old, new));
                }
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, old, new) in scored {
            if free_old.contains(&old) && free_new.contains(&new) {
                free_old.retain(|candidate| *candidate != old);
                free_new.retain(|candidate| *candidate != new);
                pairs.push((old, new));
            }
        }
    }

    /// How much of the two scopes' content is shared, from 0 to 1. Zero when
    /// either is smaller than `min_tokens`.
    fn similarity(&self, old: usize, new: usize, min_tokens: usize) -> f64 {
        let mut old_bags = self.old_bags.borrow_mut();
        let mut new_bags = self.new_bags.borrow_mut();
        let a = old_bags
            .entry(old)
            .or_insert_with(|| bag(self.old.text(old)));
        let b = new_bags
            .entry(new)
            .or_insert_with(|| bag(self.new.text(new)));
        if a.len() < min_tokens || b.len() < min_tokens || a.is_empty() || b.is_empty() {
            return 0.0;
        }
        let (mut i, mut j, mut shared) = (0, 0, 0usize);
        while i < a.len() && j < b.len() {
            match a[i].cmp(&b[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    shared += 1;
                    i += 1;
                    j += 1;
                }
            }
        }
        2.0 * shared as f64 / (a.len() + b.len()) as f64
    }

    fn token_count(&self, old_side: bool, scope: usize) -> usize {
        if old_side {
            let mut bags = self.old_bags.borrow_mut();
            bags.entry(scope)
                .or_insert_with(|| bag(self.old.text(scope)))
                .len()
        } else {
            let mut bags = self.new_bags.borrow_mut();
            bags.entry(scope)
                .or_insert_with(|| bag(self.new.text(scope)))
                .len()
        }
    }

    /// Pass 3: scopes still unmatched may have moved to another parent or
    /// file. Larger scopes go first, so a moved class takes its methods
    /// with it instead of each method being matched alone.
    pub fn match_moves(&mut self) {
        let movable = |kind: ScopeKind| {
            !matches!(
                kind,
                ScopeKind::Directory
                    | ScopeKind::File
                    | ScopeKind::Document
                    | ScopeKind::Import
                    | ScopeKind::Block
                    | ScopeKind::Field
                    | ScopeKind::Variant
            )
        };
        let candidates = |matcher: &Self, old_side: bool| -> Vec<usize> {
            let (side, matched) = if old_side {
                (matcher.old, &matcher.old_to_new)
            } else {
                (matcher.new, &matcher.new_to_old)
            };
            let mut list: Vec<usize> = (0..side.scopes.len())
                .filter(|scope| matched[*scope].is_none() && movable(side.scopes[*scope].kind))
                .filter(|scope| matcher.token_count(old_side, *scope) >= MIN_MOVE_TOKENS)
                .collect();
            // Largest first; position breaks ties.
            list.sort_by_key(|scope| (std::cmp::Reverse(side.scopes[*scope].range.len()), *scope));
            list
        };

        // Identical code first.
        let olds = candidates(self, true);
        let mut by_code: HashMap<String, Vec<usize>> = HashMap::new();
        for new in candidates(self, false) {
            by_code
                .entry(self.new.scopes[new].hashes.code.clone())
                .or_default()
                .push(new);
        }
        for old in olds {
            if self.old_to_new[old].is_some() {
                continue;
            }
            let Some(news) = by_code.get(&self.old.scopes[old].hashes.code) else {
                continue;
            };
            let free: Vec<usize> = news
                .iter()
                .copied()
                .filter(|new| self.new_to_old[*new].is_none())
                .filter(|new| {
                    family(self.new.scopes[*new].kind) == family(self.old.scopes[old].kind)
                })
                .collect();
            // Prefer the one that kept its name.
            let same_name = free
                .iter()
                .copied()
                .find(|new| self.new.scopes[*new].name == self.old.scopes[old].name);
            if let Some(new) = same_name.or(free.first().copied()) {
                self.pair(old, new);
            }
        }

        // Then similar code.
        let olds = candidates(self, true);
        let news = candidates(self, false);
        if olds.is_empty() || news.is_empty() || olds.len() * news.len() > MAX_MOVE_PAIRS {
            return;
        }
        let mut scored = Vec::new();
        for &old in &olds {
            for &new in &news {
                let (a, b) = (&self.old.scopes[old], &self.new.scopes[new]);
                if family(a.kind) != family(b.kind) {
                    continue;
                }
                let (short, long) = (
                    a.range.len().min(b.range.len()),
                    a.range.len().max(b.range.len()),
                );
                if long > short.saturating_mul(2) {
                    continue;
                }
                let mut score = self.similarity(old, new, MIN_MOVE_TOKENS);
                if score >= MOVE_THRESHOLD {
                    if a.name == b.name {
                        score += 0.1;
                    }
                    scored.push((score, std::cmp::Reverse(long), old, new));
                }
            }
        }
        scored.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        for (_, _, old, new) in scored {
            if self.old_to_new[old].is_none() && self.new_to_old[new].is_none() {
                self.pair(old, new);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Hash {
    Tokens,
    Code,
}

/// The segment without its numbering: `format()#2` -> `format()`. Imports
/// and blocks have no name at all, so each kind forms one group.
fn group_key(side: &Side, scope: usize) -> String {
    match side.scopes[scope].kind {
        ScopeKind::Import => String::from("{import}"),
        ScopeKind::Block => String::from("{block}"),
        _ => base_segment(side.segment(scope)).to_owned(),
    }
}

pub fn base_segment(segment: &str) -> &str {
    match segment.rsplit_once('#') {
        Some((base, number))
            if !base.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
                && !number.is_empty() =>
        {
            base
        }
        _ => segment,
    }
}

/// Kinds that a scope can change between without becoming a different thing.
fn family(kind: ScopeKind) -> u8 {
    match kind {
        ScopeKind::Function | ScopeKind::Method | ScopeKind::Constructor => 0,
        ScopeKind::Class | ScopeKind::Interface | ScopeKind::Enum => 1,
        ScopeKind::Field | ScopeKind::Variant | ScopeKind::Constant => 2,
        ScopeKind::Module | ScopeKind::Impl => 3,
        ScopeKind::TypeAlias => 4,
        ScopeKind::Macro => 5,
        ScopeKind::Import => 6,
        ScopeKind::Block => 7,
        ScopeKind::Directory | ScopeKind::File | ScopeKind::Document => 8,
    }
}

/// The scope's text as a sorted list of token hashes. Tokens are words and
/// single punctuation characters; whitespace is dropped.
fn bag(text: &[u8]) -> Vec<u64> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (position, byte) in text.iter().enumerate() {
        let word = byte.is_ascii_alphanumeric() || *byte == b'_' || *byte >= 0x80;
        match (word, start) {
            (true, None) => start = Some(position),
            (true, Some(_)) => {}
            (false, word_start) => {
                if let Some(word_start) = word_start {
                    tokens.push(xxh3_64(&text[word_start..position]));
                    start = None;
                }
                if !byte.is_ascii_whitespace() {
                    tokens.push(xxh3_64(&[*byte]));
                }
            }
        }
    }
    if let Some(word_start) = start {
        tokens.push(xxh3_64(&text[word_start..]));
    }
    tokens.sort_unstable();
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbering_is_stripped_only_when_it_is_numbering() {
        assert_eq!(base_segment("format()#2"), "format()");
        assert_eq!(base_segment("impl Circle#12"), "impl Circle");
        assert_eq!(base_segment("#id"), "#id");
        assert_eq!(base_segment("format()"), "format()");
        assert_eq!(base_segment("a#b"), "a#b");
    }

    #[test]
    fn bags_ignore_layout() {
        assert_eq!(
            bag(b"fn add(a, b) { a + b }"),
            bag(b"fn add(a,b){\n    a+b\n}")
        );
        assert_ne!(bag(b"fn add(a, b)"), bag(b"fn sub(a, b)"));
    }
}
