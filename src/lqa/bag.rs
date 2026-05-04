//! Bag-semantics combinators for LQA.
//!
//! openCypher uses **bag** (multiset) semantics by default: duplicate rows are
//! preserved unless `DISTINCT` is explicitly requested.  This module provides
//! generic bag operations used by:
//!
//! - The LQA → SPARQL lowerer (Phase 4), to reason about duplicate-preservation.
//! - The differential testing oracle ([`polygraph_difftest`]), to compare
//!   expected and actual result bags using Cypher null-propagating equality.
//!
//! # Terminology
//!
//! | Term | Meaning |
//! |------|---------|
//! | Bag | A `Vec<T>` that may contain duplicates (multiset). |
//! | Set-bag | A `Vec<T>` with duplicates removed (result of `DISTINCT`). |
//! | Row | A single binding tuple (one value per variable in scope). |
//!
//! # Null equality
//!
//! openCypher 9 §6.1: `null = null` evaluates to `null`, not `true`.  For bag
//! equality comparisons in the differential oracle, a separate null-aware
//! equality function ([`cypher_eq`]) is provided.

use std::collections::HashMap;
use std::hash::Hash;

// ── Bag type alias ────────────────────────────────────────────────────────────

/// A multiset of values.  Duplicates are significant; order is not
/// (for set-equality comparisons use [`bag_equal`]).
pub type Bag<T> = Vec<T>;

// ── Set operations ────────────────────────────────────────────────────────────

/// UNION ALL — concatenate two bags, preserving duplicates.
///
/// openCypher 9 §4.7: `UNION ALL` returns all rows from both sides,
/// including duplicates.
pub fn union_all<T>(left: Bag<T>, right: Bag<T>) -> Bag<T> {
    let mut result = left;
    result.extend(right);
    result
}

/// UNION — union of two bags with deduplication.
///
/// openCypher 9 §4.7: `UNION` (without `ALL`) removes duplicates from the
/// combined result.  Requires `T: Eq + Hash`.
pub fn union_distinct<T: Eq + Hash + Clone>(left: &[T], right: &[T]) -> Bag<T> {
    use std::collections::LinkedList;
    // Preserve first-occurrence order while deduplicating.
    let mut seen: HashMap<&T, ()> = HashMap::new();
    let mut out: Vec<T> = Vec::new();
    for item in left.iter().chain(right.iter()) {
        if seen.insert(item, ()).is_none() {
            out.push(item.clone());
        }
    }
    out
}

/// Cartesian product — every combination of `left × right` rows.
///
/// openCypher 9 §4.1: multiple comma-separated patterns in a single `MATCH`
/// without a shared variable produce a Cartesian product.
pub fn cross<T: Clone, U: Clone>(left: &[T], right: &[U]) -> Vec<(T, U)> {
    let mut out = Vec::with_capacity(left.len() * right.len());
    for l in left {
        for r in right {
            out.push((l.clone(), r.clone()));
        }
    }
    out
}

/// Map each element of a bag through a function (projection).
///
/// Corresponds to `RETURN` / `WITH` projections in openCypher.
pub fn project<T, U, F: Fn(&T) -> U>(bag: &[T], f: F) -> Bag<U> {
    bag.iter().map(f).collect()
}

/// Filter a bag by a predicate (selection).
///
/// Corresponds to `WHERE` and `WITH … WHERE` in openCypher.
/// Rows for which `predicate` returns `false` OR whose evaluation would
/// return `null` are discarded (Cypher three-valued logic filter).
pub fn select<T, F: Fn(&T) -> Option<bool>>(bag: Bag<T>, predicate: F) -> Bag<T> {
    bag.into_iter().filter(|row| predicate(row) == Some(true)).collect()
}

/// Group rows by a key function.
///
/// Returns a `HashMap<K, Bag<T>>` where each key maps to the bag of rows that
/// produced that key.  Used by `GroupBy` in the LQA lowering.
pub fn group_by<T, K, F>(bag: Bag<T>, key_fn: F) -> HashMap<K, Bag<T>>
where
    K: Eq + Hash,
    F: Fn(&T) -> K,
{
    let mut map: HashMap<K, Bag<T>> = HashMap::new();
    for row in bag {
        map.entry(key_fn(&row)).or_default().push(row);
    }
    map
}

/// Natural join — join two bags on a shared key, combining matching rows.
///
/// Used when two pattern elements in a `MATCH` share a variable.  All
/// combinations of left and right rows with equal key values are emitted.
///
/// `key_fn_l` extracts the join key from a left row; `key_fn_r` from a
/// right row; `combine` merges a matching pair into an output row.
pub fn natural_join<L, R, K, O, FL, FR, C>(
    left: &[L],
    right: &[R],
    key_fn_l: FL,
    key_fn_r: FR,
    combine: C,
) -> Vec<O>
where
    K: Eq + Hash,
    FL: Fn(&L) -> K,
    FR: Fn(&R) -> K,
    C: Fn(&L, &R) -> O,
{
    // Build a hash index on the right side.
    let mut right_index: HashMap<K, Vec<&R>> = HashMap::new();
    for row in right {
        right_index.entry(key_fn_r(row)).or_default().push(row);
    }
    let mut out = Vec::new();
    for l in left {
        let k = key_fn_l(l);
        if let Some(matching) = right_index.get(&k) {
            for r in matching {
                out.push(combine(l, r));
            }
        }
    }
    out
}

/// Left outer join — like `natural_join` but preserves left rows with no match.
///
/// Implements `OPTIONAL MATCH`: for each left row that has no matching right
/// row, a combined row with the right columns set to `None` is emitted via
/// `null_combine`.
pub fn left_outer_join<L, R, K, C, FL, FR, NC, O>(
    left: &[L],
    right: &[R],
    key_fn_l: FL,
    key_fn_r: FR,
    combine: C,
    null_combine: NC,
) -> Vec<O>
where
    K: Eq + Hash,
    FL: Fn(&L) -> K,
    FR: Fn(&R) -> K,
    C: Fn(&L, &R) -> O,
    NC: Fn(&L) -> O,
{
    let mut right_index: HashMap<K, Vec<&R>> = HashMap::new();
    for row in right {
        right_index.entry(key_fn_r(row)).or_default().push(row);
    }
    let mut out = Vec::new();
    for l in left {
        let k = key_fn_l(l);
        match right_index.get(&k) {
            Some(matching) => {
                for r in matching {
                    out.push(combine(l, r));
                }
            }
            None => {
                out.push(null_combine(l));
            }
        }
    }
    out
}

// ── Bag equality ─────────────────────────────────────────────────────────────

/// Compares two bags for equality, ignoring row order.
///
/// Returns `true` iff both bags contain the same multiset of rows.
/// Row equality uses standard `==` (i.e. requires `Eq`).
///
/// For the differential oracle use case, rows should implement a
/// null-propagating equality where Cypher `null` compares as equal to
/// `null` for *bag-membership* purposes (even though `null = null` is
/// `null` in Cypher expressions).  The oracle wraps values suitably before
/// calling this function.
pub fn bag_equal<T: Eq + Hash>(left: &[T], right: &[T]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut counts: HashMap<&T, i64> = HashMap::new();
    for item in left {
        *counts.entry(item).or_insert(0) += 1;
    }
    for item in right {
        let entry = counts.entry(item).or_insert(0);
        *entry -= 1;
        if *entry < 0 {
            return false;
        }
    }
    counts.values().all(|&c| c == 0)
}

/// Check whether a row `r` appears in the bag `bag` (membership test).
pub fn bag_contains<T: Eq>(bag: &[T], r: &T) -> bool {
    bag.contains(r)
}

/// Return the multiplicity of `item` in the bag (how many times it appears).
pub fn multiplicity<T: Eq>(bag: &[T], item: &T) -> usize {
    bag.iter().filter(|x| *x == item).count()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_all_preserves_duplicates() {
        let a = vec![1, 2, 2];
        let b = vec![2, 3];
        let result = union_all(a, b);
        assert_eq!(result, vec![1, 2, 2, 2, 3]);
    }

    #[test]
    fn union_distinct_deduplicates() {
        let a = vec![1, 2, 2, 3];
        let b = vec![2, 3, 4];
        let result = union_distinct(&a, &b);
        // Order: first occurrence preserved; {1,2,3,4}
        assert_eq!(result.len(), 4);
        assert!(result.contains(&1));
        assert!(result.contains(&2));
        assert!(result.contains(&3));
        assert!(result.contains(&4));
    }

    #[test]
    fn union_distinct_empty_inputs() {
        let empty: Vec<i32> = vec![];
        assert_eq!(union_distinct(&empty, &empty), empty);
        assert_eq!(union_distinct(&[1, 2], &empty), vec![1, 2]);
        assert_eq!(union_distinct(&empty, &[3, 4]), vec![3, 4]);
    }

    #[test]
    fn cross_all_combinations() {
        let left = vec![1, 2];
        let right = vec!['a', 'b'];
        let result = cross(&left, &right);
        assert_eq!(result.len(), 4);
        assert!(result.contains(&(1, 'a')));
        assert!(result.contains(&(1, 'b')));
        assert!(result.contains(&(2, 'a')));
        assert!(result.contains(&(2, 'b')));
    }

    #[test]
    fn cross_empty_produces_empty() {
        let empty: Vec<i32> = vec![];
        assert!(cross(&empty, &[1, 2, 3]).is_empty());
        assert!(cross(&[1, 2, 3], &empty).is_empty());
    }

    #[test]
    fn project_maps_values() {
        let bag = vec![1, 2, 3];
        let result = project(&bag, |x| x * 2);
        assert_eq!(result, vec![2, 4, 6]);
    }

    #[test]
    fn select_filters_false_and_null() {
        // 0 → Some(false) = filtered
        // 1 → Some(true) = kept
        // 2 → None (null) = filtered
        let bag = vec![0i32, 1, 2];
        let result = select(bag, |x| match x {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        });
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn group_by_even_odd() {
        let bag = vec![1, 2, 3, 4, 5];
        let groups = group_by(bag, |x| x % 2 == 0);
        assert_eq!(groups[&false], vec![1, 3, 5]);
        assert_eq!(groups[&true], vec![2, 4]);
    }

    #[test]
    fn group_by_empty() {
        let bag: Vec<i32> = vec![];
        let groups = group_by(bag, |x| x % 2);
        assert!(groups.is_empty());
    }

    #[test]
    fn bag_equal_same_order() {
        assert!(bag_equal(&[1, 2, 3], &[1, 2, 3]));
    }

    #[test]
    fn bag_equal_different_order() {
        assert!(bag_equal(&[3, 1, 2], &[1, 2, 3]));
    }

    #[test]
    fn bag_equal_with_duplicates() {
        assert!(bag_equal(&[1, 2, 2], &[2, 1, 2]));
        assert!(!bag_equal(&[1, 2, 2], &[1, 2, 3]));
    }

    #[test]
    fn bag_equal_different_sizes() {
        assert!(!bag_equal(&[1, 2], &[1, 2, 3]));
    }

    #[test]
    fn bag_equal_empty() {
        let empty: Vec<i32> = vec![];
        assert!(bag_equal(&empty, &empty));
    }

    #[test]
    fn bag_contains_found() {
        let b = vec![1, 2, 3];
        assert!(bag_contains(&b, &2));
        assert!(!bag_contains(&b, &5));
    }

    #[test]
    fn multiplicity_count() {
        let b = vec![1, 2, 2, 3, 2];
        assert_eq!(multiplicity(&b, &2), 3);
        assert_eq!(multiplicity(&b, &1), 1);
        assert_eq!(multiplicity(&b, &5), 0);
    }

    #[test]
    fn left_outer_join_with_matches() {
        // Left: [(1,"a"), (2,"b"), (3,"c")]
        // Right: [(1,10), (2,20)]
        // Expected: [(1,"a",Some(10)), (2,"b",Some(20)), (3,"c",None)]
        let left = vec![(1u32, "a"), (2u32, "b"), (3u32, "c")];
        let right = vec![(1u32, 10i32), (2u32, 20i32)];
        let result: Vec<(u32, &str, Option<i32>)> = left_outer_join(
            &left,
            &right,
            |(k, _)| *k,
            |(k, _)| *k,
            |(k, v), (_, rv)| (*k, *v, Some(*rv)),
            |(k, v)| (*k, *v, None),
        );
        assert_eq!(result.len(), 3);
        assert!(result.contains(&(1, "a", Some(10))));
        assert!(result.contains(&(2, "b", Some(20))));
        assert!(result.contains(&(3, "c", None)));
    }

    #[test]
    fn natural_join_matching_rows() {
        // Left: [(1,"Alice"), (2,"Bob")]
        // Right: [(1,"KNOWS"), (1,"LIKES")]
        // Expected: [(1,"Alice","KNOWS"), (1,"Alice","LIKES")]
        let left = vec![(1u32, "Alice"), (2u32, "Bob")];
        let right = vec![(1u32, "KNOWS"), (1u32, "LIKES")];
        let result: Vec<(u32, &str, &str)> = natural_join(
            &left,
            &right,
            |(k, _)| *k,
            |(k, _)| *k,
            |(k, lv), (_, rv)| (*k, *lv, *rv),
        );
        assert_eq!(result.len(), 2);
        assert!(result.contains(&(1, "Alice", "KNOWS")));
        assert!(result.contains(&(1, "Alice", "LIKES")));
    }
}
