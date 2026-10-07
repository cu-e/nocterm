// Nocterm modifications, licensed under the upstream Apache-2.0 license.
use std::ops::Range;

pub(super) fn within_budget(instances: usize, rectangles: usize) -> bool {
    instances <= 65_536
        && rectangles <= 64
        && instances
            .checked_mul(rectangles)
            .is_some_and(|work| work <= 1_000_000)
}

pub(super) struct Selected<T> {
    pub(super) instances: Vec<T>,
    prefixes: Vec<Vec<usize>>,
}

impl<T> Selected<T> {
    pub(super) fn map_range(&self, range: Range<usize>, strip: usize) -> Range<usize> {
        self.prefixes[strip][range.start]..self.prefixes[strip][range.end]
    }
}

pub(super) fn select<T: Copy>(
    instances: &[T],
    strips: usize,
    limit: usize,
    mut visible: impl FnMut(usize, usize) -> bool,
) -> Option<Selected<T>> {
    let mut selected = Selected {
        instances: Vec::new(),
        prefixes: Vec::with_capacity(strips),
    };
    for strip in 0..strips {
        let mut prefix = Vec::with_capacity(instances.len() + 1);
        prefix.push(selected.instances.len());
        for (index, instance) in instances.iter().enumerate() {
            if visible(strip, index) {
                if selected.instances.len() == limit {
                    return None;
                }
                selected.instances.push(*instance);
            }
            prefix.push(selected.instances.len());
        }
        selected.prefixes.push(prefix);
    }
    Some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_prefix_preserves_order_and_original_batch_boundaries() {
        let selected = select(&[10, 11, 12, 13, 14, 15], 1, 6, |_, i| i == 1 || i >= 4).unwrap();
        assert_eq!(selected.instances, vec![11, 14, 15]);
        assert_eq!(selected.prefixes[0], vec![0, 0, 1, 1, 1, 2, 3]);
        assert_eq!(selected.map_range(0..3, 0), 0..1);
        assert_eq!(selected.map_range(3..6, 0), 1..3);
        assert_eq!(selected.map_range(2..4, 0), 1..1);
    }
    #[test]
    fn fragmented_overlapping_strips_duplicate_in_original_order_once_per_stream() {
        let original: Vec<_> = (0..130).map(|i| (i, i * 7)).collect();
        let selected = select(&original, 2, 130, |_, i| i % 2 == 0).unwrap();
        let expected: Vec<_> = original.iter().step_by(2).copied().collect();
        assert_eq!(selected.instances, [expected.clone(), expected].concat());
        assert_eq!(selected.map_range(10..130, 0), 5..65);
        assert_eq!(selected.map_range(10..130, 1), 70..130);
        assert_eq!(original[129], (129, 903));
    }
    #[test]
    fn empty_strips_preserve_rebased_endpoints_and_disjoint_selection() {
        let selected = select(&[10, 20, 30], 5, 3, |strip, index| {
            (strip == 1 && index == 2) || (strip == 3 && index == 0)
        })
        .unwrap();
        assert_eq!(selected.instances, vec![30, 10]);
        assert_eq!(selected.map_range(0..3, 0), 0..0);
        assert_eq!(selected.map_range(0..3, 1), 0..1);
        assert_eq!(selected.map_range(0..3, 2), 1..1);
        assert_eq!(selected.map_range(0..3, 3), 1..2);
        assert_eq!(selected.map_range(0..3, 4), 2..2);
        assert_eq!(selected.map_range(0..2, 1), 0..0);
        assert_eq!(selected.map_range(1..3, 3), 2..2);
    }
    #[test]
    fn preparation_limits_bound_prefix_work_and_combined_packed_instances() {
        assert!(within_budget(65_536, 1));
        assert!(!within_budget(65_537, 1));
        assert!(within_budget(15_625, 64));
        assert!(!within_budget(15_626, 64));
        assert!(!within_budget(1, 65));
        assert!(!within_budget(usize::MAX, usize::MAX));
        assert_eq!(
            select(&[1, 2], 2, 4, |_, _| true).unwrap().instances,
            [1, 2, 1, 2]
        );
        assert!(select(&[1, 2], 2, 3, |_, _| true).is_none());
        assert!(select(&[1], 1, 0, |_, _| true).is_none());
        assert!(
            select(&[1], 1, 0, |_, _| false)
                .unwrap()
                .instances
                .is_empty()
        );
    }
}
