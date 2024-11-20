use redb::{Database, ReadableTable, TableDefinition};
use serde::Deserialize;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;

mod bloggen;

async fn send_direct_message(msg: &str, user_id: u64) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let mut id = "[".to_string();
    id.push_str(&user_id.to_string());
    id.push_str("]");
    let response = client
        .post("https://recurse.zulipchat.com/api/v1/messages")
        .basic_auth(
            "hypertxt-bot@recurse.zulipchat.com",
            Some(env::var("BOT_PASSWORD").unwrap_or_default()),
        )
        .query(&[("type", "direct"), ("to", &id), ("content", msg)])
        .send()
        .await?;

    if response.status() != 200 {
        println!("Error response: {:?}", response.text().await);
    }

    Ok(())
}

async fn send_message(
    msg: &str,
    topic: &str,
    channel_id: u64,
) -> Result<(), Box<dyn std::error::Error>> {
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
        .await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file.
    // Fails if .env file not found, not readable or invalid.
    dotenvy::dotenv()?;

    let db_path = env::var("DATABASE_PATH").unwrap();
    let db = Arc::new(Database::create(db_path)?);
    let dm_db = Arc::clone(&db);
    let mention_db = Arc::clone(&db);

    let dm_handle = tokio::spawn(async move {
        let resp = register_event_queue(ListenType::DM).await.unwrap();

        // assume user dm's bot to make new blog
        let mut last_event_id = -1i64;
        loop {
            let resp = match get_events(&resp.queue_id, last_event_id).await {
                Ok(resp) => resp,
                Err(e) => {
                    println!("error getting events {:?}", e);
                    // Usually just means we're polling and didn't get anything
                    time::sleep(Duration::from_millis(2500)).await;
                    continue;
                }
            };
            for ev in resp.events {
                last_event_id = i64::max(last_event_id, ev.id as i64);
                if ev.message.sender_full_name.contains("Blog Bot") {
                    // Ignore DMs sent by Blog Bot
                    continue;
                }
                let response_msg = match create_blog(&dm_db, &ev) {
                    Ok(_) => "Blog created successfully!",
                    Err(e) => &format!("Uh oh, something went wrong. Error: {:?}", e),
                };
                println!("Response {}", response_msg);
                send_direct_message(response_msg, ev.message.sender_id)
                    .await
                    .unwrap();
            }
        }
    });

    let mention_handle = tokio::spawn(async move {
        let resp = register_event_queue(ListenType::Mention).await.unwrap();

        // assume users mention post in checkins channel
        let mut last_event_id = -1i64;
        loop {
            let resp = match get_events(&resp.queue_id, last_event_id).await {
                Ok(resp) => resp,
                Err(_) => {
                    // Usually just means we're polling and didn't get anything
                    time::sleep(Duration::from_millis(2500)).await;
                    continue;
                }
            };
            for ev in resp.events {
                last_event_id = i64::max(last_event_id, ev.id as i64);
                let response_msg = match add_post(&mention_db, &ev) {
                    Ok(_) => "Post published successfully!",
                    Err(e) => &format!("Uh oh, something went wrong. Error: {:?}", e),
                };
                if let Some(stream_id) = ev.message.stream_id {
                    send_message(response_msg, &ev.message.subject, stream_id)
                        .await
                        .unwrap();
                }
            }
        }
    });

    futures::future::join_all([dm_handle, mention_handle]).await;

    Ok(())
}

const USER_ID_TO_SUBDOMAIN_TABLE: TableDefinition<u64, &str> =
    TableDefinition::new("user_id_to_subdomain");
const SUBDOMAIN_TO_USER_ID_TABLE: TableDefinition<&str, u64> =
    TableDefinition::new("subdomain_to_user_id");
const USER_ID_TO_POST_IDS_TABLE: TableDefinition<u64, Vec<u64>> =
    TableDefinition::new("user_id_to_post_ids");
const POST_ID_TO_POST_TABLE: TableDefinition<u64, &str> = TableDefinition::new("post_id_to_post");

fn create_blog(db: &Database, event: &Event) -> Result<(), Box<dyn std::error::Error>> {
    // create directory structure with fs:
    // parse message to get relevant metadata of format:
    // SUBDOMAIN: ...
    // BLOG_NAME: ...
    // AUTHOR: ...

    println!("Creating blog for {}", event.message.sender_full_name);
    let user_id = event.message.sender_id;
    // let user_root = Path::new(&env::var("USER_CONTENT_ROOT").unwrap())
    //     .join("user_content")
    //     .join(user_id.to_string());
    // let metadata_path = user_root.join("metadata");

    // fs::create_dir(user_root)?;

    // let mut f = fs::File::create_new(metadata_path)?;
    // f.write_all(metadata.as_bytes())?;

    let subdomain = bloggen::create_blog(&event.message.content)?;
    println!("Created blog at {}", subdomain);

    let txn = db.begin_write()?;
    {
        let mut t1 = txn.open_table(USER_ID_TO_SUBDOMAIN_TABLE)?;
        let mut t2 = txn.open_table(SUBDOMAIN_TO_USER_ID_TABLE)?;
        t1.insert(&user_id, subdomain.as_str())?;
        t2.insert(subdomain.as_str(), &user_id)?;
    }
    txn.commit()?;
    println!("Wrote metadata for {} to DB", subdomain);

    Ok(())
}

fn add_post(db: &Database, event: &Event) -> Result<(), Box<dyn std::error::Error>> {
    // assuming a blog is created, publish a post!
    // in markdown at file: user_content/{sender_id}/{id}.md
    // takes post_title from top of md file, demarcated by #

    let user_id = event.message.sender_id;
    let message_id = event.message.id;

    let txn = db.begin_write()?;
    let subdomain = {
        let mut t1 = txn.open_table(USER_ID_TO_POST_IDS_TABLE)?;
        let mut t2 = txn.open_table(POST_ID_TO_POST_TABLE)?;
        let t3 = txn.open_table(USER_ID_TO_SUBDOMAIN_TABLE)?;

        let mut post_ids = match t1.get(&user_id)? {
            Some(v) => v.value(),
            None => vec![],
        };
        post_ids.push(message_id);

        t1.insert(&user_id, post_ids)?;
        t2.insert(&message_id, event.message.content.as_str())?;

        let subdomain = {
            match t3.get(&user_id)? {
                Some(v) => String::from(v.value()),
                None => "".to_string(),
            }
        };
        subdomain
    };
    txn.commit()?;

    bloggen::add_post(
        &subdomain,
        message_id,
        &event.message.content,
        event.message.timestamp,
    )?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct GetEventsResponse {
    events: Vec<Event>,
}

#[derive(Debug, Deserialize)]
struct Event {
    // r#type: String,
    id: u64,
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
    id: u64,
    sender_id: u64,
    stream_id: Option<u64>,
    timestamp: u64,
    subject: String,
    sender_full_name: String,
}

async fn get_events(queue_id: &str, last_event_id: i64) -> Result<GetEventsResponse, String> {
    let client = reqwest::Client::new();
    let response = client
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
        .map_err(|e| format!("error from reqwest {:?}", e))?;

    if response.status() != 200 {
        return Err(format!(
            "non-200 status code {}: {:?}",
            response.status(),
            response.text().await,
        )
        .into());
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("failed to get response text: {:?}", e))?;

    match serde_json::from_str::<GetEventsResponse>(&body) {
        Ok(resp) => Ok(resp),
        Err(e) => {
            if body.contains(r#"heartbeat"#) {
                time::sleep(Duration::from_millis(2500)).await;
                return Ok(GetEventsResponse { events: vec![] });
            }
            println!("raw body: {}", body);
            Err(format!("failed to deserialize: {:?}", e).into())
        }
    }

    // return Ok(response
    //     .json::<GetEventsResponse>()
    //     .await
    //     .map_err(|e| format!("failed to convert to JSON: {:?}", e))?);
}

#[derive(Debug, Deserialize)]
pub struct RegisterEventResponse {
    queue_id: String,
}

enum ListenType {
    DM,
    Mention,
}

async fn register_event_queue(
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
