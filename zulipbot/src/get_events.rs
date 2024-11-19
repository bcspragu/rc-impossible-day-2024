use std::collections::HashMap;
use std::env;

struct GetEventsResponse {
    events: Vec<Event>
}

struct Event {
    type: String,
    message: Message
}

struct Message {
    content: String,
    id: uint64,
    sender_id: uint64,
    timestamp: uint64
}

async fn get_events() -> Result<GetEventsResponse, reqwest::Error>> {
    let client = reqwest::Client::new();
    let response = client.get("https://recurse.zulipchat.com/api/v1/events")
        .basic_auth("hypertxt-bot@recurse.zulipchat.com", Some(env::var("BOT_PASSWORD").unwrap_or_default()))
        .query(&[
            ("queue_id", env::var("QUEUE_ID").unwrap_or_default()),
            ("last_event_id", "-1")
        ])
        .send()
        .await?;
        .json::<GetEventsResponse>()
        .await?;
    println!("{resp:#?}");

    Ok(())
}
