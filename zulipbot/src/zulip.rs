use std::{env, path::Path, time::Duration};

use serde::Deserialize;
use tokio::time;

#[derive(Debug, Deserialize)]
struct GetEventsResponse {
    events: Option<Vec<Event>>,

    // "error" or "success"
    result: String,
    // Set for errors
    msg: Option<String>,
    // Also set for errors
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetMessagesResponse {
    messages: Option<Vec<Message>>,
}

#[derive(Debug, Deserialize)]
struct GetImageResponse {
    result: String,
    url: String,
}

#[derive(Debug, Deserialize)]
pub struct Event {
    r#type: String,
    pub id: u64,
    pub message: Option<Message>,
    pub message_id: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Message {
    pub content: String,
    pub id: u64,
    pub sender_id: u64,
    pub stream_id: Option<u64>,
    pub timestamp: u64,
    pub subject: String,
    pub sender_full_name: String,
}

pub enum SendMessageType {
    Direct(u64),
    Channel(String, u64),
}

pub struct SendMessage {
    pub msg_type: SendMessageType,
    pub msg: String,
}

pub async fn call_on_each_message<F, Fut>(
    listen_type: ListenType,
    event_type: EventType,
    mut callback: F,
) -> Result<(), String>
where
    F: FnMut(Message) -> Fut,
    Fut: std::future::Future<Output = Option<SendMessage>>,
{
    let queue_id = register_event_queue(listen_type, event_type).await.unwrap();

    let mut last_event_id = -1i64;
    loop {
        let events = match get_events(&queue_id, last_event_id).await {
            Ok(ev) => ev,
            Err(e) => {
                println!("error getting events {:?}", e);
                // Usually just means we're polling and didn't get anything
                time::sleep(Duration::from_millis(2500)).await;
                continue;
            }
        };
        for ev in events {
            last_event_id = i64::max(last_event_id, ev.id as i64);
            if ev.r#type == "heartbeat" {
                continue;
            }
            let msg = match &ev {
                Event {
                    message: Some(msg), ..
                } => msg,
                Event {
                    message_id: Some(message_id),
                    ..
                } => &get_message(*message_id).await?,
                _ => {
                    println!(
                        "message with type {} had no message or message_id",
                        ev.r#type
                    );
                    continue;
                }
            };
            if msg.sender_full_name.contains("Blog Bot") {
                // Ignore DMs sent by Blog Bot
                continue;
            }
            let send_msg = callback(msg.clone()).await;

            if let Some(sm) = send_msg {
                match sm.msg_type {
                    SendMessageType::Direct(recipient_id) => {
                        send_direct_message(&sm.msg, recipient_id).await?;
                    }
                    SendMessageType::Channel(topic, channel_id) => {
                        send_message(&sm.msg, &topic, channel_id).await?;
                    }
                }
            }
        }
    }
}

async fn get_events(queue_id: &str, last_event_id: i64) -> Result<Vec<Event>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90)) // because longpolling
        .build()
        .map_err(|e| format!("failed to build client: {:?}", e))?;

    let resp = client
        .get("https://recurse.zulipchat.com/api/v1/events")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .query(&[
            ("queue_id", queue_id.to_string()),
            ("last_event_id", last_event_id.to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("failed to get events response: {:?}", e))?
        .json::<GetEventsResponse>()
        .await
        .map_err(|e| format!("failed to JSON format get events response: {:?}", e))?;

    if resp.result != "success" {
        return Err(format!(
            "got an error registering queue: {:?} {:?}",
            resp.msg, resp.code
        ));
    }

    resp.events.ok_or_else(|| "no events in response".into())
}

#[derive(Debug, Deserialize)]
pub struct RegisterEventResponse {
    queue_id: Option<String>,

    // "error" or "success"
    result: String,
    // Set for errors
    msg: Option<String>,
    // Also set for errors
    code: Option<String>,
}

pub enum ListenType {
    DM,
    Mention,
}

pub enum EventType {
    Message,
    UpdateMessage,
}

impl EventType {
    fn to_json(&self) -> &str {
        match self {
            EventType::Message => r#"["message"]"#,
            EventType::UpdateMessage => r#"["update_message"]"#,
        }
    }
}

// Returns the queue ID
async fn register_event_queue(
    listen_type: ListenType,
    event_type: EventType,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://recurse.zulipchat.com/api/v1/register")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .form(&[
            ("event_types", event_type.to_json()),
            ("all_public_streams", "true"),
            (
                "narrow",
                match listen_type {
                    ListenType::DM => r#"[["is", "dm"]]"#,
                    ListenType::Mention => r#"[["is", "mentioned"]]"#,
                },
            ),
            ("include_subscribers", "false"),
        ])
        .send()
        .await?
        .json::<RegisterEventResponse>()
        .await?;

    if resp.result != "success" {
        return Err(format!(
            "got an error registering queue: {:?} {:?}",
            resp.msg, resp.code
        )
        .into());
    }

    resp.queue_id
        .ok_or_else(|| "no queue id in response".into())
}

async fn send_direct_message(msg: &str, user_id: u64) -> Result<(), String> {
    let client = reqwest::Client::new();
    let mut id = "[".to_string();
    id.push_str(&user_id.to_string());
    id.push(']');
    client
        .post("https://recurse.zulipchat.com/api/v1/messages")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .query(&[("type", "direct"), ("to", &id), ("content", msg)])
        .send()
        .await
        .map_err(|e| format!("failed to get send direct message: {:?}", e))?;

    Ok(())
}

pub async fn get_message(msg_id: u64) -> Result<Message, String> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://recurse.zulipchat.com/api/v1/messages")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .query(&[
            ("message_ids", format!("[{msg_id}]")),
            ("apply_markdown", "false".to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("failed to get message: {:?}", e))?
        .json::<GetMessagesResponse>()
        .await
        .map_err(|e| format!("failed to JSON format get messages response: {:?}", e))?;

    match response.messages {
        Some(mut messages) => {
            if messages.len() != 1 {
                return Err("wrong number of messages".to_string());
            }
            Ok(messages.pop().unwrap())
        }
        None => Err("no messages in response".to_string()),
    }
}

async fn send_message(msg: &str, topic: &str, channel_id: u64) -> Result<(), String> {
    let mut id = "[".to_string();
    id.push_str(&channel_id.to_string());
    id.push(']');

    let client = reqwest::Client::new();
    client
        .post("https://recurse.zulipchat.com/api/v1/messages")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .query(&[
            ("type", "stream"),
            ("to", &id),
            ("topic", topic),
            ("content", msg),
        ])
        .send()
        .await
        .map_err(|e| format!("failed to get send message: {:?}", e))?;

    Ok(())
}

pub async fn download_image(path: &str, dst: &str) -> Result<(), String> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    if Path::new(dst).exists() {
        println!(
            "Skipping download for {} as we already have it locally",
            path
        );
    }

    let client = reqwest::Client::new();

    // Construct the full URL
    let url = format!("https://recurse.zulipchat.com/api/v1{}", path);

    // Make the GET request with authentication
    let response = client
        .get(&url)
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .send()
        .await
        .map_err(|e| format!("failed to get image URL for download: {:?}", e))?
        .json::<GetImageResponse>()
        .await
        .map_err(|e| format!("failed to JSON format get image response: {:?}", e))?;

    if response.result != "success" {
        return Err(format!("unexpected response result {}", response.result));
    }

    let response = client
        .get(format!("https://recurse.zulipchat.com{}", response.url))
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .send()
        .await
        .map_err(|e| format!("failed to download image: {:?}", e))?;

    // Check if the download request was successful
    if !response.status().is_success() {
        return Err(format!(
            "failed to download image, status: {}",
            response.status()
        ));
    }

    // Create the destination file
    let mut file = tokio::fs::File::create(dst)
        .await
        .map_err(|e| format!("failed to create file {}: {:?}", dst, e))?;

    // Stream the response body directly to the file
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("failed to read chunk: {:?}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("failed to write chunk to file: {:?}", e))?;
    }

    file.flush()
        .await
        .map_err(|e| format!("failed to flush file: {:?}", e))?;

    Ok(())
}
