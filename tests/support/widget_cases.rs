use serde_json::{json, Value};

pub const NODES: &[&str] = &["row", "column", "label", "leader", "spacer", "list"];
fn label(text: &str) -> Value {
    json!({"type":"label","text":text,"role":"label"})
}
fn row(children: Vec<Value>) -> Value {
    json!({"type":"row","children":children})
}
fn column(children: Vec<Value>) -> Value {
    json!({"type":"column","children":children})
}
pub fn declaration(node: &str) -> Value {
    let tree = match node {
        "row" => row(vec![
            column(vec![label("A"), label("B")]),
            column(vec![label("C"), label("D")]),
        ]),
        "column" => column(vec![
            row(vec![label("A"), label("B")]),
            row(vec![]),
            column(vec![]),
            row(vec![label("C"), label("D")]),
        ]),
        "label" => row(vec![
            column(vec![]),
            column(vec![label("A界"), label("")]),
            label("B"),
        ]),
        "leader" => row(vec![
            column(vec![label("A")]),
            json!({"type":"leader"}),
            column(vec![label("B")]),
        ]),
        "spacer" => column(vec![
            row(vec![
                label("A"),
                json!({"type":"spacer","size":0}),
                json!({"type":"spacer","size":2}),
                label("B"),
            ]),
            json!({"type":"spacer","size":1}),
            label("C"),
        ]),
        "list" => {
            json!({"type":"list","entries":[row(vec![label("A"),label("B")]), {"type":"list","entries":[]}, column(vec![label("C"),label("D")])]})
        }
        _ => panic!("unknown example: {node}"),
    };
    json!({"herdr":"widget_tree","version":1,"zones":[],"composition":{"priority":100,"tree":column(vec![row(vec![column(vec![tree])]), label("END")])}})
}
pub fn expected(node: &str, width: usize) -> Vec<String> {
    let rows = match node {
        "row" => vec!["AC".into(), "BD".into(), "END".into()],
        "column" => vec!["AB".into(), "CD".into(), "END".into()],
        "label" => vec!["A界 B".into(), "".into(), "END".into()],
        "leader" => vec![
            format!("A{}B", "·".repeat(width.saturating_sub(2))),
            "END".into(),
        ],
        "spacer" => vec!["A  B".into(), "".into(), "C".into(), "END".into()],
        "list" => vec!["AB".into(), "C".into(), "D".into(), "END".into()],
        _ => panic!("unknown example: {node}"),
    };
    rows.into_iter()
        .enumerate()
        .map(|(index, row): (usize, String)| {
            if node == "label" && index == 0 {
                if width < 3 {
                    "A".into()
                } else if width == 3 {
                    "A界".into()
                } else {
                    row
                }
            } else {
                row.chars()
                    .take(width)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            }
        })
        .collect()
}
pub fn assert_rows(node: &str, rows: &[String], width: usize) {
    let expected = expected(node, width);
    for (index, expected) in expected.iter().enumerate() {
        assert_eq!(
            rows[index].trim_end(),
            expected,
            "{node} row {index} at width {width}: {rows:?}"
        );
    }
    assert!(
        rows[expected.len()..]
            .iter()
            .all(|row| row.trim().is_empty()),
        "{node}: unexpected content {rows:?}"
    );
}
