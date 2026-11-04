# Character-level history with moves, built in-house

Every character in a Work records the Change that wrote it, and moving text is its own kind of Change, so history and comments follow text wherever it goes. That covers splitting and joining paragraphs and moving a sentence, a paragraph or a whole chapter. Tracking moved text is the author's core need, and line-based tools like git can't do it. The merge logic is custom: it's designed as a graph (pieces of text linked to what they follow) and stored as plain rows in SQLite. Changes are recorded at fine detail and grouped into Sessions and Milestones only for display.

## Considered Options

- **Line-based history (git-style).** A paragraph is one line in prose, so this marks whole paragraphs as changed and loses history on every move.
- **Paragraph ownership.** Breaks when paragraphs are split or joined, or when a sentence moves between paragraphs.
- **A merge-safe text library (Yjs, Automerge, Loro).** None handles moving ranges of text well, and all store data in their own binary formats, which conflicts with ADR 0002.
- **A graph database.** Doesn't solve the merge rules, and isn't a file an ordinary person can open.
