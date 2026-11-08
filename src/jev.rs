//! Minimal typed client for TypeSafe's System One API (Jev).
//! Docs: https://docs.typesafe.ai/introduction/quickstart.md

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

const DEFAULT_URL: &str = "https://api.typesafe.ai/v1/systemone";

/// A typed question (the engine uses Score and Noul). Score levels keep their order.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    Noul {
        instructions: String,
    },
}

impl Question {
    pub fn score(instructions: &str, levels: &[&str]) -> Self {
        Question::Score {
            instructions: instructions.into(),
            criteria: levels.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn noul(instructions: &str) -> Self {
        Question::Noul {
            instructions: instructions.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Choice {
        choice: String,
        confidence: f64,
        probabilities: HashMap<String, f64>,
    },
    Score {
        score: f64,
        confidence: f64,
        probabilities: HashMap<String, f64>,
    },
    Noul {
        noul: f64,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Response {
    pub model: String,
    pub answers: HashMap<String, Answer>,
    pub usage: Option<Usage>,
}

impl Response {
    fn get(&self, id: &str) -> Result<&Answer> {
        self.answers
            .get(id)
            .ok_or_else(|| anyhow!("missing answer '{id}'"))
    }

    /// Expected level normalised to 0..=1 (computed from the probability
    /// distribution, so it doesn't depend on how the API scales `score`), plus confidence.
    pub fn score_expected(&self, id: &str) -> Result<(f64, f64)> {
        match self.get(id)? {
            Answer::Score {
                probabilities,
                confidence,
                ..
            } => {
                let mut levels: Vec<(usize, f64)> = probabilities
                    .iter()
                    .map(|(k, p)| Ok((k.parse::<usize>()?, *p)))
                    .collect::<Result<_>>()
                    .with_context(|| format!("non-numeric score level in '{id}'"))?;
                levels.sort_by_key(|(i, _)| *i);
                let max = levels.last().map(|(i, _)| *i).unwrap_or(0);
                if max == 0 {
                    return Ok((0.5, *confidence));
                }
                let total: f64 = levels.iter().map(|(_, p)| p).sum();
                let ev: f64 = levels.iter().map(|(i, p)| *i as f64 * p).sum();
                Ok((ev / total.max(1e-9) / max as f64, *confidence))
            }
            other => bail!("answer '{id}' is not a score: {other:?}"),
        }
    }

    /// 0.5: reject non-finite or out-of-range answers instead of trading on them.
    pub fn validate(&self) -> Result<()> {
        let unit = |x: f64| x.is_finite() && (-1e-9..=1.0 + 1e-9).contains(&x);
        for (id, a) in &self.answers {
            match a {
                Answer::Choice { confidence, probabilities, .. }
                | Answer::Score { confidence, probabilities, .. } => {
                    if !unit(*confidence) {
                        bail!("answer '{id}': confidence {confidence} out of range");
                    }
                    if probabilities.values().any(|p| !unit(*p)) {
                        bail!("answer '{id}': probability out of range");
                    }
                    let sum: f64 = probabilities.values().sum();
                    if (sum - 1.0).abs() > 0.05 {
                        bail!("answer '{id}': probabilities sum to {sum:.3}");
                    }
                }
                Answer::Noul { noul } => {
                    if !unit(*noul) {
                        bail!("answer '{id}': noul {noul} out of range");
                    }
                }
                Answer::Other => bail!("answer '{id}' has an unknown type"),
            }
        }
        Ok(())
    }

    pub fn noul(&self, id: &str) -> Result<f64> {
        match self.get(id)? {
            Answer::Noul { noul } => Ok(*noul),
            other => bail!("answer '{id}' is not a noul: {other:?}"),
        }
    }
}

#[derive(Serialize)]
struct Request<'a, S: Serialize + ?Sized> {
    state: &'a S,
    model: &'a str,
    questions: &'a IndexMap<String, Question>,
}

#[derive(Clone)]
pub struct JevClient {
    http: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
}

impl JevClient {
    /// Reads TYPESAFE_API_KEY (required) and TYPESAFE_API_URL (optional).
    pub fn from_env(model: &str, timeout: Duration) -> Result<Self> {
        let api_key = std::env::var("TYPESAFE_API_KEY")
            .context("TYPESAFE_API_KEY is not set (get a key at https://typesafe.ai)")?;
        let url = std::env::var("TYPESAFE_API_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("jev-trader/0.1")
            .build()?;
        Ok(Self {
            http,
            url,
            api_key,
            model: model.into(),
        })
    }

    /// `state` is a string or any JSON-serializable object (TypeSafe recommends objects
    /// with descriptive field names for structured context).
    pub async fn ask<S: Serialize + ?Sized>(&self, state: &S, questions: &IndexMap<String, Question>) -> Result<Response> {
        let resp = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .json(&Request {
                state,
                model: &self.model,
                questions,
            })
            .send()
            .await
            .context("Jev request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Jev returned {status}: {body}");
        }
        resp.json().await.context("could not decode Jev response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docs_example() {
        let json = r#"{
          "model": "jev-1.13.0",
          "answers": {
            "department": {"type":"choice","choice":"technical","confidence":1.0,
              "probabilities":{"technical":1.0,"billing":0.0,"sales":0.0}},
            "frustration": {"type":"score","score":0.67,"confidence":0.5,
              "legend":{"0":"a","1":"b","2":"c"},
              "probabilities":{"0":0.33,"1":0.67,"2":0.0}},
            "is_urgent": {"type":"noul","noul":0.99}
          },
          "usage": {"input_tokens":428,"output_tokens":73}
        }"#;
        let r: Response = serde_json::from_str(json).unwrap();
        assert!(matches!(r.answers["department"], Answer::Choice { .. }));
        let (s, c) = r.score_expected("frustration").unwrap();
        assert!((s - 0.335).abs() < 1e-9 && c == 0.5);
        assert_eq!(r.noul("is_urgent").unwrap(), 0.99);
        r.validate().unwrap();
    }

    #[test]
    fn rejects_bad_answers() {
        let bad = r#"{"model":"m","answers":{"a":{"type":"choice","choice":"x","confidence":1.0,
            "probabilities":{"x":0.5,"y":0.1}}}}"#;
        let r: Response = serde_json::from_str(bad).unwrap();
        assert!(r.validate().is_err());
        let bad = r#"{"model":"m","answers":{"n":{"type":"noul","noul":1.7}}}"#;
        let r: Response = serde_json::from_str(bad).unwrap();
        assert!(r.validate().is_err());
    }

    #[test]
    fn serializes_questions() {
        let mut q = IndexMap::new();
        q.insert("s".to_string(), Question::score("x", &["lo", "hi"]));
        q.insert("n".to_string(), Question::noul("y"));
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["s"]["type"], "score");
        assert_eq!(v["s"]["criteria"][1], "hi");
        assert_eq!(v["n"]["type"], "noul");
    }
}
