use std::env

struct RegisterEventResponse {
  queue_id: String
}

async fn register_event_queue() -> Result<RegisterEventResponse, reqwest::Error> {
    let client = reqwest::Client::new();
    let response = client.post("https://recurse.zulipchat.com/api/v1/register")
        .basic_auth("username", Some("password"))
        .form(&[
            ("event_types", r#"["message"]"#),
            ("all_public_streams", "true"),
            ("narrow", r#"[["is", "dm"]]"#),
            ("include_subscribers", "false")
        ])
        .send()
        .await?;
        .json::<RegisterEventResponse>()
        .await?;
    
    Ok(())
}