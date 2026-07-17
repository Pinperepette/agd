//! Operation-based editing for AGD documents. Operations are pure data
//! → safe to serialize, log, replay across agents.
//!
//! Operations target blocks by their stable `[#id]`. A document with no
//! IDs at all is still parseable but not editable through this API; use
//! `agd id --add` to auto-assign IDs first.

use serde::{Deserialize, Serialize};

use crate::ast::{AttrValue, Block, BlockContent, BlockKind, Document, Inline};
use crate::error::{AgdError, Result};
use crate::parser::is_valid_ident;

// The parser enforces the grammar on READ, but blocks arriving through this
// API skip the parser: without the checks below, an invalid id/tag/attr is
// serialized as-is with exit 0 and every subsequent parse of the file fails.
// Newlines are rejected wherever the line-oriented syntax cannot represent
// them (attr values, inline runs, list items); fenced bodies are exempt —
// the serializer sizes the fence around them.
fn invalid(message: String) -> AgdError {
    AgdError::InvalidEdit { message }
}

fn validate_attr(key: &str, value: &AttrValue) -> Result<()> {
    if !is_valid_ident(key) {
        return Err(invalid(format!(
            "attribute key `{key}`: must match [a-zA-Z_][a-zA-Z0-9_-]*"
        )));
    }
    if let AttrValue::Str(s) = value {
        if s.contains('\n') {
            return Err(invalid(format!(
                "attribute `{key}` value contains a newline, which the \
                 line-oriented attribute syntax cannot represent"
            )));
        }
    }
    Ok(())
}

fn validate_inlines(what: &str, inlines: &[Inline]) -> Result<()> {
    let text = |i: &Inline| match i {
        Inline::Text(s) | Inline::Bold(s) | Inline::Italic(s)
        | Inline::Code(s) | Inline::Ref(s) => s.contains('\n'),
    };
    if inlines.iter().any(text) {
        return Err(invalid(format!("{what} contains a newline")));
    }
    Ok(())
}

fn validate_new_block(block: &Block) -> Result<()> {
    let tag = block.kind.as_str();
    let custom_ok = tag.strip_prefix("x-").is_some_and(is_valid_ident);
    if !BlockKind::BUILTINS.contains(&tag) && !custom_ok {
        return Err(invalid(format!(
            "block tag `{tag}`: must be a builtin or `x-` plus an identifier"
        )));
    }
    if let Some(id) = block.id.as_deref() {
        if !is_valid_ident(id) {
            return Err(invalid(format!(
                "id `{id}`: must match [a-zA-Z_][a-zA-Z0-9_-]*"
            )));
        }
    }
    for (key, value) in &block.attrs {
        validate_attr(key, value)?;
    }
    match &block.content {
        BlockContent::Inline(inlines) => validate_inlines("inline content", inlines)?,
        BlockContent::Items(items) => {
            for item in items {
                validate_inlines("list item", item)?;
            }
        }
        BlockContent::Fenced(_) | BlockContent::Empty => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    Replace { id: String, with: Block },
    InsertAfter { id: String, block: Block },
    InsertBefore { id: String, block: Block },
    Delete { id: String },
    SetAttr { id: String, key: String, value: AttrValue },
    RemoveAttr { id: String, key: String },
}

impl Document {
    pub fn apply(&mut self, op: Operation) -> Result<()> {
        match op {
            Operation::Replace { id, mut with } => {
                validate_new_block(&with)?;
                let pos = self.position(&id).ok_or(AgdError::IdNotFound { id: id.clone() })?;
                if with.id.is_none() {
                    with.id = Some(id.clone());
                }
                if with.id.as_deref() != Some(id.as_str()) {
                    self.assert_unique_id(with.id.as_deref(), Some(pos))?;
                }
                self.blocks[pos] = with;
                self.invalidate_index();  // id may have changed
                Ok(())
            }
            Operation::InsertAfter { id, block } => {
                validate_new_block(&block)?;
                let pos = self.position(&id).ok_or(AgdError::IdNotFound { id })?;
                self.assert_unique_id(block.id.as_deref(), None)?;
                self.blocks.insert(pos + 1, block);
                self.invalidate_index();
                Ok(())
            }
            Operation::InsertBefore { id, block } => {
                validate_new_block(&block)?;
                let pos = self.position(&id).ok_or(AgdError::IdNotFound { id })?;
                self.assert_unique_id(block.id.as_deref(), None)?;
                self.blocks.insert(pos, block);
                self.invalidate_index();
                Ok(())
            }
            Operation::Delete { id } => {
                let pos = self.position(&id).ok_or(AgdError::IdNotFound { id })?;
                self.blocks.remove(pos);
                self.invalidate_index();
                Ok(())
            }
            Operation::SetAttr { id, key, value } => {
                validate_attr(&key, &value)?;
                // SetAttr / RemoveAttr don't change ids → cache stays valid.
                let block = self.find_mut(&id).ok_or(AgdError::IdNotFound { id })?;
                block.attrs.insert(key, value);
                Ok(())
            }
            Operation::RemoveAttr { id, key } => {
                let block = self.find_mut(&id).ok_or(AgdError::IdNotFound { id })?;
                block.attrs.remove(&key);
                Ok(())
            }
        }
    }

    pub fn apply_all(&mut self, ops: impl IntoIterator<Item = Operation>) -> Result<()> {
        for op in ops {
            self.apply(op)?;
        }
        Ok(())
    }

    fn assert_unique_id(&self, id: Option<&str>, ignore_pos: Option<usize>) -> Result<()> {
        let Some(id) = id else { return Ok(()); };
        for (i, b) in self.blocks.iter().enumerate() {
            if Some(i) == ignore_pos { continue; }
            if b.id.as_deref() == Some(id) {
                return Err(AgdError::DuplicateId {
                    id: id.to_string(),
                    first_line: 0,
                    dup_line: 0,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Block, BlockContent, Inline};
    use crate::parser::parse;
    use crate::serializer::serialize;

    fn fixture() -> Document {
        parse("@h1 A [#a]\n@p first [#p1]\n@p second [#p2]\n").unwrap()
    }

    #[test]
    fn replace_block() {
        let mut doc = fixture();
        let new = Block::new("p", BlockContent::Inline(vec![Inline::Text("changed".into())]))
            .with_id("p1");
        doc.apply(Operation::Replace { id: "p1".into(), with: new }).unwrap();
        let out = serialize(&doc);
        assert!(out.contains("changed"));
        assert!(!out.contains("first"));
    }

    #[test]
    fn insert_after() {
        let mut doc = fixture();
        let new = Block::new("p", BlockContent::Inline(vec![Inline::Text("inserted".into())]))
            .with_id("ins");
        doc.apply(Operation::InsertAfter { id: "p1".into(), block: new }).unwrap();
        let ids: Vec<_> = doc.ids();
        assert_eq!(ids, vec!["a", "p1", "ins", "p2"]);
    }

    #[test]
    fn delete_block() {
        let mut doc = fixture();
        doc.apply(Operation::Delete { id: "p1".into() }).unwrap();
        assert!(doc.find("p1").is_none());
        assert_eq!(doc.blocks.len(), 2);
    }

    #[test]
    fn set_attr() {
        let mut doc = parse("@code lang=python [#c]\n~~~\nx=1\n~~~\n").unwrap();
        doc.apply(Operation::SetAttr {
            id: "c".into(),
            key: "lang".into(),
            value: AttrValue::Str("rust".into()),
        }).unwrap();
        assert_eq!(doc.find("c").unwrap().attrs.get("lang"),
                   Some(&AttrValue::Str("rust".into())));
    }

    #[test]
    fn missing_id_errors() {
        let mut doc = fixture();
        let r = doc.apply(Operation::Delete { id: "nope".into() });
        assert!(matches!(r, Err(AgdError::IdNotFound { .. })));
    }

    // Every case below used to be WRITTEN with exit 0 and then rejected by
    // the parser at the next read — one bad edit made the file unreadable.
    #[test]
    fn invalid_id_rejected_on_write() {
        let mut doc = fixture();
        let bad = Block::new("p", BlockContent::Inline(vec![Inline::Text("x".into())]))
            .with_id("bad.id-1.9.0");
        let r = doc.apply(Operation::InsertAfter { id: "p1".into(), block: bad });
        assert!(matches!(r, Err(AgdError::InvalidEdit { .. })));
        // the document must be untouched and still round-trip
        assert_eq!(doc.blocks.len(), 3);
        parse(&serialize(&doc)).unwrap();
    }

    #[test]
    fn invalid_tag_rejected_on_write() {
        let mut doc = fixture();
        for tag in ["x bad", "x-bad stuff", "notbuiltin"] {
            let bad = Block::new(tag, BlockContent::Empty).with_id("t");
            let r = doc.apply(Operation::InsertAfter { id: "p1".into(), block: bad });
            assert!(matches!(r, Err(AgdError::InvalidEdit { .. })), "tag `{tag}` accepted");
        }
    }

    #[test]
    fn newline_in_attr_value_rejected() {
        let mut doc = fixture();
        let r = doc.apply(Operation::SetAttr {
            id: "p1".into(),
            key: "desc".into(),
            value: AttrValue::Str("line1\nline2".into()),
        });
        assert!(matches!(r, Err(AgdError::InvalidEdit { .. })));
        // and through a new block's attrs too
        let mut bad = Block::new("x-note", BlockContent::Empty).with_id("n");
        bad.attrs.insert("desc".into(), AttrValue::Str("a\nb".into()));
        let r = doc.apply(Operation::InsertAfter { id: "p1".into(), block: bad });
        assert!(matches!(r, Err(AgdError::InvalidEdit { .. })));
    }

    #[test]
    fn invalid_attr_key_rejected() {
        let mut doc = fixture();
        let r = doc.apply(Operation::SetAttr {
            id: "p1".into(),
            key: "bad key".into(),
            value: AttrValue::Str("v".into()),
        });
        assert!(matches!(r, Err(AgdError::InvalidEdit { .. })));
    }

    #[test]
    fn newline_in_inline_content_rejected() {
        let mut doc = fixture();
        let bad = Block::new("p", BlockContent::Inline(vec![Inline::Text("a\nb".into())]))
            .with_id("nl");
        let r = doc.apply(Operation::InsertAfter { id: "p1".into(), block: bad });
        assert!(matches!(r, Err(AgdError::InvalidEdit { .. })));
    }

    #[test]
    fn fenced_body_with_newlines_still_fine() {
        let mut doc = fixture();
        let ok = Block::new("x-note", BlockContent::Fenced("a\nb\n~~~ inside\n".into()))
            .with_id("f");
        doc.apply(Operation::InsertAfter { id: "p1".into(), block: ok }).unwrap();
        parse(&serialize(&doc)).unwrap();
    }

    #[test]
    fn ops_serialize_as_json() {
        let op = Operation::Delete { id: "x".into() };
        let s = serde_json::to_string(&op).unwrap();
        assert!(s.contains("\"op\":\"delete\""));
        let back: Operation = serde_json::from_str(&s).unwrap();
        assert_eq!(op, back);
    }
}
