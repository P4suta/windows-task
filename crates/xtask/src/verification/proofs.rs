use super::{REQUIRED_PROOFS, verify_proof_batch};

#[kani::proof]
#[kani::unwind(9)]
fn proof_batch_stops_on_first_failure() {
    let outcomes: [bool; REQUIRED_PROOFS.len()] = kani::any();
    let preparation_succeeded: bool = kani::any();
    let preparation = if preparation_succeeded {
        Ok(())
    } else {
        Err(REQUIRED_PROOFS.len())
    };
    let mut calls = 0;
    let result = verify_proof_batch(preparation, |(), _, _| {
        let index = calls;
        calls += 1;
        if outcomes[index] { Ok(()) } else { Err(index) }
    });
    match result {
        Ok(()) => {
            assert!(preparation_succeeded);
            assert_eq!(calls, REQUIRED_PROOFS.len());
            for outcome in outcomes {
                assert!(outcome);
            }
        }
        Err(index) => {
            if index == REQUIRED_PROOFS.len() {
                assert!(!preparation_succeeded);
                assert_eq!(calls, 0);
                return;
            }
            assert!(preparation_succeeded);
            assert_eq!(calls, index + 1);
            assert!(!outcomes[index]);
            for &outcome in &outcomes[..index] {
                assert!(outcome);
            }
        }
    }
}
