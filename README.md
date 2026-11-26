# lsdocs-sphinx-addons

Part of [lsdocs](https://github.com/lsdocs), a static-site generator whose
parsers, transformers and renderers are plugins.

`crates/sphinx-addons` (`lsdocs-sphinx-addons`, builtin `sphinx-addons`):
the directives and roles Sphinx adds to docutils (code blocks and
`literalinclude`, version notes, `seealso`, `todo`, `only`, `glossary`,
`toctree`, `math` labels, the index, the cross-reference roles: `ref`,
`doc`, `numref`, `term`, ..., and the rest of the standard domain:
`program`, `option`, `envvar`, `confval`, `productionlist`, `describe`
and their roles), in reStructuredText and in Markdown (MyST's syntax)
alike, declared as syntax extensions. Its tests parse with
lsdocs-parser-rst and lsdocs-parser-markdown (its MyST dialect).

Its settings and behavior are described in the lsdocs documentation
(https://lsdocs.github.io, "sphinx-addons").

## Known limitations

- `doc` and `toctree` know the documents the plugin collected from: the
  files parsers took, whatever their extension. A document the plugin's
  `files` patterns leave out is not one of them, so it cannot be named.
- Not supported: `literalinclude`'s `:pyobject:` and `:diff:`, encodings
  other than UTF-8; `conf.py` settings that change parsing
  (`default_role`, `rst_prolog` / `rst_epilog`, the `|version|` and
  `|release|` substitutions).
- Standard domain: references to an object described twice go to the
  first description (Sphinx takes the last, and warns); ids drop
  characters outside ASCII (Sphinx first decomposes them, so `e` with an
  accent is `e`). A `productionlist` is drawn like a code block (`pre`
  and `code`, through `html_tag`) where the renderer takes a block
  `html_tag` of `pre` (render-html does); elsewhere it is a container
  whose spaces may not be kept.
- `confval`'s `:type:` and `:default:`, and a glossary's terms and
  definitions, are parsed by the site's parser for the document's
  extension (`host/parse`); a document no parser answers for keeps them
  as text. What it parses has the directive's span, not its own.
- A glossary term's classifier (`term : key`) is not used as its index
  key (Sphinx groups the entry under it): the index has no keys.

## TODO

- [ ] `literalinclude`'s `:pyobject:` and `:diff:`; other encodings.
- [ ] Index keys (a glossary term's classifier) in the generated index.
- [ ] The parsing settings of `conf.py` (`default_role`, `rst_prolog` /
      `rst_epilog`, `|version|` / `|release|`).
