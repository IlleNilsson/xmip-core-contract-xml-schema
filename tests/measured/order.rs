//! The order schema the crate's tests bind: what its tests and its timing
//! program both validate against. Shared, not copied: `src/lib.rs`'s
//! tests take it with `#[path]`.

/// An order of one currency, an id and at least one line, each a sku and
/// a positive quantity.
pub const ORDER: &str = r#"<?xml version="1.0"?>
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
