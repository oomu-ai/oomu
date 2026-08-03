use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

pub(crate) fn evaluate_basic_condition(
    expression: &str,
    memory: &HashMap<String, Value>,
    input: &Value,
) -> Option<bool> {
    let expression = expression.trim();
    if expression.eq_ignore_ascii_case("true") {
        return Some(true);
    }
    if expression.eq_ignore_ascii_case("false") {
        return Some(false);
    }
    let condition =
        Regex::new(r#"^\s*([A-Za-z0-9_.$:-]+)\s*(==|!=|>=|<=|>|<|contains)\s*(.+?)\s*$"#).ok()?;
    let captures = condition.captures(expression)?;
    let left = lookup_condition_value(captures.get(1)?.as_str(), memory, input)?;
    let operator = captures.get(2)?.as_str();
    let right = parse_literal(captures.get(3)?.as_str());
    match operator {
        "==" => Some(left == &right),
        "!=" => Some(left != &right),
        "contains" => match (left, right) {
            (Value::String(left), Value::String(right)) => Some(left.contains(&right)),
            (Value::Array(left), right) => Some(left.contains(&right)),
            _ => None,
        },
        ">" | ">=" | "<" | "<=" => {
            let left = left.as_f64()?;
            let right = right.as_f64()?;
            Some(match operator {
                ">" => left > right,
                ">=" => left >= right,
                "<" => left < right,
                "<=" => left <= right,
                _ => unreachable!(),
            })
        }
        _ => None,
    }
}

fn lookup_condition_value<'a>(
    path: &str,
    memory: &'a HashMap<String, Value>,
    input: &'a Value,
) -> Option<&'a Value> {
    if path == "$" || path == "input" {
        return Some(input);
    }
    if let Some(value) = memory.get(path) {
        return Some(value);
    }
    let path = path.trim_start_matches("$.").trim_start_matches("input.");
    path.split('.')
        .try_fold(input, |value, segment| value.get(segment))
}

fn parse_literal(value: &str) -> Value {
    let value = value.trim();
    serde_json::from_str(value)
        .unwrap_or_else(|_| Value::String(value.trim_matches('"').to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn evaluates_the_runtime_condition_contract_without_a_runtime_dependency() {
        let input = json!({"hasException": true, "variance": 1500, "labels": ["freight"]});
        let memory = HashMap::new();

        assert_eq!(
            evaluate_basic_condition("$.hasException == true", &memory, &input),
            Some(true)
        );
        assert_eq!(
            evaluate_basic_condition("$.variance >= 1500", &memory, &input),
            Some(true)
        );
        assert_eq!(
            evaluate_basic_condition("$.labels contains \"freight\"", &memory, &input),
            Some(true)
        );
    }
}
