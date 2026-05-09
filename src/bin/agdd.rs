//! agdd — long-lived AGD daemon driven by Redis Streams.
//!
//! Reads logical edit operations from a Redis Stream (one per
//! consumer-group, FIFO via XREADGROUP), applies them in-memory to a
//! parsed `Document` via the library API and a `DocumentIndex` for
//! O(1) ID lookups, persists the canonical bytes back to a Redis Hash,
//! and ACKs each message.
//!
//! This is the production-pattern counterpart to the subprocess-based
//! Python daemon used in the blog lab. The Python daemon shells out
//! to `agd parse + agd edit` per op (~16 ms each due to fork/exec).
//! `agdd` parses once, applies ops in microseconds.
//!
//! Logical op format (JSON in the stream `data` field):
//!
//! ```json
//! {"kind":"append_item","target":"findings","payload":{"text":"..."},"agent":"analyst"}
//! {"kind":"rename_section","target":"findings","payload":{"new_name":"..."},"agent":"analyst"}
//! {"kind":"set_attr","target":"meta","payload":{"key":"severity","value":"high"},"agent":"auditor"}
//! ```
//!
//! Heading rename targets the heading-id derived from the section id by
//! convention `<target>-h` (e.g. target="findings" → heading id="findings-h").

use std::time::{Duration, Instant};

use agd::ast::{AttrValue, BlockContent, Inline};
use agd::index::DocumentIndex;
use agd::{parse, serialize, Document};

use clap::Parser;
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::{Commands, RedisResult, Value};
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(name = "agdd", about = "AGD Redis Streams daemon")]
struct Args {
    #[arg(long, default_value = "redis://127.0.0.1:6391/")]
    redis_url: String,
    /// Stream key carrying logical ops, e.g. `agd-lab:agd:ops`
    #[arg(long)]
    stream: String,
    /// Hash key storing canonical AGD content, e.g. `agd-lab:agd:state`
    #[arg(long)]
    state_key: String,
    #[arg(long, default_value = "agdd-group")]
    group: String,
    #[arg(long, default_value = "agdd-1")]
    consumer: String,
    /// Exit after this many seconds of stream idleness.
    #[arg(long, default_value_t = 2)]
    idle_exit_secs: u64,
}

#[derive(Debug, Deserialize)]
struct LogicalOp {
    kind: String,
    target: String,
    payload: serde_json::Value,
    agent: String,
}

fn extract_text(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_to_attr(v: &serde_json::Value) -> AttrValue {
    if let Some(b) = v.as_bool() {
        AttrValue::Bool(b)
    } else if let Some(n) = v.as_i64() {
        AttrValue::Int(n)
    } else if let Some(s) = v.as_str() {
        AttrValue::Str(s.to_string())
    } else {
        AttrValue::Str(v.to_string())
    }
}

/// Apply one logical op to the in-memory `Document`. Returns the
/// status string mirroring the Python adapters' contract.
fn apply_logical(doc: &mut Document, idx: &DocumentIndex, op: &LogicalOp) -> &'static str {
    match op.kind.as_str() {
        "append_item" => {
            let pos = match idx.position(&op.target) {
                Some(p) => p,
                None => return "target_not_found",
            };
            let block = &mut doc.blocks[pos];
            if block.kind.as_str() != "ul" && block.kind.as_str() != "ol" {
                return "target_not_found";
            }
            let text = extract_text(&op.payload, "text");
            match &mut block.content {
                BlockContent::Items(items) => {
                    items.push(vec![Inline::Text(text)]);
                    "applied"
                }
                _ => "target_not_found",
            }
        }
        "rename_section" => {
            let heading_id = format!("{}-h", op.target);
            let pos = match idx.position(&heading_id) {
                Some(p) => p,
                None => return "target_not_found",
            };
            let block = &mut doc.blocks[pos];
            let new_name = extract_text(&op.payload, "new_name");
            block.content = BlockContent::Inline(vec![Inline::Text(new_name)]);
            "applied"
        }
        "set_attr" => {
            let pos = match idx.position(&op.target) {
                Some(p) => p,
                None => return "target_not_found",
            };
            let block = &mut doc.blocks[pos];
            let key = extract_text(&op.payload, "key");
            let value = match op.payload.get("value") {
                Some(v) => json_to_attr(v),
                None => return "target_not_found",
            };
            block.attrs.insert(key, value);
            "applied"
        }
        _ => "target_not_found",
    }
}

fn run() -> RedisResult<()> {
    let args = Args::parse();
    let client = redis::Client::open(args.redis_url.as_str())?;
    let mut con = client.get_connection()?;

    // Ensure the consumer group exists.
    let _: Result<(), _> = con.xgroup_create_mkstream(&args.stream, &args.group, "0");

    // Initial state: parse once.
    let initial: String = con.hget(&args.state_key, "content")?;
    let mut doc = parse(&initial).expect("initial state must parse");
    let mut idx = DocumentIndex::build(&doc);

    let mut applied: u32 = 0;
    let mut not_found: u32 = 0;
    let mut last_traffic = Instant::now();
    let opts = StreamReadOptions::default()
        .count(10)
        .block(400)
        .group(&args.group, &args.consumer);

    loop {
        let res: Option<StreamReadReply> =
            con.xread_options(&[args.stream.as_str()], &[">"], &opts)?;
        let reply = match res {
            Some(r) if !r.keys.is_empty() => r,
            _ => {
                if last_traffic.elapsed() >= Duration::from_secs(args.idle_exit_secs) {
                    break;
                }
                continue;
            }
        };
        last_traffic = Instant::now();

        for stream_key in reply.keys {
            for entry in stream_key.ids {
                let raw = match entry.map.get("data") {
                    Some(Value::BulkString(b)) => String::from_utf8_lossy(b).to_string(),
                    Some(Value::SimpleString(s)) => s.clone(),
                    _ => continue,
                };
                let op: LogicalOp = match serde_json::from_str(&raw) {
                    Ok(o) => o,
                    Err(e) => {
                        eprintln!("[agdd] malformed op: {e}");
                        let _: () = con.xack(args.stream.as_str(), &args.group, &[entry.id])?;
                        continue;
                    }
                };

                let t0 = Instant::now();
                let status = apply_logical(&mut doc, &idx, &op);
                let dt_us = t0.elapsed().as_micros();

                if status == "applied" {
                    applied += 1;
                    // Rebuild the index after each apply so subsequent ops
                    // see consistent positions. Cheap (HashMap rebuild over
                    // a typically-small number of ID-bearing blocks).
                    idx = DocumentIndex::build(&doc);
                } else {
                    not_found += 1;
                }

                let new_content = serialize(&doc);
                let _: () = con.hset(&args.state_key, "content", &new_content)?;
                let _: () = con.hset(&args.state_key, "last_status", status)?;
                let _: () = con.xack(args.stream.as_str(), &args.group, &[entry.id.clone()])?;

                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}us",
                    entry.id, op.agent, op.kind, op.target, status, dt_us
                );
            }
        }
    }

    eprintln!("[agdd] applied={applied} not_found={not_found}");
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("agdd: fatal: {e}");
        std::process::exit(1);
    }
}
