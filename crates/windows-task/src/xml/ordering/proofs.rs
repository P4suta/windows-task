use super::{AbsoluteOrdinal, Positioned, merge};

#[kani::proof]
fn absolute_ordinal_is_bounded_without_arithmetic() {
    let ordinal: usize = kani::any();
    let child_count: usize = kani::any();
    let index = AbsoluteOrdinal(ordinal).index(child_count);
    assert!(index <= child_count);
    assert_eq!(
        index,
        if ordinal <= child_count {
            ordinal
        } else {
            child_count
        }
    );
}

#[kani::proof]
#[kani::unwind(5)]
fn extension_merge_without_known_children() {
    assert_merge_contract([false, false]);
}

#[kani::proof]
#[kani::unwind(5)]
fn extension_merge_with_first_known_child() {
    assert_merge_contract([true, false]);
}

#[kani::proof]
#[kani::unwind(5)]
fn extension_merge_with_second_known_child() {
    assert_merge_contract([false, true]);
}

#[kani::proof]
#[kani::unwind(5)]
fn extension_merge_with_both_known_children() {
    assert_merge_contract([true, true]);
}

fn assert_merge_contract(present: [bool; 2]) {
    let ordinals: [usize; 2] = kani::any();
    let known = vec![present[0].then_some(1_u8), present[1].then_some(2_u8)];
    let extensions = vec![
        Positioned::new(ordinals[0], 3_u8),
        Positioned::new(ordinals[1], 4_u8),
    ];
    let output = merge(known, extensions);
    assert_eq!(
        output.len(),
        usize::from(present[0]) + usize::from(present[1]) + 2
    );
    if ordinals[0] != ordinals[1] && ordinals[0] < output.len() && ordinals[1] < output.len() {
        assert_eq!(output[ordinals[0]], 3);
        assert_eq!(output[ordinals[1]], 4);
    }
    if present[0] && present[1] {
        let first = output
            .iter()
            .position(|&child| child == 1)
            .expect("first known child");
        let second = output
            .iter()
            .position(|&child| child == 2)
            .expect("second known child");
        assert!(first < second);
    }
}

#[kani::proof]
#[kani::unwind(5)]
fn extension_merge_regression_is_reachable() {
    assert_eq!(
        merge(
            vec![Some(1_u8), None, Some(2_u8)],
            vec![Positioned::new(0, 3), Positioned::new(1, 4)]
        ),
        [3, 4, 1, 2],
    );
}
