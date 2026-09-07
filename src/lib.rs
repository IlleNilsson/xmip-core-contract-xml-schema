#![forbid(unsafe_code)]

//! The XML content contract — a technology of `xmip-core-contract`.
//!
//! Two claims, decided 2026-09-07: **well-formedness is a given** and is always
//! checked, and **conformance is a given once a contract is named** — a Receive
//! or Send Location that refers to this contract with a schema bound has every
//! Stream validated against that schema. An unbound contract is the first claim
//! alone.
//!
//! The schema language is the XML Schema subset that [`schema`] documents:
//! element declarations, `sequence`, `all` and `choice` content, occurrence
//! bounds, attributes and the built-in simple types. Nothing outside it is
//! silently accepted — a schema that uses more is refused when it is bound, so
//! an operator learns at configuration time, not from a Stream that passed.

pub mod check;
pub mod schema;

use contract::{
    Contract, ContractDescriptor, ContractError, ContractFactory, ContractId, ValidationIssue,
    ValidationResult,
};
use schema::Schema;
use stream::Stream;

/// The XML contract, bare or bound to a schema.
pub struct XmlSchema {
    descriptor: ContractDescriptor,
    schema: Option<Schema>,
}

impl XmlSchema {
    /// Well-formedness only.
    #[must_use]
    pub fn new() -> Self {
        Self {
            descriptor: descriptor("xml-schema"),
            schema: None,
        }
    }

    /// Well-formedness and conformance to the XML Schema document `text`.
    ///
    /// # Errors
    /// The schema must be well-formed, rooted at `xs:schema`, and within the
    /// subset [`schema`] documents.
    pub fn with_schema(text: &str) -> Result<Self, ContractError> {
        let schema = Schema::parse(text)?;
        let name = schema.target_namespace().unwrap_or("bound").to_string();
        Ok(Self {
            descriptor: descriptor(&format!("xml-schema:{name}")),
            schema: Some(schema),
        })
    }

    /// Whether a schema is bound.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.schema.is_some()
    }
}

impl Default for XmlSchema {
    fn default() -> Self {
        Self::new()
    }
}

fn descriptor(id: &str) -> ContractDescriptor {
    ContractDescriptor {
        id: ContractId(id.to_string()),
        version: "1".to_string(),
        representation: "application/xml".to_string(),
    }
}

impl Contract for XmlSchema {
    fn descriptor(&self) -> &ContractDescriptor {
        &self.descriptor
    }

    fn identify(&self, stream: &Stream) -> Result<bool, ContractError> {
        if stream.media_type().is_some_and(is_xml_media_type) {
            return Ok(true);
        }
        let first = stream
            .bytes()
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace());
        Ok(first == Some(b'<'))
    }

    fn validate(&self, stream: &Stream) -> Result<ValidationResult, ContractError> {
        let text = match std::str::from_utf8(stream.bytes()) {
            Ok(text) => text,
            Err(error) => return Ok(malformed(&format!("not UTF-8 text: {error}"), None)),
        };
        let document = match roxmltree::Document::parse(text) {
            Ok(document) => document,
            Err(error) => {
                let position = error.pos();
                let at = format!("line {} column {}", position.row, position.col);
                return Ok(malformed(
                    &format!("not well-formed XML: {error}"),
                    Some(at),
                ));
            }
        };
        let issues = match &self.schema {
            Some(schema) => check::check(schema, document.root_element()),
            None => Vec::new(),
        };
        Ok(ValidationResult {
            valid: issues.is_empty(),
            issues,
        })
    }
}

fn is_xml_media_type(media_type: &str) -> bool {
    let essence = media_type.split(';').next().unwrap_or("").trim();
    essence.eq_ignore_ascii_case("application/xml")
        || essence.eq_ignore_ascii_case("text/xml")
        || essence.ends_with("+xml")
}

fn malformed(message: &str, path: Option<String>) -> ValidationResult {
    ValidationResult {
        valid: false,
        issues: vec![ValidationIssue {
            code: "malformed".to_string(),
            message: message.to_string(),
            path,
        }],
    }
}

/// Loads the contract a Location names: an empty reference is the bare
/// contract, anything else is the path of a schema file.
pub struct XmlSchemaFactory;

impl ContractFactory for XmlSchemaFactory {
    fn technology(&self) -> &'static str {
        "xml-schema"
    }

    fn load(&self, reference: &str) -> Result<Box<dyn Contract>, ContractError> {
        if reference.trim().is_empty() {
            return Ok(Box::new(XmlSchema::new()));
        }
        let text = std::fs::read_to_string(reference).map_err(|error| ContractError {
            message: format!("cannot read schema {reference}: {error}"),
        })?;
        Ok(Box::new(XmlSchema::with_schema(&text)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcore::StreamId;

    fn stream(text: &str, media_type: Option<&str>) -> Stream {
        Stream::new(
            StreamId::new(1),
            text.as_bytes().to_vec(),
            media_type.map(str::to_string),
        )
    }

    const ORDER: &str = r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           targetNamespace="urn:example:order" elementFormDefault="qualified">
  <xs:element name="order">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="id" type="xs:string"/>
        <xs:element name="line" type="Line" minOccurs="1" maxOccurs="unbounded"/>
        <xs:element name="note" type="xs:string" minOccurs="0"/>
      </xs:sequence>
      <xs:attribute name="currency" type="xs:string" use="required"/>
    </xs:complexType>
  </xs:element>
  <xs:complexType name="Line">
    <xs:sequence>
      <xs:element name="sku" type="xs:string"/>
      <xs:element name="qty" type="xs:positiveInteger"/>
    </xs:sequence>
  </xs:complexType>
</xs:schema>"#;

    #[test]
    fn bare_contract_holds_well_formed_xml_only() {
        let bare = XmlSchema::new();
        let held = bare
            .validate(&stream("<a><b/>text</a>", None))
            .expect("validates");
        assert!(held.valid);
        let broken = bare
            .validate(&stream("<a><b></a>", None))
            .expect("validates");
        assert_eq!(broken.issues[0].code, "malformed");
        assert!(
            broken.issues[0]
                .path
                .as_deref()
                .is_some_and(|p| p.starts_with("line 1"))
        );
    }

    #[test]
    fn bound_contract_holds_a_conforming_order() {
        let bound = XmlSchema::with_schema(ORDER).expect("a schema");
        assert_eq!(bound.descriptor().id.0, "xml-schema:urn:example:order");
        let text = r#"<order xmlns="urn:example:order" currency="SEK">
  <id>A1</id><line><sku>X</sku><qty>2</qty></line><line><sku>Y</sku><qty>1</qty></line>
</order>"#;
        let held = bound.validate(&stream(text, None)).expect("validates");
        assert!(held.valid, "issues: {:?}", held.issues);
    }

    #[test]
    fn bound_contract_names_every_departure_with_its_path() {
        let bound = XmlSchema::with_schema(ORDER).expect("a schema");
        let text = r"<order><id>A1</id><line><sku>X</sku><qty>0</qty></line><extra/></order>";
        let held = bound.validate(&stream(text, None)).expect("validates");
        assert!(!held.valid);
        let seen: Vec<(String, String)> = held
            .issues
            .iter()
            .map(|i| (i.code.clone(), i.path.clone().unwrap_or_default()))
            .collect();
        let has = |code: &str, path: &str| seen.iter().any(|(c, p)| c == code && p == path);
        assert!(has("attribute", "/order/@currency"), "{seen:?}");
        assert!(has("value", "/order/line[1]/qty"), "{seen:?}");
        assert!(has("content", "/order/extra"), "{seen:?}");
    }

    #[test]
    fn identifies_by_media_type_or_first_byte() {
        let bare = XmlSchema::new();
        assert!(
            bare.identify(&stream("x", Some("text/xml")))
                .expect("identifies")
        );
        assert!(
            bare.identify(&stream("x", Some("application/soap+xml; charset=utf-8")))
                .expect("identifies")
        );
        assert!(bare.identify(&stream("  <a/>", None)).expect("identifies"));
        assert!(
            !bare
                .identify(&stream("a,b", Some("text/csv")))
                .expect("identifies")
        );
    }

    #[test]
    fn the_factory_loads_bare_and_bound() {
        let factory = XmlSchemaFactory;
        assert_eq!(factory.technology(), "xml-schema");
        assert_eq!(
            factory.load("").expect("bare").descriptor().id.0,
            "xml-schema"
        );
        let dir = std::env::temp_dir().join("xmip-xml-schema-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("order.xsd");
        std::fs::write(&file, ORDER).expect("write schema");
        let bound = factory
            .load(file.to_str().expect("utf-8 path"))
            .expect("bound");
        assert_eq!(bound.descriptor().id.0, "xml-schema:urn:example:order");
        assert!(
            factory
                .load(dir.join("missing.xsd").to_str().expect("path"))
                .is_err()
        );
    }
}
