use std::{fs, io::Write, path::Path, process::Command};

use anyhow::{bail, Context};
use chrono::Datelike;
use log::{debug, error, info, trace, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use toml_edit::Document;

pub fn walk_directory(root_path: &Path) -> anyhow::Result<()> {
    if root_path.is_file() {
        if let Err(e) =
            process_file(root_path).with_context(|| format!("Processing failed for: {root_path:?}"))
        {
            error!("{e:?}");
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
        let mut data = extract_file_data(path)?;
        let last_edit_date =
            get_git_last_edit_date(path).context("Failed to get last edit date from git")?;
        data.update_front_matter(last_edit_date)
            .context("Failed to update front_matter")?;
        data.write(path).context("Failed to write to file")?;
        info!("{path:?} (processed)");
    } else {
        trace!("Skipped {path:?}");
    }
    Ok(())
}

fn get_git_last_edit_date(path: &Path) -> anyhow::Result<Option<toml_edit::Date>> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%cs", path.to_string_lossy().as_ref()])
        .output()
        .context("Failed to execute git command")?;
    if !output.status.success() || !output.stderr.is_empty() {
        bail!(
            "Running git failed. status: {} stdout: {}, stderr: {}",
            output.status,
            std::str::from_utf8(&output.stdout)?,
            std::str::from_utf8(&output.stderr)?
        );
    }
    let stdout = std::str::from_utf8(&output.stdout)?;
    debug!("Git Date: {stdout:?} - {path:?}");

    if stdout.is_empty() {
        Ok(None)
    } else {
        let year: u16 = stdout[..=3].parse().context("Failed to parse year")?;
        let month: u8 = stdout[5..=6].parse().context("Failed to parse month")?;
        let day: u8 = stdout[8..=9].parse().context("Failed to parse day")?;
        Ok(Some(toml_edit::Date { year, month, day }))
    }
}

struct FileData<'a> {
    path: &'a Path,
    front_matter: String,
    content: String,
}
impl<'a> FileData<'a> {
    fn write(&self, path: &Path) -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)?;
        let mut s = "+++".to_string();
        s.push_str(&self.front_matter);
        s.push_str("+++\n");
        if !self.content.is_empty() {
            // Added a space between to match `dprint`
            s.push('\n');
        }
        s.push_str(&self.content);
        file.write_all(s.as_bytes())?;
        Ok(())
    }

    /// See cli::Cli command.long for explanation of rules (or readme)
    fn update_front_matter(
        &mut self,
        last_edit_date: Option<toml_edit::Date>,
    ) -> anyhow::Result<()> {
        let key_date = "date";
        let key_updated = "updated";
        let toml = &self.front_matter[..];
        let mut doc = toml
            .parse::<Document>()
            .context("Failed to parse TOML in front matter")?;
        debug_assert_eq!(doc.to_string(), toml);
        let mut date = doc.get(key_date);
        let mut updated = doc.get(key_updated);

        // Check for wrong type
        if let Some(d) = date {
            if !d.is_datetime() {
                warn!("Non date value found for `date` in {:?}", self.path);
                date = None; // Only allow dates
            }
        }
        if let Some(u) = updated {
            if !u.is_datetime() {
                warn!("Non date value found for `updated` in {:?}", self.path);
                updated = None; // Only allow dates
            }
        }

        // Ensure if updated exists it is greater than or equal to date otherwise discard value
        if let Some(updated_date) = updated {
            if let Some(date) = date {
                if is_less_than_date(updated_date, date) {
                    warn!("`updated` is before `date` but this should never happen. `updated` being ignored in {:?}", self.path);
                    updated = None;
                }
            }
        }

        // Set new date values base on the rules.
        // Prefer reusing the existing values as only the date is set in the generated value and not time nor offset
        let (new_date, new_updated) = match (last_edit_date, date, updated) {
            (None, None, _) => (TODAY.clone(), None),
            (None, Some(curr_date), _) => {
                if is_less_than_date(curr_date, &TODAY) {
                    (curr_date.clone(), Some(TODAY.clone()))
                } else if is_equal_date(curr_date, &TODAY) {
                    (curr_date.clone(), None)
                } else {
                    debug_assert!(is_less_than_date(&TODAY, curr_date));
                    (TODAY.clone(), None)
                }
            }
            (Some(last), None, _) => {
                let last = item_from_date(last);
                if is_equal_date(&last, &TODAY) {
                    (last, None)
                } else {
                    (last, Some(TODAY.clone()))
                }
            }
            (Some(last), Some(date), None) => {
                let last = item_from_date(last);
                if is_equal_date(&last, date) {
                    (date.clone(), None)
                } else if is_less_than_date(date, &TODAY) {
                    (date.clone(), Some(TODAY.clone()))
                } else if is_equal_date(date, &TODAY) {
                    (date.clone(), None)
                } else {
                    debug_assert!(is_less_than_date(&TODAY, date));
                    (last, Some(TODAY.clone()))
                }
            }
            (Some(last), Some(date), Some(updated)) => {
                debug_assert!(
                    !is_less_than_date(updated, date),
                    "THIS SHOULDN'T HAPPEN WAS SUPPOSED TO HAVE BEEN CHECKED"
                );
                let last = item_from_date(last);
                if is_less_than_date(date, &last) || is_equal_date(date, &last) {
                    if is_equal_date(date, &last) || is_equal_date(updated, &TODAY) {
                        (date.clone(), Some(updated.clone()))
                    } else {
                        (date.clone(), Some(TODAY.clone()))
                    }
                } else if is_less_than_date(date, &TODAY) || is_equal_date(date, &TODAY) {
                    if is_equal_date(updated, &TODAY) {
                        (date.clone(), Some(updated.clone()))
                    } else {
                        (date.clone(), Some(TODAY.clone()))
                    }
                } else {
                    debug_assert!(is_less_than_date(&TODAY, date));

                    if is_equal_date(updated, &TODAY) {
                        (TODAY.clone(), Some(updated.clone()))
                    } else {
                        (TODAY.clone(), None)
                    }
                }
            }
        };

        doc.insert(key_date, new_date);
        if let Some(nu) = new_updated {
            doc.insert(key_updated, nu);
        } else {
            doc.remove(key_updated);
        }
        self.front_matter = doc.to_string();
        Ok(())
    }
}

/// Checks if both a and b are dates and if a < b
fn is_less_than_date(a: &toml_edit::Item, b: &toml_edit::Item) -> bool {
    match (a, b) {
        (toml_edit::Item::Value(a), toml_edit::Item::Value(b)) => match (a, b) {
            (toml_edit::Value::Datetime(a), toml_edit::Value::Datetime(b)) => {
                match (a.value().date, b.value().date) {
                    (Some(a), Some(b)) => match a.year.cmp(&b.year) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Equal => match a.month.cmp(&b.month) {
                            std::cmp::Ordering::Less => true,
                            std::cmp::Ordering::Equal => a.day < b.day,
                            std::cmp::Ordering::Greater => false,
                        },
                        std::cmp::Ordering::Greater => false,
                    },
                    _ => false,
                }
            }
            _ => false,
        },
        _ => false,
    }
}

fn item_from_date(d: toml_edit::Date) -> toml_edit::Item {
    toml_edit::Item::Value(toml_edit::Value::Datetime(toml_edit::Formatted::new(
        toml_edit::Datetime {
            date: Some(d),
            time: None,
            offset: None,
        },
    )))
}

// Check if both a and b are dates and a == b
fn is_equal_date(a: &toml_edit::Item, b: &toml_edit::Item) -> bool {
    match (a, b) {
        (toml_edit::Item::Value(a), toml_edit::Item::Value(b)) => match (a, b) {
            (toml_edit::Value::Datetime(a), toml_edit::Value::Datetime(b)) => {
                match (a.value().date, b.value().date) {
                    (Some(a), Some(b)) => a.year == b.year && a.month == b.month && a.day == b.day,
                    _ => false,
                }
            }
            _ => false,
        },
        _ => false,
    }
}

#[test]
fn test_is_equal_date() {
    assert!(is_equal_date(&TODAY, &TODAY));
}

#[test]
fn test_is_less_than() {
    let past = item_from_date(toml_edit::Date {
        year: 1900,
        month: 1,
        day: 1,
    });
    assert!(is_less_than_date(&past, &TODAY));
    assert!(!is_less_than_date(&TODAY, &past));
    assert!(!is_less_than_date(&TODAY, &TODAY));
}

static TODAY: Lazy<toml_edit::Item> = Lazy::new(|| {
    let now = chrono::Local::now();
    item_from_date(toml_edit::Date {
        year: now.year() as _,
        month: now.month() as _,
        day: now.day() as _,
    })
});

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
        path,
        front_matter,
        content,
    })
}

fn should_skip_file(path: &Path) -> bool {
    !path.extension().is_some_and(|ext| ext == "md") || path.ends_with("_index.md")
}
