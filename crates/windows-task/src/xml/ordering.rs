#[cfg(kani)]
mod proofs;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AbsoluteOrdinal(usize);

impl AbsoluteOrdinal {
    fn index(self, child_count: usize) -> usize {
        self.0.min(child_count)
    }
}

pub(super) struct Positioned<T> {
    ordinal: AbsoluteOrdinal,
    child: T,
}

impl<T> Positioned<T> {
    pub(super) const fn new(ordinal: usize, child: T) -> Self {
        Self {
            ordinal: AbsoluteOrdinal(ordinal),
            child,
        }
    }
}

pub(super) fn merge<T>(known: Vec<Option<T>>, mut extensions: Vec<Positioned<T>>) -> Vec<T> {
    let capacity = known
        .len()
        .checked_add(extensions.len())
        .expect("child capacity fits usize");
    let mut output = Vec::with_capacity(capacity);
    for child in known.into_iter().flatten() {
        output.push(child);
    }
    extensions.sort_by_key(|extension| extension.ordinal);
    for extension in extensions {
        output.insert(extension.ordinal.index(output.len()), extension.child);
    }
    output
}
