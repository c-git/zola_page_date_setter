use std::{fs, path::Path};

use anyhow::{bail, Context};
use log::{error, info, trace};
use once_cell::sync::Lazy;
use regex::Regex;

pub fn walk_directory(root_path: &Path) -> anyhow::Result<()> {
    if root_path.is_file() {
        if let Err(e) =
            process_file(root_path).with_context(|| format!("Processing failed for: {root_path:?}"))
        {
            error!("{e}");
        };
    } else {
        for entry in fs::read_dir(root_path)
            .with_context(|| format!("Failed to read directory: {root_path:?}"))?
        {
            let entry =
                entry.with_context(|| format!("Failed to extract a DirEntry in {root_path:?}"))?;
            let path = entry.path();
            walk_directory(&path)?;
        }
    }

    Ok(())
}

fn process_file(path: &Path) -> anyhow::Result<()> {
    if !should_skip_file(path) {
        let data = extract_file_data(path)?;

        // TODO Parse toml with https://docs.rs/toml_edit/latest/toml_edit/visit_mut/index.html
        info!("{path:?} (processed)");
    } else {
        trace!("Skipped {path:?}");
    }
    Ok(())
}

struct FileData {
    front_matter: String,
    content: String,
}

static TOML_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^[[:space:]]*\+\+\+(\r?\n(?s).*?(?-s))\+\+\+[[:space:]]*(?:$|(?:\r?\n((?s).*(?-s))$))",
    )
    .unwrap()
});

/// Split the file data into front matter and content
fn extract_file_data(path: &Path) -> anyhow::Result<FileData> {
    // Patterned on zola code https://github.com/c-git/zola/blob/3a73c9c5449f2deda0d287f9359927b0440a77af/components/content/src/front_matter/split.rs#L46

    let content = fs::read_to_string(path).context("Failed to read file")?;

    // 2. extract the front matter and the content
    let caps = if let Some(caps) = TOML_RE.captures(&content) {
        caps
    } else {
        bail!("Failed to find front matter");
    };
    // caps[0] is the full match
    // caps[1] => front matter
    // caps[2] => content
    let front_matter = caps.get(1).unwrap().as_str().to_string();
    let content = caps.get(2).map_or("", |m| m.as_str()).to_string();

    Ok(FileData {
        front_matter,
        content,
    })
}

fn should_skip_file(path: &Path) -> bool {
    !path.extension().is_some_and(|ext| ext == "md") || path.ends_with("_index.md")
}
