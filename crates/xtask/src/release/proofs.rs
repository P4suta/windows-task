use super::Fingerprint;

#[kani::proof]
#[kani::unwind(33)]
fn fingerprint_identity_includes_size_and_digest() {
    let left = Fingerprint {
        size: kani::any(),
        digest: kani::any(),
    };
    let right = Fingerprint {
        size: kani::any(),
        digest: kani::any(),
    };
    if left == right {
        assert_eq!(left.size, right.size);
        assert_eq!(left.digest, right.digest);
    } else {
        assert!(left.size != right.size || left.digest != right.digest);
    }
}
