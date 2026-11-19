//! Expanding directives: each `custom` node of a declared directive
//! becomes what it stands for. Nothing here depends on other documents, so
//! the collect and apply passes expand a document the same way.

use std::collections::BTreeMap;
use std::path::Path;

use lsdocs_plugin_sdk::doctree::{
    Admonition, Anchor, Caption, CodeBlock, Custom, DefinitionList, Document, Figure, List, Math,
    Node, Paragraph, Span, Strong, plain_text, text,
};
use lsdocs_plugin_sdk::protocol::{Diagnostic, SiteInfo};
use serde_json::{Value, json};

use crate::{Options, warning};

/// What expanding a document found, for the collect pass.
#[derive(Debug, Default)]
pub(crate) struct Notes {
    /// Glossary terms (lower case) and their anchors.
    pub terms: BTreeMap<String, String>,
    /// Equation labels and their numbers.
    pub equations: BTreeMap<String, u32>,
    /// Todos: their anchor and content.
    pub todos: Vec<Value>,
    /// Documents named by toctrees, hidden or not, in order.
    pub toctree: Vec<String>,
    /// Each toctree: the documents it names and how deep `:numbered:`
    /// numbers them (0 for not at all).
    pub toctrees: Vec<Value>,
    /// Index entries: `type`, `value`, `main` and the `anchor` they point
    /// to.
    pub index: Vec<Value>,
    /// Files read (or looked for), the results' dependencies.
    pub files: Vec<String>,
}

/// A directive's parts, as the parsers make them.
struct Parts {
    kind: String,
    span: Option<Span>,
    argument: String,
    options: serde_json::Map<String, Value>,
    content: Option<String>,
    /// The parsed argument, for directives declared with an inline one.
    argument_nodes: Vec<Node>,
    /// The parsed content.
    children: Vec<Node>,
}

impl Parts {
    fn new(custom: Custom) -> Self {
        let string = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
        let mut argument_nodes = Vec::new();
        let mut children = Vec::new();
        for child in custom.children {
            match child {
                Node::Custom(c) if c.kind == "directive_argument" && argument_nodes.is_empty() => {
                    argument_nodes = c.children;
                }
                other => children.push(other),
            }
        }
        Self {
            argument: string(custom.attrs.get("argument")).unwrap_or_default(),
            options: custom
                .attrs
                .get("options")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            content: string(custom.attrs.get("content")),
            kind: custom.kind,
            span: custom.span,
            argument_nodes,
            children,
        }
    }

    /// The value of option `name`, if given (empty for a flag).
    fn option(&self, name: &str) -> Option<&str> {
        match self.options.get(name)? {
            Value::String(s) => Some(s),
            _ => Some(""),
        }
    }

    fn flag(&self, name: &str) -> bool {
        self.options.contains_key(name)
    }
}

/// Expands the directives of one document.
pub(crate) struct Expand<'a> {
    options: &'a Options,
    /// The document's source path.
    source: String,
    site: &'a SiteInfo,
    /// The language a `highlight` directive set.
    highlight: Option<String>,
    todos: u32,
    equations: u32,
    indexes: u32,
    pub notes: Notes,
    pub diagnostics: Vec<Diagnostic>,
}

/// The directives handled here.
const DIRECTIVES: &[&str] = &[
    "code-block",
    "sourcecode",
    "literalinclude",
    "highlight",
    "versionadded",
    "versionchanged",
    "deprecated",
    "versionremoved",
    "seealso",
    "todo",
    "only",
    "centered",
    "hlist",
    "tab-set",
    "tab-item",
    "glossary",
    "toctree",
    "index",
    "math",
    "sectionauthor",
    "codeauthor",
    "acks",
    "tabularcolumns",
    "rubric",
];

impl<'a> Expand<'a> {
    pub fn new(options: &'a Options, source: &str, site: &'a SiteInfo) -> Self {
        Self {
            options,
            source: source.to_owned(),
            site,
            highlight: options.highlight_language.clone(),
            todos: 0,
            equations: 0,
            indexes: 0,
            notes: Notes::default(),
            diagnostics: Vec::new(),
        }
    }

    pub fn document(&mut self, document: &mut Document) {
        let children = std::mem::take(&mut document.children);
        document.children = self.nodes(children);
    }

    /// Expands `nodes` and everything below them, in document order.
    fn nodes(&mut self, nodes: Vec<Node>) -> Vec<Node> {
        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            match node {
                Node::Custom(c) if DIRECTIVES.contains(&c.kind.as_str()) => {
                    let parts = Parts::new(c);
                    out.extend(self.directive(parts));
                }
                // The `index` role makes entries, like the directive.
                Node::Custom(c)
                    if c.kind == "role"
                        && c.attrs.get("name").and_then(Value::as_str) == Some("index") =>
                {
                    let raw = c
                        .attrs
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let (title, target) = crate::roles::split_title(raw);
                    let id = self.index_anchor(None);
                    let entries = match &title {
                        Some(_) => index_entries(&target, true),
                        None => vec![("single".to_owned(), target.clone(), false)],
                    };
                    self.add_index_entries(entries, &id, c.span);
                    out.push(anchor(&id));
                    out.push(Node::Text(lsdocs_plugin_sdk::doctree::Text {
                        span: c.span,
                        value: title.unwrap_or(target),
                    }));
                }
                Node::CodeBlock(mut code) => {
                    if code.language.is_none() {
                        code.language.clone_from(&self.highlight);
                    }
                    out.push(Node::CodeBlock(code));
                }
                mut node => {
                    if let Some(children) = node.children_mut() {
                        let expanded = self.nodes(std::mem::take(children));
                        *children = expanded;
                    }
                    out.push(node);
                }
            }
        }
        out
    }

    fn warn(&mut self, span: Option<Span>, message: impl Into<String>) {
        self.diagnostics.push(warning(span, message));
    }

    fn directive(&mut self, mut d: Parts) -> Vec<Node> {
        let children = std::mem::take(&mut d.children);
        let mut children = self.nodes(children);
        match d.kind.as_str() {
            "code-block" | "sourcecode" => {
                let language = Some(d.argument.clone())
                    .filter(|l| !l.is_empty())
                    .or_else(|| self.highlight.clone());
                let mut value = d.content.clone().unwrap_or_default();
                if let Some(n) = d.option("dedent") {
                    value = self.dedent(&d, &value, n);
                }
                self.code(&d, language, value, None)
            }
            "literalinclude" => self.literalinclude(&d),
            "highlight" => {
                self.highlight = Some(d.argument.clone());
                Vec::new()
            }
            "versionadded" | "versionchanged" | "deprecated" | "versionremoved" => {
                let label = match d.kind.as_str() {
                    "versionadded" => "Added in version",
                    "versionchanged" => "Changed in version",
                    "deprecated" => "Deprecated since version",
                    _ => "Removed in version",
                };
                let version = d.argument.split_whitespace().next().unwrap_or_default();
                let mut body = Vec::new();
                let explanation = strip_prefix(std::mem::take(&mut d.argument_nodes), version);
                if !explanation.is_empty() {
                    body.push(paragraph(explanation, d.span));
                }
                body.append(&mut children);
                vec![admonition(
                    &d.kind,
                    format!("{label} {version}"),
                    body,
                    d.span,
                )]
            }
            "seealso" => {
                let mut body = Vec::new();
                if !d.argument_nodes.is_empty() {
                    body.push(paragraph(std::mem::take(&mut d.argument_nodes), d.span));
                }
                body.append(&mut children);
                vec![admonition("seealso", "See also".into(), body, d.span)]
            }
            "todo" => {
                if !self.options.todo_include_todos {
                    return Vec::new();
                }
                self.todos += 1;
                let id = match d.option("name") {
                    Some(name) => lsdocs_rst::make_id(name),
                    None => format!("todo-{}", self.todos),
                };
                self.notes.todos.push(json!({
                    "anchor": id,
                    "children": children,
                }));
                vec![
                    anchor(&id),
                    admonition("todo", "Todo".into(), children, d.span),
                ]
            }
            "only" => match tags::evaluate(&d.argument, &self.tags()) {
                Ok(true) => children,
                Ok(false) => Vec::new(),
                Err(e) => {
                    self.warn(
                        d.span,
                        format!("invalid \"only\" expression {:?}: {e}", d.argument),
                    );
                    Vec::new()
                }
            },
            "centered" => vec![custom(
                "centered",
                vec![paragraph(
                    vec![Node::Strong(Strong {
                        span: d.span,
                        children: std::mem::take(&mut d.argument_nodes),
                    })],
                    d.span,
                )],
                d.span,
            )],
            "hlist" => self.hlist(&d, children),
            "tab-set" => vec![custom("tab-set", children, d.span)],
            "tab-item" => {
                let mut node = Custom {
                    span: d.span,
                    kind: "tab-item".into(),
                    attrs: BTreeMap::new(),
                    children,
                };
                node.attrs.insert("label".into(), json!(d.argument.trim()));
                vec![Node::Custom(node)]
            }
            "glossary" => self.glossary(&d, children),
            "toctree" => self.toctree(&d),
            "index" => {
                let id = self.index_anchor(d.option("name"));
                let mut text = d.argument.clone();
                if let Some(content) = &d.content {
                    text.push('\n');
                    text.push_str(content);
                }
                let entries = index_entries(&text, true);
                self.add_index_entries(entries, &id, d.span);
                vec![anchor(&id)]
            }
            "rubric" => {
                let tag = match d.option("heading-level").map(str::trim) {
                    None => "p".to_owned(),
                    Some(level @ ("1" | "2" | "3" | "4" | "5" | "6")) => format!("h{level}"),
                    Some(other) => {
                        self.warn(
                            d.span,
                            format!("rubric: :heading-level: must be 1 to 6, not {other:?}"),
                        );
                        "p".to_owned()
                    }
                };
                let mut node = Custom {
                    span: d.span,
                    kind: "rubric".into(),
                    attrs: BTreeMap::new(),
                    children: std::mem::take(&mut d.argument_nodes),
                };
                node.attrs.insert("html_tag".into(), json!(tag));
                if let Some(name) = d.option("name") {
                    node.attrs
                        .insert("id".into(), json!(lsdocs_rst::make_id(name)));
                }
                vec![Node::Custom(node)]
            }
            "math" => self.math(&d),
            "sectionauthor" | "codeauthor" => {
                if !self.options.show_authors {
                    return Vec::new();
                }
                let label = if d.kind == "sectionauthor" {
                    "Section author: "
                } else {
                    "Code author: "
                };
                let mut inlines = vec![text(label)];
                inlines.append(&mut d.argument_nodes);
                vec![paragraph(inlines, d.span)]
            }
            "acks" => vec![custom("acks", children, d.span)],
            // "tabularcolumns": for LaTeX only.
            _ => Vec::new(),
        }
    }

    /// Tags `only` tests.
    fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = ["html", "format_html", "builder_html"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        tags.extend(self.options.tags.iter().cloned());
        tags
    }

    /// `text` dedented: by `n` columns, or (for an empty `n`) by the
    /// indentation all its lines share.
    fn dedent(&mut self, d: &Parts, text: &str, n: &str) -> String {
        let lines: Vec<&str> = text.split('\n').collect();
        let indent = |l: &str| l.len() - l.trim_start().len();
        let by = if n.trim().is_empty() {
            lines
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| indent(l))
                .min()
                .unwrap_or(0)
        } else {
            match n.trim().parse::<usize>() {
                Ok(by) => by,
                Err(_) => {
                    self.warn(d.span, format!("invalid dedent value: {n:?}"));
                    return text.to_owned();
                }
            }
        };
        // As Sphinx: `by` characters go, whitespace or not.
        let mut stripped = false;
        let out: Vec<String> = lines
            .iter()
            .map(|l| {
                if l.chars().take(by).any(|c| !c.is_whitespace()) {
                    stripped = true;
                }
                l.chars().skip(by).collect()
            })
            .collect();
        if stripped {
            self.warn(d.span, "non-whitespace stripped by dedent");
        }
        out.join("\n")
    }

    /// A code block with the options `code-block` and `literalinclude`
    /// share: an id, a caption, line numbers.
    fn code(
        &mut self,
        d: &Parts,
        language: Option<String>,
        value: String,
        lineno_start: Option<usize>,
    ) -> Vec<Node> {
        let mut info = Vec::new();
        if d.flag("linenos") || d.flag("lineno-start") || lineno_start.is_some() {
            info.push("linenos".to_owned());
        }
        if let Some(start) = lineno_start {
            info.push(format!("lineno-start={start}"));
        } else if let Some(start) = d.option("lineno-start") {
            info.push(format!("lineno-start={}", start.trim()));
        }
        if let Some(lines) = d.option("emphasize-lines") {
            let count = value.lines().count();
            match line_spec(lines, count) {
                Ok(_) => info.push(format!("emphasize-lines={}", lines.replace(' ', ""))),
                Err(e) => self.warn(d.span, format!("invalid emphasize-lines {lines:?}: {e}")),
            }
        }
        let block = Node::CodeBlock(CodeBlock {
            span: d.span,
            language,
            info: (!info.is_empty()).then(|| info.join(" ")),
            value: value.trim_end_matches('\n').to_owned(),
        });
        let id = d.option("name").map(lsdocs_rst::make_id);
        match d.option("caption").filter(|c| !c.is_empty()) {
            Some(caption) => vec![Node::Figure(Figure {
                span: d.span,
                id,
                children: vec![
                    block,
                    Node::Caption(Caption {
                        span: d.span,
                        children: vec![text(caption)],
                    }),
                ],
            })],
            None => match id {
                Some(id) => vec![anchor(&id), block],
                None => vec![block],
            },
        }
    }

    fn literalinclude(&mut self, d: &Parts) -> Vec<Node> {
        for unsupported in ["pyobject", "diff"] {
            if d.flag(unsupported) {
                self.warn(
                    d.span,
                    format!("literalinclude: the :{unsupported}: option is not supported"),
                );
                return Vec::new();
            }
        }
        if let Some(encoding) = d.option("encoding")
            && !matches!(
                encoding.trim().to_lowercase().replace('_', "-").as_str(),
                "utf-8" | "utf8"
            )
        {
            self.warn(
                d.span,
                format!("literalinclude: only UTF-8 files can be read, not {encoding:?}"),
            );
            return Vec::new();
        }
        if !self.options.file_access {
            self.warn(d.span, "\"literalinclude\" directive disabled.");
            return Vec::new();
        }
        let path = d.argument.trim();
        let Some(file) = self.resolve_file(path) else {
            self.warn(d.span, format!("literalinclude: cannot find {path:?}"));
            return Vec::new();
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text.replace("\r\n", "\n"),
            Err(e) => {
                self.warn(d.span, format!("literalinclude: cannot read {path:?}: {e}"));
                return Vec::new();
            }
        };
        let mut lines: Vec<(usize, &str)> =
            text.lines().enumerate().map(|(i, l)| (i + 1, l)).collect();
        // As Sphinx: start, end, lines, then dedent, prepend, append.
        for (option, after) in [("start-at", false), ("start-after", true)] {
            if let Some(pattern) = d.option(option) {
                match lines.iter().position(|(_, l)| l.contains(pattern)) {
                    Some(i) => {
                        lines.drain(..i + usize::from(after));
                    }
                    None => {
                        self.warn(
                            d.span,
                            format!("literalinclude: {option} pattern not found: {pattern:?}"),
                        );
                        return Vec::new();
                    }
                }
            }
        }
        for (option, inclusive) in [("end-at", true), ("end-before", false)] {
            if let Some(pattern) = d.option(option) {
                match lines.iter().position(|(_, l)| l.contains(pattern)) {
                    Some(i) => lines.truncate(i + usize::from(inclusive)),
                    None => {
                        self.warn(
                            d.span,
                            format!("literalinclude: {option} pattern not found: {pattern:?}"),
                        );
                        return Vec::new();
                    }
                }
            }
        }
        if let Some(spec) = d.option("lines") {
            match line_spec(spec, lines.len()) {
                Ok(picked) if !picked.is_empty() => {
                    lines = picked
                        .iter()
                        .filter_map(|&i| lines.get(i - 1).copied())
                        .collect();
                }
                Ok(_) => {
                    self.warn(
                        d.span,
                        format!(
                            "literalinclude: line spec {spec:?}: no lines pulled from {path:?}"
                        ),
                    );
                    return Vec::new();
                }
                Err(e) => {
                    self.warn(
                        d.span,
                        format!("literalinclude: invalid line spec {spec:?}: {e}"),
                    );
                    return Vec::new();
                }
            }
        }
        let lineno_start = if d.flag("lineno-match") {
            let contiguous = lines.windows(2).all(|w| w[1].0 == w[0].0 + 1);
            if !contiguous {
                self.warn(
                    d.span,
                    "literalinclude: cannot use \"lineno-match\" with a disjoint set of \"lines\"",
                );
            }
            lines.first().map(|(n, _)| *n).filter(|_| contiguous)
        } else {
            None
        };
        let mut value: String = lines.iter().map(|(_, l)| *l).collect::<Vec<_>>().join("\n");
        if let Some(n) = d.option("dedent") {
            value = self.dedent(d, &value, n);
        }
        if let Some(prepend) = d.option("prepend") {
            value = format!("{prepend}\n{value}");
        }
        if let Some(append) = d.option("append") {
            value = format!("{value}\n{append}");
        }
        let language = d
            .option("language")
            .map(str::to_owned)
            .or_else(|| {
                Path::new(path)
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
            })
            .or_else(|| self.highlight.clone());
        self.code(d, language, value, lineno_start)
    }

    /// The file `path` names: relative to this document's directory, or
    /// to the source directory with a leading `/`.
    fn resolve_file(&mut self, path: &str) -> Option<std::path::PathBuf> {
        let dir = self.site.source_dir.as_deref()?;
        let relative = match path.strip_prefix('/') {
            Some(rest) => rest.to_owned(),
            None => match self.source.rsplit_once('/') {
                Some((parent, _)) => format!("{parent}/{path}"),
                None => path.to_owned(),
            },
        };
        let file = Path::new(dir).join(relative);
        // A dependency of the result, found or not.
        self.notes
            .files
            .push(file.to_string_lossy().replace('\\', "/"));
        file.is_file().then_some(file)
    }

    fn hlist(&mut self, d: &Parts, children: Vec<Node>) -> Vec<Node> {
        let columns = match d.option("columns").map(|c| c.trim().parse::<usize>()) {
            None => 2,
            Some(Ok(n)) if n > 0 => n,
            Some(_) => {
                self.warn(d.span, "hlist: :columns: must be a positive number");
                2
            }
        };
        let Some(Node::List(list)) = children.into_iter().find(|n| matches!(n, Node::List(_)))
        else {
            self.warn(d.span, ".. hlist content is not a list");
            return Vec::new();
        };
        // As Sphinx: the first columns take one more item each.
        let n = list.children.len();
        let (full, extra) = (n / columns, n % columns);
        let mut items = list.children.into_iter();
        let mut out = Vec::new();
        for column in 0..columns {
            let take = full + usize::from(column < extra);
            out.push(Node::List(List {
                span: list.span,
                ordered: list.ordered,
                start: None,
                children: items.by_ref().take(take).collect(),
            }));
        }
        let mut node = Custom {
            span: d.span,
            kind: "hlist".into(),
            attrs: BTreeMap::new(),
            children: out,
        };
        node.attrs.insert("columns".into(), json!(columns));
        vec![Node::Custom(node)]
    }

    fn glossary(&mut self, d: &Parts, children: Vec<Node>) -> Vec<Node> {
        let mut found = false;
        let mut out = Vec::new();
        for child in children {
            let Node::DefinitionList(list) = child else {
                out.push(child);
                continue;
            };
            found = true;
            // Entries: terms, then their descriptions.
            let mut entries: Vec<(String, Vec<Node>)> = Vec::new();
            for item in list.children {
                match item {
                    Node::DefinitionTerm(mut term) => {
                        let name = plain_text(&term.children);
                        let id = format!("term-{}", lsdocs_rst::make_id(&name));
                        self.notes
                            .terms
                            .entry(name.to_lowercase())
                            .or_insert_with(|| id.clone());
                        term.children.insert(0, anchor(&id));
                        let starts_entry = entries.last().is_none_or(|(_, nodes)| {
                            nodes
                                .last()
                                .is_some_and(|n| matches!(n, Node::DefinitionDescription(_)))
                        });
                        if starts_entry {
                            entries.push((name.to_lowercase(), Vec::new()));
                        }
                        if let Some((_, nodes)) = entries.last_mut() {
                            nodes.push(Node::DefinitionTerm(term));
                        }
                    }
                    other => match entries.last_mut() {
                        Some((_, nodes)) => nodes.push(other),
                        None => entries.push((String::new(), vec![other])),
                    },
                }
            }
            if d.flag("sorted") {
                entries.sort_by(|a, b| a.0.cmp(&b.0));
            }
            out.push(Node::DefinitionList(DefinitionList {
                span: list.span,
                children: entries.into_iter().flat_map(|(_, nodes)| nodes).collect(),
            }));
        }
        if !found {
            self.warn(
                d.span,
                "glossary: its content must be a definition list of terms",
            );
        }
        vec![custom("glossary", out, d.span)]
    }

    /// A toctree: a placeholder holding its entries, made into links once
    /// every document is known (`Site::resolve`).
    fn toctree(&mut self, d: &Parts) -> Vec<Node> {
        let docs = crate::roles::documents(&self.site.files);
        let mut entries = Vec::new();
        for line in d.content.as_deref().unwrap_or_default().lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (title, target) = crate::roles::split_title(line);
            if target == "self" || target.contains("://") || target.starts_with("mailto:") {
                entries.push(json!({"title": title, "url": target}));
                continue;
            }
            if d.flag("glob") && title.is_none() && target.contains(['*', '?', '[']) {
                let pattern = crate::roles::absolute(&self.source, &target);
                let mut matched: Vec<&String> = docs
                    .iter()
                    .filter(|(name, source)| **source != self.source && glob(&pattern, name))
                    .map(|(_, source)| source)
                    .collect();
                matched.sort();
                if matched.is_empty() {
                    self.warn(
                        d.span,
                        format!("toctree glob pattern {target:?} didn't match any documents"),
                    );
                }
                for source in matched {
                    entries.push(json!({"document": source}));
                }
                continue;
            }
            match crate::roles::find_document(&docs, &self.source, &target) {
                Some(source) => {
                    entries.push(json!({"title": title, "document": source}));
                }
                None if target == "genindex" => {
                    entries.push(json!({"title": title, "document": crate::roles::GENINDEX}));
                }
                None => self.warn(
                    d.span,
                    format!("toctree contains reference to nonexisting document {target:?}"),
                ),
            }
        }
        if d.flag("reversed") {
            entries.reverse();
        }
        let named: Vec<String> = entries
            .iter()
            .filter_map(|e| e["document"].as_str())
            .filter(|s| *s != crate::roles::GENINDEX)
            .map(str::to_owned)
            .collect();
        let numbered = match d.option("numbered").map(str::trim) {
            None => 0,
            Some("") => 999,
            Some(n) => n.parse::<u32>().unwrap_or(999),
        };
        self.notes.toctree.extend(named.iter().cloned());
        self.notes
            .toctrees
            .push(json!({"entries": named, "numbered": numbered}));
        if d.flag("hidden") {
            return Vec::new();
        }
        let mut attrs = BTreeMap::new();
        attrs.insert("entries".to_owned(), Value::Array(entries));
        for name in ["caption", "maxdepth", "name"] {
            if let Some(value) = d.option(name) {
                attrs.insert(name.to_owned(), json!(value.trim()));
            }
        }
        if d.flag("titlesonly") {
            attrs.insert("titlesonly".to_owned(), json!(true));
        }
        vec![Node::Custom(Custom {
            span: d.span,
            kind: "toctree".into(),
            attrs,
            children: Vec::new(),
        })]
    }

    /// The id of the next index anchor: `name` made an id, or `index-<n>`.
    fn index_anchor(&mut self, name: Option<&str>) -> String {
        self.indexes += 1;
        match name {
            Some(name) => lsdocs_rst::make_id(name),
            None => format!("index-{}", self.indexes - 1),
        }
    }

    fn add_index_entries(
        &mut self,
        entries: Vec<(String, String, bool)>,
        anchor: &str,
        span: Option<Span>,
    ) {
        for (kind, value, main) in entries {
            let parts = value.split(';').filter(|p| !p.trim().is_empty()).count();
            let needed = match kind.as_str() {
                "pair" | "see" | "seealso" => 2,
                "triple" => 3,
                _ => 1,
            };
            if parts < needed || (kind != "single" && parts > needed) {
                self.warn(span, format!("invalid {kind} index entry {value:?}"));
                continue;
            }
            self.notes.index.push(json!({
                "type": kind,
                "value": value,
                "main": main,
                "anchor": anchor,
            }));
        }
    }

    fn math(&mut self, d: &Parts) -> Vec<Node> {
        let mut value = d.argument.clone();
        if let Some(content) = &d.content {
            if !value.is_empty() {
                value.push_str("\n\n");
            }
            value.push_str(content);
        }
        let label = d.option("label").or(d.option("name")).map(str::to_owned);
        let mut out = Vec::new();
        if label.is_some() || self.options.math_number_all {
            self.equations += 1;
            let number = self.equations;
            if !d.flag("nowrap") {
                value = format!("{value} \\tag{{{number}}}");
            }
            if let Some(label) = label {
                if self.notes.equations.contains_key(&label) {
                    self.warn(d.span, format!("duplicate label of equation {label}"));
                }
                self.notes.equations.insert(label.clone(), number);
                out.push(anchor(&format!("equation-{label}")));
            }
        }
        out.push(Node::Math(Math {
            span: d.span,
            value,
            display: true,
        }));
        out
    }
}

fn anchor(id: &str) -> Node {
    Node::Anchor(Anchor {
        span: None,
        id: id.to_owned(),
    })
}

fn paragraph(children: Vec<Node>, span: Option<Span>) -> Node {
    Node::Paragraph(Paragraph { span, children })
}

fn custom(kind: &str, children: Vec<Node>, span: Option<Span>) -> Node {
    Node::Custom(Custom {
        span,
        kind: kind.into(),
        attrs: BTreeMap::new(),
        children,
    })
}

fn admonition(kind: &str, title: String, children: Vec<Node>, span: Option<Span>) -> Node {
    Node::Admonition(Admonition {
        span,
        kind: kind.into(),
        title: Some(title),
        children,
    })
}

/// `nodes` without the text `prefix` at their start, and the whitespace
/// after it.
fn strip_prefix(mut nodes: Vec<Node>, prefix: &str) -> Vec<Node> {
    if let Some(Node::Text(first)) = nodes.first_mut()
        && let Some(rest) = first.value.strip_prefix(prefix)
    {
        first.value = rest.trim_start().to_owned();
        if first.value.is_empty() {
            nodes.remove(0);
        }
    }
    nodes
}

/// Index entries in Sphinx's syntax, one per line: `type: value` (types
/// `single`, `pair`, `triple`, `see`, `seealso`; `!` before marks a main
/// entry), or values without a type, separated by commas (`single`
/// entries) if `split` is set.
pub(crate) fn index_entries(text: &str, split: bool) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let (main, line) = match line.strip_prefix('!') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, line),
        };
        if line.is_empty() {
            continue;
        }
        if let Some((kind, value)) = line.split_once(':')
            && matches!(
                kind.trim(),
                "single" | "pair" | "triple" | "see" | "seealso"
            )
        {
            out.push((kind.trim().to_owned(), value.trim().to_owned(), main));
            continue;
        }
        if split {
            for value in line.split(',').map(str::trim).filter(|v| !v.is_empty()) {
                out.push(("single".to_owned(), value.to_owned(), main));
            }
        } else {
            out.push(("single".to_owned(), line.to_owned(), main));
        }
    }
    out
}

/// The line numbers (from 1) a Sphinx line spec such as `1,3-5,8-` picks
/// out of `total` lines, in order.
pub(crate) fn line_spec(spec: &str, total: usize) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let number = |s: &str| {
            s.trim()
                .parse::<usize>()
                .ok()
                .filter(|&n| n > 0)
                .ok_or_else(|| format!("invalid line number {s:?}"))
        };
        match part.split_once('-') {
            None => out.push(number(part)?),
            Some((start, end)) => {
                let start = if start.trim().is_empty() {
                    1
                } else {
                    number(start)?
                };
                let end = if end.trim().is_empty() {
                    total
                } else {
                    number(end)?
                };
                if start > end && !(end == total && start > total) {
                    return Err(format!("invalid line range {part:?}"));
                }
                out.extend(start..=end);
            }
        }
    }
    Ok(out.into_iter().filter(|&n| n <= total).collect())
}

/// Whether the document name `name` matches the glob `pattern`: `*` and
/// `?` stay within one path component, `**` crosses them.
pub(crate) fn glob(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p {
            [] => n.is_empty(),
            ['*', '*', rest @ ..] => (0..=n.len()).any(|i| go(rest, &n[i..])),
            ['*', rest @ ..] => {
                for i in 0..=n.len() {
                    if go(rest, &n[i..]) {
                        return true;
                    }
                    if n.get(i) == Some(&'/') {
                        break;
                    }
                }
                false
            }
            ['?', rest @ ..] => n.first().is_some_and(|c| *c != '/') && go(rest, &n[1..]),
            [c, rest @ ..] => n.first() == Some(c) && go(rest, &n[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// Tag expressions for `only`.
mod tags {
    /// Evaluates `expr` (tags joined by `and`, `or` and `not`, with
    /// parentheses) with `tags` true and every other tag false.
    pub fn evaluate(expr: &str, tags: &[String]) -> Result<bool, String> {
        let spaced = expr.replace('(', " ( ").replace(')', " ) ");
        let tokens: Vec<&str> = spaced.split_whitespace().collect();
        let mut pos = 0;
        let value = or(&tokens, &mut pos, tags)?;
        match tokens.get(pos) {
            None => Ok(value),
            Some(t) => Err(format!("unexpected {t:?}")),
        }
    }

    fn or(t: &[&str], pos: &mut usize, tags: &[String]) -> Result<bool, String> {
        let mut value = and(t, pos, tags)?;
        while t.get(*pos) == Some(&"or") {
            *pos += 1;
            value |= and(t, pos, tags)?;
        }
        Ok(value)
    }

    fn and(t: &[&str], pos: &mut usize, tags: &[String]) -> Result<bool, String> {
        let mut value = not(t, pos, tags)?;
        while t.get(*pos) == Some(&"and") {
            *pos += 1;
            value &= not(t, pos, tags)?;
        }
        Ok(value)
    }

    fn not(t: &[&str], pos: &mut usize, tags: &[String]) -> Result<bool, String> {
        if t.get(*pos) == Some(&"not") {
            *pos += 1;
            return Ok(!not(t, pos, tags)?);
        }
        match t.get(*pos) {
            Some(&"(") => {
                *pos += 1;
                let value = or(t, pos, tags)?;
                if t.get(*pos) != Some(&")") {
                    return Err("missing \")\"".into());
                }
                *pos += 1;
                Ok(value)
            }
            Some(tag)
                if tag
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')) =>
            {
                *pos += 1;
                Ok(tags.iter().any(|t| t == tag))
            }
            Some(t) => Err(format!("unexpected {t:?}")),
            None => Err("unexpected end".into()),
        }
    }
}
