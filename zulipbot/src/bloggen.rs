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

// TODO(russell): Use this, e.g. zulip::download_image
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

    let out_dir = match env::var("STATIC_ROOT") {
        Ok(v) => Some(Path::new(&v).join(user_domain)),
        Err(_) => None,
    };

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
        local_time.to_rfc3339()
    } else {
        local_time.format("%Y-%m-%d").to_string()
    }
}

fn extract_user_upload_urls(markdown: &str) -> Vec<String> {
    let mut urls = Vec::new();

    // Match markdown images: ![alt text](/user_uploads/...)
    // Match markdown links: [text](/user_uploads/...)
    // Match HTML img tags: <img src="/user_uploads/...">

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

pub async fn add_post(
    user_subdomain: &str,
    post_id: u64,
    raw_msg: &str,
    timestamp: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let raw_msg = raw_msg.replace("@**Blog Bot (HyperTXT)**", "");

    let (post_title, line_to_remove) = find_title(&raw_msg);

    let post_title = match post_title {
        Some(v) => v.to_string(),
        None => todays_date(timestamp, false),
    };

    let post_markdown = match line_to_remove {
        Some(ltr) => {
            let val = raw_msg
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
        None => raw_msg,
    };

    // Extract and download images from /user_uploads/
    let static_root = env::var("STATIC_ROOT").ok();
    if let Some(static_root_path) = &static_root {
        // Find all /user_uploads/ URLs in the markdown
        let image_urls = extract_user_upload_urls(&post_markdown);

        for url in image_urls {
            // Create the destination path: STATIC_ROOT/../user_uploads/...
            // The URL is like /user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583.png
            let relative_path = url.trim_start_matches('/');
            let dst_path = Path::new(static_root_path)
                .parent()
                .ok_or("STATIC_ROOT has no parent directory")?
                .join(relative_path);

            // Create parent directories if they don't exist
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent).ok();
            }

            // Download the image
            if let Err(e) = zulip::download_image(&url, dst_path.to_str().unwrap()).await {
                eprintln!("Failed to download image {}: {}", url, e);
            }
        }
    }

    let tera = Tera::new(
        Path::new(&env::var("TEMPLATES_ROOT").unwrap())
            .join("*.md")
            .to_str()
            .unwrap(),
    )?;

    let mut context = tera::Context::new();
    context.insert("post_title", &post_title);
    context.insert("post_date", &todays_date(timestamp, true));
    context.insert("post_markdown", &post_markdown);

    let root = env::var("BLOG_ROOT").unwrap(); // Something like path/to/blogs/
    let blog_dir = Path::new(&root).join(user_subdomain);

    let post_file = File::create(blog_dir.join("content").join(post_id.to_string() + ".md"))?;
    tera.render_to("post.md", &context, &post_file).unwrap();

    let out_dir = match env::var("STATIC_ROOT") {
        Ok(v) => Some(Path::new(&v).join(user_subdomain)),
        Err(_) => None,
    };

    run_zola(blog_dir, out_dir)?;
    Ok(())
}

fn run_zola<P: AsRef<Path>, Q: AsRef<Path>>(
    zola_dir: P,
    out_dir: Option<Q>,
) -> Result<(), Box<dyn Error>> {
    let mut build_cmd = Command::new("zola");
    build_cmd.arg("build");
    build_cmd.arg("--force");
    build_cmd.current_dir(zola_dir);
    if let Some(out_dir) = out_dir {
        build_cmd.args(["--output-dir", out_dir.as_ref().to_str().unwrap()]);
        println!("output dir set to {}", out_dir.as_ref().to_str().unwrap());
    }
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
        assert_eq!(urls[0], "/user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583.png");
    }

    #[test]
    fn test_extract_markdown_link_single_url() {
        let markdown = "[click here](/user_uploads/13/abc123/file.pdf)";
        let urls = extract_user_upload_urls(markdown);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "/user_uploads/13/abc123/file.pdf");
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
        let markdown = "![img1](/user_uploads/1/a/img1.png) and ![img2](/user_uploads/2/b/img2.jpg)";
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

And here is a link: [document](/user_uploads/14/def/doc.pdf)

And another image: <img src="/user_uploads/15/ghi/banner.jpg">
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
        assert_eq!(urls[0], "/user_uploads/13/SJXAkls4A6mqvoVyWpeciPlO/DSC_0583-edited.png");
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
