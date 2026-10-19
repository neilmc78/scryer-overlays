//! Sort numbers for mirrored list collections.
//!
//! Each list provider holds a slot from 1 to 9, which is a block of sort
//! numbers: slot 1 is `!1000_`, slot 9 is `!9000_`. Each mirrored list has a
//! position inside its provider's block, numbered from 10 in steps of 5, so
//! the first IMDb list sorts as `!1010_IMDB Top 250 Movies` and the next as
//! `!1015_...`. A list moves only inside its provider's block; moving a
//! provider moves its whole block.

use std::collections::{BTreeMap, BTreeSet};

/// Provider blocks available: `!1000_` through `!9000_`.
pub const PROVIDER_SLOTS: i64 = 9;
const LIST_BASE: i64 = 10;
const LIST_STEP: i64 = 5;
/// Lists that fit in one provider's block of a thousand.
pub const MAX_LISTS_PER_PROVIDER: i64 = (1000 - LIST_BASE) / LIST_STEP;

/// Where a provider's block goes when it is free.
fn preferred_slot(provider: &str) -> Option<i64> {
    match provider {
        "imdb" => Some(1),
        "tmdb" => Some(2),
        "tvdb" => Some(3),
        "anilist" => Some(4),
        "trakt" => Some(5),
        "custom" => Some(9),
        _ => None,
    }
}

/// The sort number of the list at `position` (0-based) in `slot`'s block.
pub fn sort_number(slot: i64, position: i64) -> i64 {
    slot * 1000 + LIST_BASE + LIST_STEP * position
}

/// The sort title Plex sorts the list's collection by.
pub fn sort_title(slot: i64, position: i64, name: &str) -> String {
    format!("!{}_{}", sort_number(slot, position), name)
}

/// A slot for `provider`: its own when it holds one, else its preferred slot
/// when free, else the lowest free slot, else the slot of a provider with no
/// mirrored lists (`in_use` holds the providers that have some). Returns the
/// slot and the provider it was taken from, if any.
pub fn assign_slot(
    slots: &BTreeMap<String, i64>,
    provider: &str,
    in_use: &BTreeSet<String>,
) -> Result<(i64, Option<String>), String> {
    if let Some(slot) = slots.get(provider) {
        return Ok((*slot, None));
    }
    let taken = slots.values().copied().collect::<BTreeSet<_>>();
    if let Some(slot) = preferred_slot(provider).filter(|slot| !taken.contains(slot)) {
        return Ok((slot, None));
    }
    if let Some(slot) = (1..=PROVIDER_SLOTS).find(|slot| !taken.contains(slot)) {
        return Ok((slot, None));
    }
    slots
        .iter()
        .filter(|(holder, _)| !in_use.contains(*holder))
        .min_by_key(|(_, slot)| **slot)
        .map(|(holder, slot)| (*slot, Some(holder.clone())))
        .ok_or_else(|| {
            format!(
                "all {PROVIDER_SLOTS} Plex collection blocks are in use; switch off another \
                 provider's lists first"
            )
        })
}

/// Lists in display order: by stored position, lists without one last in id
/// order. The result's index is each list's position.
pub fn ordered(lists: &[(String, Option<i64>)]) -> Vec<String> {
    let mut lists = lists.to_vec();
    lists.sort_by(|(a_id, a_pos), (b_id, b_pos)| {
        (a_pos.is_none(), a_pos, a_id).cmp(&(b_pos.is_none(), b_pos, b_id))
    });
    lists.into_iter().map(|(id, _)| id).collect()
}

/// `order` with `id` swapped with its neighbour above (`up`) or below.
/// Unchanged at either end or when `id` is not in `order`.
pub fn move_list(order: &[String], id: &str, up: bool) -> Vec<String> {
    let mut order = order.to_vec();
    if let Some(index) = order.iter().position(|entry| entry == id) {
        let target = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|target| *target < order.len())
        };
        if let Some(target) = target {
            order.swap(index, target);
        }
    }
    order
}

/// Slots after moving `provider` one block up (`up`, towards `!1000_`) or
/// down. A provider in the target block trades places with it. Unchanged at
/// either end or when `provider` holds no slot.
pub fn move_provider(
    slots: &BTreeMap<String, i64>,
    provider: &str,
    up: bool,
) -> BTreeMap<String, i64> {
    let mut slots = slots.clone();
    let Some(current) = slots.get(provider).copied() else {
        return slots;
    };
    let target = if up { current - 1 } else { current + 1 };
    if !(1..=PROVIDER_SLOTS).contains(&target) {
        return slots;
    }
    if let Some(neighbour) = slots
        .iter()
        .find(|(_, slot)| **slot == target)
        .map(|(holder, _)| holder.clone())
    {
        slots.insert(neighbour, current);
    }
    slots.insert(provider.to_string(), target);
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(entries: &[(&str, i64)]) -> BTreeMap<String, i64> {
        entries
            .iter()
            .map(|(provider, slot)| ((*provider).to_string(), *slot))
            .collect()
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn numbers_run_from_ten_in_steps_of_five_inside_a_block() {
        assert_eq!(sort_number(1, 0), 1010);
        assert_eq!(sort_number(1, 1), 1015);
        assert_eq!(sort_number(4, 2), 4020);
        assert_eq!(
            sort_title(1, 0, "IMDB Top 250 Movies"),
            "!1010_IMDB Top 250 Movies"
        );
        assert_eq!(sort_number(1, MAX_LISTS_PER_PROVIDER - 1), 1995);
    }

    #[test]
    fn providers_take_their_preferred_block_then_the_lowest_free_one() {
        let none = BTreeSet::new();
        assert_eq!(assign_slot(&slots(&[]), "trakt", &none), Ok((5, None)));
        assert_eq!(assign_slot(&slots(&[]), "custom", &none), Ok((9, None)));
        assert_eq!(assign_slot(&slots(&[]), "simkl", &none), Ok((1, None)));
        assert_eq!(
            assign_slot(&slots(&[("simkl", 1)]), "imdb", &none),
            Ok((2, None)),
            "a preferred block already taken falls back to the lowest free one"
        );
        assert_eq!(
            assign_slot(&slots(&[("imdb", 1)]), "imdb", &none),
            Ok((1, None))
        );
    }

    #[test]
    fn a_full_set_reclaims_an_unused_block_or_refuses() {
        let full = slots(&[
            ("imdb", 1),
            ("tmdb", 2),
            ("tvdb", 3),
            ("anilist", 4),
            ("trakt", 5),
            ("mal", 6),
            ("simkl", 7),
            ("mdblist", 8),
            ("custom", 9),
        ]);
        let mut in_use = full.keys().cloned().collect::<BTreeSet<_>>();
        in_use.remove("simkl");
        assert_eq!(
            assign_slot(&full, "letterboxd", &in_use),
            Ok((7, Some("simkl".to_string())))
        );
        in_use.insert("simkl".into());
        assert!(assign_slot(&full, "letterboxd", &in_use).is_err());
    }

    #[test]
    fn lists_without_a_position_go_last() {
        let lists = vec![
            ("c".to_string(), None),
            ("b".to_string(), Some(1)),
            ("a".to_string(), Some(0)),
            ("d".to_string(), None),
        ];
        assert_eq!(ordered(&lists), ids(&["a", "b", "c", "d"]));
    }

    #[test]
    fn a_list_moves_one_place_and_stops_at_the_ends() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(move_list(&order, "b", true), ids(&["b", "a", "c"]));
        assert_eq!(move_list(&order, "b", false), ids(&["a", "c", "b"]));
        assert_eq!(move_list(&order, "a", true), order);
        assert_eq!(move_list(&order, "c", false), order);
        assert_eq!(move_list(&order, "missing", true), order);
    }

    #[test]
    fn moving_a_provider_swaps_whole_blocks() {
        let current = slots(&[("tvdb", 3), ("anilist", 4)]);
        // 4020 becomes 3020 and 3020 becomes 4020.
        let moved = move_provider(&current, "anilist", true);
        assert_eq!(moved, slots(&[("tvdb", 4), ("anilist", 3)]));
        assert_eq!(sort_number(moved["anilist"], 2), 3020);
        assert_eq!(sort_number(moved["tvdb"], 2), 4020);
        // An empty neighbouring block is simply taken.
        assert_eq!(
            move_provider(&slots(&[("imdb", 1)]), "imdb", false),
            slots(&[("imdb", 2)])
        );
        // The ends stay put.
        assert_eq!(
            move_provider(&slots(&[("imdb", 1)]), "imdb", true),
            slots(&[("imdb", 1)])
        );
        assert_eq!(
            move_provider(&slots(&[("custom", 9)]), "custom", false),
            slots(&[("custom", 9)])
        );
    }
}
