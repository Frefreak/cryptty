// QWERTY preference: home-row keys first, then nearby reaches.
const KEYS: &[u8] = b"fjdkhgslaurieowvntmcbpyxqz";
const LEFT: &[u8] = b"qwertasdfgzxcvb";

pub(super) fn generate(count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    let mut labels: Vec<(usize, String)> = KEYS
        .iter()
        .enumerate()
        .filter(|(_, key)| **key != b's')
        .map(|(cost, &key)| (cost, char::from(key).to_string()))
        .collect();
    // Equal lengths keep immediate copying unambiguous. Each expansion
    // produces fewer than KEYS.len() times the requested target count.
    while labels.len() < count {
        labels = labels
            .into_iter()
            .flat_map(|(cost, prefix)| {
                let left = LEFT.contains(prefix.as_bytes().last().unwrap());
                KEYS.iter().enumerate().map(move |(rank, &key)| {
                    let same_hand = left == LEFT.contains(&key);
                    let cost = cost + rank + usize::from(same_hand) * 8;
                    (cost, format!("{prefix}{}", char::from(key)))
                })
            })
            .collect();
    }
    labels.sort_by_key(|(cost, _)| *cost);
    labels
        .into_iter()
        .take(count)
        .map(|(_, label)| label)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn labels_stay_unique_and_prefix_free_across_capacity_boundaries() {
        for count in [0, 1, 25, 26, 650, 651] {
            let labels = generate(count);
            assert_eq!(labels.len(), count);
            assert_eq!(labels.iter().collect::<HashSet<_>>().len(), count);
            assert!(labels.iter().all(|label| !label.starts_with('s')));
            if let Some(first) = labels.first() {
                assert!(labels.iter().all(|label| label.len() == first.len()));
            }
        }
    }

    #[test]
    fn favors_easy_keys_and_alternating_hands() {
        let single = generate(25);
        assert_eq!(single[0], "f");
        assert!(
            single.iter().position(|s| s == "j")
                < single.iter().position(|s| s == "a")
        );
        let pairs = generate(650);
        let rank = |label| pairs.iter().position(|s| s == label).unwrap();
        assert!(rank("fh") < rank("fr"));
        assert!(rank("fj") < rank("ff"));
        assert!(pairs.iter().any(|s| s.ends_with('s')));
        assert_eq!(pairs, generate(650));
    }
}
