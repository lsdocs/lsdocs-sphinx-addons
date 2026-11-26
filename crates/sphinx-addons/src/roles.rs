//! Roles, and what needs the whole site: the data the collect pass
//! gathers from each document, and the apply pass that resolves roles,
//! toctrees and todo lists with it.

use std::collections::{BTreeMap, BTreeSet};

use lsdocs_plugin_sdk::doctree::{
    Custom, Document, Emphasis, Figure, InlineCode, Link, LinkTarget, List, ListItem, Node,
    Paragraph, Span, Strong, Text, plain_text, text,
};
use lsdocs_plugin_sdk::protocol::Diagnostic;
use lsdocs_plugin_sdk::relative_path;
use serde_json::{Value, json};

use crate::directives::{Notes, glob};
use crate::{Options, warning};

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
    "option",
    "envvar",
    "confval",
    "token",
    "keyword",
];

/// The source path of the generated index page.
pub(crate) const GENINDEX: &str = "genindex.md";

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

/// The site's documents, from their source paths (the files parsers
/// took, whatever their extension): their names (paths without the
/// extension) and source paths.
fn documents<'s>(sources: impl IntoIterator<Item = &'s str>) -> Vec<(String, String)> {
    sources
        .into_iter()
        .map(|source| {
            let name = match source.rsplit_once('.') {
                Some((name, _)) if !name.is_empty() && !name.ends_with('/') => name,
                _ => source,
            };
            (name.to_owned(), source.to_owned())
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
pub(crate) fn anchors(document: &Document) -> Vec<String> {
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
        "toctrees": notes.toctrees,
        "index": notes.index,
        "options": notes.options,
        "envvars": notes.envvars,
        "confvals": notes.confvals,
        "tokens": notes.tokens,
    })
}

/// Everything the apply pass knows about the site.
pub(crate) struct Site<'a> {
    /// The collected data of every document, by source.
    pages: BTreeMap<&'a str, &'a Value>,
    /// The site's documents: the ones collected from.
    docs: Vec<(String, String)>,
    /// The toctrees of each document, resolved.
    trees: BTreeMap<&'a str, Vec<Tree>>,
    files: BTreeSet<&'a str>,
    options: &'a Options,
    /// Section numbers from numbered toctrees, by document and section id.
    numbers: BTreeMap<String, BTreeMap<String, Vec<u32>>>,
    /// Whether the index page exists.
    has_index: bool,
}

impl<'a> Site<'a> {
    pub fn new(collected: &'a Value, files: &'a [String], options: &'a Options) -> Self {
        let pages: BTreeMap<&str, &Value> = collected
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.as_str(), v)).collect())
            .unwrap_or_default();
        let docs = documents(pages.keys().copied());
        let has_index = pages
            .values()
            .any(|p| p["index"].as_array().is_some_and(|a| !a.is_empty()));
        let trees = pages
            .iter()
            .map(|(source, page)| {
                let trees = page["toctrees"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|tree| Tree {
                        documents: resolve_entries(
                            &docs,
                            source,
                            tree["entries"].as_array().map_or(&[], Vec::as_slice),
                            tree["reversed"].as_bool().unwrap_or(false),
                            &mut |_| {},
                        )
                        .iter()
                        .filter_map(|e| e["document"].as_str())
                        .filter(|d| *d != GENINDEX)
                        .map(str::to_owned)
                        .collect(),
                        numbered: tree["numbered"].as_u64().unwrap_or(0),
                    })
                    .collect();
                (*source, trees)
            })
            .collect();
        let numbers = section_numbers(&pages, &trees);
        Self {
            pages,
            docs,
            trees,
            files: files.iter().map(String::as_str).collect(),
            options,
            numbers,
            has_index,
        }
    }

    /// The documents the toctrees of `doc` name, hidden or not, in order.
    pub fn toctree_documents(&self, doc: &str) -> Vec<&str> {
        toctree_documents(&self.trees, doc)
    }

    /// The number of section `id` in `doc`, as `1.2`, if it has one.
    fn section_number(&self, doc: &str, id: &str) -> Option<String> {
        let number = self.numbers.get(doc)?.get(id)?;
        Some(
            number
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join("."),
        )
    }

    /// The number of the page `doc` (its one top section), if it has one.
    fn page_number(&self, doc: &str) -> Option<String> {
        let sections = self.pages.get(doc)?["sections"].as_array()?;
        match sections.as_slice() {
            [only] => self.section_number(doc, only["id"].as_str()?),
            _ => None,
        }
    }

    /// A page's title in a toctree: numbered if its section is.
    fn numbered_title(&self, doc: &str) -> Option<String> {
        let number = self.page_number(doc)?;
        let title = self.pages.get(doc)?["title"].as_str()?;
        Some(format!("{number}. {title}"))
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
        if let Some(numbers) = self.numbers.get(&source) {
            number_headings(&mut document.children, numbers);
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
                    result.extend(self.toctree(&c, source, out));
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
            "doc" if target == "genindex" && self.has_index => {
                let children = plain(title.as_deref().unwrap_or("Index"));
                vec![self.link_to(source, GENINDEX, None, children, span)]
            }
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
            "option" | "envvar" | "confval" | "token" | "keyword" => {
                self.std_object(c, name, title.as_deref(), &target, source, out)
            }
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
            // A role declared by another plugin: left for it.
            _ => vec![Node::Custom(c.clone())],
        }
    }

    /// A link to a label (`ref`): to the document defining it, by label,
    /// for `transform-links` to resolve and fill in. A figure's caption is
    /// its text if none is given.
    fn reference(&self, target: &str, title: Option<&str>, span: Option<Span>) -> Node {
        if target == "genindex" && self.has_index && self.anchor_document(target).is_none() {
            return Node::Link(Link {
                span,
                url: format!("/{GENINDEX}"),
                title: None,
                target: Some(LinkTarget {
                    document: Some(GENINDEX.to_owned()),
                    ..LinkTarget::default()
                }),
                children: vec![text(title.unwrap_or("Index"))],
            });
        }
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
            // A numbered section.
            if let Some(doc) = self.anchor_document(&label)
                && let Some(number) = self.section_number(doc, &label)
            {
                let name = self
                    .pages
                    .get(doc)
                    .and_then(|p| section_title(&p["sections"], &label));
                let format = title
                    .map(str::to_owned)
                    .unwrap_or_else(|| self.options.numfig_format("section"));
                let label_text = format
                    .replace("%s", &number)
                    .replace("{number}", &number)
                    .replace("{name}", name.as_deref().unwrap_or_default());
                return vec![self.link_to(source, doc, Some(&label), vec![text(label_text)], span)];
            }
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

    /// A reference of the standard domain (`option`, `envvar`, `confval`,
    /// `token`, `keyword`): a link around the code it shows, or the code
    /// alone if nothing has the name (a warning, except for `envvar` and
    /// `token`, as Sphinx).
    fn std_object(
        &self,
        c: &Custom,
        name: &str,
        title: Option<&str>,
        target: &str,
        source: &str,
        out: &mut Vec<Diagnostic>,
    ) -> Vec<Node> {
        let span = c.span;
        let (target, shown) = if name == "token" {
            // `~group:name` shows `name`.
            let shown = match title {
                Some(title) => title.to_owned(),
                None => match target.strip_prefix('~') {
                    Some(rest) => rest.rsplit_once(':').map_or(rest, |(_, n)| n).to_owned(),
                    None => target.to_owned(),
                },
            };
            (target.trim_start_matches('~').to_owned(), shown)
        } else {
            (target.to_owned(), title.unwrap_or(target).to_owned())
        };
        let found: Option<(&str, String)> = match name {
            "option" => {
                let program = c.attrs.get("program").and_then(Value::as_str);
                self.find_option(program, &target)
                    .and_then(|(doc, id)| Some((doc, id.as_str()?.to_owned())))
            }
            "keyword" => {
                let label = self.label(&target);
                self.anchor_document(&label).map(|doc| (doc, label))
            }
            _ => {
                let field = format!("{name}s");
                self.find(&field, &target)
                    .and_then(|(doc, id)| Some((doc, id.as_str()?.to_owned())))
            }
        };
        let code = vec![Node::InlineCode(InlineCode { span, value: shown })];
        match found {
            Some((doc, id)) => vec![self.link_to(source, doc, Some(&id), code, span)],
            None => {
                let message = match name {
                    "option" => Some(format!("unknown option: {target:?}")),
                    "keyword" => Some(format!("unknown keyword: {target:?}")),
                    "confval" => Some(format!("'confval' reference target not found: {target}")),
                    _ => None,
                };
                if let Some(message) = message {
                    out.push(warning(span, message));
                }
                code
            }
        }
    }

    /// The option `target` (`-o`, `--output=FILE`, `prog sub -o`) of
    /// `program`, found as Sphinx finds it: as written, then without its
    /// value, then with its first words taken for the (sub)program.
    fn find_option(&self, program: Option<&str>, target: &str) -> Option<(&'a str, &'a Value)> {
        let find = |program: Option<&str>, option: &str| {
            self.find(
                "options",
                &crate::directives::std_domain::option_key(program, option),
            )
        };
        let target = target.trim();
        if let Some(found) = find(program, target) {
            return Some(found);
        }
        for needle in ["=", "[=", " "] {
            if let Some((stem, _)) = target.split_once(needle)
                && let Some(found) = find(program, stem)
            {
                return Some(found);
            }
        }
        let mut commands = Vec::new();
        let mut rest = target;
        while let Some((command, after)) = rest.split_once(char::is_whitespace) {
            commands.push(command);
            rest = after.trim_start();
            if let Some(found) = find(Some(&commands.join("-")), rest) {
                return Some(found);
            }
        }
        None
    }

    /// A toctree's list of links, from the placeholder `Expand` made
    /// (nothing for a hidden one, whose entries are still checked).
    fn toctree(&self, c: &Custom, source: &str, out: &mut Vec<Diagnostic>) -> Vec<Node> {
        let entries = resolve_entries(
            &self.docs,
            source,
            c.attrs
                .get("entries")
                .and_then(Value::as_array)
                .map_or(&[], Vec::as_slice),
            c.attrs
                .get("reversed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            &mut |message| out.push(warning(c.span, message)),
        );
        if c.attrs.contains_key("hidden") {
            return Vec::new();
        }
        let maxdepth = c
            .attrs
            .get("maxdepth")
            .and_then(Value::as_str)
            .and_then(|d| d.parse::<i64>().ok())
            .filter(|d| *d > 0)
            .map_or(usize::MAX, |d| usize::try_from(d).unwrap_or(usize::MAX));
        let titles_only = c.attrs.contains_key("titlesonly");
        let mut items = Vec::new();
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
            if doc == GENINDEX {
                if self.has_index {
                    let link = self.link_to(
                        source,
                        GENINDEX,
                        None,
                        title.map(|t| vec![text(t)]).unwrap_or_default(),
                        c.span,
                    );
                    items.push(item(link, None));
                }
                continue;
            }
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
        let shown = title
            .map(str::to_owned)
            .or_else(|| self.numbered_title(doc));
        let link = self.link_to(
            from,
            doc,
            None,
            shown.map(|t| vec![text(t)]).unwrap_or_default(),
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
        for sub in self.toctree_documents(doc) {
            below.push(self.toc_entry(from, sub, None, depth + 1, maxdepth, titles_only, visited));
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
                let mut title = s["title"].as_str().unwrap_or_default().to_owned();
                if let Some(number) = self.section_number(doc, id) {
                    title = format!("{number}. {title}");
                }
                let link = self.link_to(from, doc, Some(id), vec![text(&title)], None);
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

/// A toctree, resolved: the documents it names and how deep `:numbered:`
/// numbers them (0 for not at all).
pub(crate) struct Tree {
    documents: Vec<String>,
    numbered: u64,
}

/// The documents the toctrees of `doc` name, hidden or not, in order.
fn toctree_documents<'t>(trees: &'t BTreeMap<&str, Vec<Tree>>, doc: &str) -> Vec<&'t str> {
    trees
        .get(doc)
        .into_iter()
        .flatten()
        .flat_map(|t| t.documents.iter().map(String::as_str))
        .collect()
}

/// Resolves the entries of a toctree in `from`, as `Expand` wrote them,
/// against the site's documents `docs`: each becomes `{title, url}` or
/// `{title, document}` (a glob, one per document it matches, in order).
/// Calls `warn` for each entry that names no document.
fn resolve_entries(
    docs: &[(String, String)],
    from: &str,
    entries: &[Value],
    reversed: bool,
    warn: &mut dyn FnMut(String),
) -> Vec<Value> {
    let mut out = Vec::new();
    for entry in entries {
        let title = &entry["title"];
        if entry.get("url").is_some() {
            out.push(entry.clone());
        } else if let Some(target) = entry["glob"].as_str() {
            let pattern = absolute(from, target);
            let mut matched: Vec<&String> = docs
                .iter()
                .filter(|(name, source)| source != from && glob(&pattern, name))
                .map(|(_, source)| source)
                .collect();
            matched.sort();
            if matched.is_empty() {
                warn(format!(
                    "toctree glob pattern {target:?} didn't match any documents"
                ));
            }
            out.extend(matched.into_iter().map(|s| json!({"document": s})));
        } else if let Some(target) = entry["target"].as_str() {
            match find_document(docs, from, target) {
                Some(source) => out.push(json!({"title": title, "document": source})),
                None if target == "genindex" => {
                    out.push(json!({"title": title, "document": GENINDEX}));
                }
                None => warn(format!(
                    "toctree contains reference to nonexisting document {target:?}"
                )),
            }
        }
    }
    if reversed {
        out.reverse();
    }
    out
}

/// Section numbers from numbered toctrees: for each toctree with
/// `:numbered:`, its pages are numbered 1, 2, ... (below the number of the
/// page holding it, if that has one), their sections below that, and the
/// pages their own toctrees name after their sections, as deep as
/// `:numbered:` says.
fn section_numbers(
    pages: &BTreeMap<&str, &Value>,
    trees: &BTreeMap<&str, Vec<Tree>>,
) -> BTreeMap<String, BTreeMap<String, Vec<u32>>> {
    let mut out: BTreeMap<String, BTreeMap<String, Vec<u32>>> = BTreeMap::new();
    let mut done = BTreeSet::new();
    for (source, page) in pages {
        for tree in trees.get(source).into_iter().flatten() {
            let depth = tree.numbered;
            if depth == 0 {
                continue;
            }
            let prefix: Vec<u32> = page["sections"]
                .as_array()
                .and_then(|s| match s.as_slice() {
                    [only] => only["id"].as_str(),
                    _ => None,
                })
                .and_then(|id| out.get(*source)?.get(id).cloned())
                .unwrap_or_default();
            let mut n = 0;
            for doc in &tree.documents {
                n += 1;
                let mut number = prefix.clone();
                number.push(n);
                number_page(
                    doc,
                    number,
                    depth as usize,
                    pages,
                    trees,
                    &mut out,
                    &mut done,
                );
            }
        }
    }
    out
}

fn number_page(
    doc: &str,
    number: Vec<u32>,
    depth: usize,
    pages: &BTreeMap<&str, &Value>,
    trees: &BTreeMap<&str, Vec<Tree>>,
    out: &mut BTreeMap<String, BTreeMap<String, Vec<u32>>>,
    done: &mut BTreeSet<String>,
) {
    if number.len() > depth || !done.insert(doc.to_owned()) {
        return;
    }
    let Some(page) = pages.get(doc) else {
        return;
    };
    let sections = page["sections"].as_array().cloned().unwrap_or_default();
    let children = match sections.as_slice() {
        [only] => {
            if let Some(id) = only["id"].as_str() {
                out.entry(doc.to_owned())
                    .or_default()
                    .insert(id.to_owned(), number.clone());
            }
            only["children"].as_array().cloned().unwrap_or_default()
        }
        _ => sections,
    };
    let mut n = number_sections(doc, &children, &number, depth, out);
    for sub in toctree_documents(trees, doc) {
        n += 1;
        let mut below = number.clone();
        below.push(n);
        number_page(sub, below, depth, pages, trees, out, done);
    }
}

/// Numbers `sections` below `number`; returns how many there are.
fn number_sections(
    doc: &str,
    sections: &[Value],
    number: &[u32],
    depth: usize,
    out: &mut BTreeMap<String, BTreeMap<String, Vec<u32>>>,
) -> u32 {
    let mut n = 0;
    for section in sections {
        n += 1;
        let mut own = number.to_vec();
        own.push(n);
        if own.len() > depth {
            continue;
        }
        if let Some(id) = section["id"].as_str() {
            out.entry(doc.to_owned())
                .or_default()
                .insert(id.to_owned(), own.clone());
        }
        let children = section["children"].as_array().cloned().unwrap_or_default();
        number_sections(doc, &children, &own, depth, out);
    }
    n
}

/// Puts each numbered section's number before its title: `1.2. Title`.
fn number_headings(nodes: &mut [Node], numbers: &BTreeMap<String, Vec<u32>>) {
    for node in nodes {
        if let Node::Section(section) = node
            && let Some(number) = section.id.as_ref().and_then(|id| numbers.get(id))
            && let Some(Node::Heading(heading)) = section.children.first_mut()
        {
            let number: Vec<String> = number.iter().map(u32::to_string).collect();
            heading
                .children
                .insert(0, text(format!("{}. ", number.join("."))));
        }
        if let Some(children) = node.children_mut() {
            number_headings(children, numbers);
        }
    }
}

/// The title of section `id` in a collected section tree.
fn section_title(sections: &Value, id: &str) -> Option<String> {
    for section in sections.as_array()? {
        if section["id"] == id {
            return section["title"].as_str().map(str::to_owned);
        }
        if let Some(title) = section_title(&section["children"], id) {
            return Some(title);
        }
    }
    None
}

/// The index page, from every page's index entries, if there are any.
pub(crate) fn genindex(collected: &Value) -> Option<Document> {
    /// An entry: its name, its targets (page, anchor, main) and its
    /// subentries.
    #[derive(Default)]
    struct Entry {
        name: String,
        targets: Vec<(String, String, bool)>,
        subs: BTreeMap<String, Entry>,
    }
    let mut letters: BTreeMap<String, BTreeMap<String, Entry>> = BTreeMap::new();
    let pages = collected.as_object()?;
    for (source, page) in pages {
        for e in page["index"].as_array().into_iter().flatten() {
            let kind = e["type"].as_str().unwrap_or("single");
            let value = e["value"].as_str().unwrap_or_default();
            let anchor = e["anchor"].as_str().unwrap_or_default();
            let main = e["main"].as_bool().unwrap_or(false);
            let parts: Vec<&str> = value.split(';').map(str::trim).collect();
            let get = |i: usize| parts.get(i).copied().unwrap_or_default();
            // (entry, subentry, linked)
            let made: Vec<(String, Option<String>, bool)> = match kind {
                "pair" => vec![
                    (get(0).into(), Some(get(1).into()), true),
                    (get(1).into(), Some(get(0).into()), true),
                ],
                "triple" => vec![
                    (get(0).into(), Some(format!("{} {}", get(1), get(2))), true),
                    (get(1).into(), Some(format!("{}, {}", get(2), get(0))), true),
                    (get(2).into(), Some(format!("{} {}", get(0), get(1))), true),
                ],
                "see" => vec![(get(0).into(), Some(format!("see {}", get(1))), false)],
                "seealso" => vec![(get(0).into(), Some(format!("see also {}", get(1))), false)],
                _ => vec![(
                    get(0).into(),
                    (parts.len() > 1).then(|| parts[1..].join("; ")),
                    true,
                )],
            };
            for (name, sub, linked) in made {
                let letter = match name.chars().next() {
                    Some(c) if c.is_alphabetic() => c.to_uppercase().collect(),
                    _ => "Symbols".to_owned(),
                };
                let entry = letters
                    .entry(letter)
                    .or_default()
                    .entry(name.to_lowercase())
                    .or_default();
                entry.name.clone_from(&name);
                let target = (source.clone(), anchor.to_owned(), main);
                match sub {
                    Some(sub) => {
                        let s = entry.subs.entry(sub.to_lowercase()).or_default();
                        s.name = sub;
                        if linked {
                            s.targets.push(target);
                        }
                    }
                    None if linked => entry.targets.push(target),
                    None => {}
                }
            }
        }
    }
    if letters.is_empty() {
        return None;
    }
    fn line(entry: &Entry) -> Node {
        let link = |(doc, anchor, main): &(String, String, bool), label: Node| {
            let label = if *main {
                Node::Strong(Strong {
                    span: None,
                    children: vec![label],
                })
            } else {
                label
            };
            Node::Link(Link {
                span: None,
                url: format!("{}#{anchor}", relative_path(GENINDEX, doc)),
                title: None,
                target: Some(LinkTarget {
                    document: Some(doc.clone()),
                    anchor: Some(anchor.clone()),
                    label: None,
                }),
                children: vec![label],
            })
        };
        let mut inlines = Vec::new();
        match entry.targets.split_first() {
            Some((first, rest)) => {
                inlines.push(link(first, text(&entry.name)));
                for (i, target) in rest.iter().enumerate() {
                    inlines.push(text(", "));
                    inlines.push(link(target, text(format!("[{}]", i + 2))));
                }
            }
            None => inlines.push(text(&entry.name)),
        }
        let mut children = vec![Node::Paragraph(Paragraph {
            span: None,
            children: inlines,
        })];
        if !entry.subs.is_empty() {
            children.push(list(entry.subs.values().map(line).collect()));
        }
        Node::ListItem(ListItem {
            span: None,
            checked: None,
            children,
        })
    }
    let mut body = vec![Node::Heading(lsdocs_plugin_sdk::doctree::Heading {
        span: None,
        level: 1,
        id: None,
        children: vec![text("Index")],
    })];
    // Letters first, then symbols.
    let (symbols, letters): (Vec<_>, Vec<_>) = letters
        .into_iter()
        .partition(|(letter, _)| letter == "Symbols");
    for (letter, entries) in letters.into_iter().chain(symbols) {
        body.push(Node::Section(lsdocs_plugin_sdk::doctree::Section {
            span: None,
            id: None,
            children: vec![
                Node::Heading(lsdocs_plugin_sdk::doctree::Heading {
                    span: None,
                    level: 2,
                    id: None,
                    children: vec![text(letter)],
                }),
                list(entries.values().map(line).collect()),
            ],
        }));
    }
    let mut document = Document::new(GENINDEX);
    document.metadata.insert("title".into(), json!("Index"));
    document.metadata.insert("nav_exclude".into(), json!(true));
    document.children = vec![Node::Section(lsdocs_plugin_sdk::doctree::Section {
        span: None,
        id: Some("genindex".into()),
        children: body,
    })];
    Some(document)
}

/// A `custom` inline node.
pub(crate) fn inline(
    kind: &str,
    tag: Option<&str>,
    children: Vec<Node>,
    span: Option<Span>,
) -> Node {
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
pub(crate) fn variable_parts(value: &str) -> Vec<(bool, String)> {
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
