//! First-party plugin for the directives and roles Sphinx adds to
//! docutils.
//!
//! It is a transformer that declares its directives and roles as syntax
//! extensions (see "Syntax extensions" in `spec/wire.md`), so they work in
//! every parser that supports them: in reStructuredText as Sphinx writes
//! them (`.. versionadded:: 2.0`, `` :ref:`label` ``), and in Markdown in
//! MyST's syntax (```` ```{versionadded} 2.0 ````, `` {ref}`label` ``).
//! The parsers turn them into `custom` nodes; this plugin turns those into
//! core nodes (and a few custom ones renderers style), and resolves
//! references across the site.
//!
//! List it before `transform-links`, `transform-toc` and
//! `transform-highlight`, which see what it makes: the links of `ref` and
//! `doc`, and the code blocks of `code-block` and `literalinclude`.
//!
//! Directives:
//!
//! | Directive | Result |
//! |---|---|
//! | `code-block`, `sourcecode` | A code block; `:caption:` makes it a figure, `:name:` gives it an id. `:linenos:`, `:lineno-start:` and `:emphasize-lines:` go to the code block's `info`. |
//! | `literalinclude` | A code block from a file (`:lines:`, `:start-after:`, `:start-at:`, `:end-before:`, `:end-at:`, `:lineno-match:`, `:dedent:`, `:prepend:`, `:append:`, and those of `code-block`). The language defaults to the file's extension. |
//! | `highlight` | Sets the language of later code blocks that have none. |
//! | `versionadded`, `versionchanged`, `deprecated`, `versionremoved` | An admonition titled "Added in version 2.0" and the like. |
//! | `seealso` | A `seealso` admonition. |
//! | `todo`, `todolist` | A `todo` admonition, and the list of every todo, if `todo_include_todos` is on; nothing otherwise. |
//! | `only` | Its content if its tag expression is true (`html and not draft`); tags are `html`, `format_html`, `builder_html` and those of the `tags` option. |
//! | `centered` | A `custom:centered` node with a bold paragraph. |
//! | `hlist` | A `custom:hlist` node whose children are the columns: the bullet list in its content, split. |
//! | `glossary` | A `custom:glossary` node around the definition list in its content; each term gets an id (`term-<term>`) for `term` references. `:sorted:` sorts it. |
//! | `toctree` | A `custom:toctree` node with a nested list of links to the documents it names (their titles, and their sections to `:maxdepth:`), unless `:hidden:`. `:glob:`, `:caption:`, `:titlesonly:` and `:reversed:` work as in Sphinx. The pages a page's toctrees name (hidden ones too) become its `nav_children`, which `transform-nav` follows; `:numbered:` numbers their sections (`1.2. Title`), with the pages each one's toctrees name after its sections. |
//! | `index` | An anchor, and index entries (`single`, `pair`, `triple`, `see`, `seealso`; `!` for main entries) for the generated index page, `genindex.md`. |
//! | `rubric` | A `custom:rubric` paragraph, or heading with `:heading-level:` (as `html_tag`). |
//! | `math` | Display math; with `:label:` (or the `math_number_all` option) it is numbered (`\tag{n}`) and gets the id `equation-<label>` for `eq`. |
//! | `sectionauthor`, `codeauthor` | A paragraph naming the author if `show_authors` is on; nothing otherwise. |
//! | `acks` | A `custom:acks` node with its content. |
//! | `tabularcolumns` | Nothing (it is for LaTeX). |
//! | `program` | Names the program the `option`s after it belong to (`None` for none). |
//! | `option`, `cmdoption` | A `custom:desc` node (`domain: std`, `objtype`) around a definition list: each signature (one per line, `-o FILE, --output=FILE`) a term, with an id per option (`cmdoption-<program>-<option>`) for `option` references and index entries, and the content its description. |
//! | `envvar`, `confval` | Likewise, with the ids `envvar-<name>` and `confval-<name>`; `confval`'s `:type:` and `:default:` come first in its description. |
//! | `describe`, `object` | Likewise, with no id or index entry. |
//! | `productionlist` | A `custom:productionlist` node with a paragraph of grammar productions (`name: tokens`), one per line, each name with an id (`grammar-token-<group>-<name>`) for `token` references, and each `` `token` `` in them a reference. A first line without `:` names their group. |
//!
//! `option`, `envvar` and `confval` take `:no-index:` (no ids),
//! `:no-index-entry:`, `:no-contents-entry:` and `:no-typesetting:` (the
//! ids alone). The directives and roles of Sphinx's standard domain can be
//! written with its prefix too (`std:option`, `` :std:ref:`label` ``).
//!
//! Roles:
//!
//! | Role | Result |
//! |---|---|
//! | `ref` | A link to a label anywhere in the site (`` :ref:`text <label>` ``). Without text, `transform-links` fills in the title of what it points to (a figure's caption, for figures). |
//! | `doc` | A link to another document, by its path without extension, relative to this one or (with a leading `/`) to the source directory. |
//! | `numref` | A link to a numbered figure, table or code block, as "Fig. 2" (`numfig_format`), or with the text given (`%s` or `{number}` for the number, `{name}` for the caption). Needs `numfig`. |
//! | `term` | A link to a glossary term. |
//! | `eq` | A link to a numbered equation, as "(2)". |
//! | `any` | A label, else a term, else a document. |
//! | `download` | A link to a file in the source directory. |
//! | `abbr` | `` :abbr:`LIFO (last-in, first-out)` ``: a `custom:abbr` inline with a `title`. |
//! | `kbd`, `guilabel`, `menuselection` | `custom` inlines of their name; `&` marks an accelerator key, `-->` separates menu items. |
//! | `file`, `samp` | Inline code, or a `custom` inline with the `{variable}` parts emphasized. |
//! | `command`, `program` | Strong text. |
//! | `dfn`, `mailheader`, `mimetype`, `newsgroup`, `manpage` | Emphasized text; `manpage` links to `manpages_url` if set. |
//! | `makevar`, `regexp` | Inline code. |
//! | `index` | The text, with an anchor and an index entry. |
//! | `option` | A link to a command line option, around its code: `-o` of the current `program`, or `prog -o`, `prog sub -o` (`-o=FILE` finds `-o`). |
//! | `envvar` | A link to an environment variable, around its code, with an index entry. |
//! | `confval`, `token` | A link to a configuration value or grammar token (`group:name`; `~group:name` shows `name`), around its code. |
//! | `keyword` | A link to a label, around its code. |
//! | the `extlinks` names | Links built from a URL pattern, as `sphinx.ext.extlinks` does. |
//!
//! `custom` inlines have the attribute `inline: true` (and `html_tag`,
//! `title` where they apply), which `render-html` draws as inline
//! elements.
//!
//! Options:
//!
//! | Option | Default | Meaning |
//! |---|---|---|
//! | `file_access` | `true` | Let `literalinclude` read files (the files read are the results' dependencies, for the cache). |
//! | `tags` | `[]` | Tags `only` expressions may test. |
//! | `todo_include_todos` | `false` | Show `todo` and `todolist`. |
//! | `numfig` | `false` | Number figures, tables and code blocks with a caption, and let `numref` refer to them. |
//! | `numfig_format` | Sphinx's | Formats by kind: `figure` (`"Fig. %s"`), `table`, `code-block`. |
//! | `math_number_all` | `false` | Number every `math` directive, labeled or not. |
//! | `highlight_language` | none | Language of code blocks with none (until a `highlight` directive). |
//! | `show_authors` | `false` | Show `sectionauthor` and `codeauthor`. |
//! | `manpages_url` | none | URL pattern for `manpage` (`{page}`, `{section}`, `{path}`). |
//! | `extlinks` | `{}` | Roles making links: `{ issue = { url = "https://example.com/issues/%s", caption = "#%s" } }`. |
//! | `toctree_nav` | `true` | Set `nav_children` from toctrees. |
//! | `option_emphasise_placeholders` | `false` | Emphasize the `{placeholders}` of `option` signatures (`--output={FILE}`), as `samp` does, without their braces. |
//!
//! If any page has index entries, the plugin generates the index page,
//! `genindex.md` (`genindex.html`): entries by letter, with links to where
//! they are. `` :ref:`genindex` ``, `` :doc:`genindex` `` and a `genindex`
//! toctree entry link to it.
//!
//! Sphinx's standard domain is here, as Sphinx adds it; other domains
//! (`py:function` and the like) belong to the API documentation framework.

mod directives;
mod roles;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use lsdocs_plugin_sdk::doctree::{Document, Span};
use lsdocs_plugin_sdk::protocol::{
    ArgumentSyntax, ContentSyntax, Diagnostic, DirectiveSyntax, OptionSyntax,
    ParseTransformAllResult, ParseTransformEachParams, ParseTransformEachResult,
    RenderTransformEachParams, RenderTransformEachResult, RoleSyntax, Severity, Syntax,
};
use lsdocs_plugin_sdk::{Context, Plugin, PluginError, PluginResult, Setup, Transformer};
use serde::Deserialize;
use serde_json::Value;

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    Plugin::new("sphinx-addons", env!("CARGO_PKG_VERSION"))
        .settings(lsdocs_plugin_sdk::config::settings_toml_as::<Options>)
        .transformer(SphinxAddons)
        .syntax(|setup: &Setup| syntax(&Options::read(&setup.settings).unwrap_or_default()))
        .cacheable()
}

/// The transformer.
#[derive(Debug, Clone, Copy, Default)]
pub struct SphinxAddons;

/// The plugin's options.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Options {
    pub file_access: bool,
    pub tags: Vec<String>,
    pub todo_include_todos: bool,
    pub numfig: bool,
    pub numfig_format: BTreeMap<String, String>,
    pub math_number_all: bool,
    pub highlight_language: Option<String>,
    pub show_authors: bool,
    pub manpages_url: Option<String>,
    pub extlinks: BTreeMap<String, ExtLink>,
    pub toctree_nav: bool,
    pub option_emphasise_placeholders: bool,
}

/// An `extlinks` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtLink {
    /// The URL, with `%s` for the role's text.
    pub url: String,
    /// The link text, with `%s` for the role's text; the URL if none.
    #[serde(default)]
    pub caption: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            file_access: true,
            tags: Vec::new(),
            todo_include_todos: false,
            numfig: false,
            numfig_format: BTreeMap::new(),
            math_number_all: false,
            highlight_language: None,
            show_authors: false,
            manpages_url: None,
            extlinks: BTreeMap::new(),
            toctree_nav: true,
            option_emphasise_placeholders: false,
        }
    }
}

impl Options {
    /// The options given in the site config (null for none).
    pub fn read(value: &Value) -> Result<Self, PluginError> {
        if value.is_null() {
            return Ok(Self::default());
        }
        Self::deserialize(value).map_err(|e| PluginError::new(format!("invalid options: {e}")))
    }

    /// The `numref` format for `kind` (`figure`, `table`, `code-block`).
    pub fn numfig_format(&self, kind: &str) -> String {
        if let Some(format) = self.numfig_format.get(kind) {
            return format.clone();
        }
        match kind {
            "table" => "Table %s",
            "code-block" => "Listing %s",
            "section" => "Section %s",
            _ => "Fig. %s",
        }
        .to_owned()
    }
}

/// A directive declaration.
fn directive(name: &str) -> DirectiveSyntax {
    DirectiveSyntax {
        name: name.to_owned(),
        ..DirectiveSyntax::default()
    }
}

fn option_kinds<const N: usize>(list: [(&str, OptionSyntax); N]) -> BTreeMap<String, OptionSyntax> {
    option_kinds_of(list)
}

fn option_kinds_of<'n>(
    list: impl IntoIterator<Item = (&'n str, OptionSyntax)>,
) -> BTreeMap<String, OptionSyntax> {
    list.into_iter().map(|(n, k)| (n.to_owned(), k)).collect()
}

/// Options `code-block` and `literalinclude` share.
fn code_options() -> Vec<(&'static str, OptionSyntax)> {
    use OptionSyntax::{Flag, Required, Value};
    vec![
        ("linenos", Flag),
        ("lineno-start", Required),
        ("emphasize-lines", Required),
        ("caption", Value),
        ("name", Required),
        ("class", Value),
        ("dedent", Value),
        ("force", Flag),
    ]
}

/// The directives and roles the plugin handles.
pub(crate) fn syntax(options: &Options) -> Syntax {
    use OptionSyntax::{Flag, Required, Value};
    let blocks = ContentSyntax::Blocks;
    let mut directives = Vec::new();
    for name in ["code-block", "sourcecode"] {
        directives.push(DirectiveSyntax {
            optional_arguments: 1,
            options: code_options()
                .into_iter()
                .map(|(n, k)| (n.to_owned(), k))
                .collect(),
            content: ContentSyntax::Text,
            ..directive(name)
        });
    }
    let mut include = code_options();
    include.extend([
        ("language", Required),
        ("lines", Required),
        ("start-after", Required),
        ("start-at", Required),
        ("end-before", Required),
        ("end-at", Required),
        ("lineno-match", Flag),
        ("prepend", Required),
        ("append", Required),
        ("encoding", Required),
        ("tab-width", Required),
        ("pyobject", Required),
        ("diff", Required),
    ]);
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        final_argument_whitespace: true,
        options: include
            .into_iter()
            .map(|(n, k)| (n.to_owned(), k))
            .collect(),
        ..directive("literalinclude")
    });
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        options: option_kinds([("linenothreshold", Value), ("force", Flag)]),
        ..directive("highlight")
    });
    for name in [
        "versionadded",
        "versionchanged",
        "deprecated",
        "versionremoved",
    ] {
        directives.push(DirectiveSyntax {
            required_arguments: 1,
            optional_arguments: 1,
            final_argument_whitespace: true,
            argument: ArgumentSyntax::Inline,
            content: blocks,
            ..directive(name)
        });
    }
    directives.push(DirectiveSyntax {
        optional_arguments: 1,
        final_argument_whitespace: true,
        argument: ArgumentSyntax::Inline,
        content: blocks,
        ..directive("seealso")
    });
    directives.push(DirectiveSyntax {
        options: option_kinds([("class", Value), ("name", Required)]),
        content: blocks,
        ..directive("todo")
    });
    directives.push(directive("todolist"));
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        final_argument_whitespace: true,
        content: blocks,
        ..directive("only")
    });
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        final_argument_whitespace: true,
        argument: ArgumentSyntax::Inline,
        ..directive("centered")
    });
    directives.push(DirectiveSyntax {
        options: option_kinds([("columns", Required)]),
        content: blocks,
        ..directive("hlist")
    });
    directives.push(DirectiveSyntax {
        options: option_kinds([("sorted", Flag)]),
        content: blocks,
        ..directive("glossary")
    });
    directives.push(DirectiveSyntax {
        options: option_kinds([
            ("maxdepth", Required),
            ("caption", Required),
            ("name", Required),
            ("class", Value),
            ("hidden", Flag),
            ("glob", Flag),
            ("titlesonly", Flag),
            ("numbered", Value),
            ("reversed", Flag),
            ("includehidden", Flag),
        ]),
        content: ContentSyntax::Text,
        ..directive("toctree")
    });
    directives.push(DirectiveSyntax {
        optional_arguments: 1,
        final_argument_whitespace: true,
        options: option_kinds([("name", Required)]),
        content: ContentSyntax::Text,
        ..directive("index")
    });
    directives.push(DirectiveSyntax {
        optional_arguments: 1,
        final_argument_whitespace: true,
        options: option_kinds([
            ("label", Required),
            ("name", Required),
            ("class", Value),
            ("nowrap", Flag),
        ]),
        content: ContentSyntax::Text,
        ..directive("math")
    });
    for name in ["sectionauthor", "codeauthor"] {
        directives.push(DirectiveSyntax {
            required_arguments: 1,
            final_argument_whitespace: true,
            argument: ArgumentSyntax::Inline,
            ..directive(name)
        });
    }
    directives.push(DirectiveSyntax {
        content: blocks,
        ..directive("acks")
    });
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        final_argument_whitespace: true,
        argument: ArgumentSyntax::Inline,
        options: option_kinds([
            ("class", Value),
            ("name", Required),
            ("heading-level", Required),
        ]),
        ..directive("rubric")
    });
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        final_argument_whitespace: true,
        ..directive("tabularcolumns")
    });
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        final_argument_whitespace: true,
        ..directive("program")
    });
    let object_flags = [
        ("no-index", Flag),
        ("noindex", Flag),
        ("no-index-entry", Flag),
        ("noindexentry", Flag),
        ("no-contents-entry", Flag),
        ("no-typesetting", Flag),
    ];
    for name in ["option", "cmdoption", "envvar", "describe", "object"] {
        directives.push(DirectiveSyntax {
            required_arguments: 1,
            final_argument_whitespace: true,
            options: option_kinds(object_flags),
            content: blocks,
            ..directive(name)
        });
    }
    let mut confval = object_flags.to_vec();
    confval.extend([("type", Required), ("default", Required)]);
    directives.push(DirectiveSyntax {
        required_arguments: 1,
        final_argument_whitespace: true,
        options: option_kinds_of(confval),
        content: blocks,
        ..directive("confval")
    });
    // The productions are its argument in reStructuredText (as Sphinx),
    // its content in Markdown.
    directives.push(DirectiveSyntax {
        optional_arguments: 1,
        final_argument_whitespace: true,
        content: ContentSyntax::Text,
        ..directive("productionlist")
    });
    // The standard domain's names, with its prefix.
    let prefixed: Vec<DirectiveSyntax> = directives
        .iter()
        .filter(|d| directives::std_domain::DOMAIN_DIRECTIVES.contains(&d.name.as_str()))
        .map(|d| DirectiveSyntax {
            name: format!("std:{}", d.name),
            ..d.clone()
        })
        .collect();
    directives.extend(prefixed);
    let mut roles: Vec<RoleSyntax> = roles::NAMES
        .iter()
        .map(|name| RoleSyntax {
            name: (*name).to_owned(),
        })
        .collect();
    for name in directives::std_domain::DOMAIN_ROLES {
        roles.push(RoleSyntax {
            name: format!("std:{name}"),
        });
    }
    for name in options.extlinks.keys() {
        roles.push(RoleSyntax {
            name: name.to_lowercase(),
        });
    }
    Syntax { directives, roles }
}

/// A warning about a directive or role.
pub(crate) fn warning(span: Option<Span>, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        message: message.into(),
        span,
        code: Some("sphinx".into()),
        ..Diagnostic::default()
    }
}

/// Expands the directives of a document.
fn expand(
    document: &mut Document,
    options: &Options,
    ctx: &Context,
) -> (directives::Notes, Vec<Diagnostic>) {
    let mut cx = directives::Expand::new(options, &document.source, ctx.site());
    cx.document(document);
    (cx.notes, cx.diagnostics)
}

impl Transformer for SphinxAddons {
    const READS: bool = true;

    async fn read(
        &self,
        params: ParseTransformEachParams,
        ctx: &Context,
    ) -> PluginResult<ParseTransformEachResult> {
        let options = Options::read(ctx.settings())?;
        let mut document = params.document;
        directives::std_domain::parse_options(&mut document, ctx).await;
        let (notes, _) = expand(&mut document, &options, ctx);
        Ok(ParseTransformEachResult {
            data: roles::collect(&document, &notes, &options),
            diagnostics: Vec::new(),
            dependencies: notes.files,
        })
    }

    async fn conclude(
        &self,
        collected: &Value,
        _ctx: &Context,
    ) -> PluginResult<ParseTransformAllResult> {
        Ok(ParseTransformAllResult {
            documents: roles::genindex(collected).into_iter().collect(),
            diagnostics: Vec::new(),
            dependencies: Vec::new(),
        })
    }

    async fn apply(
        &self,
        params: RenderTransformEachParams,
        ctx: &Context,
    ) -> PluginResult<RenderTransformEachResult> {
        let options = Options::read(ctx.settings())?;
        let mut document = params.document;
        directives::std_domain::parse_options(&mut document, ctx).await;
        let (notes, mut diagnostics) = expand(&mut document, &options, ctx);
        let site = roles::Site::new(&params.collected, &ctx.site().files, &options);
        // The pages toctrees name are this page's children in the
        // navigation (`transform-nav`), unless the page says otherwise.
        let named = site.toctree_documents(&document.source);
        if options.toctree_nav && !named.is_empty() {
            let children: Vec<Value> = named
                .iter()
                .map(|source| Value::String(format!("/{source}")))
                .collect();
            document
                .metadata
                .entry("nav_children".to_owned())
                .or_insert(Value::Array(children));
        }
        site.resolve(&mut document, &mut diagnostics);
        Ok(RenderTransformEachResult {
            document,
            diagnostics,
            dependencies: notes.files,
        })
    }
}
