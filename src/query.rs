//! X-Query v0.3 — общий движок запросов над заголовками.
//!
//! Работает на любом наборе заголовков (контейнеры EMLBox, записи tagdb).
//! Синтаксис: `поле OP значение` с группировкой скобками и булевыми связками:
//!   `expr := and (OR and)*`, `and := factor (AND factor)*`,
//!   `factor := NOT factor | ( expr ) | поле OP значение`
//!   — NOT сильнее AND, AND сильнее OR (как в SQL/баше).
//!   OP: == != >= <= > <
//!   поле: имя заголовка (CI), например X-Tag, X-Device-ID, X-Timestamp,
//!         X-Entity-ID, X-EML-Type, Subject
//!   значение: "в кавычках" или голый токен; скобки и AND/OR/NOT внутри
//!         кавычек — часть значения (токенайзер их не режет)
//! Численное сравнение, если оба значения парсятся как число (X-Timestamp).
//! Множественные заголовки (X-Tag): == матчит любой, != матчит если ни один.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
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

/// AST одного запроса: скобки, NOT/AND/OR с обычными приоритетами.
#[derive(Debug, Clone, PartialEq)]
pub enum Q {
    Leaf(Clause),
    Not(Box<Q>),
    And(Vec<Q>),
    Or(Vec<Q>),
}

impl Q {
    pub fn eval(&self, headers: &[(String, String)]) -> bool {
        match self {
            Q::Leaf(c) => c.eval(headers),
            Q::Not(q) => !q.eval(headers),
            Q::And(v) => v.iter().all(|q| q.eval(headers)),
            Q::Or(v) => v.iter().any(|q| q.eval(headers)),
        }
    }
}

/// Токены после сборки атомов: клозы, ключевые слова, скобки.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    LParen,
    RParen,
    And,
    Or,
    Not,
    Clause(String), // сырой текст вида `field OP value`
}

/// Quote-aware lexer: поток атомов. Кавычки делают атом цельным (скобки и
/// AND/OR/NOT внутри кавычек — часть значения); скобки вне кавычек — свои атомы.
fn lex(q: &str) -> Result<Vec<String>, String> {
    let mut atoms: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = q.chars().collect();
    for &c in &chars {
        if in_quotes {
            cur.push(c);
            if c == '"' {
                in_quotes = false;
                atoms.push(std::mem::take(&mut cur));
            }
        } else {
            match c {
                '"' => {
                    if !cur.is_empty() {
                        atoms.push(std::mem::take(&mut cur));
                    }
                    cur.push(c);
                    in_quotes = true;
                }
                '(' | ')' => {
                    if !cur.is_empty() {
                        atoms.push(std::mem::take(&mut cur));
                    }
                    atoms.push(c.to_string());
                }
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        atoms.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(c),
            }
        }
    }
    if in_quotes {
        return Err("unbalanced quotes in query".into());
    }
    if !cur.is_empty() {
        atoms.push(cur);
    }
    Ok(atoms)
}

/// Сборка атомов в токены: клозы до AND/OR/NOT/скобки.
fn tokenize(q: &str) -> Result<Vec<Tok>, String> {
    let atoms = lex(q)?;
    let mut toks: Vec<Tok> = Vec::new();
    let mut clause: Vec<String> = Vec::new();
    for a in atoms {
        match a.as_str() {
            "AND" => {
                if !clause.is_empty() {
                    clause_tok(&mut clause, &mut toks)?;
                }
                toks.push(Tok::And);
            }
            "OR" => {
                if !clause.is_empty() {
                    clause_tok(&mut clause, &mut toks)?;
                }
                toks.push(Tok::Or);
            }
            "NOT" => {
                if !clause.is_empty() {
                    clause_tok(&mut clause, &mut toks)?;
                }
                toks.push(Tok::Not);
            }
            "(" => {
                if !clause.is_empty() {
                    clause_tok(&mut clause, &mut toks)?;
                }
                toks.push(Tok::LParen);
            }
            ")" => {
                if !clause.is_empty() {
                    clause_tok(&mut clause, &mut toks)?;
                }
                toks.push(Tok::RParen);
            }
            _ => clause.push(a),
        }
    }
    if !clause.is_empty() {
        clause_tok(&mut clause, &mut toks)?;
    }
    Ok(toks)
}

fn clause_tok(clause: &mut Vec<String>, toks: &mut Vec<Tok>) -> Result<(), String> {
    let raw = clause.join(" ");
    clause.clear();
    if !contains_op_outside_quotes(&raw) {
        return Err(format!("bad clause (no operator): {raw}"));
    }
    toks.push(Tok::Clause(raw));
    Ok(())
}

fn contains_op_outside_quotes(t: &str) -> bool {
    let mut in_quotes = false;
    let chars: Vec<char> = t.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' => in_quotes = !in_quotes,
            '=' | '!' | '>' | '<' if !in_quotes => {
                let rest: String = chars[i..].iter().collect();
                return rest.starts_with("==")
                    || rest.starts_with("!=")
                    || rest.starts_with(">=")
                    || rest.starts_with("<=")
                    || rest.starts_with('>')
                    || rest.starts_with('<');
            }
            _ => {}
        }
    }
    false
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn parse_expr(&mut self) -> Result<Q, String> {
        let mut left = self.parse_and()?;
        while matches!(self.toks.get(self.pos), Some(Tok::Or)) {
            self.pos += 1;
            let right = self.parse_and()?;
            left = match left {
                Q::Or(mut v) => {
                    v.push(right);
                    Q::Or(v)
                }
                other => Q::Or(vec![other, right]),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Q, String> {
        let mut left = self.parse_factor()?;
        while matches!(self.toks.get(self.pos), Some(Tok::And)) {
            self.pos += 1;
            let right = self.parse_factor()?;
            left = match left {
                Q::And(mut v) => {
                    v.push(right);
                    Q::And(v)
                }
                other => Q::And(vec![other, right]),
            };
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Q, String> {
        match self.toks.get(self.pos) {
            Some(Tok::Not) => {
                self.pos += 1;
                Ok(Q::Not(Box::new(self.parse_factor()?)))
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let inner = self.parse_expr()?;
                match self.toks.get(self.pos) {
                    Some(Tok::RParen) => self.pos += 1,
                    _ => return Err("unbalanced parentheses: expected ')'".into()),
                }
                Ok(inner)
            }
            Some(Tok::Clause(raw)) => {
                let raw = raw.clone();
                self.pos += 1;
                parse_clause(&raw).map(Q::Leaf)
            }
            Some(Tok::RParen) => Err("unexpected ')'".into()),
            None => Err("unexpected end of query".into()),
            Some(Tok::And) | Some(Tok::Or) => Err("boolean keyword without a clause".into()),
        }
    }
}

fn split_field(s: &str) -> Result<(String, &str), String> {
    for i in 0..s.len() {
        let rest = &s[i..];
        if rest.starts_with("==")
            || rest.starts_with("!=")
            || rest.starts_with(">=")
            || rest.starts_with("<=")
            || rest.starts_with('>')
            || rest.starts_with('<')
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

fn parse_clause(part: &str) -> Result<Clause, String> {
    let (field, rest) = split_field(part)?;
    let (op, raw) = split_op(rest)?;
    let value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
    if value.is_empty() {
        return Err(format!("bad clause (empty value): {part}"));
    }
    Ok(Clause { field, op, value })
}

/// Разбор полного запроса (скобки, NOT/AND/OR) в AST.
pub fn parse_query(q: &str) -> Result<Q, String> {
    let toks = tokenize(q)?;
    if toks.is_empty() {
        return Err("empty query".into());
    }
    let mut p = Parser { toks, pos: 0 };
    let ast = p.parse_expr()?;
    if p.pos != p.toks.len() {
        return Err(format!(
            "trailing tokens after a valid query: {:?}",
            &p.toks[p.pos..]
        ));
    }
    Ok(ast)
}

/// Синтаксическая проверка без вычисления (для fs index).
pub fn validate(q: &str) -> Result<(), String> {
    parse_query(q).map(|_| ())
}

/// Разбор простого AND-списка (совместимость с v0.2: без скобок, OR, NOT).
pub fn parse(query: &str) -> Result<Vec<Clause>, String> {
    match parse_query(query)? {
        Q::And(v) => Ok(v
            .into_iter()
            .map(|q| match q {
                Q::Leaf(c) => Ok(c),
                other => Err(format!("complex query, use parse_query: {other:?}")),
            })
            .collect::<Result<Vec<_>, String>>()?),
        Q::Or(_) | Q::Not(_) => Err("complex query, use parse_query".into()),
        Q::Leaf(c) => Ok(vec![c]),
    }
}

/// Вычислить запрос над набором заголовков.
pub fn eval_headers(headers: &[(String, String)], query: &str) -> Result<bool, String> {
    Ok(parse_query(query)?.eval(headers))
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
        assert!(
            !eval_headers(&h, "X-Tag != \"telemetry\"").unwrap(),
            "!='telemetry' must fail: one tag equals"
        );
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
        assert!(eval_headers(&h, "X-Tag == ").is_err());
        assert!(
            eval_headers(&h, "(X-Tag == \"telemetry\"").is_err(),
            "unbalanced paren"
        );
        assert!(eval_headers(&h, "X-Tag == \"t\")").is_err(), "stray rparen");
        assert!(eval_headers(&h, "X-Tag == \"a\" AND AND X-Tag == \"b\"").is_err());
    }

    #[test]
    fn subject_substring_ci() {
        let h = vec![("Subject".into(), "Retro Arcade — one file".into())];
        assert!(eval_headers(&h, "Subject == \"arcade\"").unwrap());
        assert!(!eval_headers(&h, "Subject == \"physics\"").unwrap());
    }

    #[test]
    fn or_and_not() {
        let hdr = |tags: &[&str]| -> Vec<(String, String)> {
            tags.iter()
                .map(|t| ("X-Tag".to_string(), t.to_string()))
                .collect()
        };
        let rec1 = hdr(&["vita", "rust", "retro"]);
        let rec2 = hdr(&["vita"]);
        let rec3 = hdr(&["rust", "embedded"]);
        assert!(eval_headers(&rec1, "X-Tag == \"vita\" AND X-Tag == \"rust\"").unwrap());
        assert!(!eval_headers(&rec2, "X-Tag == \"vita\" AND X-Tag == \"rust\"").unwrap());
        assert!(eval_headers(&rec3, "X-Tag == \"vita\" OR X-Tag == \"embedded\"").unwrap());
        assert!(eval_headers(&rec1, "X-Tag == \"vita\" OR X-Tag == \"embedded\"").unwrap());
        assert!(!eval_headers(&rec1, "NOT X-Tag == \"retro\"").unwrap());
        assert!(eval_headers(&rec2, "NOT X-Tag == \"retro\"").unwrap());
        assert!(eval_headers(&rec3, "NOT X-Tag == \"retro\"").unwrap());
    }

    // ------------------------------------------------------------ v0.3 ----

    #[test]
    fn parentheses_group() {
        let h = h();
        // without parens AND binds tighter: (telemetry AND ts>=1) OR bogus
        assert!(eval_headers(
            &h,
            "X-Tag == \"telemetry\" AND X-Timestamp >= 1 OR X-Tag == \"bogus\""
        )
        .unwrap());
        // with parens the OR comes first: telemetry AND (ts>=1 OR bogus)
        assert!(eval_headers(
            &h,
            "X-Tag == \"telemetry\" AND (X-Timestamp >= 1 OR X-Tag == \"bogus\")"
        )
        .unwrap());
        // the same parens with a failing left side: nothing matches
        assert!(!eval_headers(
            &h,
            "X-Tag == \"nope\" AND (X-Timestamp >= 1 OR X-Tag == \"telemetry\")"
        )
        .unwrap());
    }

    #[test]
    fn nested_parens_and_not() {
        let h = h();
        assert!(eval_headers(
            &h,
            "NOT (X-Tag == \"vita\" AND (X-Device-ID == \"node_02\" OR X-Timestamp > 1000))"
        )
        .unwrap());
        // ts = 1786528800 >= 1000, so NOT (< 1000) holds → the conjunction holds
        assert!(eval_headers(
            &h,
            "(X-Tag == \"telemetry\" OR X-Tag == \"vita\") AND NOT (X-Timestamp < 1000)"
        )
        .unwrap());
        // and the mirror case fails: NOT (> 1000) is false here
        assert!(!eval_headers(
            &h,
            "(X-Tag == \"telemetry\" OR X-Tag == \"vita\") AND NOT (X-Timestamp > 1000)"
        )
        .unwrap());
        assert!(
            eval_headers(&h, "((X-Timestamp >= 1))").unwrap(),
            "double parens"
        );
    }

    #[test]
    fn quoted_values_survive_keywords_and_parens() {
        let h = vec![
            ("X-Note".into(), "buy (milk) AND eggs".into()),
            ("X-Tag".into(), "list".into()),
        ];
        assert!(eval_headers(&h, "X-Note == \"buy (milk) AND eggs\"").unwrap());
        assert!(eval_headers(
            &h,
            "X-Tag == \"list\" AND X-Note == \"buy (milk) AND eggs\""
        )
        .unwrap());
        assert!(!eval_headers(&h, "X-Note != \"buy (milk) AND eggs\"").unwrap());
    }

    #[test]
    fn parse_query_ast_shape() {
        // NOT binds tighter than AND: NOT a AND b == (NOT a) AND b
        let q = parse_query("NOT X-Tag == \"a\" AND X-Tag == \"b\"").unwrap();
        match q {
            Q::And(v) => {
                assert_eq!(v.len(), 2);
                assert!(matches!(v[0], Q::Not(_)));
                assert!(matches!(v[1], Q::Leaf(_)));
            }
            other => panic!("wrong AST: {other:?}"),
        }
        // OR is top-level with two AND arms
        let q = parse_query("X-Tag == \"a\" AND X-Tag == \"b\" OR X-Tag == \"c\"").unwrap();
        match q {
            Q::Or(v) => assert_eq!(v.len(), 2),
            other => panic!("wrong AST: {other:?}"),
        }
    }

    #[test]
    fn legacy_parse_still_works_for_and_lists() {
        let c = parse("X-Timestamp >= 1 AND X-Tag == \"telemetry\"").unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].field, "x-timestamp");
        assert!(parse("X-Tag == \"a\" OR X-Tag == \"b\"").is_err());
        // a single parenthesized clause unwraps to its leaf — parens are transparent
        assert_eq!(parse("(X-Tag == \"a\")").unwrap().len(), 1);
        assert!(parse("NOT X-Tag == \"a\"").is_err());
        assert!(parse("(X-Tag == \"a\") AND (X-Timestamp < 9 OR X-Tag != \"z\")").is_err());
    }

    #[test]
    fn validate_reports_bad_syntax() {
        assert!(validate("X-Tag == \"a\"").is_ok());
        assert!(validate("(X-Tag == \"a\" OR X-Timestamp < 9) AND X-Tag != \"z\"").is_ok());
        assert!(validate("X-Tag ==").is_err());
        assert!(validate("NOT").is_err());
        assert!(validate("()").is_err());
    }
}
