//! Reading the server's XML answers. A document is read into a small element tree first — elements by local name,
//! so the S3 namespace is optional; a DTD, an entity other than the five predefined ones, nesting deeper than an S3
//! answer goes, or a body over the size cap is refused — and the typed readers in `parse` then read the tree.

mod parse;

pub(crate) use parse::{copy_result, error_document, list_buckets, list_objects_v2};

use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;

use crate::Error;
use crate::constants::{XML_BODY_MAX_BYTES, XML_MAX_DEPTH};

/// One element: its local name, its text and its children.
#[derive(Debug, Default)]
pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) children: Vec<Node>,
}

impl Node {
    /// The first child called `name`.
    pub(crate) fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Every child called `name`, in document order.
    pub(crate) fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// The trimmed text of the first child called `name`.
    pub(crate) fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|child| child.text.trim())
    }
}

pub(crate) const fn malformed(what: &'static str) -> Error {
    Error::InvalidResponse { what }
}

/// The element tree of `document`.
pub(crate) fn tree(document: &[u8]) -> Result<Node, Error> {
    if document.len() > XML_BODY_MAX_BYTES {
        return Err(malformed("an XML document over 8 MiB"));
    }
    let bad = || malformed("an XML document that is not well formed");
    let mut reader = Reader::from_reader(document);
    let mut open: Vec<Node> = Vec::new();
    loop {
        match reader.read_event().map_err(|_| bad())? {
            Event::DocType(_) => return Err(malformed("an XML document declaring a DTD")),
            Event::Start(start) => {
                if open.len() >= XML_MAX_DEPTH {
                    return Err(malformed("an XML document nested too deeply"));
                }
                open.push(element(start.local_name().as_ref()));
            }
            Event::Empty(empty) => {
                let node = element(empty.local_name().as_ref());
                match open.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            Event::Text(content) => match open.last_mut() {
                Some(node) => node.text.push_str(&content.xml10_content()),
                None if content.xml10_content().trim().is_empty() => {}
                None => return Err(bad()),
            },
            Event::CData(content) => open
                .last_mut()
                .ok_or_else(bad)?
                .text
                .push_str(&content.xml10_content()),
            Event::GeneralRef(reference) => {
                let resolved = match reference.resolve_char_ref().map_err(|_| bad())? {
                    Some(character) => character,
                    None => resolve_predefined_entity(&reference.xml10_content())
                        .and_then(|entity| entity.chars().next())
                        .ok_or_else(|| malformed("an XML document using an undeclared entity"))?,
                };
                open.last_mut().ok_or_else(bad)?.text.push(resolved);
            }
            Event::End(_) => {
                let node = open.pop().ok_or_else(bad)?;
                match open.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            Event::Eof => return Err(bad()),
            _ => {}
        }
    }
}

fn element(name: &str) -> Node {
    Node {
        name: name.to_owned(),
        ..Node::default()
    }
}

#[cfg(test)]
mod tests;
