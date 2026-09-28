# Notebooks

NZAP Engine has two kinds of notebooks. Both run on your Colab runtime, never
on your computer.

| Kind         | Where it lives                                                                         | Editable                |
| ------------ | -------------------------------------------------------------------------------------- | ----------------------- |
| **Public**   | [`nzap-labs/nzap-notebooks`](https://github.com/nzap-labs/nzap-notebooks) on GitHub      | Fork it to a local copy |
| **Your own** | `notebooks/` in the app's data directory, one JSON file each                           | Yes                     |

## Parameters

A notebook declares parameters. When you run it, the app validates the values
you entered against that schema and puts them ahead of the source as a Python
dict:

```python
import json as _nzap_json
params = _nzap_json.loads('{"string_to_print": "Hello"}')
del _nzap_json
```

Your code reads `params["key"]`. Values arrive as real Python types (`True`,
`42`, `3.5`, strings), because they go through JSON.

| `type`    | Input in the app     | Python value      |
| --------- | -------------------- | ----------------- |
| `string`  | one-line text        | `str`             |
| `text`    | multi-line text      | `str`             |
| `integer` | number, whole        | `int`             |
| `number`  | number               | `float` / `int`   |
| `boolean` | checkbox             | `bool`            |
| `select`  | dropdown (`options`) | `str`             |

Each parameter has a `key` (a Python identifier), a `label`, and optionally
`default`, `required` and `description`.

## Writing your own

**Notebooks → New notebook** opens an editor with a title, a slug, the source
and a parameter list. **Fork** on a public notebook copies it into your own
notebooks so you can change it. **Export** writes a `.json` file you can share,
and **Import** reads one back.

## Contributing a public notebook

Public notebooks are pull requests to
[`nzap-labs/nzap-notebooks`](https://github.com/nzap-labs/nzap-notebooks):

1. Add `notebooks/<slug>/notebook.py` (the source) and
   `notebooks/<slug>/notebook.json` (title, description, tags, author, params).
2. Run `python scripts/build_index.py` to regenerate `index.json`.
3. Open a pull request. CI checks the schema and that `index.json` is up to
   date.

## How the app fetches the collection

- It downloads `index.json` from the catalog URL (Settings → Public notebook
  collection), revalidating with `ETag`, and caches it on disk.
- It downloads a notebook's source only when you open or run it, and refuses it
  unless its SHA-256 matches the index.
- Offline, or before the first download, it falls back to the last cached copy
  and then to a snapshot bundled into the app.

To curate your own collection, fork the repository and point the catalog URL at
`https://raw.githubusercontent.com/<you>/<fork>/main/`.
