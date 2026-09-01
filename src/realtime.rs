use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::cli::StreamArgs;

const SOCKET_EVENT_PREFIX: &str = "42";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Registration<'a> {
    dev_type: &'static str,
    device_id: &'a str,
    request_id: u64,
    product_code: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Subscription<'a> {
    topic: &'a str,
    sub_type: &'static str,
    push_type: &'static str,
    request_id: u64,
    product_code: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct SocketPayload {
    #[serde(rename = "type")]
    kind: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct RealtimeEvent {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    push_type: Option<String>,
    data: Value,
}

/// Connects to the public Engine.IO 3 endpoint and streams decoded JSON lines.
///
/// # Errors
///
/// Returns an error when the URL, WebSocket handshake, protocol frames, or
/// upstream payloads are invalid.
pub async fn stream(args: &StreamArgs) -> Result<()> {
    validate(args)?;
    let device_id = args
        .device_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let initial_backoff = Duration::from_millis(args.initial_backoff_ms);
    let max_backoff = Duration::from_millis(args.max_backoff_ms);
    let mut backoff = initial_backoff;

    loop {
        let mut subscribed = false;
        match stream_connection(args, &device_id, &mut subscribed).await {
            Ok(()) => return Ok(()),
            Err(error) if args.no_reconnect => return Err(error),
            Err(error) => {
                if subscribed {
                    backoff = initial_backoff;
                }
                eprintln!(
                    "realtime connection lost; reconnecting in {} ms: {error:#}",
                    backoff.as_millis()
                );
                tokio::select! {
                    () = sleep(backoff) => {}
                    signal = tokio::signal::ctrl_c() => {
                        signal.context("failed to listen for Ctrl-C")?;
                        return Ok(());
                    }
                }
                backoff = next_backoff(backoff, max_backoff);
            }
        }
    }
}

async fn stream_connection(
    args: &StreamArgs,
    device_id: &str,
    subscribed: &mut bool,
) -> Result<()> {
    let (socket, _) = connect_async(&args.socket_url)
        .await
        .context("failed to connect to the SportyBet realtime socket")?;
    let (mut writer, mut reader) = socket.split();

    loop {
        tokio::select! {
            message = reader.next() => {
                let Some(message) = message else {
                    bail!("realtime socket closed without a close frame");
                };
                let message = message.context("realtime socket read failed")?;
                match message {
                    Message::Text(text) => {
                        let frame = text.as_str();
                        if args.raw {
                            println!("{frame}");
                        }
                        if frame.starts_with('0') {
                            writer.send(Message::Text("40".into())).await?;
                        } else if let Some(ping_data) = frame.strip_prefix('2') {
                            let pong = format!("3{ping_data}");
                            writer.send(Message::Text(pong.into())).await?;
                        } else if frame == "40" && !*subscribed {
                            register_and_subscribe(&mut writer, args, device_id).await?;
                            *subscribed = true;
                        } else if let Some(event) = parse_event_frame(frame)? {
                            println!("{}", serde_json::to_string(&event)?);
                        } else if frame.starts_with('1') {
                            bail!("realtime socket was closed by the server");
                        }
                    }
                    Message::Ping(data) => writer.send(Message::Pong(data)).await?,
                    Message::Close(frame) => bail!("realtime socket closed: {frame:?}"),
                    Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for Ctrl-C")?;
                let _ = writer.send(Message::Close(None)).await;
                return Ok(());
            }
        }
    }
}

fn validate(args: &StreamArgs) -> Result<()> {
    if args.topic.iter().any(|topic| topic.trim().is_empty()) {
        bail!("subscription topics cannot be empty");
    }
    if args.push_type.as_upstream() == "MULTI" && args.account_id.is_none() {
        bail!("--account-id is required for --push-type multi");
    }
    if args.initial_backoff_ms == 0 {
        bail!("--initial-backoff-ms must be greater than zero");
    }
    if args.max_backoff_ms < args.initial_backoff_ms {
        bail!("--max-backoff-ms cannot be smaller than --initial-backoff-ms");
    }
    Ok(())
}

fn next_backoff(current: Duration, maximum: Duration) -> Duration {
    current.saturating_mul(2).min(maximum)
}

async fn register_and_subscribe<S>(writer: &mut S, args: &StreamArgs, device_id: &str) -> Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let registration = Registration {
        dev_type: "WEB",
        device_id,
        request_id: 1,
        product_code: args.product_code,
    };
    writer
        .send(Message::Text(event_packet("reg", &registration)?.into()))
        .await?;

    for (index, topic) in args.topic.iter().enumerate() {
        let subscription = Subscription {
            topic,
            sub_type: "SUB",
            push_type: args.push_type.as_upstream(),
            request_id: u64::try_from(index).unwrap_or(u64::MAX).saturating_add(2),
            product_code: args.product_code,
            account_id: args.account_id.as_deref(),
        };
        writer
            .send(Message::Text(event_packet("sub", &subscription)?.into()))
            .await?;
    }
    Ok(())
}

fn event_packet<T: Serialize>(kind: &str, data: &T) -> Result<String> {
    let payload = json!({
        "data": serde_json::to_string(data)?,
        "type": kind,
    });
    Ok(format!(
        "{SOCKET_EVENT_PREFIX}{}",
        serde_json::to_string(&json!(["data", payload]))?
    ))
}

fn parse_event_frame(frame: &str) -> Result<Option<RealtimeEvent>> {
    let Some(payload) = frame.strip_prefix(SOCKET_EVENT_PREFIX) else {
        return Ok(None);
    };
    let event: Value = serde_json::from_str(payload).context("invalid Socket.IO event frame")?;
    let Some(parts) = event.as_array() else {
        bail!("Socket.IO event was not an array");
    };
    if parts.first().and_then(Value::as_str) != Some("data") {
        return Ok(None);
    }
    let socket_payload: SocketPayload = serde_json::from_value(
        parts
            .get(1)
            .cloned()
            .context("Socket.IO data event had no payload")?,
    )?;
    let mut data: Value = serde_json::from_str(&socket_payload.data)
        .context("Socket.IO data payload was not JSON")?;
    let topic = data.get("topic").and_then(Value::as_str).map(str::to_owned);
    let push_type = data
        .get("pushType")
        .and_then(Value::as_str)
        .map(str::to_owned);

    if let Some(body) = data.get("body").and_then(Value::as_str) {
        let decoded = STANDARD
            .decode(body)
            .context("realtime body was not valid Base64")?;
        let text = String::from_utf8(decoded).context("realtime body was not UTF-8")?;
        let decoded_body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if let Some(object) = data.as_object_mut() {
            object.insert("body".to_owned(), decoded_body);
        }
    }

    Ok(Some(RealtimeEvent {
        kind: socket_payload.kind,
        topic,
        push_type,
        data,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_socket_io_registration_frame() {
        let frame = event_packet(
            "reg",
            &Registration {
                dev_type: "WEB",
                device_id: "device-1",
                request_id: 1,
                product_code: 7,
            },
        )
        .unwrap();
        assert!(frame.starts_with("42[\"data\","));
        assert!(frame.contains("\\\"productCode\\\":7"));
    }

    #[test]
    fn decodes_base64_push_body() {
        let body = STANDARD.encode(r#"{"odds":"2.10"}"#);
        let inner = json!({"topic":"1^2", "pushType":"GROUP", "body":body});
        let outer = json!(["data", {"type":"ret", "data":inner.to_string()}]);
        let frame = format!("42{outer}");
        let event = parse_event_frame(&frame).unwrap().unwrap();
        assert_eq!(event.topic.as_deref(), Some("1^2"));
        assert_eq!(event.data["body"]["odds"], "2.10");
    }

    #[test]
    fn ignores_other_socket_events() {
        assert!(parse_event_frame(r#"42["other",{}]"#).unwrap().is_none());
    }

    #[test]
    fn reconnect_backoff_doubles_and_caps() {
        let maximum = Duration::from_secs(10);
        assert_eq!(
            next_backoff(Duration::from_millis(250), maximum),
            Duration::from_millis(500)
        );
        assert_eq!(next_backoff(Duration::from_secs(8), maximum), maximum);
    }
}
