//! Inert prompt data compiled from the observed Apache-2.0 Python source.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}
#[derive(Deserialize)]
struct Library {
    prompts: BTreeMap<String, Prompt>,
}
#[derive(Deserialize)]
struct Prompt {
    messages: Vec<Template>,
    schema: Value,
}
#[derive(Deserialize)]
struct Template {
    role: String,
    chunks: Vec<Chunk>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Chunk {
    Literal {
        literal: String,
    },
    Field {
        context: String,
        style: String,
        offset: i64,
    },
}
fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        serde_json::from_str(include_str!("prompts/graphiti-0282.json"))
            .expect("checked-in prompt data")
    })
}
pub fn schema(stage: &str) -> Result<Value> {
    Ok(library()
        .prompts
        .get(stage)
        .context("unknown ingestion prompt")?
        .schema
        .clone())
}
pub fn render(stage: &str, context: &Value) -> Result<Vec<Message>> {
    if stage == "extract_edges.edge" {
        ensure!(
            context["edge_types"].as_array().is_some_and(Vec::is_empty),
            "compiled MCP profile supports generic relations only"
        );
    }
    let prompt = library()
        .prompts
        .get(stage)
        .context("unknown ingestion prompt")?;
    let mut messages = vec![];
    for template in &prompt.messages {
        let mut content = String::new();
        for chunk in &template.chunks {
            match chunk {
                Chunk::Literal { literal } => content.push_str(literal),
                Chunk::Field {
                    context: key,
                    style,
                    offset,
                } => {
                    let value = context
                        .get(key)
                        .context("missing ingestion prompt context")?;
                    let rendered = match style.as_str() {
                        "python" => python_repr(value, false)?,
                        "json" => spaced_json(value)?,
                        "length" => (i64::try_from(
                            value
                                .as_array()
                                .context("prompt length requires an array")?
                                .len(),
                        )? + offset)
                            .to_string(),
                        _ => anyhow::bail!("unknown prompt formatter"),
                    };
                    content.push_str(&rendered);
                }
            }
        }
        ensure!(
            content.len() <= 2 * 1024 * 1024,
            "ingestion prompt exceeds bound"
        );
        messages.push(Message {
            role: template.role.clone(),
            content,
        });
    }
    Ok(messages)
}
fn spaced_json(value: &Value) -> Result<String> {
    Ok(match value {
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(spaced_json)
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| Ok(format!(
                    "{}: {}",
                    serde_json::to_string(k)?,
                    spaced_json(v)?
                )))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        ),
        _ => serde_json::to_string(value)?,
    })
}
fn python_repr(value: &Value, nested: bool) -> Result<String> {
    Ok(match value {
        Value::Null => "None".into(),
        Value::Bool(v) => if *v { "True" } else { "False" }.into(),
        Value::String(v) if !nested => v.clone(),
        Value::String(v) => {
            let quote = if v.contains('\'') && !v.contains('"') {
                '"'
            } else {
                '\''
            };
            let mut out = quote.to_string();
            for c in v.chars() {
                match c {
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    '\\' => out.push_str("\\\\"),
                    c if c == quote => {
                        out.push('\\');
                        out.push(c);
                    }
                    c if c.is_control() && (c as u32) <= 255 => {
                        out.push_str(&format!("\\x{:02x}", c as u32))
                    }
                    c => out.push(c),
                }
            }
            out.push(quote);
            out
        }
        Value::Number(v) => v.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|v| python_repr(v, true))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| Ok(format!(
                    "{}: {}",
                    python_repr(&json!(k), true)?,
                    python_repr(v, true)?
                )))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        ),
    })
}
