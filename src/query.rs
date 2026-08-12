//! X-Query v0.2 — общий движок запросов над заголовками.
//!
//! Работает на любом наборе заголовков (контейнеры EMLBox, записи tagdb).
//! Синтаксис: `поле OP значение [AND поле OP значение ...]`
//!   OP: == != >= <= > <
//!   поле: имя заголовка (CI), например X-Tag, X-Device-ID, X-Timestamp,
//!         X-Entity-ID, X-EML-Type, Subject
//!   значение: "в кавычках" или голый токен
//! Численное сравнение, если оба значения парсятся как число (X-Timestamp).
//! Множественные заголовки (X-Tag): == матчит любой, != матчит если ни один.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Clause {
    pub field: String,
    pub op: String,
    pub value: String,
}

impl Clause {
    fn compare(v: &str, value: &str, op: &str) -> bool {
        let num = |s: &str| s.trim().parse::<f64>().ok();
        match (num(v), num(value), op) {
            (Some(x), Some(y), ">=") => x >= y,
            (Some(x), Some(y), "<=") => x <= y,
            (Some(x), Some(y), ">") => x > y,
            (Some(x), Some(y), "<") => x < y,
            (Some(x), Some(y), "==") => (x - y).abs() < 1e-9,
            (Some(x), Some(y), "!=") => (x - y).abs() >= 1e-9,
            (_, _, "==") => v == value,
            (_, _, "!=") => v != value,
            _ => false,
        }
    }

    pub fn eval(&self, headers: &[(String, String)]) -> bool {
        let vals: Vec<&str> = headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(&self.field))
            .map(|(_, v)| v.as_str())
            .collect();
        if self.field.eq_ignore_ascii_case("subject") {
            // substring match, case-insensitive
            return vals
                .iter()
                .any(|v| v.to_lowercase().contains(&self.value.to_lowercase()));
        }
        if vals.is_empty() {
            // absent field: `!= value` holds vacuously, everything else fails
            return self.op == "!=";
        }
        let any_eq = vals.iter().any(|v| Self::compare(v, &self.value, "=="));
        if self.op == "!=" {
            // "no value equals" — NOT "some value differs"
            !any_eq
        } else {
            vals.iter().any(|v| Self::compare(v, &self.value, &self.op))
        }
    }
}

/// Parse `field OP value [AND ...]`.
pub fn parse(query: &str) -> Result<Vec<Clause>, String> {
    let mut out = Vec::new();
    for part in query.split(" AND ") {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (field, rest) = split_field(part)?;
        let (op, raw) = split_op(rest)?;
        let value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
        if value.is_empty() {
            return Err(format!("bad clause (empty value): {part}"));
        }
        out.push(Clause { field, op, value });
    }
    if out.is_empty() {
        return Err("empty query".into());
    }
    Ok(out)
}

fn split_field(s: &str) -> Result<(String, &str), String> {
    for i in 0..s.len() {
        let rest = &s[i..];
        if rest.starts_with("==") || rest.starts_with("!=") || rest.starts_with(">=")
            || rest.starts_with("<=") || rest.starts_with('>') || rest.starts_with('<')
        {
            let field = s[..i].trim();
            if field.is_empty() {
                return Err(format!("bad clause (empty field): {s}"));
            }
            return Ok((field.to_lowercase(), rest));
        }
    }
    Err(format!("bad clause (no operator): {s}"))
}

fn split_op(rest: &str) -> Result<(String, &str), String> {
    for op in ["==", "!=", ">=", "<=", ">", "<"] {
        if let Some(r) = rest.strip_prefix(op) {
            let value = r.trim();
            if value.is_empty() {
                return Err(format!("bad clause (empty value): {rest}"));
            }
            return Ok((op.to_string(), value));
        }
    }
    Err(format!("bad operator: {rest}"))
}

/// Evaluate a full query against a header set.
pub fn eval_headers(headers: &[(String, String)], query: &str) -> Result<bool, String> {
    let clauses = parse(query)?;
    Ok(clauses.iter().all(|c| c.eval(headers)))
}

/// Evaluate a full query against a key/value map (convenience for tests).
pub fn eval_map(map: &HashMap<String, String>, query: &str) -> Result<bool, String> {
    let headers: Vec<(String, String)> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    eval_headers(&headers, query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h() -> Vec<(String, String)> {
        vec![
            ("X-Record-ID".into(), "rec_1".into()),
            ("X-Tag".into(), "telemetry".into()),
            ("X-Tag".into(), "status_ok".into()),
            ("X-Device-ID".into(), "node_01".into()),
            ("X-Timestamp".into(), "1786528800".into()),
        ]
    }

    #[test]
    fn multi_tag_any_and_none() {
        let h = h();
        assert!(eval_headers(&h, "X-Tag == \"telemetry\"").unwrap());
        assert!(eval_headers(&h, "X-Tag == \"status_ok\"").unwrap());
        assert!(!eval_headers(&h, "X-Tag == \"missing\"").unwrap());
        assert!(eval_headers(&h, "X-Tag != \"missing\"").unwrap());
        assert!(!eval_headers(&h, "X-Tag != \"telemetry\"").unwrap(), "!='telemetry' must fail: one tag equals");
    }

    #[test]
    fn numeric_timestamp_ranges() {
        let h = h();
        assert!(eval_headers(&h, "X-Timestamp >= 1000").unwrap());
        assert!(eval_headers(&h, "X-Timestamp < 2000000000").unwrap());
        assert!(!eval_headers(&h, "X-Timestamp > 2000000000").unwrap());
        assert!(eval_headers(&h, "X-Timestamp >= 1000 AND X-Timestamp < 2000000000").unwrap());
        assert!(eval_headers(&h, "X-Device-ID == \"node_01\" AND X-Tag == \"telemetry\"").unwrap());
    }

    #[test]
    fn missing_field_never_matches() {
        let h = h();
        assert!(!eval_headers(&h, "X-Bogus == \"x\"").unwrap());
        assert!(eval_headers(&h, "X-Bogus != \"x\"").unwrap());
    }

    #[test]
    fn bad_queries_error() {
        let h = h();
        assert!(eval_headers(&h, "bogus").is_err());
        assert!(eval_headers(&h, "").is_err());
    }

    #[test]
    fn subject_substring_ci() {
        let h = vec![("Subject".into(), "Retro Arcade — one file".into())];
        assert!(eval_headers(&h, "Subject == \"arcade\"").unwrap());
        assert!(!eval_headers(&h, "Subject == \"physics\"").unwrap());
    }
}
