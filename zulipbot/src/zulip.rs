use std::{env, time::Duration};

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
pub struct Event {
    r#type: String,
    pub id: u64,
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
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

pub async fn call_on_each_message<F>(listen_type: ListenType, mut callback: F) -> Result<(), String>
where
    F: FnMut(&Message) -> Option<SendMessage>,
{
    let queue_id = register_event_queue(listen_type).await.unwrap();

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
            let msg = match &ev.message {
                Some(msg) => msg,
                None => {
                    println!("message with type {} had no message", ev.r#type);
                    continue;
                }
            };
            if msg.sender_full_name.contains("Blog Bot") {
                // Ignore DMs sent by Blog Bot
                continue;
            }
            let send_msg = callback(msg);

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
        )
        .into());
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

// Returns the queue ID
async fn register_event_queue(
    listen_type: ListenType,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://recurse.zulipchat.com/api/v1/register")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .form(&[
            ("event_types", r#"["message"]"#),
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
    id.push_str("]");
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

async fn send_message(msg: &str, topic: &str, channel_id: u64) -> Result<(), String> {
    let mut id = "[".to_string();
    id.push_str(&channel_id.to_string());
    id.push_str("]");

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