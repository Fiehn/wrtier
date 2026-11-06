# Moves are ranges over a FugueMax position tree

Follows on from [ADR 0003](0003-character-level-history-with-moves.md). A character's identity is kept apart from its place in the text. Each character's ID is the ID of the Change that wrote it plus its offset within that Change, and it never changes. Places in the text are nodes in a FugueMax tree (Weidner & Kleppmann), which are created once, never move and are stored as runs. A Move doesn't delete and re-insert: it adds one empty anchor node at the destination for each piece it moves, and the characters it takes are drawn at that anchor instead of at their own node, following the idea of Kleppmann's *Moving Elements in List CRDTs*, so the characters keep their History and Comments. Unlike that paper, a piece covers a range (every node from its first to its last character in the tree's fixed order), so text another Device added inside the range concurrently goes with it. Where two Moves claim the same character, the later Move in the fixed Change order wins: a hybrid logical clock, then Device ID. We chose this because text that follows its Moves is the Author's core need, and a passage left behind in pieces is the worst outcome for prose.

## How a Move is resolved

- **Pieces.** A selection that is contiguous on screen may not be contiguous in the tree, for example when it covers text moved earlier plus text typed next to it. The Move then splits into pieces, each contiguous in the fixed order of one place (the main text, or one earlier Move's anchor), and their anchors are chained at the destination in reading order.
- **Who owns a node.** All Moves in effect are applied in the fixed Change order. A piece takes every node in its range, except nodes that an earlier Move had already taken and that the moving Device had seen (other than the exact piece it was selected from), and except nodes another piece of the same Move already took. Each Move records the highest Change counter it had seen from every Device for this. So text moved out of a passage earlier isn't pulled back when the passage is moved, while overlapping concurrent Moves still resolve to the later one.
- **Text typed into moved text.** An insertion records the piece it was typed into, and so does a Move for its anchors. Once Moves overlap, the place in the tree alone can't tell, because the same spot can lie inside several Moves' ranges. The new text belongs to that piece unless a later Move takes it. If that piece's Move is reverted, the record does nothing and the text falls back to the range rules.
- **Loops.** If applying a Move would put one of its anchors inside its own text (two Devices moving passages into each other), that Move is skipped: its text stays where it was, and the Author gets a Merge notice. Every Device skips the same Move because the order is fixed.
- **Typing at the edge of moved text.** Text typed right after the last character of a moved piece, at its new place, is placed after the piece's anchor, so it shows where it was typed rather than at the old place.

Everything is recomputed from the set of Changes, so the result doesn't depend on the order in which Changes arrive.

## Considered Options

- **Move as delete + insert.** It makes new characters, so History and Comments are lost, and two Devices moving the same paragraph offline leave two copies of it.
- **Moving single characters, as in the paper.** Text added inside a moved passage on another Device stays behind as a stray fragment.
- **One range per Move, whatever the selection.** A selection spanning moved text and text next to it would take everything between them in the tree, which can be most of the document.
- **No record of what the moving Device had seen.** Simpler, but either text moved out of a passage earlier jumps back when the passage is moved, or overlapping concurrent Moves resolve to the earlier one.
- **Fractional positions (Logoot/LSEQ) or RGA as the position layer.** Text typed concurrently at the same spot can interleave, and Logoot/LSEQ IDs keep growing.
- **Index-based operations replayed through a temporary CRDT (Eg-walker).** The easiest rows to read, but nobody has worked out Moves for it.
- **Storing word, sentence or paragraph boundaries as their own nodes.** They shift constantly while writing and would need their own merge rules. Paragraphs already come from newline characters, and each Move brings its own start and end.
- **Lamport counters for "later".** Every Device would agree, but the winner would depend on which Device had made more Changes instead of when you made them.

## Consequences

- Nodes are never removed. Old nodes are what let a Revert of a Move and a return to a Milestone put text back exactly, so the tree only grows. It grows by runs (one per Change, roughly), not by characters.
- Text added just *before* or *after* a moved range on another Device stays where it was. Sentence-aware edges are planned for v2.
- Overlapping Moves made offline on two Devices can split a passage. Nothing is lost or duplicated, and the Author gets a Merge notice.
- Text typed at the boundary between two pieces of a Move, at its new place, goes between their anchors, so it isn't part of either piece.
- Two Devices with the same Device ID would create clashing Change IDs, so the Device ID is kept out of Android backups and a merge that finds the same ID with different content refuses loudly.
