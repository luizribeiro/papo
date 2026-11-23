#!/usr/bin/env python3
"""List explicitly doc-hidden items reachable through the facade's public rustdoc items."""

import json
import sys
from pathlib import Path


DOC_HIDDEN = {"other": "#[doc(hidden)]"}


class Rustdoc:
    def __init__(self, filename):
        data = json.loads(Path(filename).read_text())
        self.index = data["index"]
        self.root = str(data["root"])
        self.name = self.index[self.root]["name"]
        self.paths = {
            item_id: tuple(summary["path"])
            for item_id, summary in data["paths"].items()
        }
        self.local_ids = {
            path: item_id for item_id, path in self.paths.items() if item_id in self.index
        }


def fields(body):
    kind = body.get("kind")
    if not isinstance(kind, dict):
        return body.get("fields", [])
    layout = next(iter(kind.values()))
    if isinstance(layout, list):
        return [field for field in layout if field is not None]
    return layout.get("fields", [])


def public_hidden(facade, dependencies):
    hidden = set()

    def resolve(doc, item_id):
        item_id = str(item_id)
        path = doc.paths.get(item_id)
        if item_id in doc.index:
            return doc, doc.index[item_id], path
        if path and path[0] in dependencies:
            source = dependencies[path[0]]
            return source, source.index[source.local_ids[path]], path
        return doc, None, None

    def visit(doc, item_id, parent, ancestors, *, alias=None, default_public=False):
        doc, item, _ = resolve(doc, item_id)
        if item is None:
            return
        glob = alias == ""
        visibility = item["visibility"]
        if not (glob or visibility == "public" or default_public and visibility == "default"):
            return
        key = (doc.name, item["id"])
        if key in ancestors:
            return
        ancestors = (*ancestors, key)

        kind, body = next(iter(item["inner"].items()))
        if kind == "use":
            path = parent if body["is_glob"] else (*parent, body["name"])
        else:
            path = parent if glob else (*parent, alias or item["name"])
        if not glob and DOC_HIDDEN in item["attrs"]:
            hidden.add("::".join(path))

        if kind == "use" and body["id"] is not None:
            visit(
                doc, body["id"], parent, ancestors,
                alias="" if body["is_glob"] else body["name"],
            )
        elif kind == "function":
            output = (body["sig"]["output"] or {}).get("resolved_path")
            if output:
                source, returned, return_path = resolve(doc, output["id"])
                # A caller can invoke methods on an inferred hidden return type.
                if returned and return_path and DOC_HIDDEN in returned["attrs"]:
                    visit(source, returned["id"], return_path[:-1], ancestors)
        elif kind in {"module", "trait", "enum", "struct", "union", "variant"}:
            for child in body.get("items", body.get("variants", fields(body))):
                visit(
                    doc, child, path, ancestors,
                    default_public=kind in {"trait", "enum", "variant"},
                )
            # Trait methods are visited on the trait itself.
            for impl_id in body.get("impls", []):
                implementation = doc.index[str(impl_id)]["inner"]["impl"]
                if implementation["trait"] is None:
                    for child in implementation["items"]:
                        visit(doc, child, path, ancestors)

    visit(facade, facade.root, (), ())
    return sorted(hidden)


if __name__ == "__main__":
    facade, *dependency_docs = map(Rustdoc, sys.argv[1:])
    for name in public_hidden(facade, {doc.name: doc for doc in dependency_docs}):
        print(name)
