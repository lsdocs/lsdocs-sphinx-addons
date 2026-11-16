#![allow(clippy::unwrap_used, clippy::panic)]

use lsdocs_plugin_sdk::doctree::{Document, Node, plain_text};
use lsdocs_plugin_sdk::protocol::{Diagnostic, InitializeParams, SiteInfo};
use lsdocs_plugin_sdk::testing::run_transform;
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
    let syntax = syntax(&Options::read(&options).unwrap());
    let mut documents = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (source, text) in files {
        names.push((*source).to_owned());
        let (doc, diagnostics) = if source.ends_with(".rst") {
            lsdocs_parser_rst::parse(source, text, None, &syntax)
        } else if source.ends_with(".md") {
            lsdocs_parser_markdown::parse_with_syntax(source, text, &syntax)
        } else {
            continue;
        };
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        documents.push(doc);
    }
    names.sort();
    let init = InitializeParams {
        site: SiteInfo {
            files: names,
            source_dir: source_dir.map(str::to_owned),
            ..SiteInfo::default()
        },
        options,
        ..InitializeParams::default()
    };
    run_transform(&plugin(), &init, documents).await.unwrap()
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
        json!({"type": "anchor", "id": "term-apple"})
    );
    assert_eq!(list[2]["children"][0]["id"], "term-zebra");
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
            json!(["terms.rst#term-apple", {"document": "terms.rst", "anchor": "term-apple"}, "apple"]),
            json!(["terms.rst#term-zebra", {"document": "terms.rst", "anchor": "term-zebra"}, "the animal"]),
            json!(["terms.rst#fig-one", {"document": "terms.rst", "anchor": "fig-one"}, "Fig. 1"]),
            json!(["terms.rst#fig-one", {"document": "terms.rst", "anchor": "fig-one"}, "Figure 1: The caption."]),
            json!(["terms.rst#equation-euler", {"document": "terms.rst", "anchor": "equation-euler"}, "(1)"]),
            json!(["#fig-one", {"label": "fig-one"}, "The caption."]),
            json!(["#fig-one", {"label": "fig-one"}, "text"]),
            json!(["terms.rst", {"document": "terms.rst"}, ""]),
            json!(["terms.rst#term-apple", {"document": "terms.rst", "anchor": "term-apple"}, "apple"]),
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

#[tokio::test]
async fn inline_roles() {
    let t = same(
        ":kbd:`Ctrl+C` :abbr:`LIFO (last-in, first-out)` :guilabel:`&Cancel` \
         :menuselection:`File --> &Open` :samp:`print({x})` :file:`a.txt` \
         :command:`ls` :dfn:`term` :regexp:`a+`\n",
        "{kbd}`Ctrl+C` {abbr}`LIFO (last-in, first-out)` {guilabel}`&Cancel` \
         {menuselection}`File --> &Open` {samp}`print({x})` {file}`a.txt` \
         {command}`ls` {dfn}`term` {regexp}`a+`\n",
        Value::Null,
    )
    .await;
    let p = &t[0]["children"];
    assert_eq!(
        p[0],
        json!({"type": "custom", "kind": "kbd", "attrs": {"inline": true, "html_tag": "kbd"},
               "children": [{"type": "text", "value": "Ctrl+C"}]})
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
