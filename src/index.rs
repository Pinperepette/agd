//! O(1) random-access index for AGD documents.
//!
//! `Document::find` is a linear scan — fine for small documents but
//! O(n) per lookup. For long-running services holding parsed documents
//! in memory, build a [`DocumentIndex`] once and amortise the cost
//! across many lookups.
//!
//! ```
//! use agd::{parse, index::DocumentIndex};
//!
//! let doc = parse("@h1 A [#a]\n@p body [#b]\n").unwrap();
//! let idx = DocumentIndex::build(&doc);
//! assert_eq!(idx.position("a"), Some(0));
//! assert_eq!(idx.position("b"), Some(1));
//! ```

use std::collections::HashMap;

use crate::ast::{Block, Document};

/// Read-only ID → position lookup table for a document. Build once,
/// query many times. Becomes stale if the underlying document is
/// edited; rebuild after each batch of writes.
#[derive(Debug, Clone)]
pub struct DocumentIndex {
    by_id: HashMap<String, usize>,
}

impl DocumentIndex {
    /// Build an index over a document. O(n) once.
    pub fn build(doc: &Document) -> Self {
        let mut by_id = HashMap::with_capacity(doc.blocks.len());
        for (i, block) in doc.blocks.iter().enumerate() {
            if let Some(id) = &block.id {
                // Documents passed through `parse` already have unique IDs;
                // duplicates here would only arise from hand-built docs.
                // We keep the FIRST occurrence to mirror `Document::find`.
                by_id.entry(id.clone()).or_insert(i);
            }
        }
        Self { by_id }
    }

    /// Return the position of the block with the given ID, or `None`.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    /// Return a reference to the block with the given ID, or `None`.
    pub fn get<'a>(&self, doc: &'a Document, id: &str) -> Option<&'a Block> {
        self.position(id).map(|i| &doc.blocks[i])
    }

    /// Number of indexed IDs.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Iterate over `(id, position)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, usize)> {
        self.by_id.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn empty_index() {
        let doc = parse("@h1 No id\n@p Also no id\n").unwrap();
        let idx = DocumentIndex::build(&doc);
        assert!(idx.is_empty());
    }

    #[test]
    fn round_trip_lookup() {
        let doc = parse("@h1 A [#a]\n@p B [#b]\n@h2 C [#c]\n").unwrap();
        let idx = DocumentIndex::build(&doc);
        assert_eq!(idx.len(), 3);
        assert_eq!(idx.position("a"), Some(0));
        assert_eq!(idx.position("b"), Some(1));
        assert_eq!(idx.position("c"), Some(2));
        assert_eq!(idx.position("missing"), None);
        assert_eq!(idx.get(&doc, "b").unwrap().kind.as_str(), "p");
    }

    #[test]
    fn agrees_with_find() {
        let src = crate::corpus::generate(200, 3);
        let doc = parse(&src).unwrap();
        let idx = DocumentIndex::build(&doc);
        for (id, expected_pos) in idx.iter() {
            assert_eq!(doc.position(id), Some(expected_pos));
        }
    }
}
