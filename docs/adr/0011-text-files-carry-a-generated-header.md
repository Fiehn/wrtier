# Text files carry a generated header with the Work's recorded Styles

Every Text file the app writes starts with a generated header, between two marker comments, holding the Work's recorded copies of its Story element functions, its extra Styles, each of its Looks as one wrapping function (`#let paperback(doc) = { set …; show …; doc }`), its settings, and a final `#show:` line that picks the Work's first Look. The Work's text follows. That makes the file compile on its own, anywhere, with exactly the pages the app shows, which a Text file with only the text (it fails at the first `#scene-break()`) or one that imports from `styles/` (today's library, not the recorded one, and only from inside the Vault) can't do.

Like the version line from [ADR 0010](0010-text-files-name-the-version-they-were-written-from.md), the header exists only in the file on disk: it's added when the file is written and removed when it's read, so it's never part of the Work's text, and the editor never shows it. Edits made inside it in another editor are dropped on the next write, and the Author is told; the Preamble is only edited in the app's Preamble dialog.

## Consequences

- The file's Look is the Work's first Look, never the one a Device last picked in the Page preview, or every Device would write a different file and keep making Conflict copies. Reordering the Looks changes it.
- The header is built from the Work's recorded Styles, not the Style library, so editing a Style doesn't rewrite the Text file of a Work until that Work records the change.
