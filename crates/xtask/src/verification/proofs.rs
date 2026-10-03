use super::{XML_HARNESSES, verify_xml_batch};

#[kani::proof]
#[kani::unwind(7)]
fn proof_batch_stops_on_first_failure() {
    let outcomes: [bool; XML_HARNESSES.len()] = kani::any();
    let mut calls = 0;
    let result = verify_xml_batch(|_| {
        let index = calls;
        calls += 1;
        if outcomes[index] { Ok(()) } else { Err(index) }
    });
    match result {
        Ok(()) => {
            assert_eq!(calls, XML_HARNESSES.len());
            for outcome in outcomes {
                assert!(outcome);
            }
        }
        Err(index) => {
            assert_eq!(calls, index + 1);
            assert!(!outcomes[index]);
            for &outcome in &outcomes[..index] {
                assert!(outcome);
            }
        }
    }
}
