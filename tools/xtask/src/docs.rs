//! Deterministic validation for the Markdown specification set.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

const MOJIBAKE_MARKERS: [&str; 5] = ["Ã", "Â", "â€", "ðŸ", "ï»¿"];

#[derive(Debug)]
struct Document {
    path: PathBuf,
    anchors: HashSet<String>,
    links: Vec<Link>,
}

#[derive(Debug)]
struct Link {
    line: usize,
    target: String,
}

#[derive(Debug)]
struct Fence {
    language: String,
    line: usize,
    has_content: bool,
}

pub(crate) fn check(docs_root: &Path) -> Result<(), String> {
    let root = fs::canonicalize(docs_root).map_err(|error| {
        format!(
            "documentation root `{}` is unavailable: {error}",
            docs_root.display()
        )
    })?;
    let files = markdown_files(&root)?;
    if files.is_empty() {
        return Err(format!(
            "no Markdown files found under `{}`",
            root.display()
        ));
    }

    let mut errors = Vec::new();
    let mut documents = HashMap::new();
    for path in files {
        let (document, mut document_errors) = parse_document(&path);
        errors.append(&mut document_errors);
        documents.insert(path, document);
    }
    validate_links(&root, &documents, &mut errors);

    errors.sort();
    if errors.is_empty() {
        println!("docs-check: validated {} Markdown files", documents.len());
        Ok(())
    } else {
        Err(format!(
            "documentation validation failed:\n{}",
            errors
                .iter()
                .map(|error| format!("- {error}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

fn markdown_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("cannot read an entry in `{}`: {error}", directory.display())
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("cannot inspect `{}`: {error}", path.display()))?;
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() || path.extension().is_none_or(|value| value != "md") {
                continue;
            }
            let resolved = fs::canonicalize(&path)
                .map_err(|error| format!("cannot resolve `{}`: {error}", path.display()))?;
            files.push(resolved);
        }
    }
    files.sort();
    Ok(files)
}

fn parse_document(path: &Path) -> (Document, Vec<String>) {
    let mut errors = Vec::new();
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            errors.push(location(path, 1, &format!("cannot read file: {error}")));
            return (empty_document(path), errors);
        }
    };
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) => {
            errors.push(location(
                path,
                1,
                &format!("file is not valid UTF-8: {error}"),
            ));
            return (empty_document(path), errors);
        }
    };

    validate_text_encoding(path, &content, &mut errors);
    let (anchors, links) = validate_structure(path, &content, &mut errors);
    (
        Document {
            path: path.to_path_buf(),
            anchors,
            links,
        },
        errors,
    )
}

fn empty_document(path: &Path) -> Document {
    Document {
        path: path.to_path_buf(),
        anchors: HashSet::new(),
        links: Vec::new(),
    }
}

fn validate_text_encoding(path: &Path, content: &str, errors: &mut Vec<String>) {
    for (line_index, line) in content.lines().enumerate() {
        if line.contains('\u{fffd}') {
            errors.push(location(
                path,
                line_index + 1,
                "contains Unicode replacement character U+FFFD",
            ));
        }
        for marker in MOJIBAKE_MARKERS {
            if line.contains(marker) {
                errors.push(location(
                    path,
                    line_index + 1,
                    &format!("contains known mojibake marker `{marker}`"),
                ));
            }
        }
    }
}

fn validate_structure(
    path: &Path,
    content: &str,
    errors: &mut Vec<String>,
) -> (HashSet<String>, Vec<Link>) {
    let mut anchors = HashSet::new();
    let mut duplicate_anchors = HashMap::<String, usize>::new();
    let mut links = Vec::new();
    let mut fence: Option<Fence> = None;
    let mut first_heading = None;
    let mut previous_heading = None;
    let mut h1_count = 0;

    for (line_index, line) in content.lines().enumerate() {
        let line_number = line_index + 1;
        if handle_fence(path, line, line_number, &mut fence, errors) {
            continue;
        }
        if fence.is_some() {
            if let Some(open) = fence.as_mut()
                && !line.trim().is_empty()
            {
                open.has_content = true;
            }
            continue;
        }

        if let Some((level, heading)) = heading(line) {
            validate_heading(
                path,
                line_number,
                level,
                heading,
                &mut first_heading,
                &mut previous_heading,
                &mut h1_count,
                errors,
            );
            let base = heading_anchor(heading);
            let occurrence = duplicate_anchors.entry(base.clone()).or_default();
            let anchor = if *occurrence == 0 {
                base
            } else {
                format!("{base}-{occurrence}")
            };
            *occurrence += 1;
            anchors.insert(anchor);
        }
        links.extend(markdown_links(line, line_number));
    }

    if let Some(open) = fence {
        errors.push(location(
            path,
            open.line,
            &format!("unclosed `{}` code fence", open.language),
        ));
    }
    if h1_count != 1 {
        errors.push(location(
            path,
            1,
            &format!("must contain exactly one H1 heading; found {h1_count}"),
        ));
    }
    if first_heading != Some(1) {
        errors.push(location(path, 1, "the first heading must be H1"));
    }

    (anchors, links)
}

fn handle_fence(
    path: &Path,
    line: &str,
    line_number: usize,
    fence: &mut Option<Fence>,
    errors: &mut Vec<String>,
) -> bool {
    let trimmed = line.trim();
    let Some(after_ticks) = trimmed.strip_prefix("```") else {
        return false;
    };
    if let Some(open) = fence.take() {
        if !after_ticks.trim().is_empty() {
            errors.push(location(
                path,
                line_number,
                "a closing code fence must not declare a type",
            ));
            *fence = Some(open);
            return true;
        }
        if open.language == "mermaid" && !open.has_content {
            errors.push(location(
                path,
                open.line,
                "Mermaid fence must contain a diagram",
            ));
        }
    } else {
        let language = after_ticks.trim();
        if language.is_empty() {
            errors.push(location(
                path,
                line_number,
                "opening code fence must declare a type",
            ));
        }
        *fence = Some(Fence {
            language: language.to_owned(),
            line: line_number,
            has_content: false,
        });
    }
    true
}

#[expect(
    clippy::too_many_arguments,
    reason = "heading validation updates the small set of document-structure counters together"
)]
fn validate_heading(
    path: &Path,
    line: usize,
    level: usize,
    heading: &str,
    first_heading: &mut Option<usize>,
    previous_heading: &mut Option<usize>,
    h1_count: &mut usize,
    errors: &mut Vec<String>,
) {
    if first_heading.is_none() {
        *first_heading = Some(level);
    }
    if level == 1 {
        *h1_count += 1;
    }
    if heading.trim().is_empty() {
        errors.push(location(path, line, "heading text must not be empty"));
    }
    if previous_heading.is_some_and(|previous| level > previous + 1) {
        errors.push(location(
            path,
            line,
            &format!(
                "heading level jumps from H{} to H{level}",
                previous_heading.unwrap_or(0)
            ),
        ));
    }
    *previous_heading = Some(level);
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let remainder = &trimmed[level..];
    if remainder.is_empty() {
        Some((level, remainder))
    } else if remainder.starts_with(char::is_whitespace) {
        Some((level, remainder.trim()))
    } else {
        None
    }
}

fn heading_anchor(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|character| {
            if character.is_alphanumeric() || character == '-' || character == '_' {
                Some(character.to_ascii_lowercase())
            } else if character.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

fn markdown_links(line: &str, line_number: usize) -> Vec<Link> {
    let mut links = Vec::new();
    let mut remainder = line;
    while let Some(start) = remainder.find("](") {
        let destination = &remainder[start + 2..];
        let Some(end) = destination.find(')') else {
            break;
        };
        let target = destination[..end].trim();
        if !target.is_empty() {
            links.push(Link {
                line: line_number,
                target: target.to_owned(),
            });
        }
        remainder = &destination[end + 1..];
    }
    links
}

fn validate_links(root: &Path, documents: &HashMap<PathBuf, Document>, errors: &mut Vec<String>) {
    for document in documents.values() {
        for link in &document.links {
            if let Err(error) = validate_link(root, documents, document, &link.target) {
                errors.push(location(&document.path, link.line, &error));
            }
        }
    }
}

fn validate_link(
    root: &Path,
    documents: &HashMap<PathBuf, Document>,
    source: &Document,
    raw_target: &str,
) -> Result<(), String> {
    let target = link_destination(raw_target);
    if is_external(target) {
        return Ok(());
    }
    let (raw_path, raw_fragment) = target.split_once('#').unwrap_or((target, ""));
    let decoded_path = percent_decode(raw_path)?;
    let candidate = if decoded_path.is_empty() {
        source.path.clone()
    } else {
        let parent = source
            .path
            .parent()
            .ok_or_else(|| "source document has no parent directory".to_owned())?;
        normalize_relative(parent, Path::new(&decoded_path))?
    };
    let resolved = fs::canonicalize(&candidate)
        .map_err(|_| format!("local link target `{raw_path}` does not exist"))?;
    if !resolved.starts_with(root) {
        return Err(format!("local link target `{raw_path}` escapes docs/gui"));
    }
    if !resolved.is_file() {
        return Err(format!("local link target `{raw_path}` is not a file"));
    }

    if !raw_fragment.is_empty() {
        let fragment = percent_decode(raw_fragment)?;
        let target_document = documents.get(&resolved).ok_or_else(|| {
            format!("anchor `#{fragment}` targets a non-Markdown file `{raw_path}`")
        })?;
        if !target_document.anchors.contains(&fragment) {
            return Err(format!(
                "anchor `#{fragment}` does not exist in `{raw_path}`"
            ));
        }
    }
    Ok(())
}

fn link_destination(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(without_open) = trimmed.strip_prefix('<') {
        without_open
            .split_once('>')
            .map_or(without_open, |(target, _)| target)
    } else {
        trimmed.split_whitespace().next().unwrap_or_default()
    }
}

fn is_external(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://") || target.starts_with("mailto:")
}

fn normalize_relative(parent: &Path, target: &Path) -> Result<PathBuf, String> {
    if target.is_absolute() {
        return Err(format!(
            "local link target `{}` must be relative",
            target.display()
        ));
    }
    let mut path = parent.to_path_buf();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => path.push(part),
            Component::ParentDir => {
                if !path.pop() {
                    return Err(format!(
                        "local link target `{}` is invalid",
                        target.display()
                    ));
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(format!(
                    "local link target `{}` must be relative",
                    target.display()
                ));
            }
        }
    }
    Ok(path)
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let encoded = bytes
                .get(index + 1..index + 3)
                .ok_or_else(|| format!("invalid percent encoding in `{value}`"))?;
            let text = std::str::from_utf8(encoded)
                .map_err(|_| format!("invalid percent encoding in `{value}`"))?;
            let byte = u8::from_str_radix(text, 16)
                .map_err(|_| format!("invalid percent encoding in `{value}`"))?;
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| format!("decoded link `{value}` is not UTF-8"))
}

fn location(path: &Path, line: usize, message: &str) -> String {
    format!("{}:{line}: {message}", path.display())
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs, io,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::check;

    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> io::Result<Self> {
            Ok(Self {
                root: next_fixture_root()?,
            })
        }

        fn write(&self, relative: &str, content: &str) -> io::Result<()> {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, content)
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.root);
        }
    }

    fn create_fixture_root(root: &Path) -> io::Result<bool> {
        match fs::create_dir(root) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn next_fixture_root() -> io::Result<PathBuf> {
        loop {
            let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("openmanic-xtask-docs-{}-{id}", std::process::id()));
            if create_fixture_root(&root)? {
                return Ok(root);
            }
        }
    }

    #[test]
    fn accepts_valid_structure_links_anchors_and_typed_fences() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        fixture.write(
            "README.md",
            "# Guide\n\n## Diagram\n\n```mermaid\nflowchart LR\n A --> B\n```\n\n[Details](nested/details.md#details)\n",
        )?;
        fixture.write(
            "nested/details.md",
            "# Details\n\nBack to [diagram](../README.md#diagram).\n",
        )?;

        let result = check(fixture.path());
        assert!(result.is_ok(), "{result:?}");
        Ok(())
    }

    #[test]
    fn reports_broken_paths_and_anchors() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        fixture.write(
            "README.md",
            "# Guide\n\n[Missing](missing.md) and [bad anchor](#absent).\n",
        )?;

        let error = check(fixture.path()).expect_err("broken links must fail");
        assert!(error.contains("does not exist"));
        assert!(error.contains("anchor `#absent`"));
        Ok(())
    }

    #[test]
    fn rejects_replacement_characters_and_known_mojibake() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        fixture.write("README.md", "# Guide\n\nBroken � and FranÃ§ais.\n")?;

        let error = check(fixture.path()).expect_err("encoding damage must fail");
        assert!(error.contains("U+FFFD"));
        assert!(error.contains("mojibake marker"));
        Ok(())
    }

    #[test]
    fn rejects_heading_jumps_untyped_fences_and_empty_mermaid() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        fixture.write(
            "README.md",
            "# Guide\n\n### Skipped\n\n```\nplain\n```\n\n```mermaid\n```\n",
        )?;

        let error = check(fixture.path()).expect_err("invalid structure must fail");
        assert!(error.contains("heading level jumps"));
        assert!(error.contains("opening code fence must declare a type"));
        assert!(error.contains("Mermaid fence must contain a diagram"));
        Ok(())
    }
}
