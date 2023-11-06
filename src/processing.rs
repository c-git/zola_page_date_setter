use std::{fs, path::Path};

use anyhow::Context;
use log::{info, trace};

pub fn walk_directory(root_path: &Path) -> anyhow::Result<()> {
    if root_path.is_file() {
        process_file(root_path)?;
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
        // TODO Pattern on zola code https://github.com/c-git/zola/blob/3a73c9c5449f2deda0d287f9359927b0440a77af/components/content/src/front_matter/split.rs#L46
        // TODO Parse toml with https://docs.rs/toml_edit/latest/toml_edit/visit_mut/index.html
        info!("{path:?} (processed)");
    } else {
        trace!("Skipped {path:?}");
    }
    Ok(())
}

fn should_skip_file(path: &Path) -> bool {
    !path.extension().is_some_and(|ext| ext == "md") || path.ends_with("_index.md")
}
