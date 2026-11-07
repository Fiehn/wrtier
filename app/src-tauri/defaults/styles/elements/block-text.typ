// Story element: Block text
// Written as: #block-text[Dear Anna, …]
// Shown as: block
// .docx: indented block
// A letter, a note or another document inside the story, set in from both margins.

#let block-text(body) = [#block(width: 100%, inset: (x: 2em), above: 1.4em, below: 1.4em, body) <block-text>]
