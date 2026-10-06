use regex::Regex;
use serde_json::{Value, json};

/// Optional numeric comparison against a regex capture, shared by filters and traps.
#[derive(Clone, Debug)]
pub struct Condition {
    pub capture: usize,
    operator: String,
    value: f64,
}

pub fn sanitize(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    json!({"capture":value["capture"].as_u64().or_else(|| value["capture"].as_f64().map(|n| n as u64)).unwrap_or(1).clamp(1, 99),
        "operator":value["operator"].as_str().unwrap_or(">"),
        "value":value["value"].as_str().map(str::to_owned).unwrap_or_else(|| value["value"].to_string()).chars().take(128).collect::<String>()})
}

impl Condition {
    pub fn compile(value: &Value, expression: Option<&Regex>) -> Result<Option<Self>, String> {
        if value.is_null() {
            return Ok(None);
        }
        let expression = expression.ok_or("Numeric conditions require Regex.")?;
        let value = sanitize(value);
        let capture = value["capture"].as_u64().unwrap_or(0) as usize;
        if capture == 0 || capture >= expression.captures_len() {
            return Err("The selected capture group does not exist.".into());
        }
        let operator = value["operator"].as_str().unwrap_or_default();
        if ![">", ">=", "<", "<=", "==", "!="].contains(&operator) {
            return Err("Invalid numeric comparison operator.".into());
        }
        let number = value["value"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or("Enter a finite number to compare.")?;
        Ok(Some(Self {
            capture,
            operator: operator.into(),
            value: number,
        }))
    }

    pub fn matches(&self, captures: &regex::Captures<'_>) -> bool {
        let Some(number) = captures
            .get(self.capture)
            .and_then(|m| m.as_str().trim().parse::<f64>().ok())
            .filter(|n| n.is_finite())
        else {
            return false;
        };
        match self.operator.as_str() {
            ">" => number > self.value,
            ">=" => number >= self.value,
            "<" => number < self.value,
            "<=" => number <= self.value,
            "==" => number == self.value,
            "!=" => number != self.value,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operators_numbers_and_validation() {
        let regex = Regex::new(r"(\S+)$").unwrap();
        for (operator, yes, no) in [
            (">", "128", "100"),
            (">=", "100", "99"),
            ("<", "-1.5", "100"),
            ("<=", "100", "101"),
            ("==", "100.0", "101"),
            ("!=", "99", "100"),
        ] {
            let condition = Condition::compile(
                &json!({"capture":1,"operator":operator,"value":"100"}),
                Some(&regex),
            )
            .unwrap()
            .unwrap();
            assert!(condition.matches(&regex.captures(yes).unwrap()));
            assert!(!condition.matches(&regex.captures(no).unwrap()));
            for invalid in ["NaN", "inf", "oops"] {
                assert!(!condition.matches(&regex.captures(invalid).unwrap()));
            }
        }
        for invalid in [
            json!({"capture":2,"value":"100"}),
            json!({"value":"NaN"}),
            json!({"operator":"bad","value":"100"}),
        ] {
            assert!(Condition::compile(&invalid, Some(&regex)).is_err());
        }
        assert!(Condition::compile(&json!({}), None).is_err());
    }
}
