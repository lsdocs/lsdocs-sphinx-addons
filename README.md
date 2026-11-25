# lsdocs-sphinx-addons

Part of [lsdocs](https://github.com/lsdocs), a static-site generator whose
parsers, transformers and renderers are plugins.

`crates/sphinx-addons` (`lsdocs-sphinx-addons`, builtin `sphinx-addons`):
the directives and roles Sphinx adds to docutils (code blocks and
`literalinclude`, version notes, `seealso`, `todo`, `only`, `glossary`,
`toctree`, `math` labels, the index, and the cross-reference roles: `ref`,
`doc`, `numref`, `term`, ...), in reStructuredText and in Markdown (MyST's
syntax) alike, declared as syntax extensions. Its tests parse with
lsdocs-parser-rst and lsdocs-parser-markdown (its MyST dialect).

Its settings and behavior are described in the lsdocs documentation
(https://lsdocs.github.io, "sphinx-addons").

## Known limitations

- Documents are found by extension (`.rst`, `.rest`, `.md`, `.markdown`)
  for `doc` and `toctree`, since a transformer does not know which files
  parsers took.
- Not supported: `literalinclude`'s `:pyobject:` and `:diff:`, encodings
  other than UTF-8; compound keys in `kbd` (`Ctrl+C` is one key); several
  terms for one glossary entry (in reStructuredText the content is a
  definition list); `conf.py` settings that change parsing
  (`default_role`, `rst_prolog` / `rst_epilog`, the `|version|` and
  `|release|` substitutions).

## TODO

- [ ] Know which files parsers took, instead of guessing by extension.
- [ ] `literalinclude`'s `:pyobject:` and `:diff:`; other encodings.
- [ ] Compound `kbd` keys; several terms per glossary entry.
- [ ] The parsing settings of `conf.py` (`default_role`, `rst_prolog` /
      `rst_epilog`, `|version|` / `|release|`).
