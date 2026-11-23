//! Preferences that every window shares through one file. Each project window is its own
//! process and writes the whole file from memory, so plain saving let the last window to save
//! undo what the others changed. [`merge3`] keeps both: what this window changed since it last
//! read or wrote the file (`base`) wins, field by field; everything else is what the file holds
//! now (other windows' changes), which this window then takes over too.

use serde_json::Value;

/// Three-way merge of `ours` into `disk` since `base`. Objects merge key by key (recursively);
/// any other value is taken whole: this window's if it changed it, else the file's.
pub fn merge3(base: &Value, ours: &Value, disk: &Value) -> Value {
    if ours == base {
        return disk.clone();
    }
    match (base, ours, disk) {
        (Value::Object(base), Value::Object(ours), Value::Object(disk)) => {
            let mut out = disk.clone();
            for (key, mine) in ours {
                let was = base.get(key).unwrap_or(&Value::Null);
                if mine == was {
                    continue;
                }
                let theirs = disk.get(key).unwrap_or(&Value::Null);
                out.insert(key.clone(), merge3(was, mine, theirs));
            }
            for key in base.keys() {
                if !ours.contains_key(key) {
                    out.remove(key);
                }
            }
            Value::Object(out)
        }
        _ => ours.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Two windows change different preferences: both changes survive; the same one: the
    /// later save wins; nothing this window left alone undoes another's change.
    #[test]
    fn each_window_keeps_its_own_changes_and_takes_the_others() {
        let base = json!({"view": {"theme": "System", "font": 16, "sidebar": true}, "n": 1});
        let ours = json!({"view": {"theme": "Dark", "font": 16, "sidebar": true}, "n": 1});
        let disk = json!({"view": {"theme": "System", "font": 18, "sidebar": false}, "n": 2});
        assert_eq!(
            merge3(&base, &ours, &disk),
            json!({"view": {"theme": "Dark", "font": 18, "sidebar": false}, "n": 2})
        );
        // Nothing changed here: the file as it is.
        assert_eq!(merge3(&base, &base, &disk), disk);
        // Both changed the same value: this (later) save wins.
        let theirs = json!({"view": {"theme": "Light", "font": 16, "sidebar": true}, "n": 1});
        assert_eq!(merge3(&base, &ours, &theirs)["view"]["theme"], "Dark");
        // A value whose type changed is taken whole.
        assert_eq!(
            merge3(
                &json!({"a": [1]}),
                &json!({"a": [1, 2]}),
                &json!({"a": [3]})
            ),
            json!({"a": [1, 2]})
        );
    }
}
