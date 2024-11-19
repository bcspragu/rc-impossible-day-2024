use std::collections::HashMap;
use std::env;

async fn get_events() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client.get("https://recurse.zulipchat.com/api/v1/events")
        .basic_auth("hypertxt-bot@recurse.zulipchat.com", Some(env::var("BOT_PASSWORD").unwrap_or_default()))
        .query(&[
            ("queue_id", env::var("QUEUE_ID").unwrap_or_default()),
            ("last_event_id", "-1")
        ])
        .send()
        .await?;
        .json::<HashMap<String, String>>()
        .await?;
    println!("{resp:#?}");

    Ok(())
}
