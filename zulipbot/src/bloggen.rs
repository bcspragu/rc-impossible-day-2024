use chrono::DateTime;
use chrono_tz::Tz;
use std::collections::HashMap;
use std::error::Error;
use std::path;
use std::process::Command;
use std::{
    env,
    fs::{self, File},
    path::Path,
};
use tera::Tera;

use crate::zulip;

pub fn parse_metadata(md: &str) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    let mut m: HashMap<String, String> = HashMap::new();
    for line in md.lines() {
        let (k, v) = match line.rsplit_once(": ") {
            Some(kv) => kv,
            None => return Err("invalid line found".into()),
        };
        m.insert(k.to_string(), v.to_string());
    }

    if !m.contains_key("SUBDOMAIN") {
        return Err("no subdomain found in config!".into());
    }

    Ok(m)
}

pub fn create_blog(m: HashMap<String, String>) -> Result<(), Box<dyn std::error::Error>> {
    let tera = Tera::new(
        Path::new(&env::var("TEMPLATES_ROOT").unwrap())
            .join("*.toml")
            .to_str()
            .unwrap(),
    )?;

    println!("METADATA: {:?}", m);

    let user_domain = match m.get("SUBDOMAIN") {
        Some(v) => v,
        None => "",
    };

    let mut context = tera::Context::new();
    context.insert(
        "user_domain",
        format!("{}.hypertxt.io", user_domain).as_str(),
    );
    context.insert("blog_name", m.get("BLOG_NAME").unwrap_or(&"".to_string()));
    context.insert("author_name", m.get("AUTHOR").unwrap_or(&"".to_string()));

    let root = env::var("BLOG_ROOT").unwrap();
    let blog_dir = Path::new(&root).join(user_domain);

    // Make the directory
    fs::create_dir(&blog_dir).ok();
    // Create the subdirectories
    fs::create_dir(blog_dir.join("content")).ok();
    fs::create_dir(blog_dir.join("templates")).ok();

    // Symlink in the theme content
    let themes_root = path::absolute(env::var("THEMES_ROOT").unwrap())?;
    std::os::unix::fs::symlink(themes_root, blog_dir.join("themes"))?;

    // Write the templated config file
    let config_file = File::create(blog_dir.join("config.toml"))?;
    tera.render_to("config.toml", &context, &config_file)?;
    let content_index_file = File::create(blog_dir.join("content/_index.md"))?;
    tera.render_to("_index.md", &tera::Context::new(), &content_index_file)?;

    let static_root =
        env::var("STATIC_ROOT").map_err(|e| format!("failed to get static root: {:?}", e))?;

    let out_dir = Path::new(&static_root).join(user_domain);

    run_zola(blog_dir, out_dir)?;

    Ok(())
}

fn find_title(msg: &str) -> (Option<&str>, Option<usize>) {
    for (idx, line) in msg.lines().enumerate() {
        if let Some(title) = line.strip_prefix("# ") {
            if !title.is_empty() {
                return (Some(title), Some(idx));
            }
        }
        if let Some(title) = line.strip_prefix("TITLE: ") {
            if !title.is_empty() {
                return (Some(title), Some(idx));
            }
        }
    }
    (None, None)
}

fn todays_date(timestamp: u64, rfc3339: bool) -> String {
    let ts = DateTime::from_timestamp(timestamp as i64, 0).unwrap();
    let timezone: Tz = "America/New_York".parse().unwrap();
    let local_time: DateTime<Tz> = ts.with_timezone(&timezone);

    if rfc3339 {
        local_time
            .to_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    } else {
        local_time.format("%Y-%m-%d").to_string()
    }
}

// The idea is that posts can contain uploaded images, which start with
// `/user_uploads/...`. We want to (heuristically) find all those URLs so that
// we can download them and serve them on the blog.
fn extract_user_upload_urls(markdown: &str) -> Vec<String> {
    let mut urls = Vec::new();

    for line in markdown.lines() {
        let mut remaining = line;
        while let Some(start_idx) = remaining.find("/user_uploads/") {
            let from_start = &remaining[start_idx..];

            // Find the end of the URL (space, ), ", >, or end of line)
            let end_idx = from_start
                .find(|c: char| c.is_whitespace() || c == ')' || c == '"' || c == '>')
                .unwrap_or(from_start.len());

            let url = &from_start[..end_idx];
            urls.push(url.to_string());

            remaining = &from_start[end_idx..];
        }
    }

    urls
}

pub async fn refresh_all_posts(
    user_subdomain: &str,
    post_ids: Vec<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = env::var("BLOG_ROOT").unwrap(); // Something like path/to/blogs/
    let blog_dir = Path::new(&root).join(user_subdomain);
    let static_root =
        env::var("STATIC_ROOT").map_err(|e| format!("failed to get static root: {:?}", e))?;

    // TODO: Probably update this to also regenerate other files, like the config.toml + the content/_index.md

    println!("Refreshing {} posts", post_ids.len());
    for post_id in post_ids {
        let msg = zulip::get_message(post_id).await?;
        let parsed_message = parse_raw_message(&msg.content, msg.timestamp);
        download_images(parsed_message.image_urls, &static_root).await?;
        write_post(
            &blog_dir,
            PostToWrite {
                title: parsed_message.title,
                timestamp: msg.timestamp,
                body: parsed_message.body,
                post_id,
            },
        )?;
    }

    let out_dir = Path::new(&static_root).join(user_subdomain);

    run_zola(blog_dir, out_dir)?;

    Ok(())
}

struct ParsedMessage {
    title: String,
    body: String,
    image_urls: Vec<String>,
}

fn parse_raw_message(raw_msg: &str, timestamp: u64) -> ParsedMessage {
    let msg = raw_msg.replace("@**Blog Bot (HyperTXT)**", "");

    let (post_title, line_to_remove) = find_title(&msg);

    let post_markdown = match line_to_remove {
        Some(ltr) => {
            let val = msg
                .lines()
                .enumerate()
                .filter(|(idx, _)| *idx != ltr)
                .map(|(_, v)| v)
                .fold(String::new(), |mut a, b| {
                    a.reserve(b.len() + 1);
                    a.push_str(b);
                    a.push('\n');
                    a
                });
            val.trim().to_owned()
        }
        None => msg.clone(),
    };

    // TODO: Zulip messages only inline the images as links, we should
    // update this to automatically turn them into Markdown images too.
    // E.g. ![text](path/to/image) insteaad of [text](path/to/image)
    let image_urls = extract_user_upload_urls(&post_markdown);

    ParsedMessage {
        title: post_title
            .map(|v| v.to_string())
            .unwrap_or_else(|| todays_date(timestamp, false)),
        body: post_markdown,
        image_urls,
    }
}

pub async fn add_post(
    user_subdomain: &str,
    post_id: u64,
    raw_msg: &str,
    timestamp: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let msg = parse_raw_message(raw_msg, timestamp);

    let static_root =
        env::var("STATIC_ROOT").map_err(|e| format!("failed to get static root: {:?}", e))?;

    download_images(msg.image_urls, &static_root).await?;

    let root = env::var("BLOG_ROOT").unwrap(); // Something like path/to/blogs/
    let blog_dir = Path::new(&root).join(user_subdomain);

    write_post(
        &blog_dir,
        PostToWrite {
            title: msg.title,
            timestamp,
            body: msg.body,
            post_id,
        },
    )?;

    let out_dir = Path::new(&static_root).join(user_subdomain);

    run_zola(blog_dir, out_dir)?;
    Ok(())
}

async fn download_images(
    image_urls: Vec<String>,
    static_root: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for url in image_urls {
        // Create the destination path: STATIC_ROOT/../user_uploads/...
        // The URL is like /user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583.png
        let relative_path = url.trim_start_matches('/');
        let dst_path = Path::new(static_root)
            .parent()
            .ok_or("STATIC_ROOT has no parent directory")?
            .join(relative_path);

        // Create parent directories if they don't exist
        if let Some(parent) = dst_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create parent dirs for image {:?}: {:?}",
                    parent, e
                )
            })?;
        }

        // Download the image
        println!("Downloading image {}", url);
        if let Err(e) = zulip::download_image(&url, dst_path.to_str().unwrap()).await {
            eprintln!("Failed to download image {}: {}", url, e);
        }
    }

    Ok(())
}

struct PostToWrite {
    title: String,
    timestamp: u64,
    body: String,
    post_id: u64,
}

fn write_post(blog_dir: &Path, post: PostToWrite) -> Result<(), Box<dyn std::error::Error>> {
    let tera = Tera::new(
        Path::new(&env::var("TEMPLATES_ROOT").unwrap())
            .join("*.md")
            .to_str()
            .unwrap(),
    )?;

    let mut context = tera::Context::new();
    context.insert("post_title", &post.title);
    context.insert("post_date", &todays_date(post.timestamp, true));
    context.insert("post_markdown", &post.body);

    let post_file = File::create(
        blog_dir
            .join("content")
            .join(post.post_id.to_string() + ".md"),
    )?;
    tera.render_to("post.md", &context, &post_file).unwrap();
    Ok(())
}

fn run_zola<P: AsRef<Path>, Q: AsRef<Path>>(zola_dir: P, out_dir: Q) -> Result<(), Box<dyn Error>> {
    let mut build_cmd = Command::new("zola");
    build_cmd.arg("build");
    build_cmd.arg("--force");
    build_cmd.current_dir(zola_dir);
    build_cmd.args(["--output-dir", out_dir.as_ref().to_str().unwrap()]);
    let status = build_cmd.status()?;

    if !status.success() {
        return Err("error running command".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_markdown_image_single_url() {
        let markdown = "![alt text](/user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583.png)";
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0],
            "/user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583.png"
        );
    }

    #[test]
    fn test_extract_html_img_tag() {
        let markdown = r#"<img src="/user_uploads/42/xyz789/image.jpg">"#;
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "/user_uploads/42/xyz789/image.jpg");
    }

    #[test]
    fn test_extract_multiple_urls_single_line() {
        let markdown =
            "![img1](/user_uploads/1/a/img1.png) and ![img2](/user_uploads/2/b/img2.jpg)";
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "/user_uploads/1/a/img1.png");
        assert_eq!(urls[1], "/user_uploads/2/b/img2.jpg");
    }

    #[test]
    fn test_extract_multiple_urls_multiple_lines() {
        let markdown = r#"
# My Post

Here is an image: ![photo](/user_uploads/13/abc/photo.png)

And here is another one: [photo 2](/user_uploads/14/def/p2.jpeg)

And a third: <img src="/user_uploads/15/ghi/banner.jpg">
"#;
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], "/user_uploads/13/abc/photo.png");
        assert_eq!(urls[1], "/user_uploads/14/def/doc.pdf");
        assert_eq!(urls[2], "/user_uploads/15/ghi/banner.jpg");
    }

    #[test]
    fn test_extract_empty_string() {
        let markdown = "";
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 0);
    }

    #[test]
    fn test_extract_no_urls() {
        let markdown = r#"
# Blog Post

This is a regular blog post with no user uploads.
It has regular images from external sources:
![external](https://example.com/image.png)
"#;
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 0);
    }

    #[test]
    fn test_extract_mixed_content() {
        let markdown = r#"
Here's my post with [a link](/user_uploads/1/a/file.pdf) and regular text.
Also an external ![image](https://example.com/pic.jpg) and an internal ![image](/user_uploads/2/b/pic2.png).
"#;
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "/user_uploads/1/a/file.pdf");
        assert_eq!(urls[1], "/user_uploads/2/b/pic2.png");
    }

    #[test]
    fn test_extract_url_with_special_characters() {
        let markdown = "![image](/user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583-edited.png)";
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0],
            "/user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583-edited.png"
        );
    }

    #[test]
    fn test_extract_url_at_end_of_line() {
        let markdown = "Check out this file: /user_uploads/20/xyz/document.txt";
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "/user_uploads/20/xyz/document.txt");
    }

    #[test]
    fn test_extract_url_with_trailing_whitespace() {
        let markdown = "![image](/user_uploads/30/abc/image.png) ";
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "/user_uploads/30/abc/image.png");
    }
}
