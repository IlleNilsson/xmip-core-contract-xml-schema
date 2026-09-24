//! Holding a document to a bound [`Schema`].
//!
//! An issue's `code` names what failed — `root`, `type`, `attribute`,
//! `content`, `occurs`, `value` — and its `path` is where, XPath-style:
//! `/order/line[2]/qty`, `/order/@currency`.

use crate::schema::{ComplexType, Content, Element, Kind, Schema};
use roxmltree::Node;
use sdk::contract::ValidationIssue;

/// Every departure of the document rooted at `root` from `schema`.
#[must_use]
pub fn check(schema: &Schema, root: Node) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let name = root.tag_name().name();
    let path = format!("/{name}");
    match schema.elements.get(name) {
        Some(declaration) => element(schema, declaration, root, &path, &mut issues),
        None => issues.push(ValidationIssue::at(
            "root",
            &format!("element {name} is not declared"),
            &path,
        )),
    }
    issues
}

fn element(
    schema: &Schema,
    declaration: &Element,
    node: Node,
    path: &str,
    out: &mut Vec<ValidationIssue>,
) {
    match &declaration.kind {
        Kind::Any => {}
        Kind::Builtin(builtin) => {
            reject_children(node, path, out);
            value(builtin, text_of(node), path, out);
        }
        Kind::Named(name) => match schema.types.get(name) {
            Some(complex) => complex_type(schema, complex, node, path, out),
            None => out.push(ValidationIssue::at(
                "type",
                &format!("type {name} is not declared"),
                path,
            )),
        },
        Kind::Inline(complex) => complex_type(schema, complex, node, path, out),
    }
}

fn complex_type(
    schema: &Schema,
    complex: &ComplexType,
    node: Node,
    path: &str,
    out: &mut Vec<ValidationIssue>,
) {
    attributes(complex, node, path, out);
    match &complex.content {
        Content::Empty => reject_children(node, path, out),
        Content::Simple(builtin) => {
            reject_children(node, path, out);
            value(builtin, text_of(node), path, out);
        }
        Content::Sequence(particles) => sequence(schema, particles, node, path, out),
        Content::All(particles) => all(schema, particles, node, path, out),
        Content::Choice(particles) => choice(schema, particles, node, path, out),
    }
}

fn attributes(complex: &ComplexType, node: Node, path: &str, out: &mut Vec<ValidationIssue>) {
    for declared in &complex.attributes {
        let at = format!("{path}/@{}", declared.name);
        match node.attribute(declared.name.as_str()) {
            Some(text) => value(&declared.builtin, text, &at, out),
            None if declared.required => {
                out.push(ValidationIssue::at(
                    "attribute",
                    "required attribute is missing",
                    &at,
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
            let at = format!("{path}/@{}", present.name());
            out.push(ValidationIssue::at(
                "attribute",
                "attribute is not declared",
                &at,
            ));
        }
    }
}

fn reject_children(node: Node, path: &str, out: &mut Vec<ValidationIssue>) {
    for child in node.children().filter(Node::is_element) {
        let at = format!("{path}/{}", child.tag_name().name());
        out.push(ValidationIssue::at(
            "content",
            "no child element is allowed here",
            &at,
        ));
    }
}

fn text_of<'a>(node: Node<'a, '_>) -> &'a str {
    node.text().unwrap_or("")
}

fn child_path(path: &str, particle: &Element, ordinal: u32) -> String {
    // An ordinal only where the schema lets the element repeat: `line[2]`, but
    // `qty`, so a path reads the way the operator wrote the schema.
    match particle.max {
        Some(1) => format!("{path}/{}", particle.name),
        _ => format!("{path}/{}[{ordinal}]", particle.name),
    }
}

fn sequence(
    schema: &Schema,
    particles: &[Element],
    node: Node,
    path: &str,
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
            let at = child_path(path, particle, count);
            element(schema, particle, children[next], &at, out);
            next += 1;
        }
        occurs(particle, count, path, out);
    }
    for child in &children[next..] {
        let at = format!("{path}/{}", child.tag_name().name());
        out.push(ValidationIssue::at(
            "content",
            "element is not expected here",
            &at,
        ));
    }
}

fn all(
    schema: &Schema,
    particles: &[Element],
    node: Node,
    path: &str,
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
            let at = child_path(path, particle, count);
            element(schema, particle, *child, &at, out);
        }
        occurs(particle, count, path, out);
    }
    unexpected(particles, &children, path, out);
}

fn choice(
    schema: &Schema,
    particles: &[Element],
    node: Node,
    path: &str,
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
            out.push(ValidationIssue::at("occurs", &message, path));
        }
        [one] => all(schema, std::slice::from_ref(*one), node, path, out),
        many => {
            let names: Vec<&str> = many.iter().map(|p| p.name.as_str()).collect();
            let message = format!("only one of {} may appear", names.join(", "));
            out.push(ValidationIssue::at("content", &message, path));
        }
    }
    unexpected(particles, &children, path, out);
}

fn unexpected(
    particles: &[Element],
    children: &[Node],
    path: &str,
    out: &mut Vec<ValidationIssue>,
) {
    for child in children {
        let name = child.tag_name().name();
        if !particles.iter().any(|p| p.name == name) {
            out.push(ValidationIssue::at(
                "content",
                "element is not expected here",
                &format!("{path}/{name}"),
            ));
        }
    }
}

fn occurs(particle: &Element, count: u32, path: &str, out: &mut Vec<ValidationIssue>) {
    let at = format!("{path}/{}", particle.name);
    if count < particle.min {
        let message = format!("occurs {count} times, at least {} required", particle.min);
        out.push(ValidationIssue::at("occurs", &message, &at));
    }
    if let Some(max) = particle.max
        && count > max
    {
        let message = format!("occurs {count} times, at most {max} allowed");
        out.push(ValidationIssue::at("occurs", &message, &at));
    }
}

/// A built-in simple type's lexical check. Unknown names hold: a type this
/// contract does not check is not a type it refuses.
fn value(builtin: &str, text: &str, path: &str, out: &mut Vec<ValidationIssue>) {
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
        "date" => is_date(text),
        "dateTime" => text
            .split_once('T')
            .is_some_and(|(d, t)| is_date(d) && is_time(t)),
        "time" => is_time(text),
        _ => true,
    };
    if !held {
        out.push(ValidationIssue::at(
            "value",
            &format!("{text:?} is not an xs:{builtin}"),
            path,
        ));
    }
}

fn is_date(text: &str) -> bool {
    let core = text
        .trim_end_matches('Z')
        .split(['+', '-'])
        .next()
        .unwrap_or("");
    let parts: Vec<&str> = text
        .get(..core.len().max(10))
        .unwrap_or("")
        .split('-')
        .collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_time(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 8
        && bytes[..8].iter().enumerate().all(|(i, b)| {
            if i == 2 || i == 5 {
                *b == b':'
            } else {
                b.is_ascii_digit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issues(schema: &str, document: &str) -> Vec<(String, String)> {
        let schema = Schema::parse(schema).expect("schema");
        let document = roxmltree::Document::parse(document).expect("document");
        check(&schema, document.root_element())
            .into_iter()
            .map(|i| (i.code, i.path.unwrap_or_default()))
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
        value("date", "2026-09-07", "/d", &mut out);
        value("dateTime", "2026-09-07T13:45:00Z", "/dt", &mut out);
        value("time", "13:45:00", "/t", &mut out);
        value("boolean", "true", "/b", &mut out);
        value("decimal", "-1.50", "/n", &mut out);
        value("customType", "anything", "/c", &mut out);
        assert!(out.is_empty(), "{out:?}");
        value("date", "7/9/2026", "/d", &mut out);
        value("positiveInteger", "0", "/p", &mut out);
        value("boolean", "yes", "/b", &mut out);
        assert_eq!(out.len(), 3);
    }
}
