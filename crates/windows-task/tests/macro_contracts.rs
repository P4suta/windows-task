#![cfg(feature = "handler")]

#[test]
fn handler_macro_contracts() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/valid_handler.rs");
    cases.compile_fail("tests/ui/missing_clsid.rs");
    cases.compile_fail("tests/ui/generic_handler.rs");
    cases.compile_fail("tests/ui/inherent_handler.rs");
}
