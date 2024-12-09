use redb::{Database, ReadableTable, TableDefinition};
use std::env;
use std::sync::Arc;
use zulip::{ListenType, EventType, Message, SendMessage};

mod bloggen;
mod zulip;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file.
    // Fails if .env file not found, not readable or invalid.
    dotenvy::dotenv()?;

    let db_path = env::var("DATABASE_PATH").unwrap();
    let db = Arc::new(Database::create(db_path)?);
    let dm_db = Arc::clone(&db);
    let mention_db = Arc::clone(&db);
    let update_db = Arc::clone(&db);

    let dm_handle = tokio::spawn(async move {
        zulip::call_on_each_message(ListenType::DM, EventType::Message, |msg| {
            let response_msg = match create_blog(&dm_db, &msg) {
                Ok(subdomain) => &format!(
                    "Blog created successfully! You can access your beautiful new blog at https://{}.hypertxt.io",
                    subdomain
                ),
                Err(e) => &format!("Uh oh, something went wrong. Error: {:?}", e),
            };
            println!("Response {}", response_msg);
            return Some(SendMessage {
                msg_type: zulip::SendMessageType::Direct(msg.sender_id),
                msg: response_msg.to_string(),
            });
        })
        .await
        .unwrap();
    });

    let mention_handle = tokio::spawn(async move {
        zulip::call_on_each_message(ListenType::Mention, EventType::Message, |msg| {
            let response_msg = match add_post(&mention_db, &msg) {
                Ok(subdomain) => &format!(
                    "Post published successfully! You can view it at https://{}.hypertxt.io",
                    subdomain
                ),
                Err(e) => &format!("Uh oh, something went wrong. Error: {:?}", e),
            };
            println!("Response {}", response_msg);
            if let Some(stream_id) = msg.stream_id {
                return Some(SendMessage {
                    msg_type: zulip::SendMessageType::Channel(msg.subject.clone(), stream_id),
                    msg: response_msg.to_string(),
                });
            }
            return None;
        })
        .await
        .unwrap();
    });

    let update_handle = tokio::spawn(async move {
        zulip::call_on_each_message(ListenType::Mention, EventType::UpdateMessage, |msg| {
            let response_msg = match add_post(&update_db, &msg) {
                Ok(subdomain) => &format!(
                    "Post edited successfully! You can view it at https://{}.hypertxt.io",
                    subdomain
                ),
                Err(e) => &format!("Uh oh, something went wrong. Error: {:?}", e),
            };
            println!("Response {}", response_msg);
            if let Some(stream_id) = msg.stream_id {
                return Some(SendMessage {
                    msg_type: zulip::SendMessageType::Channel(msg.subject.clone(), stream_id),
                    msg: response_msg.to_string(),
                });
            }
            return None;
        })
        .await
        .unwrap();
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

fn create_blog(db: &Database, msg: &Message) -> Result<String, Box<dyn std::error::Error>> {
    let user_id = msg.sender_id;

    let md = bloggen::parse_metadata(&msg.content)?;

    let subdomain = match md.get("SUBDOMAIN") {
        Some(v) => v.clone(),
        None => return Err("couldn't find a requested SUBDOMAIN".into()),
    };

    let read_tx = db.begin_read()?;
    {
        let t1 = read_tx.open_table(USER_ID_TO_SUBDOMAIN_TABLE)?;
        let t2 = read_tx.open_table(SUBDOMAIN_TO_USER_ID_TABLE)?;

        if let Some(v) = t1.get(&user_id)? {
            if v.value() != subdomain {
                return Err(format!("You've already got a blog at https://{}.hypertxt.io and you can only have one!", v.value()).into());
            }
            return Err("You've already got a blog at that subdomain".into());
        }

        if let Some(v) = t2.get(subdomain.as_str())? {
            if v.value() != user_id {
                return Err("That subdomain is taken! Try another one".into());
            }
            return Err("You've already got a blog at that subdomain".into());
        }
    }

    println!("Creating blog for {}", msg.sender_full_name);
    bloggen::create_blog(md)?;
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

    Ok(subdomain)
}

fn add_post(db: &Database, msg: &Message) -> Result<String, Box<dyn std::error::Error>> {
    // assuming a blog is created, publish a post!
    // in markdown at file: user_content/{sender_id}/{id}.md
    // takes post_title from top of md file, demarcated by #

    let user_id = msg.sender_id;
    let message_id = msg.id;

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
        t2.insert(&message_id, msg.content.as_str())?;

        let subdomain = {
            match t3.get(&user_id)? {
                Some(v) => String::from(v.value()),
                None => "".to_string(),
            }
        };
        subdomain
    };
    txn.commit()?;

    bloggen::add_post(&subdomain, message_id, &msg.content, msg.timestamp)?;

    Ok(subdomain)
}
