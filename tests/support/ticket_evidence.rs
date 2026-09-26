//! Cross-process comparisons for the public Rust dispatch_ticket fixture.
//! Never retry provider calls or ignore differences in ranking/source evidence.
use serde_json::{Value, json};

const SKIPPED: &str =
    "Code reader changed; graph enrichment and coverage are unavailable for this request";

fn skipped_facets(response: &Value) -> bool {
    let code = &response["code"];
    let before = code["reader_generation_before"].as_u64().unwrap();
    let after = code["reader_generation_after"].as_u64().unwrap();
    before != after
        && code["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["facets"] == json!([]))
        && code["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning == SKIPPED)
}

pub fn assert_same_dispatch_items(left: &Value, right: &Value) {
    let skip = [skipped_facets(left), skipped_facets(right)];
    let mut rows = [
        left["code"]["items"].clone(),
        right["code"]["items"].clone(),
    ];
    for (position, response) in [left, right].iter().enumerate() {
        assert_eq!(rows[position].as_array().unwrap().len(), 1);
        if skip[position] {
            println!(
                "ticket_fixture_skipped_facets={}",
                json!({"before":response["code"]["reader_generation_before"],
                    "after":response["code"]["reader_generation_after"],
                    "warnings":response["code"]["warnings"]})
            );
        }
        for item in rows[position].as_array_mut().unwrap() {
            assert_eq!(item["name"], "dispatch_ticket");
            if !skip[position] {
                for (facet, value) in [
                    ("kind", "Function"),
                    ("language", "rust"),
                    ("visibility", "Public"),
                ] {
                    assert!(
                        item["facets"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|entry| { entry["facet"] == facet && entry["value"] == value })
                    );
                }
            }
            if skip.contains(&true) {
                item.as_object_mut().unwrap().remove("facets");
            }
        }
    }
    assert_eq!(rows[0], rows[1]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(skipped: bool) -> Value {
        let facets = if skipped {
            json!([])
        } else {
            json!([
                {"facet":"kind","value":"Function"},
                {"facet":"language","value":"rust"},
                {"facet":"visibility","value":"Public"}
            ])
        };
        json!({"code": {
            "reader_generation_before":1, "reader_generation_after":if skipped {2} else {1},
            "warnings":if skipped {vec![SKIPPED]} else {vec![]},
            "items":[{"name":"dispatch_ticket","fusion_score":0.1,"facets":facets}]
        }})
    }

    #[test]
    fn explicit_drift_can_omit_facets_on_either_side() {
        assert_same_dispatch_items(&response(true), &response(false));
        assert_same_dispatch_items(&response(false), &response(true));
    }

    #[test]
    #[should_panic]
    fn explicit_drift_never_hides_score_changes() {
        let mut changed = response(true);
        changed["code"]["items"][0]["fusion_score"] = json!(0.2);
        assert_same_dispatch_items(&response(false), &changed);
    }

    #[test]
    #[should_panic]
    fn missing_warning_never_justifies_empty_facets() {
        let mut changed = response(true);
        changed["code"]["warnings"] = json!([]);
        assert_same_dispatch_items(&response(false), &changed);
    }

    #[test]
    #[should_panic]
    fn stable_reader_never_justifies_empty_facets() {
        let mut changed = response(true);
        changed["code"]["reader_generation_after"] = json!(1);
        assert_same_dispatch_items(&response(false), &changed);
    }

    #[test]
    #[should_panic]
    fn empty_results_never_count_as_preserved_evidence() {
        let mut empty = response(false);
        empty["code"]["items"] = json!([]);
        assert_same_dispatch_items(&empty, &empty);
    }
}
