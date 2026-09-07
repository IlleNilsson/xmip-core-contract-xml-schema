# xmip-core-contract-xml-schema

The XML content contract, a technology of
[xmip-core-contract](https://github.com/IlleNilsson/xmip-core-contract).

Two claims. **Well-formedness is a given**: every Stream this contract sees is
parsed, and a Stream that is not XML fails with the line and column.
**Conformance is a given once the contract is named**: a Receive or Send Location
that refers to this contract with a schema bound has every Stream validated
against that schema, and each departure is reported with the XPath of where it
happened and what refused it.

`src/schema.rs` lists the XML Schema subset supported. A schema outside it is
refused when bound, by name, so the operator learns at configuration time.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
