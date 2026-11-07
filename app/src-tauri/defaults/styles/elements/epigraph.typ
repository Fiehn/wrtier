// Story element: Epigraph
// Written as: #epigraph[A quotation. \ — Its source]
// Shown as: block
// .docx: indented block
// A quotation set before a Chapter or Part.

#let epigraph(body) = [#block(width: 100%, inset: (left: 35%), above: 2em, below: 2.5em, text(size: 0.9em, style: "italic", body)) <epigraph>]
