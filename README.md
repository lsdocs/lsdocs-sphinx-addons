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
  other than UTF-8; compound keys in `kbd` (`Ctrl+C` is one key); several
  terms for one glossary entry (in reStructuredText the content is a
  definition list); `conf.py` settings that change parsing
  (`default_role`, `rst_prolog` / `rst_epilog`, the `|version|` and
  `|release|` substitutions).
- Standard domain: `confval`'s `:type:` and `:default:` are shown as
  text, not parsed as inline markup; `option_emphasise_placeholders` is
  not supported; an object described twice gets the same id twice
  (Sphinx numbers the second), and references go to the first; ids drop
  characters outside ASCII (Sphinx first decomposes them, so `e` with an
  accent is `e`). A `productionlist`'s names line up only where the
  renderer keeps its spaces (Sphinx draws it preformatted).

## TODO

- [ ] `literalinclude`'s `:pyobject:` and `:diff:`; other encodings.
- [ ] Compound `kbd` keys; several terms per glossary entry.
- [ ] Standard domain: inline markup in `confval`'s `:type:` and
      `:default:`; `option_emphasise_placeholders`; unique ids for
      objects described twice.
- [ ] The parsing settings of `conf.py` (`default_role`, `rst_prolog` /
      `rst_epilog`, `|version|` / `|release|`).
