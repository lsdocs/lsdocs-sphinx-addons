//! Roles, and what needs the whole site: the data the collect pass
//! gathers from each document, and the apply pass that resolves roles,
//! toctrees and todo lists with it.

use std::collections::{BTreeMap, BTreeSet};

use lsdocs_plugin_sdk::doctree::{
    Anchor, Custom, Document, Emphasis, Figure, InlineCode, Link, LinkTarget, List, ListItem, Node,
    Paragraph, Span, Strong, Text, plain_text, text,
};
use lsdocs_plugin_sdk::protocol::Diagnostic;
use lsdocs_plugin_sdk::relative_path;
use serde_json::{Value, json};

use crate::{Options, directives::Notes, warning};

/// The roles handled here (besides the `extlinks` ones).
pub(crate) const NAMES: &[&str] = &[
    "ref",
    "doc",
    "numref",
    "term",
    "eq",
    "any",
    "download",
    "abbr",
    "command",
    "dfn",
    "file",
    "guilabel",
    "kbd",
    "mailheader",
    "makevar",
    "manpage",
    "menuselection",
    "mimetype",
    "newsgroup",
    "program",
    "regexp",
    "samp",
    "index",
];

/// Extensions of the files `doc` and `toctree` take as documents.
const DOCUMENT_EXTENSIONS: &[&str] = &["rst", "rest", "md", "markdown"];

/// `text <target>`: the text (if any) and the target.
pub(crate) fn split_title(text: &str) -> (Option<String>, String) {
    let text = text.trim();
    if let Some(rest) = text.strip_suffix('>')
        && let Some(i) = rest.rfind('<')
        && i > 0
        && rest[..i].ends_with(char::is_whitespace)
    {
        let title = rest[..i].trim();
        return (Some(title.to_owned()), rest[i + 1..].trim().to_owned());
    }
    (None, text.to_owned())
}

/// The site's documents: their names (paths without the extension) and
/// source paths.
pub(crate) fn documents(files: &[String]) -> Vec<(String, String)> {
    files
        .iter()
        .filter_map(|f| {
            let (name, ext) = f.rsplit_once('.')?;
            DOCUMENT_EXTENSIONS
                .contains(&ext.to_lowercase().as_str())
                .then(|| (name.to_owned(), f.clone()))
        })
        .collect()
}

/// `target` as a path from the source directory: relative to the
/// document `from`, or to the source directory with a leading `/`.
pub(crate) fn absolute(from: &str, target: &str) -> String {
    let joined = match target.strip_prefix('/') {
        Some(rest) => rest.to_owned(),
        None => match from.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{target}"),
            None => target.to_owned(),
        },
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// The source of the document `target` names, from the document `from`.
pub(crate) fn find_document(docs: &[(String, String)], from: &str, target: &str) -> Option<String> {
    let path = absolute(from, target);
    docs.iter()
        .find(|(name, source)| *name == path || *source == path)
        .map(|(_, source)| source.clone())
}

/// Ids a reference can point to.
fn anchors(document: &Document) -> Vec<String> {
    document
        .descendants()
        .filter_map(|n| match n {
            Node::Section(s) => s.id.clone(),
            Node::Heading(h) => h.id.clone(),
            Node::Figure(f) => f.id.clone(),
            Node::Anchor(a) => Some(a.id.clone()),
            _ => None,
        })
        .collect()
}

/// What a figure numbers as: `table`, `code-block` or `figure`.
fn figure_kind(figure: &Figure) -> &'static str {
    let has = |kind: &str| figure.children.iter().any(|c| c.kind() == kind);
    if has("table") {
        "table"
    } else if has("code_block") {
        "code-block"
    } else {
        "figure"
    }
}

/// Calls `f` with every figure that has a caption, its kind and number,
/// in document order. Numbers count per kind, from 1, in each document.
fn number_figures(
    nodes: &mut [Node],
    counts: &mut BTreeMap<&'static str, u32>,
    f: &mut impl FnMut(&mut Figure, &'static str, u32),
) {
    for node in nodes {
        if let Node::Figure(figure) = node
            && figure
                .children
                .iter()
                .any(|c| matches!(c, Node::Caption(_)))
        {
            let kind = figure_kind(figure);
            let n = counts.entry(kind).or_insert(0);
            *n += 1;
            let number = *n;
            f(figure, kind, number);
        }
        if let Some(children) = node.children_mut() {
            number_figures(children, counts, f);
        }
    }
}

/// The text of a figure's caption.
fn caption(figure: &Figure) -> String {
    figure
        .children
        .iter()
        .find_map(|c| match c {
            Node::Caption(c) => Some(plain_text(&c.children)),
            _ => None,
        })
        .unwrap_or_default()
}

/// The section tree: ids, titles and subsections.
fn sections(nodes: &[Node]) -> Vec<Value> {
    nodes
        .iter()
        .filter_map(|n| match n {
            Node::Section(s) => {
                let title = match s.children.first() {
                    Some(Node::Heading(h)) => plain_text(&h.children),
                    _ => String::new(),
                };
                Some(json!({
                    "id": s.id,
                    "title": title,
                    "children": sections(&s.children),
                }))
            }
            _ => None,
        })
        .collect()
}

/// The collect pass's data for one (expanded) document.
pub(crate) fn collect(document: &Document, notes: &Notes, _options: &Options) -> Value {
    let mut numbers = serde_json::Map::new();
    let mut children = document.children.clone();
    number_figures(
        &mut children,
        &mut BTreeMap::new(),
        &mut |figure, kind, number| {
            if let Some(id) = &figure.id {
                numbers.insert(
                    id.clone(),
                    json!({"kind": kind, "number": number, "caption": caption(figure)}),
                );
            }
        },
    );
    json!({
        "title": document.title().unwrap_or_else(|| document.source.clone()),
        "anchors": anchors(document),
        "terms": notes.terms,
        "equations": notes.equations,
        "numbers": numbers,
        "todos": notes.todos,
        "sections": sections(&document.children),
        "toctree": notes.toctree,
    })
}

/// Everything the apply pass knows about the site.
pub(crate) struct Site<'a> {
    /// The collected data of every document, by source.
    pages: BTreeMap<&'a str, &'a Value>,
    docs: Vec<(String, String)>,
    files: BTreeSet<&'a str>,
    options: &'a Options,
    /// Anchors made for `index` roles so far.
    indexes: std::cell::Cell<u32>,
}

impl<'a> Site<'a> {
    pub fn new(collected: &'a Value, files: &'a [String], options: &'a Options) -> Self {
        let pages = collected
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.as_str(), v)).collect())
            .unwrap_or_default();
        Self {
            pages,
            docs: documents(files),
            files: files.iter().map(String::as_str).collect(),
            options,
            indexes: std::cell::Cell::new(0),
        }
    }

    /// The document defining the anchor `id`, if any (the first, in
    /// source order).
    fn anchor_document(&self, id: &str) -> Option<&'a str> {
        self.pages.iter().find_map(|(source, page)| {
            page["anchors"]
                .as_array()?
                .iter()
                .any(|a| a == id)
                .then_some(*source)
        })
    }

    /// The anchor a label names: as written if some document has it, else
    /// as docutils makes ids of names (`My Label` is `my-label`).
    fn label(&self, label: &str) -> String {
        if self.anchor_document(label).is_some() {
            label.to_owned()
        } else {
            lsdocs_rst::make_id(label)
        }
    }

    /// Looks `key` up in every document's collected `field`.
    fn find(&self, field: &str, key: &str) -> Option<(&'a str, &'a Value)> {
        self.pages
            .iter()
            .find_map(|(source, page)| page[field].get(key).map(|v| (*source, v)))
    }

    /// Resolves the roles, toctrees and todo lists of an expanded
    /// document, and numbers its figures.
    pub fn resolve(&self, document: &mut Document, diagnostics: &mut Vec<Diagnostic>) {
        let source = document.source.clone();
        if self.options.numfig {
            number_figures(
                &mut document.children,
                &mut BTreeMap::new(),
                &mut |figure, kind, number| {
                    let label = self
                        .options
                        .numfig_format(kind)
                        .replace("%s", &number.to_string());
                    for child in &mut figure.children {
                        if let Node::Caption(c) = child {
                            c.children.insert(0, text(format!("{label} ")));
                        }
                    }
                },
            );
        }
        let children = std::mem::take(&mut document.children);
        document.children = self.nodes(children, &source, diagnostics);
    }

    fn nodes(&self, nodes: Vec<Node>, source: &str, out: &mut Vec<Diagnostic>) -> Vec<Node> {
        let mut result = Vec::with_capacity(nodes.len());
        for node in nodes {
            match node {
                Node::Custom(c) if c.kind == "role" => result.extend(self.role(&c, source, out)),
                Node::Custom(c) if c.kind == "toctree" && c.children.is_empty() => {
                    result.extend(self.toctree(&c, source));
                }
                Node::Custom(c) if c.kind == "todolist" && c.children.is_empty() => {
                    result.extend(self.todolist(source, c.span));
                }
                mut node => {
                    if let Some(children) = node.children_mut() {
                        let resolved = self.nodes(std::mem::take(children), source, out);
                        *children = resolved;
                    }
                    result.push(node);
                }
            }
        }
        result
    }

    /// A link to `anchor` in the document `to`, from `from`.
    fn link_to(
        &self,
        from: &str,
        to: &str,
        anchor: Option<&str>,
        children: Vec<Node>,
        span: Option<Span>,
    ) -> Node {
        let mut url = if to == from {
            String::new()
        } else {
            relative_path(from, to)
        };
        if let Some(anchor) = anchor {
            url.push('#');
            url.push_str(anchor);
        }
        Node::Link(Link {
            span,
            url,
            title: None,
            target: Some(LinkTarget {
                document: Some(to.to_owned()),
                anchor: anchor.map(str::to_owned),
                label: None,
            }),
            children,
        })
    }

    fn role(&self, c: &Custom, source: &str, out: &mut Vec<Diagnostic>) -> Vec<Node> {
        let attr = |name: &str| {
            c.attrs
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
        };
        let name = attr("name");
        let raw = attr("text");
        let span = c.span;
        let plain = |value: &str| {
            vec![Node::Text(Text {
                span,
                value: value.to_owned(),
            })]
        };
        let (title, target) = split_title(raw);
        let shown = title.clone().unwrap_or_else(|| target.clone());
        if let Some(ext) = self.options.extlinks.get(name) {
            let url = ext.url.replace("%s", &target);
            let label = match (&title, &ext.caption) {
                (Some(title), _) => title.clone(),
                (None, Some(caption)) => caption.replace("%s", &target),
                (None, None) => url.clone(),
            };
            return vec![Node::Link(Link {
                span,
                url,
                title: None,
                target: None,
                children: plain(&label),
            })];
        }
        match name {
            "ref" => vec![self.reference(&target, title.as_deref(), span)],
            "doc" => match find_document(&self.docs, source, &target) {
                Some(doc) => {
                    let children = title.map(|t| plain(&t)).unwrap_or_default();
                    vec![self.link_to(source, &doc, None, children, span)]
                }
                None => {
                    out.push(warning(span, format!("unknown document: {target:?}")));
                    plain(&shown)
                }
            },
            "numref" => self.numref(&target, title.as_deref(), source, span, out),
            "term" => {
                let key = target.to_lowercase();
                match self.find("terms", &key) {
                    Some((doc, anchor)) => {
                        vec![self.link_to(source, doc, anchor.as_str(), plain(&shown), span)]
                    }
                    None => {
                        out.push(warning(span, format!("term not in glossary: {target:?}")));
                        plain(&shown)
                    }
                }
            }
            "eq" => match self.find("equations", &target) {
                Some((doc, number)) => {
                    let anchor = format!("equation-{target}");
                    let label = format!("({number})");
                    vec![self.link_to(source, doc, Some(&anchor), plain(&label), span)]
                }
                None => {
                    out.push(warning(span, format!("equation not found: {target:?}")));
                    plain(&shown)
                }
            },
            "any" => {
                let label = self.label(&target);
                if self.anchor_document(&label).is_some() {
                    vec![self.reference(&target, title.as_deref(), span)]
                } else if let Some((doc, anchor)) = self.find("terms", &target.to_lowercase()) {
                    vec![self.link_to(source, doc, anchor.as_str(), plain(&shown), span)]
                } else if let Some(doc) = find_document(&self.docs, source, &target) {
                    let children = title.map(|t| plain(&t)).unwrap_or_default();
                    vec![self.link_to(source, &doc, None, children, span)]
                } else {
                    out.push(warning(
                        span,
                        format!("'any' reference target not found: {target}"),
                    ));
                    plain(&shown)
                }
            }
            "download" => {
                let path = absolute(source, &target);
                if !self.files.contains(path.as_str()) {
                    out.push(warning(
                        span,
                        format!("download file not readable: {target:?}"),
                    ));
                }
                let name = title
                    .unwrap_or_else(|| target.rsplit('/').next().unwrap_or_default().to_owned());
                vec![Node::Link(Link {
                    span,
                    url: relative_path(source, &path),
                    title: None,
                    target: None,
                    children: vec![Node::InlineCode(InlineCode { span, value: name })],
                })]
            }
            "abbr" => {
                let (abbr, explanation) = match raw
                    .trim()
                    .strip_suffix(')')
                    .and_then(|r| r.split_once(" ("))
                {
                    Some((abbr, explanation)) => (abbr.trim(), Some(explanation.trim())),
                    None => (raw.trim(), None),
                };
                let mut node = inline("abbr", Some("abbr"), plain(abbr), span);
                if let (Node::Custom(c), Some(explanation)) = (&mut node, explanation) {
                    c.attrs.insert("title".into(), json!(explanation));
                }
                vec![node]
            }
            "command" | "program" => vec![Node::Strong(Strong {
                span,
                children: plain(raw),
            })],
            "dfn" | "mailheader" | "mimetype" | "newsgroup" => vec![Node::Emphasis(Emphasis {
                span,
                children: plain(raw),
            })],
            "manpage" => {
                let emphasis = Node::Emphasis(Emphasis {
                    span,
                    children: plain(raw),
                });
                match &self.options.manpages_url {
                    Some(pattern) => {
                        let (page, section) =
                            match raw.trim().strip_suffix(')').and_then(|r| r.split_once('(')) {
                                Some((page, section)) => (page, section),
                                None => (raw.trim(), ""),
                            };
                        let url = pattern
                            .replace("{page}", page)
                            .replace("{section}", section)
                            .replace("{path}", raw.trim());
                        vec![Node::Link(Link {
                            span,
                            url,
                            title: None,
                            target: None,
                            children: vec![emphasis],
                        })]
                    }
                    None => vec![emphasis],
                }
            }
            "makevar" | "regexp" => vec![Node::InlineCode(InlineCode {
                span,
                value: raw.to_owned(),
            })],
            "file" | "samp" => {
                let parts = variable_parts(raw);
                if parts.iter().all(|(variable, _)| !variable) {
                    return vec![Node::InlineCode(InlineCode {
                        span,
                        value: parts.into_iter().map(|(_, t)| t).collect(),
                    })];
                }
                let children = parts
                    .into_iter()
                    .map(|(variable, value)| {
                        let t = Node::Text(Text { span, value });
                        if variable {
                            Node::Emphasis(Emphasis {
                                span,
                                children: vec![t],
                            })
                        } else {
                            t
                        }
                    })
                    .collect();
                vec![inline(name, Some("code"), children, span)]
            }
            "kbd" => vec![inline("kbd", Some("kbd"), plain(raw), span)],
            "guilabel" => vec![inline("guilabel", None, accelerated(raw, span), span)],
            "menuselection" => {
                let items = raw.replace("-->", "\u{2023}");
                vec![inline(
                    "menuselection",
                    None,
                    accelerated(&items, span),
                    span,
                )]
            }
            "index" => {
                let n = self.indexes.get();
                self.indexes.set(n + 1);
                // `:index:` text shows as it is; `text <entry>` shows the
                // text.
                vec![
                    Node::Anchor(Anchor {
                        span,
                        id: format!("index-role-{n}"),
                    }),
                    Node::Text(Text {
                        span,
                        value: title.unwrap_or_else(|| raw.to_owned()),
                    }),
                ]
            }
            // A role declared by another plugin: left for it.
            _ => vec![Node::Custom(c.clone())],
        }
    }

    /// A link to a label (`ref`): to the document defining it, by label,
    /// for `transform-links` to resolve and fill in. A figure's caption is
    /// its text if none is given.
    fn reference(&self, target: &str, title: Option<&str>, span: Option<Span>) -> Node {
        let label = self.label(target);
        let children = match title {
            Some(t) => vec![text(t)],
            None => self
                .find("numbers", &label)
                .and_then(|(_, n)| n["caption"].as_str())
                .filter(|c| !c.is_empty())
                .map(|c| vec![text(c)])
                .unwrap_or_default(),
        };
        Node::Link(Link {
            span,
            url: format!("#{label}"),
            title: None,
            target: Some(LinkTarget {
                label: Some(label),
                ..LinkTarget::default()
            }),
            children,
        })
    }

    fn numref(
        &self,
        target: &str,
        title: Option<&str>,
        source: &str,
        span: Option<Span>,
        out: &mut Vec<Diagnostic>,
    ) -> Vec<Node> {
        let shown = vec![text(title.unwrap_or(target))];
        if !self.options.numfig {
            out.push(warning(span, "numfig is disabled. :numref: is ignored."));
            return shown;
        }
        let label = self.label(target);
        let Some((doc, info)) = self.find("numbers", &label) else {
            out.push(warning(
                span,
                format!("undefined label or unnumbered target: {target:?}"),
            ));
            return shown;
        };
        let number = info["number"].as_u64().unwrap_or_default().to_string();
        let caption = info["caption"].as_str().unwrap_or_default();
        let format = match title {
            Some(t) => t.to_owned(),
            None => self
                .options
                .numfig_format(info["kind"].as_str().unwrap_or("figure")),
        };
        let label_text = format
            .replace("%s", &number)
            .replace("{number}", &number)
            .replace("{name}", caption);
        vec![self.link_to(source, doc, Some(&label), vec![text(label_text)], span)]
    }

    /// A toctree's list of links, from the placeholder `Expand` made.
    fn toctree(&self, c: &Custom, source: &str) -> Vec<Node> {
        let maxdepth = c
            .attrs
            .get("maxdepth")
            .and_then(Value::as_str)
            .and_then(|d| d.parse::<i64>().ok())
            .filter(|d| *d > 0)
            .map_or(usize::MAX, |d| usize::try_from(d).unwrap_or(usize::MAX));
        let titles_only = c.attrs.contains_key("titlesonly");
        let mut items = Vec::new();
        let entries = c
            .attrs
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entry in &entries {
            let title = entry["title"].as_str();
            if let Some(url) = entry["url"].as_str() {
                let link = if url == "self" {
                    self.link_to(
                        source,
                        source,
                        None,
                        title.map(|t| vec![text(t)]).unwrap_or_default(),
                        c.span,
                    )
                } else {
                    Node::Link(Link {
                        url: url.to_owned(),
                        children: vec![text(title.unwrap_or(url))],
                        ..Link::default()
                    })
                };
                items.push(item(link, None));
                continue;
            }
            let Some(doc) = entry["document"].as_str() else {
                continue;
            };
            let mut visited = BTreeSet::from([source.to_owned()]);
            items.push(self.toc_entry(source, doc, title, 1, maxdepth, titles_only, &mut visited));
        }
        let mut children = Vec::new();
        if let Some(caption) = c.attrs.get("caption").and_then(Value::as_str) {
            children.push(Node::Paragraph(Paragraph {
                span: c.span,
                children: vec![Node::Strong(Strong {
                    span: c.span,
                    children: vec![text(caption)],
                })],
            }));
        }
        children.push(list(items));
        let mut node = Custom {
            span: c.span,
            kind: "toctree".into(),
            attrs: BTreeMap::new(),
            children,
        };
        if let Some(name) = c.attrs.get("name").and_then(Value::as_str) {
            node.attrs
                .insert("id".into(), json!(lsdocs_rst::make_id(name)));
        }
        vec![Node::Custom(node)]
    }

    /// One toctree entry: a link to `doc`, and below it (to `maxdepth`
    /// levels) its sections and the documents its own toctrees name.
    #[allow(clippy::too_many_arguments)]
    fn toc_entry(
        &self,
        from: &str,
        doc: &str,
        title: Option<&str>,
        depth: usize,
        maxdepth: usize,
        titles_only: bool,
        visited: &mut BTreeSet<String>,
    ) -> Node {
        let link = self.link_to(
            from,
            doc,
            None,
            title.map(|t| vec![text(t)]).unwrap_or_default(),
            None,
        );
        if depth >= maxdepth || !visited.insert(doc.to_owned()) {
            return item(link, None);
        }
        let page = self.pages.get(doc);
        let mut below = Vec::new();
        if !titles_only && let Some(page) = page {
            let sections = page["sections"].as_array().cloned().unwrap_or_default();
            // A page's one top section is the page itself: list what is
            // inside it.
            let sections = match sections.as_slice() {
                [only] => only["children"].as_array().cloned().unwrap_or_default(),
                _ => sections,
            };
            below.extend(self.section_items(from, doc, &sections, depth + 1, maxdepth));
        }
        if let Some(page) = page {
            for sub in page["toctree"].as_array().into_iter().flatten() {
                if let Some(sub) = sub.as_str() {
                    below.push(self.toc_entry(
                        from,
                        sub,
                        None,
                        depth + 1,
                        maxdepth,
                        titles_only,
                        visited,
                    ));
                }
            }
        }
        item(link, (!below.is_empty()).then(|| list(below)))
    }

    fn section_items(
        &self,
        from: &str,
        doc: &str,
        sections: &[Value],
        depth: usize,
        maxdepth: usize,
    ) -> Vec<Node> {
        if depth > maxdepth {
            return Vec::new();
        }
        sections
            .iter()
            .filter_map(|s| {
                let id = s["id"].as_str()?;
                let title = s["title"].as_str().unwrap_or_default();
                let link = self.link_to(from, doc, Some(id), vec![text(title)], None);
                let children = s["children"].as_array().cloned().unwrap_or_default();
                let below = self.section_items(from, doc, &children, depth + 1, maxdepth);
                Some(item(link, (!below.is_empty()).then(|| list(below))))
            })
            .collect()
    }

    /// Every todo of the site, each with a link to where it is.
    fn todolist(&self, source: &str, span: Option<Span>) -> Vec<Node> {
        if !self.options.todo_include_todos {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (doc, page) in &self.pages {
            for todo in page["todos"].as_array().into_iter().flatten() {
                let children: Vec<Node> =
                    serde_json::from_value(todo["children"].clone()).unwrap_or_default();
                let anchor = todo["anchor"].as_str().unwrap_or_default();
                let mut body = children;
                body.push(Node::Paragraph(Paragraph {
                    span,
                    children: vec![
                        text("(The original entry is located in "),
                        self.link_to(source, doc, Some(anchor), vec![text(*doc)], span),
                        text(".)"),
                    ],
                }));
                out.push(Node::Admonition(lsdocs_plugin_sdk::doctree::Admonition {
                    span,
                    kind: "todo".into(),
                    title: Some("Todo".into()),
                    children: body,
                }));
            }
        }
        out
    }
}

/// A `custom` inline node.
fn inline(kind: &str, tag: Option<&str>, children: Vec<Node>, span: Option<Span>) -> Node {
    let mut attrs = BTreeMap::new();
    attrs.insert("inline".to_owned(), json!(true));
    if let Some(tag) = tag {
        attrs.insert("html_tag".to_owned(), json!(tag));
    }
    Node::Custom(Custom {
        span,
        kind: kind.into(),
        attrs,
        children,
    })
}

/// Text with `&` marking accelerator keys (`&&` is a literal `&`): the
/// marked letters as `custom:accelerator` inlines.
fn accelerated(value: &str, span: Option<Span>) -> Vec<Node> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            buf.push(c);
            continue;
        }
        match chars.next() {
            Some('&') => buf.push('&'),
            Some(key) if !key.is_whitespace() => {
                if !buf.is_empty() {
                    out.push(Node::Text(Text {
                        span,
                        value: std::mem::take(&mut buf),
                    }));
                }
                out.push(inline(
                    "accelerator",
                    None,
                    vec![Node::Text(Text {
                        span,
                        value: key.to_string(),
                    })],
                    span,
                ));
            }
            Some(other) => {
                buf.push('&');
                buf.push(other);
            }
            None => buf.push('&'),
        }
    }
    if !buf.is_empty() {
        out.push(Node::Text(Text { span, value: buf }));
    }
    out
}

/// The parts of `file` and `samp` text: literal text, and `{variable}`
/// parts (`\{` is a literal brace).
fn variable_parts(value: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut buf = String::new();
    let mut variable = false;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next @ ('{' | '}' | '\\')) => buf.push(next),
                Some(next) => {
                    buf.push('\\');
                    buf.push(next);
                }
                None => buf.push('\\'),
            },
            '{' if !variable => {
                if !buf.is_empty() {
                    out.push((false, std::mem::take(&mut buf)));
                }
                variable = true;
            }
            '}' if variable => {
                out.push((true, std::mem::take(&mut buf)));
                variable = false;
            }
            c => buf.push(c),
        }
    }
    if variable {
        buf.insert(0, '{');
    }
    if !buf.is_empty() {
        out.push((false, buf));
    }
    out
}

fn item(link: Node, below: Option<Node>) -> Node {
    let mut children = vec![Node::Paragraph(Paragraph {
        span: None,
        children: vec![link],
    })];
    children.extend(below);
    Node::ListItem(ListItem {
        span: None,
        checked: None,
        children,
    })
}

fn list(items: Vec<Node>) -> Node {
    Node::List(List {
        span: None,
        ordered: false,
        start: None,
        children: items,
    })
}
