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

fn todays_date(timestamp: u64) -> String {
    let ts = DateTime::from_timestamp(timestamp as i64, 0).unwrap();
    let timezone: Tz = "America/New_York".parse().unwrap();
    let local_time: DateTime<Tz> = ts.with_timezone(&timezone);
    local_time.format("%Y-%m-%d").to_string()
}

pub fn add_post(
    user_subdomain: &str,
    post_id: u64,
    raw_msg: &str,
    timestamp: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let raw_msg = raw_msg.replace("@**Blog Bot (HyperTXT)**", "");

    let (post_title, line_to_remove) = find_title(&raw_msg);

    let post_title = match post_title {
        Some(v) => v.to_string(),
        None => todays_date(timestamp),
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

    let tera = Tera::new(
        Path::new(&env::var("TEMPLATES_ROOT").unwrap())
            .join("*.md")
            .to_str()
            .unwrap(),
    )?;

    let mut context = tera::Context::new();
    context.insert("post_title", &post_title);
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
