use std::collections::HashMap;
use std::error::Error;
use std::io::BufRead;
use std::process::Command;
use std::{
    env,
    fs::{self, File},
    io::BufReader,
    path::Path,
};
use tera::Tera;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1) {
        Some(cmd) => match cmd.as_str() {
            "create_blog" => return create_blog(&args[2..]),
            "add_post" => add_post(&args[2..]),
            _ => return Err(format!("unknown command {} specified", cmd).into()),
        },
        None => return Err("no command given!".into()),
    }
}

fn read_metadata_file<P: AsRef<Path>>(
    path: P,
) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    let md_file = File::open(path)?;
    let md_reader = BufReader::new(md_file);

    let mut m: HashMap<String, String> = HashMap::new();
    for line in md_reader.lines() {
        let line = line?;
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

fn create_blog(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // metadata_file_path
    if args.len() < 1 {
        return Err("not enough args given".into());
    }
    let tera = Tera::new("templates/config.toml")?;

    let metadata_file_path = &args[0];
    let m = read_metadata_file(metadata_file_path)?;

    let user_domain = match m.get("SUBDOMAIN") {
        Some(v) => v,
        None => "",
    };

    let mut context = tera::Context::new();
    context.insert("user_domain", user_domain);
    context.insert("blog_name", m.get("BLOG_NAME").unwrap_or(&"".to_string()));
    context.insert("author_name", m.get("AUTHOR").unwrap_or(&"".to_string()));

    let root = env::var("BLOG_ROOT").unwrap(); // Something like path/to/blogs/
    let blog_dir = Path::new(&root).join(user_domain);

    // Make the directory
    fs::create_dir(&blog_dir)?;
    // Create the subdirectories
    fs::create_dir(blog_dir.join("content"))?;
    fs::create_dir(blog_dir.join("templates"))?;
    fs::create_dir(blog_dir.join("themes"))?;

    // Symlink in the theme content
    std::os::unix::fs::symlink(
        env::var("TEMPLATE_ROOT").unwrap(),
        blog_dir.join("themes").join("terminimal"),
    )?;

    // Write the templated config file
    let config_file = File::create(blog_dir.join("config.toml"))?;
    tera.render_to("templates/config.toml", &context, &config_file)
        .unwrap();

    run_zola(blog_dir)?;

    Ok(())
}

fn add_post(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // metadata_file_path post_id post_title post_markdown_file
    if args.len() < 4 {
        return Err("not enough args given".into());
    }

    let metadata_file_path = &args[0];
    let post_id = &args[1];
    let post_title = &args[2];
    let post_markdown_file = &args[3];

    let m = read_metadata_file(metadata_file_path)?;

    let user_domain = match m.get("SUBDOMAIN") {
        Some(v) => v,
        None => "",
    };

    let post_markdown = fs::read_to_string(post_markdown_file)?;

    let tera = Tera::new("templates/post.md")?;

    let mut context = tera::Context::new();
    context.insert("post_title", post_title);
    context.insert("post_markdown", &post_markdown);

    let root = env::var("BLOG_ROOT").unwrap(); // Something like path/to/blogs/
    let blog_dir = Path::new(&root).join(user_domain);

    let post_file = File::create(blog_dir.join("content").join(post_id.to_string() + ".md"))?;
    tera.render_to("templates/post.md", &context, &post_file)
        .unwrap();

    run_zola(blog_dir)?;
    Ok(())
}

fn run_zola<P: AsRef<Path>>(zola_dir: P) -> Result<(), Box<dyn Error>> {
    let mut build_cmd = Command::new("zola");
    build_cmd.arg("build");
    build_cmd.current_dir(zola_dir);
    let status = build_cmd.status()?;

    if !status.success() {
        return Err("error running command".into());
    }
    Ok(())
}
