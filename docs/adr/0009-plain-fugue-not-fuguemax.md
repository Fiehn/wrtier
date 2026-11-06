# Plain Fugue sibling order, not FugueMax

Refines [ADR 0008](0008-moves-are-ranges-over-a-fugue-position-tree.md). The position tree uses plain Fugue: siblings are kept in Change ID order. FugueMax (Weidner & Kleppmann) only changes how a node's right-hand children are ordered, using their right origin, so that text typed concurrently at the same spot never interleaves, even in the rare case where Fugue alone can. Interleaving needs two Devices typing at the same spot while offline, which for one Author writing on their own Devices is rare, and plain Fugue already keeps most such text apart. FugueMax would add complexity to the most central code for a case we expect almost never to hit.

## Considered Options

- **FugueMax from the start.** Avoids every interleaving case, at the cost of a harder sibling order to get right and test.

## Consequences

- Every insertion still stores its right origin, so FugueMax can be added later without changing what is stored.
- Switching later is still a format change: it redraws existing text wherever concurrent insertions share a spot, so every Device has to switch at the same time, gated on the History's version.
- In the rare backwards-typing case, two Devices' concurrent text at one spot can interleave character by character. Nothing is lost, and the Author can fix it by hand.
