//! The module's timing program (`doc/governance/rust-style.md` section
//! 5c): a bound XML Schema held to a time. A plain `cargo test` leaves it
//! out; the landing runs it after every other test of the module, alone,
//! one test at a time (`cargo test --test measured --all-features --
//! --test-threads=1`), so what is measured is Xmip's work and not its
//! neighbors':
//!
//! - a conforming order costs only its own walk.

mod order;

use contract::Contract;
use contract::fixture::stream_as as stream;
use order::ORDER;
use xmip_core_contract_xml_schema::XmlSchema;

#[test]
fn a_conforming_order_costs_only_its_own_walk() {
    let bound = XmlSchema::with_schema(ORDER).expect("a schema");
    let order = stream(
        r#"<order xmlns="urn:example:order" currency="SEK"><id>A1</id>
        <line><sku>X</sku><qty>2</qty></line></order>"#,
        None,
    );
    // Each message timed, the fastest of five (`xcore::latency::spread`): a
    // path spelled per element, or the schema read per message, is in every
    // one, a busy machine's preemption in some. It waits on no wake, so no
    // plain wake is measured beside it. Until 2026-11-15 the ten thousand
    // were timed together.
    let each = (0..10_000)
        .map(|_| {
            let started = std::time::Instant::now();
            let valid = bound.validate(&order).expect("validates").valid;
            let took = started.elapsed();
            assert!(valid);
            took
        })
        .collect();
    let each = xcore::latency::spread("a conforming order validated", each);
    assert!(
        each.fastest_of_five < std::time::Duration::from_micros(500),
        "{each:?} a message"
    );
}
