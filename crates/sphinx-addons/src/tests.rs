#![allow(clippy::unwrap_used, clippy::panic)]

use lsdocs_plugin_sdk::doctree::{Document, Node, plain_text};
use lsdocs_plugin_sdk::protocol::{Diagnostic, InitializeParams, SiteInfo};
use lsdocs_plugin_sdk::testing::{context_with_parsers, run_transform_in};
use serde_json::{Value, json};

use super::{Options, plugin, syntax};

/// Parses `files` (by extension, with the plugin's syntax) and runs the
/// plugin over them.
async fn build(files: &[(&str, &str)], options: Value) -> Vec<(Document, Vec<Diagnostic>)> {
    build_in(files, options, None).await
}

async fn build_in(
    files: &[(&str, &str)],
    options: Value,
    source_dir: Option<&str>,
) -> Vec<(Document, Vec<Diagnostic>)> {
    build_site(files, &[], options, source_dir).await
}

/// Like `build_in`, with `assets` among the site's files too: files no
/// parser takes, whatever their extension. `.myst` files are parsed as
/// MyST, standing for a parser with an extension of its own.
async fn build_site(
    files: &[(&str, &str)],
    assets: &[&str],
    options: Value,
    source_dir: Option<&str>,
) -> Vec<(Document, Vec<Diagnostic>)> {
    let syntax = syntax(&Options::read(&options).unwrap());
    let mut documents = Vec::new();
    let mut names: Vec<String> = assets.iter().map(|a| (*a).to_owned()).collect();
    for (source, text) in files {
        names.push((*source).to_owned());
        let (doc, diagnostics) = if source.ends_with(".rst") {
            lsdocs_parser_rst::parse(source, text, None, &syntax)
        } else if source.ends_with(".md") || source.ends_with(".myst") {
            lsdocs_parser_markdown::myst::parse_with_syntax(source, text, &syntax)
        } else {
            continue;
        };
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        documents.push(doc);
    }
    names.sort();
    let init = lsdocs_plugin_sdk::Setup::with_settings(
        InitializeParams {
            site: SiteInfo {
                files: names,
                source_dir: source_dir.map(str::to_owned),
                ..SiteInfo::default()
            },
            ..InitializeParams::default()
        },
        options,
    );
    // `host/parse` goes to the parsers, as in a site.
    let parsers = vec![
        lsdocs_parser_rst::plugin(),
        lsdocs_parser_markdown::myst::plugin(),
    ];
    let ctx = context_with_parsers(&plugin(), &init, parsers);
    run_transform_in(&plugin(), &ctx, documents).await.unwrap()
}

/// `value` without spans, for comparing trees from different sources.
fn without_spans(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            map.remove("span");
            for v in map.values_mut() {
                *v = without_spans(v.take());
            }
        }
        Value::Array(items) => {
            for v in items {
                *v = without_spans(v.take());
            }
        }
        _ => {}
    }
    value
}

fn tree(doc: &Document) -> Value {
    without_spans(serde_json::to_value(&doc.children).unwrap())
}

fn messages(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.message.as_str()).collect()
}

/// The same page in reStructuredText and in Markdown gives the same tree.
async fn same(rst: &str, md: &str, options: Value) -> Value {
    let out = build(&[("a.rst", rst), ("a.md", md)], options).await;
    let (r, rd) = &out[0];
    let (m, md) = &out[1];
    assert!(rd.is_empty(), "{rd:?}");
    assert!(md.is_empty(), "{md:?}");
    assert_eq!(tree(r), tree(m));
    tree(r)
}

#[tokio::test]
async fn version_notes_and_see_also() {
    let t = same(
        ".. versionadded:: 2.0 The *new* thing.\n\n\
         .. deprecated:: 3.1\n\n   Use other.\n\n\
         .. seealso:: Module *zipfile*\n",
        "```{versionadded} 2.0 The *new* thing.\n```\n\n\
         ```{deprecated} 3.1\nUse other.\n```\n\n\
         ~~~{seealso} Module *zipfile*\n~~~\n",
        Value::Null,
    )
    .await;
    assert_eq!(t[0]["type"], "admonition");
    assert_eq!(t[0]["kind"], "versionadded");
    assert_eq!(t[0]["title"], "Added in version 2.0");
    assert_eq!(t[0]["children"][0]["children"][0]["value"], "The ");
    assert_eq!(t[0]["children"][0]["children"][1]["type"], "emphasis");
    assert_eq!(t[1]["title"], "Deprecated since version 3.1");
    assert_eq!(t[1]["children"][0]["children"][0]["value"], "Use other.");
    assert_eq!(t[2]["kind"], "seealso");
    assert_eq!(t[2]["title"], "See also");
}

#[tokio::test]
async fn code_blocks_and_highlight() {
    let t = same(
        ".. code-block:: python\n   :caption: Example\n   :name: ex\n   :emphasize-lines: 2\n   :linenos:\n\n   a = 1\n   b = 2\n\n\
         .. highlight:: rust\n\n\
         .. code-block::\n\n   fn main() {}\n",
        "```{code-block} python\n:caption: Example\n:name: ex\n:emphasize-lines: 2\n:linenos:\n\na = 1\nb = 2\n```\n\n\
         ```{highlight} rust\n```\n\n\
         ```{code-block}\nfn main() {}\n```\n",
        Value::Null,
    )
    .await;
    assert_eq!(
        t[0],
        json!({"type": "figure", "id": "ex", "children": [
            {"type": "code_block", "language": "python", "info": "linenos emphasize-lines=2",
             "value": "a = 1\nb = 2"},
            {"type": "caption", "children": [{"type": "text", "value": "Example"}]},
        ]})
    );
    assert_eq!(
        t[1],
        json!({"type": "code_block", "language": "rust", "value": "fn main() {}"})
    );
    // A literal block after `highlight` takes its language.
    let out = build(
        &[("a.rst", ".. highlight:: c\n\nCode::\n\n   int x;\n")],
        Value::Null,
    )
    .await;
    assert_eq!(tree(&out[0].0)[1]["language"], "c");
}

#[tokio::test]
async fn literalinclude_reads_files() {
    let dir = std::env::temp_dir().join(format!("lsdocs-sphinx-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("guide")).unwrap();
    std::fs::write(
        dir.join("guide/code.py"),
        "# start\ndef f():\n    return 1\n# end\nrest\n",
    )
    .unwrap();
    let rst = ".. literalinclude:: code.py\n   :start-after: # start\n   :end-before: # end\n   :lineno-match:\n   :dedent: 4\n\n\
               .. literalinclude:: /guide/code.py\n   :lines: 1, 3-\n   :language: text\n\n\
               .. literalinclude:: missing.py\n";
    let out = build_in(
        &[("guide/a.rst", rst)],
        Value::Null,
        Some(dir.to_str().unwrap()),
    )
    .await;
    std::fs::remove_dir_all(&dir).ok();
    let (doc, diagnostics) = &out[0];
    let t = tree(doc);
    assert_eq!(
        t[0],
        json!({"type": "code_block", "language": "py", "info": "linenos lineno-start=2",
               "value": "f():\nreturn 1"})
    );
    assert_eq!(t[1]["value"], "# start\n    return 1\n# end\nrest");
    assert_eq!(t[1]["language"], "text");
    assert_eq!(
        messages(diagnostics),
        [
            "literalinclude: cannot find \"missing.py\"",
            "non-whitespace stripped by dedent"
        ]
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn todos_only_and_authors() {
    let options = json!({"todo_include_todos": true, "tags": ["draft"], "show_authors": true});
    let t = same(
        ".. todo:: Write this.\n\n\
         .. only:: html and draft\n\n   Shown.\n\n\
         .. only:: not html\n\n   Hidden.\n\n\
         .. sectionauthor:: Ann *Author*\n",
        "```{todo}\nWrite this.\n```\n\n\
         ```{only} html and draft\nShown.\n```\n\n\
         ```{only} not html\nHidden.\n```\n\n\
         ```{sectionauthor} Ann *Author*\n```\n",
        options.clone(),
    )
    .await;
    assert_eq!(t[0], json!({"type": "anchor", "id": "todo-1"}));
    assert_eq!(t[1]["kind"], "todo");
    assert_eq!(t[2]["children"][0]["value"], "Shown.");
    assert_eq!(t.as_array().unwrap().len(), 4);
    let author: Document =
        serde_json::from_value(json!({"source": "x", "children": [t[3]]})).unwrap();
    assert_eq!(plain_text(&author.children), "Section author: Ann Author");
    // The todo list has the todos of every document, with links to them.
    let out = build(
        &[
            ("a.rst", ".. todo:: First.\n"),
            ("b.md", "```{todo}\nSecond.\n```\n\n```{todolist}\n```\n"),
        ],
        options,
    )
    .await;
    let t = tree(&out[1].0);
    let listed: Vec<_> = t.as_array().unwrap()[2..]
        .iter()
        .map(|n| n["kind"].clone())
        .collect();
    assert_eq!(listed, [json!("todo"), json!("todo")]);
    assert_eq!(t[2]["children"][0]["children"][0]["value"], "First.");
    assert_eq!(t[2]["children"][1]["children"][1]["url"], "a.rst#todo-1");
    assert_eq!(t[3]["children"][1]["children"][1]["url"], "#todo-1");
    // Off by default.
    let out = build(
        &[(
            "a.rst",
            ".. todo:: x\n\n.. todolist::\n\n.. sectionauthor:: me\n",
        )],
        Value::Null,
    )
    .await;
    assert_eq!(out[0].0.children, []);
}

#[tokio::test]
async fn glossary_terms_and_references() {
    let options = json!({"numfig": true});
    let out = build(
        &[
            (
                "terms.rst",
                ".. glossary::\n   :sorted:\n\n   Zebra\n      An animal.\n\n   Apple\n      A fruit.\n\n\
                 .. _fig-one:\n\n.. figure:: pic.png\n\n   The caption.\n\n\
                 .. math:: e^{i\\pi} = -1\n   :label: euler\n",
            ),
            (
                "use.md",
                "# Use\n\nSee {term}`apple`, {term}`the animal <Zebra>`, {numref}`fig-one`, \
                 {numref}`Figure {number}: {name} <fig-one>`, {eq}`euler`, {ref}`fig-one`, \
                 {ref}`text <fig-one>`, {doc}`terms` and {any}`apple`.\n",
            ),
        ],
        options,
    )
    .await;
    let (terms, d) = &out[0];
    assert!(d.is_empty(), "{d:?}");
    let t = tree(terms);
    assert_eq!(t[0]["kind"], "glossary");
    let list = &t[0]["children"][0]["children"];
    assert_eq!(
        list[0]["children"][0],
        json!({"type": "anchor", "id": "term-Apple"})
    );
    assert_eq!(list[2]["children"][0]["id"], "term-Zebra");
    // numfig numbers the caption.
    let caption = &t[1]["children"][1]["children"];
    assert_eq!(caption[0]["value"], "Fig. 1 ");
    assert_eq!(t[2], json!({"type": "anchor", "id": "equation-euler"}));
    assert_eq!(t[3]["value"], "e^{i\\pi} = -1 \\tag{1}");

    let (doc, d) = &out[1];
    assert!(d.is_empty(), "{d:?}");
    let links: Vec<_> = doc
        .descendants()
        .filter_map(|n| match n {
            Node::Link(l) => Some(json!([l.url, l.target, plain_text(&l.children)])),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        [
            json!(["terms.rst#term-Apple", {"document": "terms.rst", "anchor": "term-Apple"}, "apple"]),
            json!(["terms.rst#term-Zebra", {"document": "terms.rst", "anchor": "term-Zebra"}, "the animal"]),
            json!(["terms.rst#fig-one", {"document": "terms.rst", "anchor": "fig-one"}, "Fig. 1"]),
            json!(["terms.rst#fig-one", {"document": "terms.rst", "anchor": "fig-one"}, "Figure 1: The caption."]),
            json!(["terms.rst#equation-euler", {"document": "terms.rst", "anchor": "equation-euler"}, "(1)"]),
            json!(["#fig-one", {"label": "fig-one"}, "The caption."]),
            json!(["#fig-one", {"label": "fig-one"}, "text"]),
            json!(["terms.rst", {"document": "terms.rst"}, ""]),
            json!(["terms.rst#term-Apple", {"document": "terms.rst", "anchor": "term-Apple"}, "apple"]),
        ]
    );
    // Problems are warnings, and the text stays.
    let out = build(
        &[("a.md", "{term}`nope` {doc}`nope` {numref}`x`\n")],
        Value::Null,
    )
    .await;
    assert_eq!(
        messages(&out[0].1),
        [
            "term not in glossary: \"nope\"",
            "unknown document: \"nope\"",
            "numfig is disabled. :numref: is ignored."
        ]
    );
    assert_eq!(plain_text(&out[0].0.children), "nope nope x");
}

/// `kbd`: a key per part, as Sphinx splits them: on `-`, `+`, `^` or
/// spaces between two characters, two-word keys (`Page Up`) kept whole.
#[tokio::test]
async fn compound_keys() {
    let t = same(
        ":kbd:`Ctrl+Alt+Del` :kbd:`Ctrl+-` :kbd:`Shift Page Up` :kbd:`Caps Lock` :kbd:`-`\n",
        "{kbd}`Ctrl+Alt+Del` {kbd}`Ctrl+-` {kbd}`Shift Page Up` {kbd}`Caps Lock` {kbd}`-`\n",
        Value::Null,
    )
    .await;
    // Each key a `kbd`, separators text: what the keys and text are.
    let shape = |nodes: &Value| -> Vec<String> {
        nodes
            .as_array()
            .unwrap()
            .iter()
            .map(|n| match n["kind"].as_str() {
                Some("kbd") => format!("[{}]", plain(&n["children"])),
                _ => n["value"].as_str().unwrap().to_owned(),
            })
            .collect()
    };
    assert_eq!(
        shape(&t[0]["children"]),
        [
            "[Ctrl]",
            "+",
            "[Alt]",
            "+",
            "[Del]",
            " ",
            "[Ctrl]",
            "+",
            "[-]",
            " ",
            "[Shift]",
            " ",
            "[Page Up]",
            " ",
            "[Caps Lock]",
            " ",
            "[-]",
        ]
    );
}

#[tokio::test]
async fn inline_roles() {
    let t = same(
        ":kbd:`Enter` :abbr:`LIFO (last-in, first-out)` :guilabel:`&Cancel` \
         :menuselection:`File --> &Open` :samp:`print({x})` :file:`a.txt` \
         :command:`ls` :dfn:`term` :regexp:`a+`\n",
        "{kbd}`Enter` {abbr}`LIFO (last-in, first-out)` {guilabel}`&Cancel` \
         {menuselection}`File --> &Open` {samp}`print({x})` {file}`a.txt` \
         {command}`ls` {dfn}`term` {regexp}`a+`\n",
        Value::Null,
    )
    .await;
    let p = &t[0]["children"];
    assert_eq!(
        p[0],
        json!({"type": "custom", "kind": "kbd", "attrs": {"inline": true, "html_tag": "kbd"},
               "children": [{"type": "text", "value": "Enter"}]})
    );
    assert_eq!(p[2]["attrs"]["title"], "last-in, first-out");
    assert_eq!(p[2]["children"][0]["value"], "LIFO");
    assert_eq!(p[4]["children"][0]["kind"], "accelerator");
    assert_eq!(p[4]["children"][1]["value"], "ancel");
    assert_eq!(p[6]["children"][0]["value"], "File \u{2023} ");
    assert_eq!(p[8]["kind"], "samp");
    assert_eq!(p[8]["children"][1]["type"], "emphasis");
    assert_eq!(p[10], json!({"type": "inline_code", "value": "a.txt"}));
    assert_eq!(p[12]["type"], "strong");
    assert_eq!(p[14]["type"], "emphasis");
    assert_eq!(p[16], json!({"type": "inline_code", "value": "a+"}));
}

#[tokio::test]
async fn hlist_centered_and_extlinks() {
    let options =
        json!({"extlinks": {"issue": {"url": "https://x.test/issues/%s", "caption": "#%s"}}});
    let t = same(
        ".. hlist::\n   :columns: 2\n\n   * a\n   * b\n   * c\n\n\
         .. centered:: Big *news*\n\n\
         See :issue:`12` and :issue:`this one <13>`.\n",
        "```{hlist}\n:columns: 2\n\n* a\n\n* b\n\n* c\n```\n\n\
         ```{centered} Big *news*\n```\n\n\
         See {issue}`12` and {issue}`this one <13>`.\n",
        options,
    )
    .await;
    assert_eq!(t[0]["kind"], "hlist");
    let sizes: Vec<_> = t[0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["children"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [2, 1]);
    assert_eq!(t[1]["kind"], "centered");
    assert_eq!(t[1]["children"][0]["children"][0]["type"], "strong");
    let links = &t[2]["children"];
    assert_eq!(links[1]["url"], "https://x.test/issues/12");
    assert_eq!(links[1]["children"][0]["value"], "#12");
    assert_eq!(links[3]["children"][0]["value"], "this one");
}

#[tokio::test]
async fn toctrees_list_documents_and_sections() {
    let out = build(
        &[
            (
                "index.rst",
                "Home\n====\n\n.. toctree::\n   :caption: Contents\n   :maxdepth: 2\n\n   guide/intro\n   Elsewhere <https://x.test>\n\n\
                 .. toctree::\n   :glob:\n   :hidden:\n\n   ref/*\n",
            ),
            (
                "guide/intro.md",
                "# Intro {#intro}\n\n## Setup {#setup}\n\n### Deep {#deep}\n",
            ),
            ("ref/a.rst", "A\n=\n"),
            ("ref/b.md", "# B\n"),
        ],
        Value::Null,
    )
    .await;
    let (index, d) = &out[0];
    assert!(d.is_empty(), "{d:?}");
    let t = tree(index);
    let toctree = &t[0]["children"][1];
    assert_eq!(toctree["kind"], "toctree");
    assert_eq!(
        toctree["children"][0]["children"][0]["children"][0]["value"],
        "Contents"
    );
    let items = &toctree["children"][1]["children"];
    let intro = &items[0]["children"];
    assert_eq!(intro[0]["children"][0]["url"], "guide/intro.md");
    // Sections to depth 2: Setup, not Deep.
    let setup = &intro[1]["children"][0]["children"];
    assert_eq!(setup[0]["children"][0]["url"], "guide/intro.md#setup");
    assert_eq!(setup.as_array().unwrap().len(), 1);
    assert_eq!(
        items[1]["children"][0]["children"][0]["url"],
        "https://x.test"
    );
    // The hidden toctree shows nothing; glob found both.
    assert_eq!(t[0]["children"].as_array().unwrap().len(), 2);
    // A bad entry is a warning.
    let out = build(&[("index.rst", ".. toctree::\n\n   nope\n")], Value::Null).await;
    assert_eq!(
        messages(&out[0].1),
        ["toctree contains reference to nonexisting document \"nope\""]
    );
}

/// Documents are the files parsers took, whatever their extension: not
/// the files with a document's extension.
#[tokio::test]
async fn documents_are_the_parsed_files() {
    let out = build_site(
        &[
            (
                "index.rst",
                "Home\n====\n\nSee :doc:`notes` and :doc:`old`.\n\n\
                 .. toctree::\n   :glob:\n\n   notes\n   old\n   ref/*\n",
            ),
            ("notes.myst", "# Notes\n"),
            ("ref/a.myst", "# A\n"),
        ],
        &["old.md", "ref/b.rst"],
        Value::Null,
        None,
    )
    .await;
    let (index, d) = &out[0];
    assert_eq!(
        messages(d),
        [
            "unknown document: \"old\"",
            "toctree contains reference to nonexisting document \"old\"",
        ]
    );
    let t = tree(index);
    let para = &t[0]["children"][1]["children"];
    assert_eq!(para[1]["url"], "notes.myst");
    let items = &t[0]["children"][2]["children"][0]["children"];
    let urls: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["children"][0]["children"][0]["url"].as_str().unwrap())
        .collect();
    assert_eq!(urls, ["notes.myst", "ref/a.myst"]);
}

#[test]
fn line_specs_and_globs() {
    use crate::directives::{glob, line_spec};
    assert_eq!(line_spec("1,3-4,6-", 7).unwrap(), [1, 3, 4, 6, 7]);
    assert_eq!(line_spec("-2", 5).unwrap(), [1, 2]);
    assert!(line_spec("x", 5).is_err());
    assert!(glob("ref/*", "ref/a"));
    assert!(!glob("ref/*", "ref/sub/a"));
    assert!(glob("ref/**", "ref/sub/a"));
    assert!(glob("r?f/a", "ref/a"));
}

#[test]
fn invalid_options_are_an_error() {
    let err = Options::read(&json!({"nope": 1})).unwrap_err();
    assert!(err.message().contains("nope"), "{err}");
}

#[tokio::test]
async fn index_entries_make_an_index_page() {
    let out = build(
        &[
            (
                "a.rst",
                ".. index::\n   single: loop; for\n   pair: list; tuple\n\n\
                 .. index::\n   !Python\n   see: snake; Python\n\n\
                 Some :index:`banana` and :index:`text <apple, pear>`.\n\n\
                 See :ref:`genindex`.\n",
            ),
            ("b.md", "{index}`Apple`\n"),
        ],
        Value::Null,
    )
    .await;
    assert!(out[0].1.is_empty(), "{:?}", out[0].1);
    let a = tree(&out[0].0);
    assert_eq!(a[0], json!({"type": "anchor", "id": "index-0"}));
    let para = &a[2]["children"];
    assert_eq!(para[1], json!({"type": "anchor", "id": "index-2"}));
    assert_eq!(para[2]["value"], "banana");
    assert_eq!(para[5]["value"], "text");
    let link = &a[3]["children"][1];
    assert_eq!(link["target"], json!({"document": "genindex.md"}));
    assert_eq!(link["children"][0]["value"], "Index");
    // The generated page comes last.
    let (index, d) = &out[2];
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(index.source, "genindex.md");
    assert_eq!(index.metadata["nav_exclude"], json!(true));
    let mut lines = Vec::new();
    fn walk(nodes: &[Node], depth: usize, out: &mut Vec<String>) {
        for node in nodes {
            match node {
                Node::Heading(h) => out.push(format!("# {}", plain_text(&h.children))),
                Node::ListItem(item) => {
                    let line = item
                        .children
                        .first()
                        .map(|p| {
                            p.children()
                                .iter()
                                .map(|n| match n {
                                    Node::Link(l) => {
                                        format!("[{}]({})", plain_text(&l.children), l.url)
                                    }
                                    other => plain_text(std::slice::from_ref(other)),
                                })
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    out.push(format!("{}{line}", "  ".repeat(depth)));
                    walk(&item.children[1..], depth + 1, out);
                    continue;
                }
                Node::List(l) => walk(&l.children, depth, out),
                _ => walk(node.children(), depth, out),
            }
        }
    }
    walk(&index.children, 0, &mut lines);
    assert_eq!(
        lines,
        [
            "# Index",
            "# A",
            "[Apple](a.rst#index-3), [[2]](b.md#index-0)",
            "# B",
            "[banana](a.rst#index-2)",
            "# L",
            "list",
            "  [tuple](a.rst#index-0)",
            "loop",
            "  [for](a.rst#index-0)",
            "# P",
            "[pear](a.rst#index-3)",
            "[Python](a.rst#index-1)",
            "# S",
            "snake",
            "  see Python",
            "# T",
            "tuple",
            "  [list](a.rst#index-0)",
        ],
        "{lines:#?}"
    );
}

#[tokio::test]
async fn rubrics() {
    let t = same(
        ".. rubric:: Notes *here*\n   :heading-level: 3\n\n.. rubric:: Plain\n",
        "```{rubric} Notes *here*\n:heading-level: 3\n```\n\n```{rubric} Plain\n```\n",
        Value::Null,
    )
    .await;
    assert_eq!(t[0]["kind"], "rubric");
    assert_eq!(t[0]["attrs"], json!({"html_tag": "h3"}));
    assert_eq!(t[0]["children"][1]["type"], "emphasis");
    assert_eq!(t[1]["attrs"], json!({"html_tag": "p"}));
}

#[tokio::test]
async fn numbered_toctrees_and_navigation() {
    let out = build(
        &[
            (
                "index.rst",
                "Home\n====\n\n.. toctree::\n   :numbered:\n   :hidden:\n\n   one\n   two\n",
            ),
            (
                "one.md",
                "# One {#one}\n\n## Part {#part}\n\nSee {numref}`part` and {numref}`Sec. %s <two>`.\n\n\
                 ```{toctree}\nsub\n```\n",
            ),
            ("two.md", "# Two {#two}\n"),
            ("sub.md", "# Sub {#sub}\n"),
        ],
        json!({"numfig": true}),
    )
    .await;
    let (index, _) = &out[0];
    assert_eq!(
        index.metadata["nav_children"],
        json!(["/one.md", "/two.md"])
    );
    // Hidden: nothing shown.
    assert_eq!(tree(index)[0]["children"].as_array().unwrap().len(), 1);
    let (one, d) = &out[1];
    assert!(d.is_empty(), "{d:?}");
    let headings: Vec<String> = one
        .descendants()
        .filter_map(|n| match n {
            Node::Heading(h) => Some(plain_text(&h.children)),
            _ => None,
        })
        .collect();
    assert_eq!(headings, ["1. One", "1.1. Part"]);
    let links: Vec<String> = one
        .descendants()
        .filter_map(|n| match n {
            Node::Link(l) => Some(format!("{} -> {}", plain_text(&l.children), l.url)),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        [
            "Section 1.1 -> #part",
            "Sec. 2 -> two.md#two",
            "1.2. Sub -> sub.md"
        ]
    );
    assert_eq!(one.metadata["nav_children"], json!(["/sub.md"]));
    let (sub, _) = &out[3];
    assert_eq!(plain_text(&sub.children), "1.2. Sub");
}

/// The ids a tree's anchors have, in order.
fn anchor_ids(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match value {
        Value::Object(map) => {
            if map.get("type") == Some(&json!("anchor")) {
                out.push(map["id"].as_str().unwrap().to_owned());
            }
            for v in map.values() {
                out.extend(anchor_ids(v));
            }
        }
        Value::Array(items) => {
            for v in items {
                out.extend(anchor_ids(v));
            }
        }
        _ => {}
    }
    out
}

/// The text of a tree.
fn plain(value: &Value) -> String {
    match value {
        Value::Object(map) => match map.get("value") {
            Some(Value::String(s)) => s.clone(),
            _ => map.get("children").map(plain).unwrap_or_default(),
        },
        Value::Array(items) => items.iter().map(plain).collect(),
        _ => String::new(),
    }
}

/// The links of a tree (`url`), in order.
fn urls(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match value {
        Value::Object(map) => {
            if map.get("type") == Some(&json!("link")) {
                out.push(map["url"].as_str().unwrap().to_owned());
            }
            for v in map.values() {
                out.extend(urls(v));
            }
        }
        Value::Array(items) => {
            for v in items {
                out.extend(urls(v));
            }
        }
        _ => {}
    }
    out
}

#[tokio::test]
async fn std_objects() {
    let t = same(
        ".. program:: git commit\n\n\
         .. option:: -m MSG, --message=MSG\n\n   Commit.\n\n\
         .. std:envvar:: GIT_DIR\n\n\
         .. confval:: core.editor\n   :type: string\n   :default: vi\n\n   The editor.\n\n\
         .. describe:: anything at all\n\n\
         .. productionlist:: git\n   rev: `name` | `name` \"~\" DIGIT+\n      : | `~other:x`\n   n: DIGIT+\n",
        "```{program} git commit\n```\n\n\
         ```{option} -m MSG, --message=MSG\n\nCommit.\n```\n\n\
         ```{std:envvar} GIT_DIR\n```\n\n\
         ```{confval} core.editor\n:type: string\n:default: vi\n\nThe editor.\n```\n\n\
         ```{describe} anything at all\n```\n\n\
         ```{productionlist} git\nrev: `name` | `name` \"~\" DIGIT+\n   : | `~other:x`\nn: DIGIT+\n```\n",
        Value::Null,
    )
    .await;
    assert_eq!(
        anchor_ids(&t),
        [
            "cmdoption-git-commit-m",
            "cmdoption-git-commit-message",
            "envvar-GIT_DIR",
            "confval-core.editor",
            "grammar-token-git-rev",
            "grammar-token-git-n",
        ]
    );
    let option = &t[0];
    assert_eq!(option["kind"], "desc");
    assert_eq!(
        option["attrs"],
        json!({"domain": "std", "objtype": "option"})
    );
    // The signature, then the description.
    let list = &option["children"][0]["children"];
    assert_eq!(list.as_array().unwrap().len(), 2);
    let signature = &list[0]["children"];
    assert_eq!(signature[2]["children"][0]["value"], "-m");
    assert_eq!(signature[3]["value"], " MSG");
    assert_eq!(signature[5]["children"][0]["value"], "--message");
    assert_eq!(signature[6]["value"], "=MSG");
    // confval: its type and default before its content.
    let confval = &t[2]["children"][0]["children"][1]["children"];
    assert_eq!(plain(&confval[0]), "Type: string");
    assert_eq!(plain(&confval[1]), "Default: vi");
    assert_eq!(plain(&confval[2]), "The editor.");
    // describe: no target, no domain.
    assert_eq!(t[3]["attrs"], json!({"objtype": "describe"}));
    // productionlist: drawn like a code block, `pre` around one `code`
    // holding the productions, a newline apart, the names aligned; a token
    // no production names is its text (linked ones: `std_references`).
    let list = &t[4];
    assert_eq!(list["attrs"], json!({"group": "git", "html_tag": "pre"}));
    let code = &list["children"][0];
    assert_eq!(code["kind"], "productions");
    assert_eq!(code["attrs"], json!({"html_tag": "code", "inline": true}));
    let production = &code["children"];
    assert_eq!(
        production[1],
        json!({"type": "strong", "children": [{"type": "text", "value": "rev"}]})
    );
    assert_eq!(production[2]["value"], " ::= ");
    assert_eq!(production[3], json!({"type": "text", "value": "name"}));
    let continued = production
        .as_array()
        .unwrap()
        .iter()
        .position(|n| n["value"] == "\n")
        .unwrap();
    assert_eq!(production[continued + 1]["value"], "        ");
    assert_eq!(
        plain(code),
        "rev ::= name | name \"~\" DIGIT+\n        | x\nn   ::= DIGIT+"
    );
}

#[tokio::test]
async fn std_references() {
    let out = build(
        &[
            (
                "cli.rst",
                ".. _commands:\n\nCommands\n========\n\n\
                 .. option:: --version\n            -V\n\n\
                 .. program:: git commit\n\n\
                 .. option:: -m MSG\n\n\
                 .. program:: None\n\n\
                 .. option:: -v\n\n\
                 .. envvar:: GIT_DIR\n\n\
                 .. confval:: core.editor\n\n\
                 .. productionlist:: git\n   rev: `name`\n   name: WORD\n",
            ),
            (
                "use.md",
                "# Use\n\n\
                 {option}`--version`, {option}`-V`, {option}`git commit -m`,\n\
                 {option}`-v`, {std:option}`-v`, {envvar}`GIT_DIR`, {confval}`core.editor`,\n\
                 {token}`~git:rev`, {token}`git:name`, {keyword}`commands`.\n\n\
                 ```{program} git commit\n```\n\n\
                 {option}`-m=hi`\n\n\
                 {option}`-x`, {keyword}`nope`, {confval}`nope`, {envvar}`NOPE`, {token}`nope`\n",
            ),
        ],
        Value::Null,
    )
    .await;
    let (use_md, d) = &out[1];
    assert_eq!(
        messages(d),
        [
            "unknown option: \"-x\"",
            "unknown keyword: \"nope\"",
            "'confval' reference target not found: nope",
        ]
    );
    let t = tree(use_md);
    assert_eq!(
        urls(&t),
        [
            "cli.rst#cmdoption-version",
            "cli.rst#cmdoption-V",
            "cli.rst#cmdoption-git-commit-m",
            "cli.rst#cmdoption-v",
            "cli.rst#cmdoption-v",
            "cli.rst#envvar-GIT_DIR",
            "cli.rst#confval-core.editor",
            "cli.rst#grammar-token-git-rev",
            "cli.rst#grammar-token-git-name",
            "cli.rst#commands",
            // After `program`, `-m` is git commit's, with or without a value.
            "cli.rst#cmdoption-git-commit-m",
        ]
    );
    let para = &t[0]["children"][1]["children"];
    // References show code; `~group:name` shows the name.
    assert_eq!(para[0]["children"][0]["type"], "inline_code");
    let shown: Vec<String> = para
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["type"] == "link")
        .map(plain)
        .collect();
    assert_eq!(shown[7], "rev");
    assert_eq!(shown[8], "git:name");
    // `envvar` is an index entry where it is too.
    assert!(anchor_ids(para).contains(&"index-0".to_owned()));
    // A signature per line in reStructuredText, each with its targets
    // (`-V` is the second); tokens link within their group.
    let cli = tree(&out[0].0);
    let version = &cli[0]["children"][1]["children"][0]["children"];
    assert_eq!(version[1]["children"][0]["id"], "cmdoption-V");
    assert!(urls(&cli).contains(&"#grammar-token-git-name".to_owned()));
}

#[tokio::test]
async fn std_objects_without_targets() {
    let out = build(
        &[(
            "a.rst",
            ".. option:: -a\n   :no-index:\n\n\
             .. option:: -b\n   :no-index-entry:\n\n\
             .. envvar:: HIDDEN\n   :no-typesetting:\n\n\
             .. option:: =bad\n\n\
             :option:`-a` :option:`-b`\n",
        )],
        Value::Null,
    )
    .await;
    let (a, d) = &out[0];
    assert_eq!(
        messages(d),
        [
            "Malformed option description \"=bad\", should look like \"opt\", \"-opt args\", \
             \"--opt args\", \"/opt args\" or \"+opt args\"",
            "unknown option: \"-a\"",
        ]
    );
    let t = tree(a);
    // No index: no target; no typesetting: the target alone.
    assert_eq!(anchor_ids(&t), ["cmdoption-b", "envvar-HIDDEN"]);
    assert_eq!(t[2]["type"], "anchor");
    // A malformed signature is shown as written.
    assert_eq!(plain(&t[3]), "=bad");
    // The index has HIDDEN (no typesetting keeps the entry), not -b.
    let index = plain(&tree(&out[1].0));
    assert!(index.contains("HIDDEN"), "{index}");
    assert!(!index.contains("-b"), "{index}");
}

#[tokio::test]
async fn confval_options_are_inline_markup() {
    let t = same(
        ".. confval:: jobs\n   :type: ``int``\n   :default: *all* cores\n",
        "```{confval} jobs\n:type: `int`\n:default: *all* cores\n```\n",
        Value::Null,
    )
    .await;
    let fields = &t[0]["children"][0]["children"][1]["children"];
    assert_eq!(
        fields[0]["children"][2],
        json!({"type": "inline_code", "value": "int"})
    );
    assert_eq!(fields[1]["children"][2]["type"], "emphasis");
    assert_eq!(plain(&fields[1]), "Default: all cores");
}

#[tokio::test]
async fn option_placeholders() {
    let rst = ".. option:: --output={FILE}, -j[={N}]\n";
    let off = build(&[("a.rst", rst)], Value::Null).await;
    let term = &tree(&off[0].0)[0]["children"][0]["children"][0]["children"];
    assert_eq!(term[3], json!({"type": "inline_code", "value": "={FILE}"}));
    let on = build(
        &[("a.rst", rst)],
        json!({"option_emphasise_placeholders": true}),
    )
    .await;
    let term = &tree(&on[0].0)[0]["children"][0]["children"][0]["children"];
    let args = &term[3];
    assert_eq!(args["kind"], "samp");
    assert_eq!(args["children"][0]["value"], "=");
    assert_eq!(args["children"][1]["type"], "emphasis");
    assert_eq!(plain(&args["children"][1]), "FILE");
    assert_eq!(plain(&term[6]), "[=N]");
}

/// Ids are unique in a document, as Sphinx makes them: a taken one is
/// `<prefix>-<n>`.
#[tokio::test]
async fn std_ids_are_unique() {
    let out = build(
        &[(
            "a.rst",
            ".. _envvar-home:\n\nHome\n====\n\n\
             .. envvar:: home\n\n.. envvar:: home\n\n\
             .. option:: -v\n\n.. option:: -v\n",
        )],
        Value::Null,
    )
    .await;
    let ids = anchor_ids(&tree(&out[0].0));
    assert_eq!(
        ids.iter()
            .filter(|i| i.starts_with("envvar") || i.starts_with("cmdoption"))
            .collect::<Vec<_>>(),
        ["envvar-0", "envvar-1", "cmdoption-v", "cmdoption-0"]
    );
}

/// A glossary entry may have several terms (as Sphinx reads glossaries:
/// lines not indented are terms, the indented ones below them the
/// definition), in both syntaxes; `term : classifier` hides the
/// classifier; terms are inline markup, definitions blocks.
#[tokio::test]
async fn glossary_entries_with_several_terms() {
    let t = same(
        ".. glossary::\n\n   environment\n   ``env``\n      Where *everything* is.\n\n      Kept.\n\n\
         \x20  source : files\n      Read from it.\n",
        "```{glossary}\nenvironment\n`env`\n   Where *everything* is.\n\n   Kept.\n\n\
         source : files\n   Read from it.\n```\n",
        Value::Null,
    )
    .await;
    let list = &t[0]["children"][0]["children"];
    let kinds: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "definition_term",
            "definition_term",
            "definition_description",
            "definition_term",
            "definition_description",
        ]
    );
    assert_eq!(
        anchor_ids(&t[0]),
        ["term-environment", "term-env", "term-source"]
    );
    // The term as inline markup, the classifier not shown.
    assert_eq!(
        list[1]["children"][1],
        json!({"type": "inline_code", "value": "env"})
    );
    assert_eq!(plain(&list[3]), "source");
    // The definition as blocks, its roles resolved.
    let definition = &list[2]["children"];
    assert_eq!(definition.as_array().unwrap().len(), 2);
    assert_eq!(definition[0]["children"][1]["type"], "emphasis");
    // Roles in a definition are resolved.
    let out = build(
        &[(
            "a.rst",
            ".. glossary::\n\n   env\n      Itself: :term:`env`.\n",
        )],
        Value::Null,
    )
    .await;
    assert_eq!(urls(&tree(&out[0].0)), ["#term-env"]);
}

#[tokio::test]
async fn glossary_format_warnings() {
    let out = build(
        &[(
            "a.rst",
            ".. glossary::\n\n   one\n\n   two\n      Both.\n   three\n      Too close.\n",
        )],
        Value::Null,
    )
    .await;
    assert_eq!(
        messages(&out[0].1),
        [
            "glossary terms must not be separated by empty lines",
            "glossary term must be preceded by empty line",
        ]
    );
    // The terms of an entry stay one entry.
    let list = &tree(&out[0].0)[0]["children"][0]["children"];
    assert_eq!(list.as_array().unwrap().len(), 5);
}

/// Glossary terms are main index entries; what the host parsed for a
/// directive has its span (not spans within the piece).
#[tokio::test]
async fn glossary_index_and_spans() {
    let out = build(
        &[(
            "a.rst",
            "Intro.\n\n.. glossary::\n\n   env\n      Where *it* is.\n",
        )],
        Value::Null,
    )
    .await;
    let doc = serde_json::to_value(&out[0].0.children).unwrap();
    let glossary = &doc[1];
    assert_eq!(glossary["kind"], "glossary");
    let span = glossary["span"].clone();
    assert!(span.is_object(), "{glossary}");
    let definition = &glossary["children"][0]["children"][1]["children"][0];
    assert_eq!(definition["type"], "paragraph");
    assert_eq!(definition["span"], span);
    assert_eq!(definition["children"][1]["span"], span);
    // The index: the term, as a main entry (in `strong`).
    let index = tree(&out[1].0);
    let entry = &index[0]["children"][1]["children"][1]["children"][0]["children"][0];
    assert_eq!(plain(entry), "env");
    assert_eq!(entry["children"][0]["type"], "link");
    assert_eq!(entry["children"][0]["children"][0]["type"], "strong");
}
