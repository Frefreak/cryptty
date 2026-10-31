// QWERTY preference: home-row keys first, then nearby reaches.
const KEYS: &[u8] = b"fjdkhgslaurieowvntmcbpyxqz";
const LEFT: &[u8] = b"qwertasdfgzxcvb";

// Field order keeps awkward finger movements behind all easier options.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Cost {
    same_finger_moves: usize,
    repeats: usize,
    same_hand_moves: usize,
    key_rank: usize,
}

fn finger(key: u8) -> u8 {
    match key {
        b'q' | b'a' | b'z' => 0,
        b'w' | b's' | b'x' => 1,
        b'e' | b'd' | b'c' => 2,
        b'r' | b't' | b'f' | b'g' | b'v' | b'b' => 3,
        b'y' | b'u' | b'h' | b'j' | b'n' | b'm' => 4,
        b'i' | b'k' => 5,
        b'o' | b'l' => 6,
        b'p' => 7,
        _ => unreachable!("label keys are QWERTY letters"),
    }
}

pub(super) fn generate(count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    let mut labels: Vec<(Cost, String)> = KEYS
        .iter()
        .enumerate()
        .filter(|(_, key)| **key != b's')
        .map(|(key_rank, &key)| {
            let cost = Cost {
                key_rank,
                ..Cost::default()
            };
            (cost, char::from(key).to_string())
        })
        .collect();
    // Equal lengths keep immediate copying unambiguous. Each expansion
    // produces fewer than KEYS.len() times the requested target count.
    while labels.len() < count {
        labels = labels
            .into_iter()
            .flat_map(|(cost, prefix)| {
                let previous = *prefix.as_bytes().last().unwrap();
                KEYS.iter().enumerate().map(move |(rank, &key)| {
                    let mut cost = cost;
                    if previous == key {
                        cost.repeats += 1;
                    } else if finger(previous) == finger(key) {
                        cost.same_finger_moves += 1;
                    } else if LEFT.contains(&previous) == LEFT.contains(&key) {
                        cost.same_hand_moves += 1;
                    }
                    cost.key_rank += rank;
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

    #[test]
    fn respects_requested_comfort_groups() {
        let pairs = generate(650);
        let rank = |label| pairs.iter().position(|s| s == label).unwrap();
        for easy in ["fj", "dk", "fk", "dj", "fl", "fh"] {
            for same_hand in ["fa", "fd"] {
                assert!(rank(easy) < rank(same_hand));
            }
        }
        for same_hand in ["fa", "fd"] {
            for repeat in ["ff", "jj", "oo", "qq"] {
                assert!(rank(same_hand) < rank(repeat));
                assert!(rank(repeat) < rank("jh"));
            }
        }
        assert!(rank("fj") < rank("dk"));
    }

    #[test]
    fn exhausts_labels_without_same_finger_movement_first() {
        let finger_groups: &[&[u8]] = &[
            b"qaz", b"wsx", b"edc", b"rtfgvb", b"yuhjnm", b"ik", b"ol", b"p",
        ];
        let has_movement = |label: &str| {
            label.as_bytes().windows(2).any(|pair| {
                pair[0] != pair[1]
                    && finger_groups.iter().any(|group| {
                        group.contains(&pair[0]) && group.contains(&pair[1])
                    })
            })
        };
        for capacity in [650, 16_900] {
            let labels = generate(capacity);
            let first_awkward =
                labels.iter().position(|label| has_movement(label)).unwrap();
            assert!(first_awkward > 0);
            assert!(labels[first_awkward..].iter().all(|s| has_movement(s)));
            assert!(generate(first_awkward).iter().all(|s| !has_movement(s)));
            assert!(has_movement(&generate(first_awkward + 1)[first_awkward]));
        }
    }
}
