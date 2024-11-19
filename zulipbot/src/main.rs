use serde::Deserialize;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file.
    // Fails if .env file not found, not readable or invalid.
    dotenvy::dotenv()?;

    let dm_handle = tokio::spawn(async move {
        let resp = register_event_queue(ListenType::DM).await.unwrap();
        println!("Created event queue with ID {}", resp.queue_id);

        let resp = get_events(&resp.queue_id).await.unwrap();
        println!("GOT SOME DM EVENTS!!!!");
        println!("{:?}", resp.events);
    });

    let mention_handle = tokio::spawn(async move {
        let resp = register_event_queue(ListenType::Mention).await.unwrap();
        println!("Created event queue with ID {}", resp.queue_id);

        let resp = get_events(&resp.queue_id).await.unwrap();
        println!("GOT SOME MENTIONED EVENTS!!!!");
        println!("{:?}", resp.events);
    });

    futures::future::join_all([dm_handle, mention_handle]).await;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct GetEventsResponse {
    events: Vec<Event>,
}

#[derive(Debug, Deserialize)]
struct Event {
    r#type: String,
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
    id: u64,
    sender_id: u64,
    timestamp: u64,
}

async fn get_events(queue_id: &str) -> Result<GetEventsResponse, reqwest::Error> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://recurse.zulipchat.com/api/v1/events")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .query(&[
            ("queue_id", queue_id.to_string()),
            ("last_event_id", "-1".to_string()),
        ])
        .send()
        .await?
        .json::<GetEventsResponse>()
        .await?;

    Ok(response)
}

#[derive(Debug, Deserialize)]
pub struct RegisterEventResponse {
    queue_id: String,
}

enum ListenType {
    DM,
    Mention,
}

pub async fn register_event_queue(
    listen_type: ListenType,
) -> Result<RegisterEventResponse, reqwest::Error> {
    let client = reqwest::Client::new();
    let response = client
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

    Ok(response)
}
