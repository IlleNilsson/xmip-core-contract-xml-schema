//! Holding a document to a bound [`Schema`].
//!
//! An issue's `code` names what failed — `root`, `type`, `attribute`,
//! `content`, `occurs`, `value` — and its `path` is where, XPath-style:
//! `/order/line[2]/qty`, `/order/@currency`.

use crate::schema::{ComplexType, Content, Element, Kind, Schema};
use codec::civil;
use contract::ValidationIssue;
use contract::place::Place;
use roxmltree::Node;

/// Every departure of the document rooted at `root` from `schema`.
#[must_use]
pub fn check(schema: &Schema, root: Node) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let name = root.tag_name().name();
    let top = Place::Root;
    let place = top.field(name);
    match schema.elements.get(name) {
        Some(declaration) => element(schema, declaration, root, &place, &mut issues),
        None => issues.push(ValidationIssue::at(
            "root",
            format!("element {name} is not declared"),
            place.xpath(),
        )),
    }
    issues
}

fn element(
    schema: &Schema,
    declaration: &Element,
    node: Node,
    place: &Place<'_>,
    out: &mut Vec<ValidationIssue>,
) {
    match &declaration.kind {
        Kind::Any => {}
        Kind::Builtin(builtin) => {
            reject_children(node, place, out);
            value(builtin, text_of(node), place, out);
        }
        Kind::Named(name) => match schema.types.get(name) {
            Some(complex) => complex_type(schema, complex, node, place, out),
            None => out.push(ValidationIssue::at(
                "type",
                format!("type {name} is not declared"),
                place.xpath(),
            )),
        },
        Kind::Inline(complex) => complex_type(schema, complex, node, place, out),
    }
}

fn complex_type(
    schema: &Schema,
    complex: &ComplexType,
    node: Node,
    place: &Place<'_>,
    out: &mut Vec<ValidationIssue>,
) {
    attributes(complex, node, place, out);
    match &complex.content {
        Content::Empty => reject_children(node, place, out),
        Content::Simple(builtin) => {
            reject_children(node, place, out);
            value(builtin, text_of(node), place, out);
        }
        Content::Sequence(particles) => sequence(schema, particles, node, place, out),
        Content::All(particles) => all(schema, particles, node, place, out),
        Content::Choice(particles) => choice(schema, particles, node, place, out),
    }
}

fn attributes(
    complex: &ComplexType,
    node: Node,
    place: &Place<'_>,
    out: &mut Vec<ValidationIssue>,
) {
    for declared in &complex.attributes {
        let at = place.attribute(&declared.name);
        match node.attribute(declared.name.as_str()) {
            Some(text) => value(&declared.builtin, text, &at, out),
            None if declared.required => {
                out.push(ValidationIssue::at(
                    "attribute",
                    "required attribute is missing",
                    at.xpath(),
                ));
            }
            None => {}
        }
    }
    for present in node.attributes() {
        // Namespace declarations and xsi:* are wiring, not content.
        if present.namespace().is_some() {
            continue;
        }
        if !complex.attributes.iter().any(|a| a.name == present.name()) {
            out.push(ValidationIssue::at(
                "attribute",
                "attribute is not declared",
                place.attribute(present.name()).xpath(),
            ));
        }
    }
}

fn reject_children(node: Node, place: &Place<'_>, out: &mut Vec<ValidationIssue>) {
    for child in node.children().filter(Node::is_element) {
        out.push(ValidationIssue::at(
            "content",
            "no child element is allowed here",
            place.field(child.tag_name().name()).xpath(),
        ));
    }
}

fn text_of<'a>(node: Node<'a, '_>) -> &'a str {
    node.text().unwrap_or("")
}

/// Check `node` as the `count`th `particle` under `place`: an ordinal only
/// where the schema lets the element repeat, `line[2]` but `qty`, so a path
/// reads the way the operator wrote the schema.
fn within(
    schema: &Schema,
    particle: &Element,
    node: Node,
    place: &Place<'_>,
    count: u32,
    out: &mut Vec<ValidationIssue>,
) {
    let named = place.field(&particle.name);
    if particle.max == Some(1) {
        element(schema, particle, node, &named, out);
    } else {
        element(schema, particle, node, &named.index(count as usize), out);
    }
}

fn sequence(
    schema: &Schema,
    particles: &[Element],
    node: Node,
    place: &Place<'_>,
    out: &mut Vec<ValidationIssue>,
) {
    let children: Vec<Node> = node.children().filter(Node::is_element).collect();
    let mut next = 0;
    for particle in particles {
        let mut count = 0;
        while next < children.len()
            && children[next].tag_name().name() == particle.name
            && particle.max.is_none_or(|max| count < max)
        {
            count += 1;
            within(schema, particle, children[next], place, count, out);
            next += 1;
        }
        occurs(particle, count, place, out);
    }
    for child in &children[next..] {
        out.push(ValidationIssue::at(
            "content",
            "element is not expected here",
            place.field(child.tag_name().name()).xpath(),
        ));
    }
}

fn all(
    schema: &Schema,
    particles: &[Element],
    node: Node,
    place: &Place<'_>,
    out: &mut Vec<ValidationIssue>,
) {
    let children: Vec<Node> = node.children().filter(Node::is_element).collect();
    for particle in particles {
        let mut count = 0;
        for child in children
            .iter()
            .filter(|c| c.tag_name().name() == particle.name)
        {
            count += 1;
            within(schema, particle, *child, place, count, out);
        }
        occurs(particle, count, place, out);
    }
    unexpected(particles, &children, place, out);
}

fn choice(
    schema: &Schema,
    particles: &[Element],
    node: Node,
    place: &Place<'_>,
    out: &mut Vec<ValidationIssue>,
) {
    let children: Vec<Node> = node.children().filter(Node::is_element).collect();
    let chosen: Vec<&Element> = particles
        .iter()
        .filter(|p| children.iter().any(|c| c.tag_name().name() == p.name))
        .collect();
    match chosen.as_slice() {
        [] => {
            let names: Vec<&str> = particles.iter().map(|p| p.name.as_str()).collect();
            let message = format!("one of {} is required", names.join(", "));
            out.push(ValidationIssue::at("occurs", message, place.xpath()));
        }
        [one] => all(schema, std::slice::from_ref(*one), node, place, out),
        many => {
            let names: Vec<&str> = many.iter().map(|p| p.name.as_str()).collect();
            let message = format!("only one of {} may appear", names.join(", "));
            out.push(ValidationIssue::at("content", message, place.xpath()));
        }
    }
    unexpected(particles, &children, place, out);
}

fn unexpected(
    particles: &[Element],
    children: &[Node],
    place: &Place<'_>,
    out: &mut Vec<ValidationIssue>,
) {
    for child in children {
        let name = child.tag_name().name();
        if !particles.iter().any(|p| p.name == name) {
            out.push(ValidationIssue::at(
                "content",
                "element is not expected here",
                place.field(name).xpath(),
            ));
        }
    }
}

fn occurs(particle: &Element, count: u32, place: &Place<'_>, out: &mut Vec<ValidationIssue>) {
    let at = || place.field(&particle.name).xpath();
    if count < particle.min {
        let message = format!("occurs {count} times, at least {} required", particle.min);
        out.push(ValidationIssue::at("occurs", message, at()));
    }
    if let Some(max) = particle.max
        && count > max
    {
        let message = format!("occurs {count} times, at most {max} allowed");
        out.push(ValidationIssue::at("occurs", message, at()));
    }
}

/// A built-in simple type's lexical check. Unknown names hold: a type this
/// contract does not check is not a type it refuses. Dates and times are read
/// by the estate's one calendar (`codec::civil`), each field in its range; a
/// time zone is optional, as XML Schema says.
fn value(builtin: &str, text: &str, place: &Place<'_>, out: &mut Vec<ValidationIssue>) {
    let text = text.trim();
    let held = match builtin {
        "boolean" => matches!(text, "true" | "false" | "1" | "0"),
        "integer" | "int" | "long" | "short" | "byte" => text.parse::<i64>().is_ok(),
        "unsignedLong" | "unsignedInt" | "unsignedShort" | "unsignedByte"
        | "nonNegativeInteger" => text.parse::<u64>().is_ok(),
        "positiveInteger" => text.parse::<u64>().is_ok_and(|n| n > 0),
        "negativeInteger" => text.parse::<i64>().is_ok_and(|n| n < 0),
        "nonPositiveInteger" => text.parse::<i64>().is_ok_and(|n| n <= 0),
        "decimal" => text.parse::<f64>().is_ok() && !text.eq_ignore_ascii_case("nan"),
        "double" | "float" => text.parse::<f64>().is_ok() || matches!(text, "INF" | "-INF" | "NaN"),
        "date" => text
            .get(..10)
            .and_then(civil::read_date)
            .is_some_and(|_| zoned(&text[10..])),
        "dateTime" => text
            .split_once('T')
            .is_some_and(|(date, time)| civil::read_date(date).is_some() && is_time(time)),
        "time" => is_time(text),
        _ => true,
    };
    if !held {
        out.push(ValidationIssue::at(
            "value",
            format!("{text:?} is not an xs:{builtin}"),
            place.xpath(),
        ));
    }
}

/// A time zone, or none: what may follow an XML Schema date or time.
fn zoned(rest: &str) -> bool {
    rest.is_empty() || civil::read_offset(rest).is_some()
}

fn is_time(text: &str) -> bool {
    civil::read_time(text).is_some_and(|(_, rest)| zoned(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issues(schema: &str, document: &str) -> Vec<(String, String)> {
        let schema = Schema::parse(schema).expect("schema");
        let document = roxmltree::Document::parse(document).expect("document");
        check(&schema, document.root_element())
            .into_iter()
            .map(|i| (i.code.into_owned(), i.path.unwrap_or_default()))
            .collect()
    }

    const XS: &str = r#"xmlns:xs="http://www.w3.org/2001/XMLSchema""#;

    #[test]
    fn a_choice_wants_exactly_one_branch() {
        let schema = format!(
            r#"<xs:schema {XS}><xs:element name="p"><xs:complexType><xs:choice>
            <xs:element name="a" type="xs:string"/><xs:element name="b" type="xs:string"/>
            </xs:choice></xs:complexType></xs:element></xs:schema>"#
        );
        assert!(issues(&schema, "<p><a/></p>").is_empty());
        assert_eq!(issues(&schema, "<p/>")[0].0, "occurs");
        assert_eq!(issues(&schema, "<p><a/><b/></p>")[0].0, "content");
    }

    #[test]
    fn all_admits_any_order_and_bounds_each() {
        let schema = format!(
            r#"<xs:schema {XS}><xs:element name="p"><xs:complexType><xs:all>
            <xs:element name="a" type="xs:int"/><xs:element name="b" type="xs:int"/>
            </xs:all></xs:complexType></xs:element></xs:schema>"#
        );
        assert!(issues(&schema, "<p><b>1</b><a>2</a></p>").is_empty());
        let twice = issues(&schema, "<p><a>1</a><a>2</a><b>3</b></p>");
        assert_eq!(twice, [("occurs".to_string(), "/p/a".to_string())]);
    }

    #[test]
    fn an_undeclared_root_and_an_undeclared_type_are_named() {
        let schema =
            format!(r#"<xs:schema {XS}><xs:element name="p" type="Missing"/></xs:schema>"#);
        assert_eq!(
            issues(&schema, "<q/>"),
            [("root".to_string(), "/q".to_string())]
        );
        assert_eq!(
            issues(&schema, "<p/>"),
            [("type".to_string(), "/p".to_string())]
        );
    }

    #[test]
    fn built_in_values_are_checked_lexically() {
        let mut out = Vec::new();
        let at = Place::Root;
        value("date", "2026-09-07", &at, &mut out);
        value("date", "2026-09-07+02:00", &at, &mut out);
        value("dateTime", "2026-09-07T13:45:00Z", &at, &mut out);
        value("dateTime", "2026-09-07T13:45:00.5", &at, &mut out);
        value("time", "13:45:00", &at, &mut out);
        value("boolean", "true", &at, &mut out);
        value("decimal", "-1.50", &at, &mut out);
        value("customType", "anything", &at, &mut out);
        assert!(out.is_empty(), "{out:?}");
        value("date", "7/9/2026", &at, &mut out);
        value("positiveInteger", "0", &at, &mut out);
        value("boolean", "yes", &at, &mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn a_date_is_held_to_the_calendar() {
        let mut out = Vec::new();
        let at = Place::Root;
        for wrong in ["2026-99-99", "2026-02-31", "2026-02-29", "2026-13-01"] {
            value("date", wrong, &at, &mut out);
            value("dateTime", &format!("{wrong}T00:00:00"), &at, &mut out);
        }
        value("time", "25:00:00", &at, &mut out);
        value("time", "12:00:00+99:00", &at, &mut out);
        assert_eq!(out.len(), 10, "{out:?}");
    }
}
