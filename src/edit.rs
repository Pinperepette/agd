//! Operation-based editing for AGD documents. Operations are pure data
//! → safe to serialize, log, replay across agents.
//!
//! Operations target blocks by their stable `[#id]`. A document with no
//! IDs at all is still parseable but not editable through this API; use
//! `agd id --add` to auto-assign IDs first.

use serde::{Deserialize, Serialize};

use crate::ast::{AttrValue, Block, Document};
use crate::error::{AgdError, Result};

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
                let pos = self.position(&id).ok_or(AgdError::IdNotFound { id })?;
                self.assert_unique_id(block.id.as_deref(), None)?;
                self.blocks.insert(pos + 1, block);
                self.invalidate_index();
                Ok(())
            }
            Operation::InsertBefore { id, block } => {
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

    #[test]
    fn ops_serialize_as_json() {
        let op = Operation::Delete { id: "x".into() };
        let s = serde_json::to_string(&op).unwrap();
        assert!(s.contains("\"op\":\"delete\""));
        let back: Operation = serde_json::from_str(&s).unwrap();
        assert_eq!(op, back);
    }
}
