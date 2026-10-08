//! Small result-column DSL, execution plan, and snapshot-local streaming state.
use super::{MAX_GROUPS, Record};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::BTreeMap;

const MAX_QUERY_BYTES: usize = 4096;
const MAX_COLUMNS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Column(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AggregateFunc {
    Avg,
    Max,
    Min,
    Sum,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Aggregate {
    Count,
    Numeric { func: AggregateFunc, column: Column },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
    Column(Column),
    Aggregate(Aggregate),
}

struct QueryAst {
    select: Vec<Expr>,
}

struct Parser<'a> {
    text: &'a str,
    offset: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("Query at byte {}: {message}", self.offset + 1)
    }

    fn whitespace(&mut self) {
        while let Some(c) = self.text[self.offset..].chars().next() {
            if !c.is_whitespace() {
                break;
            }
            self.offset += c.len_utf8();
        }
    }

    fn consume(&mut self, c: char) -> bool {
        self.whitespace();
        if self.text[self.offset..].starts_with(c) {
            self.offset += c.len_utf8();
            true
        } else {
            false
        }
    }

    // Backticks allow existing field names such as "1", "a,b", or Unicode.
    // A literal backtick inside a quoted name is written twice.
    fn name(&mut self) -> Result<(Column, bool), String> {
        self.whitespace();
        let quoted = self.consume('`');
        let mut name = String::new();
        if quoted {
            loop {
                let Some(c) = self.text[self.offset..].chars().next() else {
                    return Err(self.error("Unterminated quoted field name"));
                };
                self.offset += c.len_utf8();
                if c == '`' {
                    if self.text[self.offset..].starts_with('`') {
                        self.offset += 1;
                        name.push('`');
                    } else {
                        break;
                    }
                } else {
                    name.push(c);
                }
            }
        } else {
            while let Some(c) = self.text[self.offset..].chars().next() {
                if !(c.is_ascii_alphanumeric() || c == '_') {
                    break;
                }
                if name.is_empty() && c.is_ascii_digit() {
                    return Err(self.error("Field names starting with digits require backticks"));
                }
                self.offset += 1;
                name.push(c);
            }
        }
        if name.is_empty() || name.len() > 64 {
            return Err(self.error("Field names must be 1–64 bytes"));
        }
        Ok((Column(name), quoted))
    }

    fn expression(&mut self) -> Result<Expr, String> {
        let (name, quoted) = self.name()?;
        if !self.consume('(') {
            return Ok(Expr::Column(name));
        }
        if quoted {
            return Err(self.error("Function names cannot be quoted"));
        }
        if name.0 == "count" {
            if !self.consume(')') {
                return Err(self.error("count() takes no arguments"));
            }
            return Ok(Expr::Aggregate(Aggregate::Count));
        }
        let func = match name.0.as_str() {
            "avg" => AggregateFunc::Avg,
            "max" => AggregateFunc::Max,
            "min" => AggregateFunc::Min,
            "sum" => AggregateFunc::Sum,
            _ => return Err(self.error("Unknown aggregate; use avg, max, min, sum, or count")),
        };
        let (column, _) = self.name()?;
        if !self.consume(')') {
            return Err(self.error("Numeric aggregates take exactly one field name"));
        }
        Ok(Expr::Aggregate(Aggregate::Numeric { func, column }))
    }
}

impl QueryAst {
    fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_QUERY_BYTES {
            return Err("Query exceeds 4 KiB".into());
        }
        let mut parser = Parser { text, offset: 0 };
        let mut select = Vec::new();
        loop {
            if select.len() >= MAX_COLUMNS {
                return Err("Too many result columns (maximum 32)".into());
            }
            select.push(parser.expression()?);
            parser.whitespace();
            if parser.offset == text.len() {
                break;
            }
            if !parser.consume(',') {
                return Err(parser.error("Expected ',' between result columns"));
            }
        }
        Ok(Self { select })
    }
}

enum OutputColumn {
    Group(usize),
    Aggregate(usize),
}

pub(super) struct Query {
    groups: Vec<Column>,
    aggregates: Vec<Aggregate>,
    output: Vec<OutputColumn>,
    labels: Vec<String>,
}

fn label_name(name: &str) -> String {
    let mut chars = name.chars();
    if chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        name.into()
    } else {
        format!("`{}`", name.replace('`', "``"))
    }
}

impl Query {
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::plan(QueryAst::parse(text)?)
    }

    fn plan(ast: QueryAst) -> Result<Self, String> {
        let mut query = Self {
            groups: Vec::new(),
            aggregates: Vec::new(),
            output: Vec::new(),
            labels: Vec::new(),
        };
        for expr in ast.select {
            match expr {
                Expr::Column(column) => {
                    if query.groups.contains(&column) {
                        return Err(format!("Duplicate group field: {}", column.0));
                    }
                    query.labels.push(label_name(&column.0));
                    query.output.push(OutputColumn::Group(query.groups.len()));
                    query.groups.push(column);
                }
                Expr::Aggregate(aggregate) => {
                    if query.aggregates.contains(&aggregate) {
                        return Err("Duplicate aggregate expression".into());
                    }
                    query.labels.push(match &aggregate {
                        Aggregate::Count => "count()".into(),
                        Aggregate::Numeric { func, column } => format!(
                            "{}({})",
                            match func {
                                AggregateFunc::Avg => "avg",
                                AggregateFunc::Max => "max",
                                AggregateFunc::Min => "min",
                                AggregateFunc::Sum => "sum",
                            },
                            label_name(&column.0)
                        ),
                    });
                    query
                        .output
                        .push(OutputColumn::Aggregate(query.aggregates.len()));
                    query.aggregates.push(aggregate);
                }
            }
        }
        if query.aggregates.is_empty() {
            return Err("Select at least one aggregate (e.g. count())".into());
        }
        Ok(query)
    }

    // Preserve arbitrary existing field names without going through DSL syntax.
    pub fn legacy(group: &str, metric: &str, operation: &str) -> Result<Self, String> {
        let aggregate = match operation {
            "count" => Aggregate::Count,
            "average" | "sum" | "min" | "max" => {
                if metric.is_empty() {
                    return Err("Select a numeric field".into());
                }
                Aggregate::Numeric {
                    func: match operation {
                        "average" => AggregateFunc::Avg,
                        "sum" => AggregateFunc::Sum,
                        "min" => AggregateFunc::Min,
                        _ => AggregateFunc::Max,
                    },
                    column: Column(metric.into()),
                }
            }
            _ => return Err("Unknown aggregation operation".into()),
        };
        let mut select = Vec::new();
        if !group.is_empty() {
            select.push(Expr::Column(Column(group.into())));
        }
        select.push(Expr::Aggregate(aggregate));
        Self::plan(QueryAst { select })
    }
}

#[derive(Clone, Debug)]
enum GroupValue {
    Text(String),
    Number(f64),
}

impl PartialEq for GroupValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for GroupValue {}
impl PartialOrd for GroupValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for GroupValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(_), Self::Number(_)) => Ordering::Less,
            (Self::Number(_), Self::Text(_)) => Ordering::Greater,
        }
    }
}
impl GroupValue {
    fn value(&self) -> Value {
        match self {
            Self::Text(s) => json!(s),
            Self::Number(n) => json!(n),
        }
    }
    fn display(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Number(n) => n.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum AggregateState {
    Count(u64),
    Avg { sum: f64, count: u64 },
    Sum { sum: f64, count: u64 },
    Min(Option<f64>),
    Max(Option<f64>),
}

impl AggregateState {
    fn new(aggregate: &Aggregate) -> Self {
        match aggregate {
            Aggregate::Count => Self::Count(0),
            Aggregate::Numeric { func, .. } => match func {
                AggregateFunc::Avg => Self::Avg { sum: 0.0, count: 0 },
                AggregateFunc::Sum => Self::Sum { sum: 0.0, count: 0 },
                AggregateFunc::Min => Self::Min(None),
                AggregateFunc::Max => Self::Max(None),
            },
        }
    }
    fn add(&mut self, value: Option<f64>) -> Result<(), String> {
        match self {
            Self::Count(count) => *count = count.checked_add(1).ok_or("Count overflow")?,
            Self::Avg { sum, count } | Self::Sum { sum, count } => {
                let next = *sum + value.ok_or("Numeric field is missing")?;
                if !next.is_finite() {
                    return Err("Numeric sum overflow".into());
                }
                let next_count = count.checked_add(1).ok_or("Count overflow")?;
                *sum = next;
                *count = next_count;
            }
            Self::Min(current) => {
                let n = value.ok_or("Numeric field is missing")?;
                *current = Some(current.map_or(n, |v| v.min(n)));
            }
            Self::Max(current) => {
                let n = value.ok_or("Numeric field is missing")?;
                *current = Some(current.map_or(n, |v| v.max(n)));
            }
        }
        Ok(())
    }
    fn value(&self) -> Value {
        match self {
            Self::Count(n) => json!(n),
            Self::Avg { sum, count } => {
                if *count == 0 {
                    Value::Null
                } else {
                    json!(sum / *count as f64)
                }
            }
            Self::Sum { sum, count } => {
                if *count == 0 {
                    Value::Null
                } else {
                    json!(sum)
                }
            }
            Self::Min(n) | Self::Max(n) => json!(n),
        }
    }
}

#[derive(Clone)]
struct GroupState {
    count: u64,
    aggregates: Vec<AggregateState>,
}

pub(super) struct Aggregator<'a> {
    query: &'a Query,
    groups: BTreeMap<Vec<GroupValue>, GroupState>,
}

impl<'a> Aggregator<'a> {
    pub fn new(query: &'a Query) -> Self {
        Self {
            query,
            groups: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, record: &Record) -> Result<(), String> {
        let mut key = Vec::with_capacity(self.query.groups.len());
        for column in &self.query.groups {
            if let Some(text) = record.text.get(&column.0) {
                key.push(GroupValue::Text(text.clone()));
            } else if let Some(n) = record.numbers.get(&column.0).filter(|n| n.is_finite()) {
                // Normalize signed zero so equal numeric values share a group.
                key.push(GroupValue::Number(if *n == 0.0 { 0.0 } else { *n }));
            } else {
                return Err(format!("Group field is missing or invalid: {}", column.0));
            }
        }
        let values = self
            .query
            .aggregates
            .iter()
            .map(|aggregate| match aggregate {
                Aggregate::Count => Ok(None),
                Aggregate::Numeric { column, .. } => record
                    .numbers
                    .get(&column.0)
                    .copied()
                    .filter(|n| n.is_finite())
                    .map(Some)
                    .ok_or_else(|| format!("Numeric field is missing or invalid: {}", column.0)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !self.groups.contains_key(&key) && self.groups.len() >= MAX_GROUPS {
            return Err("Group limit reached (2,000)".into());
        }
        // Stage ALL updates, including overflow checks, before committing.
        // Failed rows cannot affect even count() or create an empty group.
        let mut next = self
            .groups
            .get(&key)
            .cloned()
            .unwrap_or_else(|| GroupState {
                count: 0,
                aggregates: self
                    .query
                    .aggregates
                    .iter()
                    .map(AggregateState::new)
                    .collect(),
            });
        next.count = next.count.checked_add(1).ok_or("Count overflow")?;
        for (state, value) in next.aggregates.iter_mut().zip(values) {
            state.add(value)?;
        }
        self.groups.insert(key, next);
        Ok(())
    }

    pub fn finish(self) -> (Vec<String>, Vec<Value>) {
        let rows = self.groups.into_iter().map(|(key, state)| {
            let cells: Vec<Value> = self.query.output.iter().map(|column| match column {
                OutputColumn::Group(i) => key[*i].value(),
                OutputColumn::Aggregate(i) => state.aggregates[*i].value(),
            }).collect();
            // Legacy fields are retained for existing callers; UI uses cells.
            let group = if key.is_empty() { "All".into() } else if key.len() == 1 { key[0].display() }
                else { serde_json::to_string(&key.iter().map(GroupValue::value).collect::<Vec<_>>()).unwrap() };
            json!({"cells":cells, "group":group, "count":state.count, "value":state.aggregates[0].value()})
        }).collect();
        (self.query.labels.clone(), rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_preserves_order_and_validates_small_grammar() {
        let query =
            Query::parse(" avg(elapsed), path, method, status, max(elapsed), count() ").unwrap();
        assert_eq!(
            query.labels,
            [
                "avg(elapsed)",
                "path",
                "method",
                "status",
                "max(elapsed)",
                "count()"
            ]
        );
        assert_eq!(query.groups.len(), 3);
        assert_eq!(query.aggregates.len(), 3);
        assert!(matches!(query.output[0], OutputColumn::Aggregate(0)));
        assert!(Query::parse("`a,b`, sum(`日本語`), count()").is_ok());
        assert!(Query::parse("`tick``name`, avg(`1`)").is_ok());
        for source in [
            "",
            " ",
            "path",
            "path,",
            ",count()",
            "ave(x)",
            "COUNT()",
            "count(x)",
            "avg()",
            "avg(x,y)",
            "avg(sum(x))",
            "path count()",
            "sum(x)+1",
            "count(*)",
            "path, path, count()",
            "count(), count()",
            "avg(x),avg(x)",
            "1,count()",
            "`bad",
            "`avg`(x)",
        ] {
            assert!(Query::parse(source).is_err(), "{source}");
        }
        assert!(Query::parse(&"x".repeat(MAX_QUERY_BYTES + 1)).is_err());
        assert!(
            Query::parse(&format!(
                "{}, count()",
                (0..32)
                    .map(|i| format!("c{i}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
            .is_err()
        );
        assert!(Query::parse(&format!("{}, count()", "a".repeat(65))).is_err());
    }

    fn record(path: &str, elapsed: f64, bytes: f64) -> Record {
        Record {
            text: BTreeMap::from([("path".into(), path.into())]),
            numbers: BTreeMap::from([("elapsed".into(), elapsed), ("bytes".into(), bytes)]),
            ..Record::default()
        }
    }

    #[test]
    fn all_aggregates_and_ordered_output() {
        let query =
            Query::parse("avg(elapsed), path, max(elapsed), min(elapsed), sum(bytes), count()")
                .unwrap();
        let mut aggregator = Aggregator::new(&query);
        aggregator.push(&record("/a", 2.0, 10.0)).unwrap();
        aggregator.push(&record("/a", 4.0, 20.0)).unwrap();
        aggregator.push(&record("/b", 8.0, 30.0)).unwrap();
        let (_, rows) = aggregator.finish();
        assert_eq!(rows[0]["cells"], json!([3.0, "/a", 4.0, 2.0, 30.0, 2]));
        assert_eq!(rows[1]["cells"], json!([8.0, "/b", 8.0, 8.0, 30.0, 1]));
    }

    #[test]
    fn rows_commit_atomically_on_missing_fields_and_overflow() {
        let query = Query::parse("path, count(), max(elapsed), sum(bytes)").unwrap();
        let mut aggregator = Aggregator::new(&query);
        aggregator.push(&record("/a", 2.0, f64::MAX)).unwrap();
        assert!(aggregator.push(&record("/a", 99.0, f64::MAX)).is_err());
        let mut missing = record("/new", 1.0, 1.0);
        missing.numbers.remove("bytes");
        assert!(aggregator.push(&missing).is_err());
        assert!(aggregator.push(&record("/new", f64::NAN, 1.0)).is_err());
        let (_, rows) = aggregator.finish();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["cells"], json!(["/a", 1, 2.0, f64::MAX]));
        let query = Query::parse("count()").unwrap();
        let mut aggregator = Aggregator::new(&query);
        aggregator.groups.insert(
            Vec::new(),
            GroupState {
                count: u64::MAX,
                aggregates: vec![AggregateState::Count(u64::MAX)],
            },
        );
        assert!(aggregator.push(&Record::default()).is_err());
        assert_eq!(aggregator.finish().1[0]["cells"], json!([u64::MAX]));
    }

    #[test]
    fn composite_keys_preserve_boundaries_types_and_signed_zero() {
        let query = Query::parse("a, b, count()").unwrap();
        let mut aggregator = Aggregator::new(&query);
        for (a, b) in [("a,b", "c"), ("a", "b,c")] {
            aggregator
                .push(&Record {
                    text: BTreeMap::from([("a".into(), a.into()), ("b".into(), b.into())]),
                    ..Record::default()
                })
                .unwrap();
        }
        assert_eq!(aggregator.finish().1.len(), 2);
        let query = Query::parse("a, count()").unwrap();
        let mut aggregator = Aggregator::new(&query);
        for n in [-0.0, 0.0] {
            aggregator
                .push(&Record {
                    numbers: BTreeMap::from([("a".into(), n)]),
                    ..Record::default()
                })
                .unwrap();
        }
        aggregator
            .push(&Record {
                text: BTreeMap::from([("a".into(), "0".into())]),
                ..Record::default()
            })
            .unwrap();
        let (_, rows) = aggregator.finish();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["cells"], json!(["0", 1]));
        assert_eq!(rows[1]["cells"], json!([0.0, 2]));
    }

    #[test]
    fn composite_group_limit_and_empty_input() {
        let query = Query::parse("path, count()").unwrap();
        assert!(Aggregator::new(&query).finish().1.is_empty());
        let mut aggregator = Aggregator::new(&query);
        for i in 0..MAX_GROUPS {
            aggregator
                .push(&record(&format!("/{i}"), 1.0, 1.0))
                .unwrap();
        }
        assert!(aggregator.push(&record("/overflow", 1.0, 1.0)).is_err());
        aggregator.push(&record("/0", 1.0, 1.0)).unwrap();
        let (_, rows) = aggregator.finish();
        assert_eq!(rows.len(), MAX_GROUPS);
        assert_eq!(rows[0]["cells"], json!(["/0", 2]));
    }
}
