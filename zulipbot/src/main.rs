#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  crate::register_event_queue()?;
  Ok(())
}