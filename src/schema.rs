//! The XML Schema subset this contract binds, and how a schema document is read
//! into it.
//!
//! Supported: top-level `xs:element` and named `xs:complexType`; an element's
//! `type` naming a built-in simple type or a named complex type, or an inline
//! `xs:complexType`; `xs:sequence`, `xs:all` and `xs:choice` holding elements;
//! `minOccurs` and `maxOccurs` (`unbounded`); `xs:attribute` with `use` and a
//! built-in type; `xs:simpleContent` over `xs:extension`; a named or inline
//! `xs:simpleType` restricting a built-in, taken as its base. An element with no
//! type at all is `anyType` and admits anything.
//!
//! Refused when bound, by name: `ref`, `group`, `complexContent`, nested
//! compositors, `import` and `include`. A refused schema is a configuration
//! error, which is where an operator wants to hear about it.

use roxmltree::Node;
use sdk::contract::ContractError;
use std::collections::BTreeMap;

pub const XS: &str = "http://www.w3.org/2001/XMLSchema";

/// One bound schema.
pub struct Schema {
    target_namespace: Option<String>,
    pub(crate) elements: BTreeMap<String, Element>,
    pub(crate) types: BTreeMap<String, ComplexType>,
    simple: BTreeMap<String, String>,
}

/// An element declaration.
pub struct Element {
    pub name: String,
    pub kind: Kind,
    pub min: u32,
    /// `None` is unbounded.
    pub max: Option<u32>,
}

/// What an element holds.
pub enum Kind {
    Any,
    Builtin(String),
    Named(String),
    Inline(Box<ComplexType>),
}

pub struct ComplexType {
    pub attributes: Vec<Attribute>,
    pub content: Content,
}

pub struct Attribute {
    pub name: String,
    pub required: bool,
    pub builtin: String,
}

pub enum Content {
    Empty,
    Simple(String),
    Sequence(Vec<Element>),
    All(Vec<Element>),
    Choice(Vec<Element>),
}

impl Schema {
    /// Read a schema document.
    ///
    /// # Errors
    /// Not well-formed, not rooted at `xs:schema`, or outside the subset.
    pub fn parse(text: &str) -> Result<Self, ContractError> {
        let document = roxmltree::Document::parse(text).map_err(refuse)?;
        let root = document.root_element();
        if !is_xs(root, "schema") {
            return Err(refuse("the document root is not xs:schema"));
        }
        let mut schema = Self {
            target_namespace: root.attribute("targetNamespace").map(str::to_string),
            elements: BTreeMap::new(),
            types: BTreeMap::new(),
            simple: BTreeMap::new(),
        };
        // Simple types first, so an element declared before its type resolves.
        for node in root.children().filter(|n| is_xs(*n, "simpleType")) {
            let name = required(node, "name")?;
            schema.simple.insert(name, simple_base(node));
        }
        for node in root.children().filter(Node::is_element) {
            match node.tag_name().name() {
                "element" => {
                    let element = schema.element(node)?;
                    schema.elements.insert(element.name.clone(), element);
                }
                "complexType" => {
                    let name = required(node, "name")?;
                    let complex = schema.complex(node)?;
                    schema.types.insert(name, complex);
                }
                "simpleType" | "annotation" => {}
                other => return Err(refuse(format!("xs:{other} is not supported"))),
            }
        }
        Ok(schema)
    }

    #[must_use]
    pub fn target_namespace(&self) -> Option<&str> {
        self.target_namespace.as_deref()
    }

    fn element(&self, node: Node) -> Result<Element, ContractError> {
        if node.has_attribute("ref") {
            return Err(refuse("xs:element ref is not supported"));
        }
        let name = required(node, "name")?;
        let kind = match node.attribute("type") {
            Some(type_name) => self.kind_of(type_name),
            None => {
                if let Some(inline) = node.children().find(|n| is_xs(*n, "complexType")) {
                    Kind::Inline(Box::new(self.complex(inline)?))
                } else if let Some(inline) = node.children().find(|n| is_xs(*n, "simpleType")) {
                    Kind::Builtin(simple_base(inline))
                } else {
                    Kind::Any
                }
            }
        };
        Ok(Element {
            name,
            kind,
            min: occurs(node, "minOccurs").unwrap_or(Some(1)).unwrap_or(1),
            max: occurs(node, "maxOccurs").unwrap_or(Some(1)),
        })
    }

    fn kind_of(&self, type_name: &str) -> Kind {
        let local = local_name(type_name);
        if let Some(base) = self.simple.get(local) {
            Kind::Builtin(base.clone())
        } else if is_builtin(local) {
            Kind::Builtin(local.to_string())
        } else {
            Kind::Named(local.to_string())
        }
    }

    fn complex(&self, node: Node) -> Result<ComplexType, ContractError> {
        let mut attributes = self.attributes(node);
        let mut content = Content::Empty;
        for child in node.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "sequence" => content = Content::Sequence(self.particles(child)?),
                "all" => content = Content::All(self.particles(child)?),
                "choice" => content = Content::Choice(self.particles(child)?),
                "simpleContent" => {
                    let extension = child
                        .children()
                        .find(|n| is_xs(*n, "extension") || is_xs(*n, "restriction"))
                        .ok_or_else(|| refuse("xs:simpleContent without extension"))?;
                    let base = extension.attribute("base").unwrap_or("string");
                    content = Content::Simple(self.builtin_of(base));
                    attributes.extend(self.attributes(extension));
                }
                "attribute" | "annotation" => {}
                other => return Err(refuse(format!("xs:{other} is not supported"))),
            }
        }
        Ok(ComplexType {
            attributes,
            content,
        })
    }

    fn particles(&self, compositor: Node) -> Result<Vec<Element>, ContractError> {
        let mut particles = Vec::new();
        for child in compositor.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "element" => particles.push(self.element(child)?),
                "annotation" => {}
                other => {
                    return Err(refuse(format!(
                        "xs:{other} inside a compositor is not supported"
                    )));
                }
            }
        }
        Ok(particles)
    }

    fn attributes(&self, node: Node) -> Vec<Attribute> {
        node.children()
            .filter(|n| is_xs(*n, "attribute"))
            .filter_map(|n| {
                Some(Attribute {
                    name: n.attribute("name")?.to_string(),
                    required: n.attribute("use") == Some("required"),
                    builtin: self.builtin_of(n.attribute("type").unwrap_or("string")),
                })
            })
            .collect()
    }

    fn builtin_of(&self, type_name: &str) -> String {
        let local = local_name(type_name);
        self.simple
            .get(local)
            .cloned()
            .unwrap_or_else(|| local.to_string())
    }
}

fn is_xs(node: Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(XS)
}

fn required(node: Node, attribute: &str) -> Result<String, ContractError> {
    node.attribute(attribute)
        .map(str::to_string)
        .ok_or_else(|| refuse(format!("xs:{} without {attribute}", node.tag_name().name())))
}

/// `Ok(None)` is unbounded; `Err` when the attribute is present but unreadable.
fn occurs(node: Node, attribute: &str) -> Result<Option<u32>, ()> {
    match node.attribute(attribute) {
        None => Ok(Some(1)),
        Some("unbounded") => Ok(None),
        Some(text) => text.parse().map(Some).map_err(|_| ()),
    }
}

fn simple_base(node: Node) -> String {
    node.children()
        .find(|n| is_xs(*n, "restriction"))
        .and_then(|r| r.attribute("base"))
        .map_or_else(|| "string".to_string(), |b| local_name(b).to_string())
}

pub(crate) fn local_name(qualified: &str) -> &str {
    qualified.rsplit(':').next().unwrap_or(qualified)
}

fn is_builtin(local: &str) -> bool {
    matches!(
        local,
        "string"
            | "normalizedString"
            | "token"
            | "anyURI"
            | "anyType"
            | "anySimpleType"
            | "QName"
            | "NMTOKEN"
            | "Name"
            | "NCName"
            | "ID"
            | "IDREF"
            | "language"
            | "boolean"
            | "decimal"
            | "double"
            | "float"
            | "integer"
            | "int"
            | "long"
            | "short"
            | "byte"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
            | "positiveInteger"
            | "nonNegativeInteger"
            | "negativeInteger"
            | "nonPositiveInteger"
            | "date"
            | "dateTime"
            | "time"
            | "base64Binary"
            | "hexBinary"
            | "duration"
            | "gYear"
            | "gYearMonth"
            | "gMonth"
            | "gDay"
            | "gMonthDay"
    )
}

fn refuse(reason: impl std::fmt::Display) -> ContractError {
    ContractError {
        message: format!("schema refused: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schema_outside_the_subset_is_refused_by_name() {
        let text = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="a"><xs:complexType><xs:sequence>
    <xs:group ref="g"/>
  </xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;
        let error = Schema::parse(text).err().expect("refused");
        assert!(error.message.contains("xs:group"), "{}", error.message);
    }

    #[test]
    fn a_named_simple_type_resolves_to_its_base() {
        let text = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="n" type="Count"/>
  <xs:simpleType name="Count"><xs:restriction base="xs:positiveInteger"/></xs:simpleType>
</xs:schema>"#;
        let schema = Schema::parse(text).expect("parses");
        match &schema.elements["n"].kind {
            Kind::Builtin(base) => assert_eq!(base, "positiveInteger"),
            _ => panic!("expected a built-in"),
        }
    }

    #[test]
    fn a_document_that_is_not_a_schema_is_refused() {
        assert!(Schema::parse("<order/>").is_err());
        assert!(Schema::parse("<broken").is_err());
    }
}
