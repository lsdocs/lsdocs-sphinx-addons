//! Sphinx's standard domain: `program`, `option` (`cmdoption`), `envvar`,
//! `confval` and `productionlist`, with `describe` (`object`), which shows
//! a signature as they do. Their targets are collected like glossary terms
//! (`Notes`), and `Site` resolves the roles that name them. The domain's
//! names can be written with a `std:` prefix (`std:option`, `:std:ref:`).

use lsdocs_plugin_sdk::Context;
use lsdocs_plugin_sdk::doctree::{
    Custom, DefinitionDescription, DefinitionList, DefinitionTerm, Document, Emphasis, InlineCode,
    Node, Span, Strong, Text,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{Expand, Parts, anchor, paragraph};

/// The attribute holding a directive's options parsed as inline markup.
pub(crate) const PARSED_OPTIONS: &str = "parsed_options";

/// The options of a directive that are inline markup.
fn inline_options(kind: &str) -> &'static [&'static str] {
    match directive_name(kind) {
        "confval" => &["type", "default"],
        _ => &[],
    }
}

/// Parses the options of the domain's directives that are inline markup
/// (`confval`'s `:type:` and `:default:`) with the document's parser
/// (`host/parse`), as Sphinx does, keeping them in the directive's
/// `PARSED_OPTIONS` attribute for `Expand`. An option no parser takes, or
/// that is not one paragraph, stays text.
pub(crate) async fn parse_options(document: &mut Document, ctx: &Context) {
    let Some((_, extension)) = document.source.rsplit_once('.') else {
        return;
    };
    let mut texts = Vec::new();
    visit(&document.children, &mut |c| {
        for option in inline_options(&c.kind) {
            if let Some(text) = c
                .attrs
                .get("options")
                .and_then(|o| o.get(*option))
                .and_then(Value::as_str)
            {
                texts.push(text.trim().to_owned());
            }
        }
    });
    if texts.is_empty() {
        return;
    }
    let mut parsed = Vec::with_capacity(texts.len());
    for text in texts {
        let nodes = match ctx.parse(extension, &document.source, &text).await {
            Ok(result) => match <[Node; 1]>::try_from(result.document.children) {
                Ok([Node::Paragraph(p)]) => Some(p.children),
                _ => None,
            },
            Err(_) => None,
        };
        parsed.push(nodes);
    }
    let mut parsed = parsed.into_iter();
    visit_mut(&mut document.children, &mut |c| {
        let mut map = serde_json::Map::new();
        for option in inline_options(&c.kind) {
            let given = c
                .attrs
                .get("options")
                .and_then(|o| o.get(*option))
                .is_some_and(Value::is_string);
            if given && let Some(Some(nodes)) = parsed.next() {
                map.insert((*option).to_owned(), json!(nodes));
            }
        }
        if !map.is_empty() {
            c.attrs
                .insert(PARSED_OPTIONS.to_owned(), Value::Object(map));
        }
    });
}

/// Calls `f` with every `custom` node of `nodes` and below, in document
/// order.
fn visit(nodes: &[Node], f: &mut impl FnMut(&Custom)) {
    for node in nodes {
        if let Node::Custom(c) = node {
            f(c);
        }
        visit(node.children(), f);
    }
}

fn visit_mut(nodes: &mut [Node], f: &mut impl FnMut(&mut Custom)) {
    for node in nodes {
        if let Node::Custom(c) = node {
            f(c);
        }
        if let Some(children) = node.children_mut() {
            visit_mut(children, f);
        }
    }
}

/// The directives handled here, without their `std:` prefix.
pub(super) const DIRECTIVES: &[&str] = &[
    "program",
    "option",
    "cmdoption",
    "envvar",
    "confval",
    "productionlist",
    "describe",
    "object",
];

/// The directives of the domain, which may be written `std:<name>`.
pub(crate) const DOMAIN_DIRECTIVES: &[&str] = &[
    "program",
    "option",
    "cmdoption",
    "envvar",
    "confval",
    "glossary",
    "productionlist",
];

/// The roles of the domain, which may be written `std:<name>`.
pub(crate) const DOMAIN_ROLES: &[&str] = &[
    "option", "confval", "envvar", "token", "term", "ref", "numref", "keyword", "doc",
];

/// `name` without the `std:` prefix of a directive of the domain.
pub(super) fn directive_name(name: &str) -> &str {
    match name.strip_prefix("std:") {
        Some(rest) if DOMAIN_DIRECTIVES.contains(&rest) => rest,
        _ => name,
    }
}

/// `name` without the `std:` prefix of a role of the domain.
fn role_name(name: &str) -> &str {
    match name.strip_prefix("std:") {
        Some(rest) if DOMAIN_ROLES.contains(&rest) => rest,
        _ => name,
    }
}

/// An id as Sphinx makes one (`sphinx.util.nodes._make_id`): case and
/// `.` and `_` are kept, other runs of characters become `-`, and leading
/// `-`, digits, `.` and `_` and trailing `-` are dropped. Characters
/// outside ASCII are dropped (Sphinx first decomposes them, so `e` with an
/// accent is `e` there).
pub(crate) fn sphinx_id(text: &str) -> String {
    let ascii: String = text.chars().filter(char::is_ascii).collect();
    let words = ascii.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut id = String::new();
    let mut gap = false;
    for c in words.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            if gap {
                id.push('-');
                gap = false;
            }
            id.push(c);
        } else {
            gap = true;
        }
    }
    if gap {
        id.push('-');
    }
    id.trim_start_matches(|c: char| c == '-' || c == '.' || c == '_' || c.is_ascii_digit())
        .trim_end_matches('-')
        .to_owned()
}

/// The signatures of a directive's argument: one per line, a backslash
/// at the end of a line joining it to the next.
fn signatures(argument: &str) -> Vec<String> {
    argument
        .replace("\\\n", "")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `text` with runs of whitespace made one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn code(value: &str) -> Node {
    Node::InlineCode(InlineCode {
        span: None,
        value: value.to_owned(),
    })
}

fn plain(value: &str) -> Node {
    Node::Text(Text {
        span: None,
        value: value.to_owned(),
    })
}

/// The name of a signature, as Sphinx's `desc_name`.
fn name(value: &str) -> Node {
    Node::Strong(Strong {
        span: None,
        children: vec![code(value)],
    })
}

/// An option of an `option` signature: its name and its arguments
/// (`--output=FILE` is `--output` and `=FILE`; `-j[=N]` is `-j` and
/// `[=N]`). `None` if it has no name.
fn split_option(text: &str) -> Option<(String, String)> {
    let end = text
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    let (mut option, mut args) = (text[..end].to_owned(), text[end..].to_owned());
    if option.ends_with('[') && args.ends_with(']') {
        option.pop();
        args.insert(0, '[');
    }
    Some((option, args))
}

/// The attribute of a role whose text is shown as is, not as code: the
/// token references of a `productionlist`, already in its `code`.
pub(crate) const UNCODED: &str = "uncoded";

/// A role of `productionlist`'s token references: `title <target>`.
fn token_role(title: &str, target: &str) -> Node {
    let mut attrs = BTreeMap::new();
    attrs.insert("name".to_owned(), json!("token"));
    attrs.insert(UNCODED.to_owned(), json!(true));
    attrs.insert("text".to_owned(), json!(format!("{title} <{target}>")));
    Node::Custom(Custom {
        span: None,
        kind: "role".into(),
        attrs,
        children: Vec::new(),
    })
}

/// Whether `text` is a token of a production: `` `name` ``,
/// `` `group:name` `` or `` `~group:name` `` (shown as `name`).
fn is_token(text: &str) -> bool {
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_');
    match text.rsplit_once(':') {
        Some((group, name)) => {
            let group = group.strip_prefix('~').unwrap_or(group);
            word(name)
                && group
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        }
        None => word(text),
    }
}

/// The tokens of a production: text, with each `` `token` `` a reference
/// to it (in `group` unless it names one).
fn token_nodes(text: &str, group: &str) -> Vec<Node> {
    let mut out = Vec::new();
    let mut plain_from = 0;
    let mut at = 0;
    while let Some(open) = text[at..].find('`').map(|i| at + i) {
        let Some(close) = text[open + 1..].find('`').map(|i| open + 1 + i) else {
            break;
        };
        let token = &text[open + 1..close];
        if !is_token(token) {
            at = open + 1;
            continue;
        }
        if open > plain_from {
            out.push(plain(&text[plain_from..open]));
        }
        let (title, target) = match token.split_once(':') {
            Some(_) if token.starts_with('~') => {
                let (_, title) = token.rsplit_once(':').unwrap_or(("", token));
                (title.to_owned(), token[1..].to_owned())
            }
            Some(("", title)) => (title.to_owned(), title.to_owned()),
            Some(_) => (token.to_owned(), token.to_owned()),
            None if group.is_empty() => (token.to_owned(), token.to_owned()),
            None => (token.to_owned(), format!("{group}:{token}")),
        };
        out.push(token_role(&title, &target));
        at = close + 1;
        plain_from = at;
    }
    if plain_from < text.len() {
        out.push(plain(&text[plain_from..]));
    }
    out
}

impl Expand<'_> {
    /// An id for `term`, as Sphinx's `make_id` makes one: `<prefix>-<term>`
    /// (`sphinx_id`), or `<prefix>-<n>` if that is taken in the document
    /// (or is the prefix alone).
    fn make_id(&mut self, prefix: &str, term: &str) -> String {
        let mut id = Some(sphinx_id(&format!("{prefix}-{term}")))
            .filter(|id| !id.is_empty() && id != prefix);
        loop {
            match id {
                Some(found) if !self.ids.contains(&found) => {
                    self.ids.insert(found.clone());
                    return found;
                }
                _ => {
                    let n = self.serials.entry(prefix.to_owned()).or_insert(0);
                    id = Some(format!("{prefix}-{n}"));
                    *n += 1;
                }
            }
        }
    }

    /// The arguments of an option (`=FILE`, ` FILE`, `[=N]`) as code; with
    /// `option_emphasise_placeholders`, as Sphinx, their `{placeholders}`
    /// emphasized (without the braces).
    fn option_args(&self, args: &str) -> Node {
        let parts = crate::roles::variable_parts(args);
        if !self.options.option_emphasise_placeholders || parts.iter().all(|(v, _)| !v) {
            return code(args);
        }
        let children = parts
            .into_iter()
            .map(|(variable, value)| {
                let text = plain(&value);
                if variable {
                    Node::Emphasis(Emphasis {
                        span: None,
                        children: vec![text],
                    })
                } else {
                    text
                }
            })
            .collect();
        crate::roles::inline("samp", Some("code"), children, None)
    }

    /// A role of the domain: its name without `std:`; `option` remembers
    /// the current program, and `envvar` makes index entries, like
    /// Sphinx.
    pub(super) fn std_role(&mut self, mut c: Custom) -> Vec<Node> {
        let name = c
            .attrs
            .get("name")
            .and_then(Value::as_str)
            .map(|n| role_name(n).to_owned())
            .unwrap_or_default();
        c.attrs.insert("name".to_owned(), json!(name));
        let mut out = Vec::new();
        match name.as_str() {
            "option" => {
                c.attrs.insert("program".to_owned(), json!(self.program));
            }
            "envvar" => {
                let raw = c
                    .attrs
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (_, target) = crate::roles::split_title(raw);
                let id = self.index_anchor(None);
                let entries = vec![
                    ("single".to_owned(), target.clone(), false),
                    (
                        "single".to_owned(),
                        format!("environment variable; {target}"),
                        false,
                    ),
                ];
                self.add_index_entries(entries, &id, c.span);
                out.push(anchor(&id));
            }
            _ => {}
        }
        out.push(Node::Custom(c));
        out
    }

    /// `program`: the program the options that follow belong to (`None`
    /// for none).
    pub(super) fn program(&mut self, d: &Parts) -> Vec<Node> {
        let program = d.argument.split_whitespace().collect::<Vec<_>>().join("-");
        self.program = (!program.is_empty() && program != "None").then_some(program);
        Vec::new()
    }

    /// `option`, `envvar`, `confval` and `describe`: each signature (one
    /// per line) a term, with its targets and index entries, and the
    /// content their description.
    pub(super) fn object(&mut self, d: &Parts, children: Vec<Node>) -> Vec<Node> {
        let kind = d.kind.as_str();
        let indexed =
            !matches!(kind, "describe" | "object") && !(d.flag("no-index") || d.flag("noindex"));
        let entries = indexed && !(d.flag("no-index-entry") || d.flag("noindexentry"));
        let mut list = Vec::new();
        let mut anchors = Vec::new();
        for signature in signatures(&d.argument) {
            let mut term = Vec::new();
            match kind {
                "option" | "cmdoption" => {
                    let names = self.option_signature(&signature, d.span, &mut term);
                    if indexed && !names.is_empty() {
                        let ids = self.option_targets(&names, entries, d.span);
                        let mut targets: Vec<Node> = ids.iter().map(|id| anchor(id)).collect();
                        anchors.extend(targets.iter().cloned());
                        targets.append(&mut term);
                        term = targets;
                    }
                }
                _ => {
                    term.push(name(&signature));
                    if indexed {
                        let name = collapse(&signature);
                        let id = self.make_id(kind, &name);
                        let (field, entry) = if kind == "envvar" {
                            ("single", format!("environment variable; {name}"))
                        } else {
                            ("pair", format!("{name}; configuration value"))
                        };
                        let collected = if kind == "envvar" {
                            &mut self.notes.envvars
                        } else {
                            &mut self.notes.confvals
                        };
                        collected.entry(name).or_insert_with(|| id.clone());
                        if entries {
                            self.add_index_entries(
                                vec![(field.to_owned(), entry, false)],
                                &id,
                                d.span,
                            );
                        }
                        anchors.push(anchor(&id));
                        term.insert(0, anchor(&id));
                    }
                }
            }
            list.push(Node::DefinitionTerm(DefinitionTerm {
                span: d.span,
                children: term,
            }));
        }
        if d.flag("no-typesetting") {
            return anchors;
        }
        let mut description = Vec::new();
        if kind == "confval" {
            for (option, label) in [("type", "Type"), ("default", "Default")] {
                if let Some(value) = d.option(option) {
                    let body = match d.parsed_options.get(option) {
                        Some(nodes) => {
                            let nodes: Vec<Node> =
                                serde_json::from_value(nodes.clone()).unwrap_or_default();
                            self.nodes(nodes)
                        }
                        None => vec![plain(value.trim())],
                    };
                    let mut line = vec![
                        Node::Strong(Strong {
                            span: d.span,
                            children: vec![plain(&format!("{label}:"))],
                        }),
                        plain(" "),
                    ];
                    line.extend(body);
                    description.push(paragraph(line, d.span));
                }
            }
        }
        description.extend(children);
        list.push(Node::DefinitionDescription(DefinitionDescription {
            span: d.span,
            children: description,
        }));
        let mut attrs = BTreeMap::new();
        if !matches!(kind, "describe" | "object") {
            attrs.insert("domain".to_owned(), json!("std"));
        }
        attrs.insert("objtype".to_owned(), json!(kind));
        vec![Node::Custom(Custom {
            span: d.span,
            kind: "desc".into(),
            attrs,
            children: vec![Node::DefinitionList(DefinitionList {
                span: d.span,
                children: list,
            })],
        })]
    }

    /// The nodes of an `option` signature (`-o FILE, --output=FILE`),
    /// pushed to `term`; returns the option names.
    fn option_signature(
        &mut self,
        signature: &str,
        span: Option<Span>,
        term: &mut Vec<Node>,
    ) -> Vec<String> {
        let mut names = Vec::new();
        for part in signature.split(", ") {
            let part = part.trim();
            let Some((option, args)) = split_option(part) else {
                self.warn(
                    span,
                    format!(
                        "Malformed option description {part:?}, should look like \"opt\", \
                         \"-opt args\", \"--opt args\", \"/opt args\" or \"+opt args\""
                    ),
                );
                continue;
            };
            if !names.is_empty() {
                term.push(plain(", "));
            }
            term.push(name(&option));
            if !args.is_empty() {
                term.push(self.option_args(&args));
            }
            names.push(option);
        }
        if names.is_empty() {
            // As Sphinx: the signature as written, with no target.
            term.clear();
            term.push(name(signature));
        }
        names
    }

    /// The ids of the options `names` (one signature), noted for
    /// `:option:` under the current program, with their index entries.
    fn option_targets(
        &mut self,
        names: &[String],
        entries: bool,
        span: Option<Span>,
    ) -> Vec<String> {
        let program = self.program.clone();
        let ids: Vec<String> = names
            .iter()
            .map(|option| {
                let mut prefix = vec!["cmdoption"];
                if let Some(program) = &program {
                    prefix.push(program);
                }
                if !option.starts_with(['-', '/']) {
                    prefix.push("arg");
                }
                self.make_id(&prefix.join("-"), option)
            })
            .collect();
        let Some(first) = ids.first().cloned() else {
            return ids;
        };
        for option in names {
            let key = option_key(program.as_deref(), option);
            self.notes
                .options
                .entry(key)
                .or_insert_with(|| first.clone());
        }
        if entries {
            let description = match &program {
                Some(program) => format!("{program} command line option"),
                None => "command line option".to_owned(),
            };
            let index = names
                .iter()
                .map(|option| ("pair".to_owned(), format!("{description}; {option}"), false))
                .collect();
            self.add_index_entries(index, &first, span);
        }
        ids
    }

    /// `productionlist`: grammar productions (`name: tokens`, a line
    /// starting with `:` continuing the one before), each name a target
    /// for `:token:` and each `` `token` `` in them a reference. An
    /// optional first line without `:` names their group.
    pub(super) fn productionlist(&mut self, d: &Parts) -> Vec<Node> {
        // The productions are the argument (reStructuredText, as Sphinx)
        // or the content (where a parser kept them apart).
        let mut text = d.argument.clone();
        if let Some(content) = &d.content {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(content);
        }
        let text = text.replace("\\\n", "");
        let mut lines: Vec<&str> = text.lines().collect();
        let mut group = String::new();
        if lines.first().is_some_and(|l| !l.contains(':')) {
            group = lines.remove(0).trim().to_owned();
        }
        let productions: Vec<(&str, &str)> = lines
            .iter()
            .take_while(|l| l.contains(':'))
            .filter_map(|l| l.split_once(':'))
            .map(|(name, tokens)| (name.trim(), tokens.trim()))
            .collect();
        if productions.is_empty() {
            self.warn(d.span, "productionlist: no productions (`name: tokens`)");
            return Vec::new();
        }
        let width = productions.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        let mut line = Vec::new();
        for (i, (production, tokens)) in productions.into_iter().enumerate() {
            if i > 0 {
                line.push(plain("\n"));
            }
            if production.is_empty() {
                line.push(plain(&" ".repeat(width + 5)));
            } else {
                let id = self.make_id(&format!("grammar-token-{group}"), production);
                let key = if group.is_empty() {
                    production.to_owned()
                } else {
                    format!("{group}:{production}")
                };
                self.notes.tokens.entry(key).or_insert_with(|| id.clone());
                line.push(anchor(&id));
                line.push(Node::Strong(Strong {
                    span: None,
                    children: vec![plain(production)],
                }));
                line.push(plain(&format!(
                    "{:>w$}",
                    " ::= ",
                    w = width - production.len() + 5
                )));
            }
            line.extend(token_nodes(tokens, &group));
        }
        // Drawn like a code block (`pre` and `code`), spaces and newlines
        // kept, so that the `::=` line up.
        let mut code = Custom {
            span: d.span,
            kind: "productions".into(),
            attrs: BTreeMap::new(),
            children: line,
        };
        code.attrs.insert("inline".to_owned(), json!(true));
        code.attrs.insert("html_tag".to_owned(), json!("code"));
        let mut node = Custom {
            span: d.span,
            kind: "productionlist".into(),
            attrs: BTreeMap::new(),
            children: vec![Node::Custom(code)],
        };
        node.attrs.insert("html_tag".to_owned(), json!("pre"));
        if !group.is_empty() {
            node.attrs.insert("group".to_owned(), json!(group));
        }
        vec![Node::Custom(node)]
    }
}

/// The key of an option in the collected `options`: the program (empty
/// for none) and the option, apart by a space.
pub(crate) fn option_key(program: Option<&str>, option: &str) -> String {
    format!("{} {option}", program.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_as_sphinx_makes_them() {
        assert_eq!(sphinx_id("cmdoption-prog--output"), "cmdoption-prog-output");
        assert_eq!(sphinx_id("envvar-PATH_VAR.x"), "envvar-PATH_VAR.x");
        assert_eq!(
            sphinx_id("grammar-token--try_stmt"),
            "grammar-token-try_stmt"
        );
        assert_eq!(sphinx_id("confval-a b"), "confval-a-b");
        assert_eq!(sphinx_id("--x-"), "x");
    }

    #[test]
    fn options_and_tokens() {
        assert_eq!(
            split_option("--output=FILE"),
            Some(("--output".into(), "=FILE".into()))
        );
        assert_eq!(split_option("-o FILE"), Some(("-o".into(), " FILE".into())));
        assert_eq!(split_option("-j[=N]"), Some(("-j".into(), "[=N]".into())));
        assert_eq!(split_option("=x"), None);
        assert!(is_token("expr"));
        assert!(is_token("~py-grammar:expr"));
        assert!(!is_token("not a token"));
    }
}
