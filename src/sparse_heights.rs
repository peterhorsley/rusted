use std::collections::HashMap;

pub(crate) struct SparseHeights {
    len: usize,
    minimum: f64,
    heights: HashMap<usize, f64>,
    // Unknown entries use the minimum; the sparse Fenwick tree stores measured differences.
    tree: HashMap<usize, f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_fractional_heights_after_remeasurement() {
        let mut heights = SparseHeights::new(4, 20.25);
        assert_eq!(SparseHeights::new(0, 20.25).index_at(0.0), None);
        heights.set(0, 35.5);
        heights.set(2, 50.75);
        heights.set(0, 21.5);
        let expected = [0.0, 21.5, 41.75, 92.5, 112.75];
        for (index, &offset) in expected.iter().enumerate() {
            assert_eq!(heights.prefix(index), offset);
            if index < 4 {
                assert_eq!(heights.index_at(offset), Some(index));
                assert_eq!(heights.index_at(expected[index + 1] - 0.01), Some(index));
            }
        }
    }
}

impl SparseHeights {
    pub(crate) fn new(len: usize, minimum: f64) -> Self {
        Self { len, minimum, heights: HashMap::new(), tree: HashMap::new() }
    }

    #[cfg(test)]
    pub(crate) fn measured_count(&self) -> usize {
        self.heights.len()
    }

    pub(crate) fn set(&mut self, index: usize, height: f64) {
        let height = height.max(self.minimum);
        let old = self.heights.insert(index, height).unwrap_or(self.minimum);
        let delta = height - old;
        if delta == 0.0 {
            return;
        }
        let mut i = index + 1;
        while i <= self.len {
            *self.tree.entry(i).or_default() += delta;
            i += i & i.wrapping_neg();
        }
    }

    pub(crate) fn prefix(&self, count: usize) -> f64 {
        let mut i = count;
        let mut height = count as f64 * self.minimum;
        while i > 0 {
            height += self.tree.get(&i).copied().unwrap_or(0.0);
            i &= i - 1;
        }
        height
    }

    pub(crate) fn index_at(&self, offset: f64) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let mut bit = 1usize;
        while bit <= self.len / 2 {
            bit *= 2;
        }
        let (mut index, mut height) = (0, 0.0);
        while bit != 0 {
            let next = index + bit;
            if next <= self.len {
                let candidate = height + (next & next.wrapping_neg()) as f64 * self.minimum
                    + self.tree.get(&next).copied().unwrap_or(0.0);
                if candidate <= offset {
                    index = next;
                    height = candidate;
                }
            }
            bit /= 2;
        }
        Some(index.min(self.len - 1))
    }
}
