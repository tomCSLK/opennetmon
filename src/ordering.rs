//! Display order of groups, devices and sensors: an optional manual position (set by an admin)
//! first, otherwise a natural name order ("Port 2" before "Port 10", case ignored).

use std::cmp::Ordering;
use std::collections::HashMap;

/// Highest accepted position from a form; larger numbers are clamped.
const MAX_POSITION: i64 = 100_000;

/// Compares names like a person would: runs of digits as numbers, letters without case.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b), // equal ignoring case: stable tie-break
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Manually placed items first (ascending), then the rest.
pub fn position_cmp(a: Option<i64>, b: Option<i64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Positions typed into an order form: `<prefix>_<id>` → number.
pub fn positions_from_form(fields: &HashMap<String, String>, prefix: &str) -> HashMap<i64, i64> {
    let prefix = format!("{prefix}_");
    fields
        .iter()
        .filter_map(|(k, v)| {
            let id = k.strip_prefix(&prefix)?.parse().ok()?;
            let position = v.trim().parse::<i64>().ok()?.clamp(1, MAX_POSITION);
            Some((id, position))
        })
        .collect()
}

/// The new order of `current` (ids in today's order): by the wanted position, ties and items
/// without a position keep their relative order. Ids not in `current` are ignored.
pub fn reorder(current: &[i64], wanted: &HashMap<i64, i64>) -> Vec<i64> {
    let mut keyed: Vec<(i64, usize, i64)> = current
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let fallback = i64::try_from(i).unwrap_or(i64::MAX).saturating_add(1);
            (wanted.get(id).copied().unwrap_or(fallback), i, *id)
        })
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, _, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_names_sort_as_numbers_and_case_is_ignored() {
        let mut names = vec![
            "Traffic Port 10",
            "traffic Port 2",
            "Traffic Port 1",
            "PoE Port 03",
            "PoE Port 2",
            "gigabitEthernet 1/0/10",
            "gigabitEthernet 1/0/9",
            "Uptime",
            "",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "",
                "gigabitEthernet 1/0/9",
                "gigabitEthernet 1/0/10",
                "PoE Port 2",
                "PoE Port 03",
                "Traffic Port 1",
                "traffic Port 2",
                "Traffic Port 10",
                "Uptime",
            ]
        );
        assert_eq!(natural_cmp("a", "A"), "a".cmp("A"), "case only breaks ties");
        assert_eq!(
            natural_cmp("x99999999999999999999999", "x1"),
            Ordering::Greater
        );
        assert_eq!(natural_cmp("Ä", "Ä"), Ordering::Equal);
    }

    #[test]
    fn placed_items_come_first() {
        let mut items = [(None, "c"), (Some(2), "b"), (None, "a"), (Some(1), "z")];
        items.sort_by(|x, y| position_cmp(x.0, y.0).then_with(|| natural_cmp(x.1, y.1)));
        let names: Vec<_> = items.iter().map(|i| i.1).collect();
        assert_eq!(names, ["z", "b", "a", "c"]);
    }

    #[test]
    fn form_positions_reorder_and_keep_ties_stable() {
        let fields: HashMap<String, String> = [
            ("s_10", "3"),
            ("s_11", " 1 "),
            ("s_12", "x"),
            ("s_13", "1"),
            ("d_10", "9"),
            ("s_99", "1"),
            ("s_14", "-5"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let wanted = positions_from_form(&fields, "s");
        assert_eq!(wanted.get(&14), Some(&1), "clamped");
        assert!(!wanted.contains_key(&12));
        // Today: 10, 11, 12, 13, 14. 12 keeps its implicit position 3, after 10 (moved to 3 too).
        assert_eq!(
            reorder(&[10, 11, 12, 13, 14], &wanted),
            [11, 13, 14, 10, 12]
        );
        assert_eq!(reorder(&[1, 2], &HashMap::new()), [1, 2]);
    }
}
