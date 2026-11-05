# Moves are ranges over a FugueMax position tree

Follows on from [ADR 0003](0003-character-level-history-with-moves.md). A character's identity is kept apart from its place in the text. Each character's ID is the ID of the Change that wrote it plus its offset within that Change, and it never changes. Places in the text are nodes in a FugueMax tree (Weidner & Kleppmann), which are created once, never move and are stored as runs. Each character points to its current node. A Move doesn't delete and re-insert: it adds empty nodes at the destination and points the moved characters at them, following Kleppmann's *Moving Elements in List CRDTs*, so the characters keep their History and Comments. Unlike that paper, a Move covers a range (every node strictly between its first and last character in the tree's fixed order), so text another Device added inside the range concurrently goes with it. Where two Moves claim the same character, the later Move in the fixed Change order wins: a hybrid logical clock, then Device ID. We chose this because text that follows its Moves is the Author's core need, and a passage left behind in pieces is the worst outcome for prose.

## Considered Options

- **Move as delete + insert.** It makes new characters, so History and Comments are lost, and two Devices moving the same paragraph offline leave two copies of it.
- **Moving single characters, as in the paper.** Text added inside a moved passage on another Device stays behind as a stray fragment.
- **Fractional positions (Logoot/LSEQ) or RGA as the position layer.** Text typed concurrently at the same spot can interleave, and Logoot/LSEQ IDs keep growing.
- **Index-based operations replayed through a temporary CRDT (Eg-walker).** The easiest rows to read, but nobody has worked out Moves for it.
- **Storing word, sentence or paragraph boundaries as their own nodes.** They shift constantly while writing and would need their own merge rules. Paragraphs already come from newline characters, and each Move brings its own start and end.
- **Lamport counters for "later".** Every Device would agree, but the winner would depend on which Device had made more Changes instead of when you made them.

## Consequences

- Nodes are never removed. Old nodes are what let a Revert of a Move and a return to a Milestone put text back exactly, so the tree only grows. It grows by runs (one per Change, roughly), not by characters.
- Text added just *before* or *after* a moved range on another Device stays where it was. Sentence-aware edges are planned for v2.
- Overlapping Moves made offline on two Devices can split a passage. Nothing is lost or duplicated, and the Author gets a Merge notice.
- Two Devices with the same Device ID would create clashing Change IDs, so the Device ID is kept out of Android backups and a merge that finds the same ID with different content refuses loudly.
