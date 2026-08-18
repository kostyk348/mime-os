//! Diff строк для совместного редактирования (LCS).
//!
//! Сравнивает старый список строк (с id) и новый (текст) → последовательность
//! дельт: Add (после id или в конец), Set (id, новый текст), Del (id).

/// Одна операция редактирования документа.
#[derive(Debug, Clone, PartialEq)]
pub enum LineOp {
    Add { after: Option<String>, text: String },
    Set { id: String, text: String },
    Del { id: String },
}

/// LCS-дифф: (старый индекс, новый индекс) совпадения.
fn lcs_matches(old: &[String], new: &[String]) -> Vec<(usize, usize)> {
    let n = old.len();
    let m = new.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Построить дельты между старым документом (id, text) и новым (text).
pub fn diff_lines(old: &[(String, String)], new: &[String]) -> Vec<LineOp> {
    let old_texts: Vec<String> = old.iter().map(|(_, t)| t.clone()).collect();
    let matches = lcs_matches(&old_texts, new);
    let matched_old: std::collections::HashSet<usize> = matches.iter().map(|(i, _)| *i).collect();
    let matched_new: std::collections::HashSet<usize> = matches.iter().map(|(_, j)| *j).collect();

    let mut ops: Vec<LineOp> = Vec::new();
    // id последней старой строки, "присутствующей" в новом (для after)
    let mut last_kept: Option<String> = None;
    // подряд идущие вставки: вторая ссылается на "__next__" (после первой)
    let mut last_op_add = false;

    // Слияние: идём по позициям нового и старого одновременно
    let mut mi = 0usize; // индекс в matches
    for j in 0..new.len() {
        if matched_new.contains(&j) {
            // совпавшая строка: если текст изменился (set), обновить last_kept
            let (oi, _) = matches[mi];
            if old[oi].1 != new[j] {
                ops.push(LineOp::Set { id: old[oi].0.clone(), text: new[j].clone() });
            }
            last_kept = Some(old[oi].0.clone());
            last_op_add = false;
            mi += 1;
        } else {
            // новая строка без матча: add после last_kept (или после предыдущей вставки)
            let after = if last_op_add { Some("__next__".to_string()) } else { last_kept.clone() };
            ops.push(LineOp::Add { after, text: new[j].clone() });
            last_op_add = true;
        }
    }
    // удалённые старые строки
    for (i, (id, _)) in old.iter().enumerate() {
        if !matched_old.contains(&i) {
            ops.push(LineOp::Del { id: id.clone() });
        }
    }
    ops
}

#[cfg(test)]
mod diff_tests {
    use super::*;
    #[test]
    fn edits_reproduce_new_document() {
        let old = vec![("a#1".to_string(), "План:".into()), ("a#2".to_string(), "X".into()), ("a#3".to_string(), "Y".into())];
        let new = vec!["План:".to_string(), "X v2".into(), "Z".into()];
        let ops = diff_lines(&old, &new);
        // применить ops к old -> new (id добавленных: a#2b, a#3b — по порядку)
        let mut cur: Vec<(String, String)> = old.clone();
        let mut nxt = 0u32;
        for op in &ops {
            match op {
                LineOp::Add { after, text } => {
                    nxt += 1;
                    let id = format!("a#{}b", nxt);
                    let eff_after = if after.as_deref() == Some("__next__") {
                        cur.last().map(|(i, _)| i.clone())
                    } else {
                        after.clone()
                    };
                    let pos = eff_after.as_ref().and_then(|a| cur.iter().position(|(i, _)| i == a)).map(|p| p + 1).unwrap_or(cur.len());
                    cur.insert(pos, (id, text.clone()));
                }
                LineOp::Set { id, text } => {
                    if let Some(e) = cur.iter_mut().find(|(i, _)| i == id) {
                        e.1 = text.clone();
                    }
                }
                LineOp::Del { id } => {
                    cur.retain(|(i, _)| i != id);
                }
            }
        }
        let texts: Vec<String> = cur.iter().map(|(_, t)| t.clone()).collect();
        assert_eq!(texts, new, "применённые дельты воспроизводят новый документ");
    }
    #[test]
    fn insert_in_middle() {
        let old = vec![("a#1".to_string(), "1".into()), ("a#2".to_string(), "3".into())];
        let new = vec!["1".to_string(), "2".into(), "3".into()];
        let ops = diff_lines(&old, &new);
        assert!(ops.iter().any(|o| matches!(o, LineOp::Add { after, .. } if after.as_deref() == Some("a#1"))));
    }
}
