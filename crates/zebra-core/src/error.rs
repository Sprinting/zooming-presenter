use serde::Serialize;

/// Structured error, designed so an agent can self-correct.
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ZError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub valid_values: Vec<String>,
}

impl ZError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), hint: None, valid_values: vec![] }
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
    pub fn values(mut self, v: Vec<String>) -> Self {
        self.valid_values = v;
        self
    }
    pub fn not_found(id: &str, known: impl Iterator<Item = String>) -> Self {
        let mut near: Vec<String> = known.collect();
        near.sort_by_key(|k| edit_distance(k, id));
        near.truncate(5);
        Self::new("not_found", format!("element '{id}' not found"))
            .hint("call `describe` or `query {\"q\":\"elements\"}` to list ids")
            .values(near)
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_params", message)
    }
}

pub type ZResult<T> = Result<T, ZError>;

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == *cb { 0 } else { 1 };
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}
