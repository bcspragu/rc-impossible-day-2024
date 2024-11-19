use serde::Deserialize;
use std::env;
use std::fs;
use std::io;

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
        // assume user dm's bot to make new blog
        create_blog(resp.events);
    });

    let mention_handle = tokio::spawn(async move {
        let resp = register_event_queue(ListenType::Mention).await.unwrap();
        println!("Created event queue with ID {}", resp.queue_id);

        let resp = get_events(&resp.queue_id).await.unwrap();
        println!("GOT SOME MENTIONED EVENTS!!!!");
        println!("{:?}", resp.events);
        // assume users mention post in checkins channel
        add_post(resp.events);
    });

    futures::future::join_all([dm_handle, mention_handle]).await;

    Ok()
}

fn create_blog(events: GetEventsResponse) -> Result<(), Box<dyn std::error::Error>> {
    // create directory structure with fs:
    // parse message to get relevant metadata of format: 
    // SUBDOMAIN: ...
    // BLOG_NAME: ...
    // AUTHOR: ...

    let root = env::var("USER_CONTENT_ROOT")

    for event in events {
      let uid = event.message.sender_id;
      // parse message here, currently assuming metadata is in desired format
      let metadata = event.message.content;

      fs::create_dir("{}/user_content/{}", root, uid)?;
      let mut f = fs::File::create_new("{}/user_content/{}/metadata", root, uid)?;
      f.write_all(metadata.as_bytes())?;
    }

    Ok()
}

fn add_post(events: GetEventsResponse) -> Result<(), Box<dyn std::error::Error>> {
    // assuming a blog is created, publish a post!
    // in markdown at file: user_content/{sender_id}/{id}.md
    // takes post_title from top of md file, demarcated by #

    let root = env::var("USER_CONTENT_ROOT")

    for event in events {
      let uid = events.message.sender_id;
      let mid = events.message.id
      let blog = events.message.content
      let mut f = fs::File::create_new("/user_content/{}/{}.md", uid, mid)?;
      f.write_all(blog.as_bytes())?;
    }

    Ok()
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
